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
        crate::events::poke();
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// Locks the session when the phone goes away, unlocks it when it comes back.
///
/// v2 woke up every two seconds to count subscribers. The count only changes
/// when a device connects or disconnects, and both of those now `poke()` the
/// event bus — so this task sleeps indefinitely and is woken by the thing that
/// actually matters. The only timer left is the grace period itself: a phone
/// that drops for a second while switching access point should not lock the
/// PC, so we wait `GRACE` before acting and re-check when it expires.
pub fn spawn(hub: Arc<ClipboardHub>, prox: Arc<ProximityLock>) {
    tokio::spawn(async move {
        let mut rx = crate::events::subscribe();
        let mut we_locked = false;
        let mut absent_since: Option<Instant> = None;
        loop {
            if !prox.is_enabled() {
                absent_since = None;
            } else if hub.subscriber_count().await > 0 {
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

            // Nothing to count down to unless the phone is absent and the
            // grace period is still running: in every other case we can park
            // until something happens.
            let countdown = match absent_since {
                Some(since) if prox.is_enabled() && !we_locked => {
                    Some(GRACE.saturating_sub(since.elapsed()))
                }
                _ => None,
            };
            match countdown {
                Some(remaining) => {
                    let _ = tokio::time::timeout(remaining, rx.changed()).await;
                }
                None => {
                    if rx.changed().await.is_err() {
                        return;
                    }
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
