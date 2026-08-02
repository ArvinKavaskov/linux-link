//! Creates the virtual monitor, one compositor at a time.
//!
//! There is no cross-desktop way to plug in a monitor that does not exist, so
//! this module is a small collection of dialects:
//!
//! * **GNOME (Wayland)** — Mutter's private ScreenCast/RemoteDesktop D-Bus
//!   API. `RecordVirtual` conjures a monitor whose size follows the video
//!   format we negotiate, and the RemoteDesktop session injects input in
//!   stream coordinates. The whole thing needs zero special permissions —
//!   it is the same door gnome-remote-desktop walks through.
//! * **Hyprland / Sway** — a headless output (`hyprctl output create` /
//!   `swaymsg create_output`), captured by wf-recorder.
//! * **KDE Plasma (Wayland)** — `krfb-virtualmonitor` makes the output exist
//!   (its VNC side is left unused, bound with a throwaway password), and the
//!   xdg-desktop-portal screencast — with a persisted restore token, so the
//!   picker dialog appears exactly once — provides the frames.
//! * **X11 (any desktop)** — the framebuffer is enlarged and a fake RandR
//!   monitor is declared over the new region; ffmpeg's x11grab reads it back.
//!   Ancient magic, works everywhere.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use zbus::zvariant::{self, OwnedObjectPath};

use super::encoder::CaptureSource;
use super::{has_cmd, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compositor {
    GnomeWayland,
    Hyprland,
    Sway,
    KdeWayland,
    X11,
}

pub fn detect() -> Result<Compositor> {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if session == "x11" {
        return Ok(Compositor::X11);
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Ok(Compositor::Hyprland);
    }
    if std::env::var_os("SWAYSOCK").is_some() {
        return Ok(Compositor::Sway);
    }
    if desktop.contains("gnome") || desktop.contains("zorin") {
        return Ok(Compositor::GnomeWayland);
    }
    if desktop.contains("kde") || desktop.contains("plasma") {
        return Ok(Compositor::KdeWayland);
    }
    if session != "wayland" && std::env::var_os("DISPLAY").is_some() {
        // No session type exported but X is reachable — good enough.
        return Ok(Compositor::X11);
    }
    anyhow::bail!(
        "unsupported compositor (XDG_CURRENT_DESKTOP={desktop:?}, XDG_SESSION_TYPE={session:?}) — \
         second screen currently supports GNOME, KDE Plasma, Hyprland, Sway and any X11 desktop"
    )
}

/// Everything a backend hands back: where to point the encoder, how input
/// gets in, and what must be undone at the end.
pub struct PreparedDisplay {
    pub source: CaptureSource,
    /// The tablet monitor's rectangle and the whole desktop's bounding box —
    /// only for uinput backends. Mutter does its own coordinate math.
    pub geometry: Option<(Rect, Rect)>,
    pub mutter: Option<MutterSession>,
    pub guards: Guards,
    pub width: u32,
    pub height: u32,
}

/// Teardown collected as data: child processes to kill and commands to run,
/// executed in `Drop` so a panic or a dropped QUIC stream still cleans up.
#[derive(Default)]
pub struct Guards {
    pub children: Vec<std::process::Child>,
    pub commands: Vec<Vec<String>>,
    /// Anything that must simply stay alive as long as the session does —
    /// e.g. the D-Bus connection owning a portal screencast session.
    pub keepalive: Vec<Box<dyn std::any::Any + Send>>,
}

impl Drop for Guards {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        for cmd in &self.commands {
            if let Some((prog, args)) = cmd.split_first() {
                let _ = std::process::Command::new(prog).args(args).status();
            }
        }
    }
}

pub async fn prepare(comp: Compositor, width: u32, height: u32, fps: u32) -> Result<PreparedDisplay> {
    match comp {
        Compositor::GnomeWayland => prepare_mutter(width, height).await,
        Compositor::Hyprland => prepare_hyprland(width, height, fps).await,
        Compositor::Sway => prepare_sway(width, height, fps).await,
        Compositor::KdeWayland => prepare_kde(width, height).await,
        Compositor::X11 => prepare_x11(width, height),
    }
}

// ---------------------------------------------------------------- GNOME ----

/// A live Mutter remote-desktop + screencast session pair. Kept for the whole
/// display session: it is both the source of frames and the input channel.
pub struct MutterSession {
    rd_session: zbus::Proxy<'static>,
    stream_path: String,
    /// Size we negotiated — Notify* input calls use these stream coordinates.
    pub width: f64,
    pub height: f64,
}

async fn prepare_mutter(width: u32, height: u32) -> Result<PreparedDisplay> {
    let conn = zbus::Connection::session().await.context("session bus")?;

    let rd = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.RemoteDesktop",
        "/org/gnome/Mutter/RemoteDesktop",
        "org.gnome.Mutter.RemoteDesktop",
    )
    .await?;
    let rd_path: OwnedObjectPath = rd
        .call("CreateSession", &())
        .await
        .context("Mutter RemoteDesktop.CreateSession — is this GNOME on Wayland?")?;
    let rd_session = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.RemoteDesktop",
        rd_path.clone(),
        "org.gnome.Mutter.RemoteDesktop.Session",
    )
    .await?;
    let session_id: String = rd_session.get_property("SessionId").await?;

    let sc = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.ScreenCast",
        "/org/gnome/Mutter/ScreenCast",
        "org.gnome.Mutter.ScreenCast",
    )
    .await?;
    let mut props: HashMap<&str, zvariant::Value> = HashMap::new();
    props.insert("remote-desktop-session-id", session_id.into());
    let sc_path: OwnedObjectPath = sc.call("CreateSession", &(props)).await?;
    let sc_session = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.ScreenCast",
        sc_path.clone(),
        "org.gnome.Mutter.ScreenCast.Session",
    )
    .await?;

    let mut sprops: HashMap<&str, zvariant::Value> = HashMap::new();
    sprops.insert("cursor-mode", zvariant::Value::U32(1)); // embedded in the frames
    let stream_path: OwnedObjectPath = sc_session
        .call("RecordVirtual", &(sprops))
        .await
        .context("Mutter RecordVirtual (GNOME too old? needs GNOME 42+)")?;
    let stream = zbus::Proxy::new(
        &conn,
        "org.gnome.Mutter.ScreenCast",
        stream_path.clone(),
        "org.gnome.Mutter.ScreenCast.Stream",
    )
    .await?;

    use futures_util::StreamExt;
    let mut added = stream.receive_signal("PipeWireStreamAdded").await?;
    rd_session.call::<_, _, ()>("Start", &()).await.context("starting the Mutter session")?;

    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), added.next())
        .await
        .context("timed out waiting for the PipeWire stream")?
        .context("Mutter closed the stream signal")?;
    let node_id: u32 = msg.body().deserialize()?;
    tracing::info!("Mutter virtual monitor up (PipeWire node {node_id})");

    Ok(PreparedDisplay {
        source: CaptureSource::PipeWire { node: node_id, negotiate: Some((width, height)) },
        geometry: None,
        mutter: Some(MutterSession {
            rd_session,
            stream_path: stream_path.to_string(),
            width: f64::from(width),
            height: f64::from(height),
        }),
        guards: Guards::default(),
        width,
        height,
    })
}

impl MutterSession {
    pub async fn stop(&self) {
        let _ = self.rd_session.call::<_, _, ()>("Stop", &()).await;
    }

    pub async fn handle(&self, ev: &super::RemoteInput) -> Result<()> {
        use super::RemoteInput as I;
        let s = &self.rd_session;
        match *ev {
            I::Move { x, y } => {
                s.call::<_, _, ()>(
                    "NotifyPointerMotionAbsolute",
                    &(self.stream_path.as_str(), x * self.width, y * self.height),
                )
                .await?;
            }
            I::Button { b, d } => {
                let code: i32 = match b {
                    0 => 0x110, // BTN_LEFT
                    1 => 0x111, // BTN_RIGHT
                    _ => 0x112, // BTN_MIDDLE
                };
                s.call::<_, _, ()>("NotifyPointerButton", &(code, d)).await?;
            }
            I::Scroll { dx, dy, end } => {
                let flags: u32 = u32::from(end); // 1 = FINISH
                s.call::<_, _, ()>("NotifyPointerAxis", &(dx, dy, flags)).await?;
            }
            I::Key { c, d } => {
                s.call::<_, _, ()>("NotifyKeyboardKeycode", &(u32::from(c), d)).await?;
            }
            I::Touch { id, ph, x, y } => match ph {
                0 => {
                    s.call::<_, _, ()>(
                        "NotifyTouchDown",
                        &(self.stream_path.as_str(), id, x * self.width, y * self.height),
                    )
                    .await?;
                }
                1 => {
                    s.call::<_, _, ()>(
                        "NotifyTouchMotion",
                        &(self.stream_path.as_str(), id, x * self.width, y * self.height),
                    )
                    .await?;
                }
                _ => {
                    s.call::<_, _, ()>("NotifyTouchUp", &(id,)).await?;
                }
            },
            I::Pen { x, y, d, prox, .. } if prox => {
                // Mutter's remote API has no tablet-tool path; a pen is a very
                // precise finger of a mouse.
                s.call::<_, _, ()>(
                    "NotifyPointerMotionAbsolute",
                    &(self.stream_path.as_str(), x * self.width, y * self.height),
                )
                .await?;
                s.call::<_, _, ()>("NotifyPointerButton", &(0x110i32, d)).await?;
            }
            // Pen out of range: nothing to do, the cursor simply stays put.
            I::Pen { .. } => {}
        }
        Ok(())
    }
}

// ------------------------------------------------------- Hyprland / Sway ----

const HYPR_NAME: &str = "linuxlink";

async fn prepare_hyprland(width: u32, height: u32, fps: u32) -> Result<PreparedDisplay> {
    run(&["hyprctl", "output", "create", "headless", HYPR_NAME]).await?;
    // Give it the tablet's mode; "auto" places it to the right of everything.
    run(&[
        "hyprctl",
        "keyword",
        "monitor",
        &format!("{HYPR_NAME},{width}x{height}@{fps},auto,1"),
    ])
    .await?;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let json = run(&["hyprctl", "-j", "monitors"]).await?;
    let geometry = wlr_geometry(&json, HYPR_NAME, "x", "y", "width", "height")
        .context("new Hyprland output not found")?;

    let mut guards = Guards::default();
    guards.commands.push(vec!["hyprctl".into(), "output".into(), "remove".into(), HYPR_NAME.into()]);
    Ok(PreparedDisplay {
        source: CaptureSource::WlrOutput { name: HYPR_NAME.into() },
        geometry: Some(geometry),
        mutter: None,
        guards,
        width,
        height,
    })
}

async fn prepare_sway(width: u32, height: u32, fps: u32) -> Result<PreparedDisplay> {
    let before = sway_output_names().await?;
    run(&["swaymsg", "create_output"]).await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let after = sway_output_names().await?;
    let name = after
        .into_iter()
        .find(|n| !before.contains(n))
        .context("sway did not create a HEADLESS output")?;

    run(&[
        "swaymsg",
        "output",
        &name,
        "mode",
        "--custom",
        &format!("{width}x{height}@{fps}Hz"),
    ])
    .await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let json = run(&["swaymsg", "-t", "get_outputs", "-r"]).await?;
    let geometry = sway_geometry(&json, &name).context("new sway output not found")?;

    let mut guards = Guards::default();
    guards.commands.push(vec!["swaymsg".into(), "output".into(), name.clone(), "unplug".into()]);
    Ok(PreparedDisplay {
        source: CaptureSource::WlrOutput { name },
        geometry: Some(geometry),
        mutter: None,
        guards,
        width,
        height,
    })
}

async fn sway_output_names() -> Result<Vec<String>> {
    let json = run(&["swaymsg", "-t", "get_outputs", "-r"]).await?;
    let v: Value = serde_json::from_str(&json)?;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| o.get("name").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

// ------------------------------------------------------------------ KDE ----

async fn prepare_kde(width: u32, height: u32) -> Result<PreparedDisplay> {
    if !has_cmd("krfb-virtualmonitor") {
        anyhow::bail!(
            "krfb-virtualmonitor not found — install the krfb package \
             (pacman -S krfb / apt install krfb); it is what creates the \
             virtual monitor on KDE Plasma"
        );
    }
    let before = kscreen_outputs().await.unwrap_or_default();

    // The VNC side of krfb-virtualmonitor is a passenger we never talk to:
    // random password, and the firewall guidance in the README already keeps
    // the port closed to the outside.
    let password = crate::pairing::new_token();
    let port = 51000 + (std::process::id() % 8000) as u16;
    let child = std::process::Command::new("krfb-virtualmonitor")
        .args([
            "--name",
            "Linux Link",
            "--resolution",
            &format!("{width}x{height}"),
            "--password",
            &password,
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning krfb-virtualmonitor")?;
    let mut guards = Guards::default();
    guards.children.push(child);

    // Wait for the new output to land in kscreen.
    let mut geometry = None;
    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if let Ok(now) = kscreen_outputs().await {
            if let Some(new) = now.iter().find(|(name, _)| !before.iter().any(|(b, _)| b == name)) {
                geometry = Some(new.1);
                break;
            }
        }
    }
    let monitor = geometry.context("the virtual monitor never appeared in kscreen")?;
    let desktop = kscreen_outputs()
        .await
        .map(|outs| union_rects(outs.iter().map(|(_, r)| *r)))
        .unwrap_or(monitor);

    // Frames come from the portal; the restore token makes the picker a
    // one-time event per PC.
    let (node, token, bus) = super::portal::open_screencast(load_restore_token()).await?;
    if let Some(t) = token {
        save_restore_token(&t);
    }
    guards.keepalive.push(Box::new(bus));

    Ok(PreparedDisplay {
        source: CaptureSource::PipeWire { node, negotiate: None },
        geometry: Some((monitor, desktop)),
        mutter: None,
        guards,
        width,
        height,
    })
}

async fn kscreen_outputs() -> Result<Vec<(String, Rect)>> {
    let json = run(&["kscreen-doctor", "-j"]).await?;
    let v: Value = serde_json::from_str(&json)?;
    let mut outs = Vec::new();
    if let Some(arr) = v.get("outputs").and_then(Value::as_array) {
        for o in arr {
            if !o.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            let name = o.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
            let x = o.pointer("/pos/x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = o.pointer("/pos/y").and_then(Value::as_f64).unwrap_or(0.0);
            let (w, h) = kscreen_size(o);
            outs.push((name, Rect { x, y, w, h }));
        }
    }
    Ok(outs)
}

/// kscreen reports the mode size; the logical size divides by the scale.
fn kscreen_size(o: &Value) -> (f64, f64) {
    let scale = o.get("scale").and_then(Value::as_f64).unwrap_or(1.0).max(0.25);
    let w = o
        .pointer("/size/width")
        .or_else(|| o.pointer("/currentModeSize/width"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let h = o
        .pointer("/size/height")
        .or_else(|| o.pointer("/currentModeSize/height"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    (w / scale, h / scale)
}

fn token_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("linuxlink")
        .join("portal-restore-token")
}

fn load_restore_token() -> Option<String> {
    std::fs::read_to_string(token_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn save_restore_token(token: &str) {
    let p = token_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, token);
}

// ------------------------------------------------------------------ X11 ----

const X11_NAME: &str = "LinuxLink";

fn prepare_x11(width: u32, height: u32) -> Result<PreparedDisplay> {
    let query = std::process::Command::new("xrandr").arg("--query").output().context("xrandr")?;
    let query = String::from_utf8_lossy(&query.stdout).to_string();
    let (cur_w, cur_h) = parse_x11_screen_size(&query).context("parsing xrandr output")?;

    let new_w = cur_w + width;
    let new_h = cur_h.max(height);
    let x_off = cur_w;

    let mm_w = width * 254 / 960; // assume 96 dpi
    let mm_h = height * 254 / 960;

    run_sync(&["xrandr", "--fb", &format!("{new_w}x{new_h}")])?;
    run_sync(&[
        "xrandr",
        "--setmonitor",
        X11_NAME,
        &format!("{width}/{mm_w}x{height}/{mm_h}+{x_off}+0"),
        "none",
    ])?;

    let mut guards = Guards::default();
    guards.commands.push(vec!["xrandr".into(), "--delmonitor".into(), X11_NAME.into()]);
    guards.commands.push(vec!["xrandr".into(), "--fb".into(), format!("{cur_w}x{cur_h}")]);

    let monitor = Rect { x: f64::from(x_off), y: 0.0, w: f64::from(width), h: f64::from(height) };
    let desktop = Rect { x: 0.0, y: 0.0, w: f64::from(new_w), h: f64::from(new_h) };

    Ok(PreparedDisplay {
        source: CaptureSource::X11Region { x: x_off, y: 0, width, height },
        geometry: Some((monitor, desktop)),
        mutter: None,
        guards,
        width,
        height,
    })
}

/// "Screen 0: minimum 320 x 200, current 1920 x 1080, maximum 16384 x 16384"
fn parse_x11_screen_size(query: &str) -> Option<(u32, u32)> {
    let line = query.lines().find(|l| l.starts_with("Screen "))?;
    let cur = line.split("current ").nth(1)?;
    let mut parts = cur.split(&[' ', ','][..]).filter(|s| !s.is_empty());
    let w: u32 = parts.next()?.parse().ok()?;
    let x = parts.next()?; // literal "x"
    if x != "x" {
        return None;
    }
    let h: u32 = parts.next()?.parse().ok()?;
    Some((w, h))
}

// -------------------------------------------------------------- helpers ----

async fn run(cmd: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .await
        .with_context(|| format!("running {}", cmd[0]))?;
    if !out.status.success() {
        anyhow::bail!("{} failed: {}", cmd.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run_sync(cmd: &[&str]) -> Result<String> {
    let out = std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .with_context(|| format!("running {}", cmd[0]))?;
    if !out.status.success() {
        anyhow::bail!("{} failed: {}", cmd.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Finds `name` in a hyprctl-style JSON monitor array and returns its rect
/// plus the union of all rects.
fn wlr_geometry(json: &str, name: &str, kx: &str, ky: &str, kw: &str, kh: &str) -> Option<(Rect, Rect)> {
    let v: Value = serde_json::from_str(json).ok()?;
    let arr = v.as_array()?;
    let rect_of = |o: &Value| -> Option<Rect> {
        Some(Rect {
            x: o.get(kx).and_then(Value::as_f64)?,
            y: o.get(ky).and_then(Value::as_f64)?,
            w: o.get(kw).and_then(Value::as_f64)?,
            h: o.get(kh).and_then(Value::as_f64)?,
        })
    };
    let monitor = arr
        .iter()
        .find(|o| o.get("name").and_then(Value::as_str) == Some(name))
        .and_then(rect_of)?;
    let desktop = union_rects(arr.iter().filter_map(rect_of));
    Some((monitor, desktop))
}

/// Sway nests geometry under "rect".
fn sway_geometry(json: &str, name: &str) -> Option<(Rect, Rect)> {
    let v: Value = serde_json::from_str(json).ok()?;
    let arr = v.as_array()?;
    let rect_of = |o: &Value| -> Option<Rect> {
        let r = o.get("rect")?;
        Some(Rect {
            x: r.get("x").and_then(Value::as_f64)?,
            y: r.get("y").and_then(Value::as_f64)?,
            w: r.get("width").and_then(Value::as_f64)?,
            h: r.get("height").and_then(Value::as_f64)?,
        })
    };
    let monitor = arr
        .iter()
        .find(|o| o.get("name").and_then(Value::as_str) == Some(name))
        .and_then(rect_of)?;
    let desktop = union_rects(arr.iter().filter_map(rect_of));
    Some((monitor, desktop))
}

fn union_rects(rects: impl Iterator<Item = Rect>) -> Rect {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut any = false;
    for r in rects {
        any = true;
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x + r.w);
        max_y = max_y.max(r.y + r.h);
    }
    if !any {
        return Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
    }
    Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_xrandr_screen_line() {
        let q = "Screen 0: minimum 320 x 200, current 1920 x 1080, maximum 16384 x 16384\n\
                 eDP-1 connected primary 1920x1080+0+0";
        assert_eq!(parse_x11_screen_size(q), Some((1920, 1080)));
    }

    #[test]
    fn hyprland_geometry_and_union() {
        let json = r#"[
            {"name":"eDP-1","x":0,"y":0,"width":1920,"height":1080},
            {"name":"linuxlink","x":1920,"y":0,"width":1280,"height":800}
        ]"#;
        let (mon, desk) = wlr_geometry(json, "linuxlink", "x", "y", "width", "height").unwrap();
        assert_eq!((mon.x, mon.w), (1920.0, 1280.0));
        assert_eq!((desk.w, desk.h), (3200.0, 1080.0));
    }

    #[test]
    fn sway_geometry_reads_the_rect_object() {
        let json = r#"[
            {"name":"HEADLESS-1","rect":{"x":2560,"y":0,"width":1280,"height":800}},
            {"name":"DP-1","rect":{"x":0,"y":0,"width":2560,"height":1440}}
        ]"#;
        let (mon, desk) = sway_geometry(json, "HEADLESS-1").unwrap();
        assert_eq!((mon.x, mon.y), (2560.0, 0.0));
        assert_eq!((desk.w, desk.h), (3840.0, 1440.0));
    }
}
