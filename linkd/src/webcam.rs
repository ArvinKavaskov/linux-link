use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

pub fn find_loopback_device() -> Option<String> {
    let base = std::path::Path::new("/sys/devices/virtual/video4linux");
    let entries = std::fs::read_dir(base).ok()?;
    for e in entries.flatten() {
        let name_path = e.path().join("name");
        if let Ok(name) = std::fs::read_to_string(&name_path) {
            if name.trim() == "Linux Link" {
                if let Some(dev) = e.file_name().to_str() {
                    return Some(format!("/dev/{dev}"));
                }
            }
        }
    }
    None
}

pub struct WebcamFeed {
    ffmpeg: Child,
}

impl WebcamFeed {
    pub fn start(width: u32, height: u32) -> Result<Self> {
        let device = find_loopback_device().context(
            "no virtual webcam “Linux Link” found. Load v4l2loopback: \
             sudo modprobe v4l2loopback card_label=\"Linux Link\" exclusive_caps=1",
        )?;
        tracing::info!("📷 Webcam: {width}x{height} stream → {device}");

        let ffmpeg = Command::new("ffmpeg")
            .args([
                "-f", "mjpeg",
                "-framerate", "30",
                "-i", "pipe:0",
                "-pix_fmt", "yuv420p",
                "-f", "v4l2",
                &device,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("launching ffmpeg (is the ffmpeg package installed?)")?;

        Ok(Self { ffmpeg })
    }

    pub async fn write_frame(&mut self, jpeg: &[u8]) -> Result<()> {
        if let Some(stdin) = self.ffmpeg.stdin.as_mut() {
            stdin.write_all(jpeg).await?;
        }
        Ok(())
    }
}

impl Drop for WebcamFeed {
    fn drop(&mut self) {
        tracing::info!("📷 Webcam: stopping the stream");
        let _ = self.ffmpeg.start_kill();
    }
}
