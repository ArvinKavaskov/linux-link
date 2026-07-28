use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const PIPE: &str = "/tmp/linuxlink-mic.pipe";
const SOURCE: &str = "linuxlink_mic";

pub struct MicFeed {
    module_id: String,
    pipe: PathBuf,
    file: tokio::fs::File,
}

impl MicFeed {
    pub async fn start(rate: u32, channels: u32) -> Result<Self> {
        let pipe = PathBuf::from(PIPE);
        let _ = std::fs::remove_file(&pipe);

        if !Command::new("mkfifo").arg(&pipe).status().await?.success() {
            anyhow::bail!("mkfifo failed");
        }

        let out = Command::new("pactl")
            .args([
                "load-module",
                "module-pipe-source",
                &format!("source_name={SOURCE}"),
                &format!("file={PIPE}"),
                "format=s16le",
                &format!("rate={rate}"),
                &format!("channels={channels}"),
                "source_properties=device.description=Linux\\ Link",
            ])
            .output()
            .await
            .context("pactl load-module (is pulseaudio-utils installed?)")?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&pipe);
            anyhow::bail!("pactl load-module: {}", String::from_utf8_lossy(&out.stderr));
        }
        let module_id = String::from_utf8_lossy(&out.stdout).trim().to_string();

        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&pipe)
            .await
            .context("opening the mic pipe")?;

        tracing::info!("🎤 Mic: source “Linux Link” {rate} Hz {channels} channel(s) (module {module_id})");
        Ok(Self { module_id, pipe, file })
    }

    pub async fn write_pcm(&mut self, data: &[u8]) -> Result<()> {
        self.file.write_all(data).await?;
        Ok(())
    }
}

impl Drop for MicFeed {
    fn drop(&mut self) {
        tracing::info!("🎤 Mic: stopping");
        let id = self.module_id.clone();
        let pipe = self.pipe.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("pactl").args(["unload-module", &id]).status();
            let _ = std::fs::remove_file(&pipe);
        });
    }
}
