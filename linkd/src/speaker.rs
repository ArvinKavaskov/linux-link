
use anyhow::{Context, Result};
use tokio::process::{Child, ChildStdout, Command};

const SINK: &str = "linuxlink_speaker";

pub struct SpeakerFeed {
    module_id: String,
    parec: Child,
}

impl SpeakerFeed {
    pub async fn start(rate: u32, channels: u32) -> Result<Self> {
        sweep_stale_sinks().await;

        let out = Command::new("pactl")
            .args([
                "load-module",
                "module-null-sink",
                &format!("sink_name={SINK}"),
                &format!("rate={rate}"),
                &format!("channels={channels}"),
                "sink_properties=device.description=\"Phone (Linux Link)\"",
            ])
            .output()
            .await
            .context("pactl load-module (is pulseaudio-utils installed?)")?;
        if !out.status.success() {
            anyhow::bail!("pactl load-module: {}", String::from_utf8_lossy(&out.stderr));
        }
        let module_id = String::from_utf8_lossy(&out.stdout).trim().to_string();

        let parec = Command::new("parec")
            .args([
                "-d",
                &format!("{SINK}.monitor"),
                "--format=s16le",
                &format!("--rate={rate}"),
                &format!("--channels={channels}"),
                "--latency-msec=40",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();

        let parec = match parec {
            Ok(child) => child,
            Err(e) => {
                let _ = Command::new("pactl").args(["unload-module", &module_id]).status().await;
                return Err(e).context("spawning parec (is pulseaudio-utils installed?)");
            }
        };

        tracing::info!("🔊 Speaker: sink “Phone (Linux Link)” {rate} Hz {channels} channel(s) (module {module_id})");
        Ok(Self { module_id, parec })
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.parec.stdout.take()
    }
}

impl Drop for SpeakerFeed {
    fn drop(&mut self) {
        tracing::info!("🔊 Speaker: stopping");
        let _ = self.parec.start_kill();
        let id = self.module_id.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("pactl").args(["unload-module", &id]).status();
        });
    }
}

async fn sweep_stale_sinks() {
    let Ok(out) = Command::new("pactl").args(["list", "short", "modules"]).output().await else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.contains("module-null-sink") && line.contains(&format!("sink_name={SINK}")) {
            if let Some(id) = line.split_whitespace().next() {
                let _ = Command::new("pactl").args(["unload-module", id]).status().await;
            }
        }
    }
}
