
pub mod backends;
pub mod encoder;
pub mod input;
pub mod portal;

use anyhow::Result;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t")]
pub enum RemoteInput {
    #[serde(rename = "mv")]
    Move { x: f64, y: f64 },
    #[serde(rename = "bt")]
    Button { b: u8, d: bool },
    #[serde(rename = "sc")]
    Scroll {
        dx: f64,
        dy: f64,
        #[serde(default)]
        end: bool,
    },
    #[serde(rename = "ky")]
    Key { c: u16, d: bool },
    #[serde(rename = "tc")]
    Touch { id: u32, ph: u8, x: f64, y: f64 },
    #[serde(rename = "pn")]
    Pen {
        x: f64,
        y: f64,
        p: f64,
        d: bool,
        #[serde(default)]
        tx: f64,
        #[serde(default)]
        ty: f64,
        #[serde(default)]
        e: bool,
        #[serde(default)]
        bar: bool,
        #[serde(default = "yes")]
        prox: bool,
    },
}

fn yes() -> bool {
    true
}

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct DisplaySession {
    pub width: u32,
    pub height: u32,
    pub units: mpsc::Receiver<Vec<u8>>,
    encoder: Option<encoder::Encoder>,
    sink: Sink,
    mutter: Option<backends::MutterSession>,
    started: std::time::Instant,
    _guards: backends::Guards,
}

enum Sink {
    Mutter,
    Uinput(Box<input::UinputSink>),
    None,
}

impl DisplaySession {
    pub async fn start(width: u32, height: u32, fps: u32) -> Result<Self> {
        if ACTIVE.swap(true, Ordering::SeqCst) {
            anyhow::bail!("a second-screen session is already running");
        }
        match Self::start_inner(width, height, fps).await {
            Ok(s) => Ok(s),
            Err(e) => {
                ACTIVE.store(false, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    async fn start_inner(width: u32, height: u32, fps: u32) -> Result<Self> {
        let width = (width.clamp(640, 3840) / 2) * 2;
        let height = (height.clamp(400, 2400) / 2) * 2;
        let fps = fps.clamp(24, 60);

        let comp = backends::detect()?;
        tracing::info!("second screen: {comp:?}, {width}x{height}@{fps}");
        let mut prepared = backends::prepare(comp, width, height, fps).await?;

        let bitrate = bitrate_kbps(prepared.width, prepared.height);
        let enc = match encoder::Encoder::start(&prepared.source, fps, bitrate).await {
            Ok(e) => e,
            Err(e) => {
                if let Some(m) = &prepared.mutter {
                    m.stop().await;
                }
                return Err(e);
            }
        };

        let sink = if prepared.mutter.is_some() {
            Sink::Mutter
        } else if let Some((monitor, desktop)) = prepared.geometry {
            match input::UinputSink::new(monitor, desktop) {
                Ok(s) => Sink::Uinput(Box::new(s)),
                Err(e) => {
                    tracing::warn!("second screen input disabled: {e:#}");
                    Sink::None
                }
            }
        } else {
            Sink::None
        };

        let mut enc = enc;
        let units = std::mem::replace(&mut enc.units, mpsc::channel(1).1);
        Ok(Self {
            width: prepared.width,
            height: prepared.height,
            units,
            encoder: Some(enc),
            sink,
            mutter: prepared.mutter.take(),
            started: std::time::Instant::now(),
            _guards: std::mem::take(&mut prepared.guards),
        })
    }

    pub async fn handle_input(&mut self, line: &str) {
        let ev: RemoteInput = match serde_json::from_str(line.trim()) {
            Ok(ev) => ev,
            Err(e) => {
                tracing::debug!("bad input event {line:?}: {e}");
                return;
            }
        };
        let now = self.now();
        let result = match &mut self.sink {
            Sink::Mutter => match &self.mutter {
                Some(m) => m.handle(&ev).await,
                None => Ok(()),
            },
            Sink::Uinput(s) => s.handle(&ev, now),
            Sink::None => Ok(()),
        };
        if let Err(e) = result {
            tracing::debug!("input injection: {e:#}");
        }
    }

    pub fn tick(&mut self) {
        let now = self.now();
        if let Sink::Uinput(s) = &mut self.sink {
            if let Err(e) = s.tick(now) {
                tracing::debug!("input tick: {e:#}");
            }
        }
    }

    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub async fn shutdown(mut self) {
        if let Some(enc) = self.encoder.take() {
            enc.shutdown().await;
        }
        if let Some(m) = self.mutter.take() {
            m.stop().await;
        }
    }
}

impl Drop for DisplaySession {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

fn bitrate_kbps(width: u32, height: u32) -> u32 {
    let px = u64::from(width) * u64::from(height);
    let kbps = px * 12000 / (1920 * 1080);
    (kbps as u32).clamp(4000, 24000)
}

pub(crate) fn has_cmd(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_events_parse_from_their_wire_form() {
        let mv: RemoteInput = serde_json::from_str(r#"{"t":"mv","x":0.25,"y":0.75}"#).unwrap();
        assert!(matches!(mv, RemoteInput::Move { x, y } if x == 0.25 && y == 0.75));

        let key: RemoteInput = serde_json::from_str(r#"{"t":"ky","c":30,"d":true}"#).unwrap();
        assert!(matches!(key, RemoteInput::Key { c: 30, d: true }));

        let sc: RemoteInput = serde_json::from_str(r#"{"t":"sc","dx":0,"dy":-18}"#).unwrap();
        assert!(matches!(sc, RemoteInput::Scroll { end: false, .. }));

        let tc: RemoteInput = serde_json::from_str(r#"{"t":"tc","id":2,"ph":0,"x":0.1,"y":0.9}"#).unwrap();
        assert!(matches!(tc, RemoteInput::Touch { id: 2, ph: 0, .. }));

        let pn: RemoteInput =
            serde_json::from_str(r#"{"t":"pn","x":0.5,"y":0.5,"p":0.66,"d":true}"#).unwrap();
        assert!(matches!(pn, RemoteInput::Pen { d: true, .. }));
    }

    #[test]
    fn bitrate_scales_with_the_pixel_count() {
        assert_eq!(bitrate_kbps(1920, 1080), 12000);
        assert!(bitrate_kbps(1280, 800) < 12000);
        assert!(bitrate_kbps(1280, 800) >= 4000);
        assert!(bitrate_kbps(2560, 1600) > 12000);
        assert!(bitrate_kbps(3840, 2400) <= 24000);
    }
}
