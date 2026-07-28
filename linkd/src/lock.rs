use crate::clipboard::ClipboardHub;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;

const GRACE: Duration = Duration::from_secs(4);

pub struct ProximityLock {
    pub enabled: AtomicBool,
}

impl ProximityLock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(load_enabled()),
        })
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        save_enabled(on);
        tracing::info!("Proximity lock: {}", if on { "enabled" } else { "disabled" });
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

pub fn spawn(hub: Arc<ClipboardHub>, prox: Arc<ProximityLock>) {
    tokio::spawn(async move {
        let mut we_locked = false;
        let mut absent_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if !prox.is_enabled() {
                absent_since = None;
                continue;
            }
            let present = hub.subscriber_count().await > 0;
            if present {
                absent_since = None;
                if we_locked {
                    unlock_session().await;
                    we_locked = false;
                }
            } else {
                let since = *absent_since.get_or_insert_with(Instant::now);
                if !we_locked && since.elapsed() >= GRACE {
                    lock_session().await;
                    we_locked = true;
                }
            }
        }
    });
}

async fn lock_session() {
    tracing::info!("🔒 Phone out of range → locking");
    if run("loginctl", &["lock-session"]).await {
        return;
    }
    let _ = run(
        "dbus-send",
        &[
            "--session",
            "--dest=org.freedesktop.ScreenSaver",
            "--type=method_call",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver.Lock",
        ],
    )
    .await;
}

async fn unlock_session() {
    tracing::info!("🔓 Phone back in range → unlocking");
    if !run("loginctl", &["unlock-session"]).await {
        tracing::warn!(
            "unlock refused: `loginctl unlock-session` failed. \
             A polkit rule may be required (see README)."
        );
    }
}

async fn run(cmd: &str, args: &[&str]) -> bool {
    match Command::new(cmd).args(args).status().await {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

fn config_file() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("linux-link");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("proximity")
}

fn load_enabled() -> bool {
    std::fs::read_to_string(config_file())
        .map(|s| s.trim() == "on")
        .unwrap_or(false)
}

fn save_enabled(on: bool) {
    let _ = std::fs::write(config_file(), if on { "on" } else { "off" });
}
