
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
        return Ok(Compositor::X11);
    }
    anyhow::bail!(
        "unsupported compositor (XDG_CURRENT_DESKTOP={desktop:?}, XDG_SESSION_TYPE={session:?}) — \
         second screen currently supports GNOME, KDE Plasma, Hyprland, Sway and any X11 desktop"
    )
}

pub struct PreparedDisplay {
    pub source: CaptureSource,
    pub geometry: Option<(Rect, Rect)>,
    pub mutter: Option<MutterSession>,
    pub guards: Guards,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct Guards {
    pub children: Vec<std::process::Child>,
    pub commands: Vec<Vec<String>>,
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
        Compositor::X11 => prepare_x11(width, height).await,
    }
}

pub struct MutterSession {
    rd_session: zbus::Proxy<'static>,
    stream_path: String,
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
                    0 => 0x110,
                    1 => 0x111,
                    _ => 0x112,
                };
                s.call::<_, _, ()>("NotifyPointerButton", &(code, d)).await?;
            }
            I::Scroll { dx, dy, end } => {
                let flags: u32 = u32::from(end);
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
                s.call::<_, _, ()>(
                    "NotifyPointerMotionAbsolute",
                    &(self.stream_path.as_str(), x * self.width, y * self.height),
                )
                .await?;
                s.call::<_, _, ()>("NotifyPointerButton", &(0x110i32, d)).await?;
            }
            I::Pen { .. } => {}
        }
        Ok(())
    }
}

const HYPR_NAME: &str = "linuxlink";

async fn prepare_hyprland(width: u32, height: u32, fps: u32) -> Result<PreparedDisplay> {
    run(&["hyprctl", "output", "create", "headless", HYPR_NAME]).await?;
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

async fn prepare_kde(width: u32, height: u32) -> Result<PreparedDisplay> {
    if !has_cmd("krfb-virtualmonitor") {
        anyhow::bail!(
            "krfb-virtualmonitor not found — install the krfb package \
             (pacman -S krfb / apt install krfb); it is what creates the \
             virtual monitor on KDE Plasma"
        );
    }
    let before = kscreen_outputs().await.unwrap_or_default();

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

const X11_NAME: &str = "LinuxLink";

async fn prepare_x11(width: u32, height: u32) -> Result<PreparedDisplay> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desktop.contains("gnome") || desktop.contains("zorin") {
        anyhow::bail!(
            "GNOME on X11 cannot host a virtual monitor. Log out and pick the Wayland \
             session (gear icon on the login screen) — there Linux Link asks GNOME \
             itself for a real virtual monitor, and the touch input comes for free"
        );
    }

    let query = run_sync(&["xrandr", "--query"])?;
    let (cur_w, cur_h) = parse_x11_screen_size(&query).context("parsing xrandr output")?;

    for out in parse_x11_disconnected(&query) {
        match try_x11_forced_output(&out, width, height, cur_w, cur_h).await {
            Ok(p) => {
                tracing::info!("virtual monitor on the unused {out} port");
                return Ok(p);
            }
            Err(e) => tracing::info!("no virtual mode on {out} ({e:#})"),
        }
    }
    try_x11_big_framebuffer(width, height, cur_w, cur_h).await
}

async fn try_x11_forced_output(
    out: &str,
    width: u32,
    height: u32,
    cur_w: u32,
    cur_h: u32,
) -> Result<PreparedDisplay> {
    let mode = format!("{X11_NAME}-{width}x{height}");
    let m = cvt_rb_modeline(width, height);
    let mut newmode: Vec<String> =
        vec!["xrandr".into(), "--newmode".into(), mode.clone(), format!("{:.2}", m.clock_mhz)];
    newmode.extend(
        [m.hd, m.hss, m.hse, m.htotal, m.vd, m.vss, m.vse, m.vtotal].map(|v| v.to_string()),
    );
    newmode.push("+hsync".into());
    newmode.push("-vsync".into());
    let _ = run_sync_owned(&newmode);
    run_sync(&["xrandr", "--addmode", out, &mode])?;

    let mut guards = Guards::default();
    guards.commands.push(vec!["xrandr".into(), "--output".into(), out.into(), "--off".into()]);
    guards.commands.push(vec!["xrandr".into(), "--delmode".into(), out.into(), mode.clone()]);
    guards.commands.push(vec!["xrandr".into(), "--rmmode".into(), mode.clone()]);
    guards.commands.push(vec!["xrandr".into(), "--fb".into(), format!("{cur_w}x{cur_h}")]);

    run_sync(&["xrandr", "--output", out, "--mode", &mode, "--pos", &format!("{cur_w}x0")])?;

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let query = run_sync(&["xrandr", "--query"])?;
    let (x, y) = parse_x11_output_pos(&query, out, width, height)
        .ok_or_else(|| anyhow::anyhow!("{out} did not keep the {width}x{height} mode"))?;

    let (dw, dh) =
        parse_x11_screen_size(&query).unwrap_or((cur_w + width, cur_h.max(height)));
    let monitor =
        Rect { x: f64::from(x), y: f64::from(y), w: f64::from(width), h: f64::from(height) };
    let desktop = Rect { x: 0.0, y: 0.0, w: f64::from(dw), h: f64::from(dh) };

    Ok(PreparedDisplay {
        source: CaptureSource::X11Region { x, y, width, height },
        geometry: Some((monitor, desktop)),
        mutter: None,
        guards,
        width,
        height,
    })
}

async fn try_x11_big_framebuffer(
    width: u32,
    height: u32,
    cur_w: u32,
    cur_h: u32,
) -> Result<PreparedDisplay> {
    let new_w = cur_w + width;
    let new_h = cur_h.max(height);
    let x_off = cur_w;

    let mm_w = width * 254 / 960;
    let mm_h = height * 254 / 960;

    run_sync(&["xrandr", "--fb", &format!("{new_w}x{new_h}")])?;
    let mut guards = Guards::default();
    guards.commands.push(vec!["xrandr".into(), "--delmonitor".into(), X11_NAME.into()]);
    guards.commands.push(vec!["xrandr".into(), "--fb".into(), format!("{cur_w}x{cur_h}")]);
    run_sync(&[
        "xrandr",
        "--setmonitor",
        X11_NAME,
        &format!("{width}/{mm_w}x{height}/{mm_h}+{x_off}+0"),
        "none",
    ])?;

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let (w_now, _) = parse_x11_screen_size(&run_sync(&["xrandr", "--query"])?)
        .context("re-reading the screen size")?;
    if w_now < new_w {
        anyhow::bail!(
            "the desktop keeps undoing the enlarged framebuffer (it went straight back to \
             {w_now}px wide) — GNOME on X11 does this. Log into the Wayland session instead \
             (GNOME there has a native virtual monitor), or free up one video port on the PC"
        );
    }

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

struct Modeline {
    clock_mhz: f64,
    hd: u32,
    hss: u32,
    hse: u32,
    htotal: u32,
    vd: u32,
    vss: u32,
    vse: u32,
    vtotal: u32,
}

fn cvt_rb_modeline(width: u32, height: u32) -> Modeline {
    const RB_H_BLANK: u32 = 160;
    const RB_MIN_VBLANK_US: f64 = 460.0;
    const V_FP: u32 = 3;
    const V_MIN_BP: u32 = 6;

    let vsync = match (width * 3 == height * 4, width * 9 == height * 16, width * 10 == height * 16, width * 4 == height * 5) {
        (true, ..) => 4,
        (_, true, ..) => 5,
        (_, _, true, _) => 6,
        (.., true) => 7,
        _ => 10,
    };

    let htotal = width + RB_H_BLANK;
    let h_period_us = (1_000_000.0 / 60.0 - RB_MIN_VBLANK_US) / f64::from(height);
    let vbi = ((RB_MIN_VBLANK_US / h_period_us).floor() as u32 + 1).max(V_FP + vsync + V_MIN_BP);
    let vtotal = height + vbi;
    let clock_mhz = (60.0 * f64::from(htotal) * f64::from(vtotal) / 250_000.0).floor() * 0.25;

    Modeline {
        clock_mhz,
        hd: width,
        hss: width + 48,
        hse: width + 80,
        htotal,
        vd: height,
        vss: height + V_FP,
        vse: height + V_FP + vsync,
        vtotal,
    }
}

fn parse_x11_disconnected(query: &str) -> Vec<String> {
    query
        .lines()
        .filter_map(|l| {
            let mut t = l.split_whitespace();
            let name = t.next()?;
            if t.next()? != "disconnected" {
                return None;
            }
            match t.next() {
                Some(tok) if tok.contains('+') && tok.contains('x') => None, // active
                _ => Some(name.to_string()),
            }
        })
        .collect()
}

fn parse_x11_output_pos(query: &str, out: &str, width: u32, height: u32) -> Option<(u32, u32)> {
    let line = query.lines().find(|l| l.starts_with(&format!("{out} ")))?;
    let geom = line.split_whitespace().find(|t| t.starts_with(&format!("{width}x{height}+")))?;
    let mut plus = geom.split('+').skip(1);
    let x: u32 = plus.next()?.parse().ok()?;
    let y: u32 = plus.next()?.parse().ok()?;
    Some((x, y))
}

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

fn run_sync_owned(cmd: &[String]) -> Result<String> {
    let v: Vec<&str> = cmd.iter().map(String::as_str).collect();
    run_sync(&v)
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
    fn reduced_blanking_matches_the_cvt_tool() {
        let m = cvt_rb_modeline(1920, 1080);
        assert_eq!(
            (m.clock_mhz, m.hss, m.hse, m.htotal, m.vss, m.vse, m.vtotal),
            (138.50, 1968, 2000, 2080, 1083, 1088, 1111)
        );
        let m = cvt_rb_modeline(2560, 1440);
        assert_eq!(
            (m.clock_mhz, m.hss, m.hse, m.htotal, m.vss, m.vse, m.vtotal),
            (241.50, 2608, 2640, 2720, 1443, 1448, 1481)
        );
    }

    #[test]
    fn finds_free_outputs_and_skips_busy_ones() {
        let q = "\
Screen 0: minimum 320 x 200, current 1920 x 1080, maximum 16384 x 16384
eDP-1 connected (normal left inverted right x axis y axis)
HDMI-1 disconnected (normal left inverted right x axis y axis)
HDMI-1-0 connected primary 1920x1080+0+0 (normal left inverted right) 598mm x 336mm
DP-2 disconnected 1696x1200+1920+0 (normal left inverted right) 0mm x 0mm";
        assert_eq!(parse_x11_disconnected(q), vec!["HDMI-1".to_string()]);
    }

    #[test]
    fn reads_an_output_position_even_with_the_primary_flag() {
        let q = "\
HDMI-1-0 connected primary 1920x1080+0+0 (normal left inverted right) 598mm x 336mm
HDMI-1 disconnected 1696x1200+1920+0 (normal left inverted right) 0mm x 0mm";
        assert_eq!(parse_x11_output_pos(q, "HDMI-1", 1696, 1200), Some((1920, 0)));
        assert_eq!(parse_x11_output_pos(q, "HDMI-1-0", 1920, 1080), Some((0, 0)));
        assert_eq!(parse_x11_output_pos(q, "HDMI-1", 1600, 1200), None);
    }

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
