//! Phone-as-speaker: the PC grows an extra audio output device.
//!
//! We load a PulseAudio/PipeWire null sink named "Phone (Linux Link)". It
//! shows up in the desktop's volume applet next to the real sound cards, and
//! whatever the user routes to it — one app, or everything — is captured from
//! the sink's monitor with `parec` and streamed to the phone as raw PCM.
//!
//! The phone initiates this (a toggle in the app), exactly like the webcam
//! and the microphone: it opens a stream, says `speaker_start`, and from then
//! on the PC pushes audio at it until either side closes the stream.

use anyhow::{Context, Result};
use tokio::process::{Child, ChildStdout, Command};

const SINK: &str = "linuxlink_speaker";

pub struct SpeakerFeed {
    module_id: String,
    parec: Child,
}

impl SpeakerFeed {
    pub async fn start(rate: u32, channels: u32) -> Result<Self> {
        // A leftover sink from a crashed session would make load-module fail;
        // sweep it first. Errors are fine — usually there is nothing to sweep.
        sweep_stale_sinks().await;

        let out = Command::new("pactl")
            .args([
                "load-module",
                "module-null-sink",
                &format!("sink_name={SINK}"),
                &format!("rate={rate}"),
                &format!("channels={channels}"),
                // PA's module-argument parser understands the inner quotes, so
                // the description can keep its spaces.
                "sink_properties=device.description=\"Phone (Linux Link)\"",
            ])
            .output()
            .await
            .context("pactl load-module (is pulseaudio-utils installed?)")?;
        if !out.status.success() {
            anyhow::bail!("pactl load-module: {}", String::from_utf8_lossy(&out.stderr));
        }
        let module_id = String::from_utf8_lossy(&out.stdout).trim().to_string();

        // parec reads the sink's monitor source and writes raw PCM to stdout.
        // 40 ms of requested latency keeps the phone close behind the PC
        // without asking the audio server for anything heroic.
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

    /// The raw PCM stream. Take it once; the server pumps it to the phone.
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

/// Unloads any null-sink module still holding our sink name — the remains of
/// a daemon that died without dropping its `SpeakerFeed`.
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
