use crate::clipboard::ClipboardHub;
use crate::protocol::Message;
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;
use tokio::process::Command;

pub struct DndSync {
    last: AtomicI8,
}

impl DndSync {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { last: AtomicI8::new(-1) })
    }

    pub async fn apply_from_phone(&self, on: bool) {
        self.last.store(on as i8, Ordering::Relaxed);
        set_pc_dnd(on).await;
        tracing::info!("🌙 DND synced from phone: {}", if on { "enabled" } else { "disabled" });
    }
}

pub fn spawn_watcher(hub: Arc<ClipboardHub>, dnd: Arc<DndSync>) {
    tokio::spawn(async move {
        let Some(initial) = read_pc_dnd().await else {
            tracing::info!("DND sync off: no GNOME notifications schema on this desktop");
            return;
        };
        dnd.last.store(initial as i8, Ordering::Relaxed);
        hub.push(Message::Dnd { on: initial }).await;
        if let Err(e) = monitor(&hub, &dnd).await {
            tracing::warn!("DND monitor unavailable ({e}) — falling back to polling");
            poll(hub, dnd).await;
        }
    });
}

async fn monitor(hub: &Arc<ClipboardHub>, dnd: &Arc<DndSync>) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = Command::new("gsettings")
        .args(["monitor", "org.gnome.desktop.notifications", "show-banners"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    tracing::info!("DND: event-driven (gsettings monitor)");

    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        let Some(value) = line.split(':').nth(1) else { continue };
        let cur = value.trim() == "false";
        let prev = dnd.last.swap(cur as i8, Ordering::Relaxed);
        if prev == cur as i8 {
            continue;
        }
        tracing::info!("🌙 PC DND changed → {} (push)", if cur { "enabled" } else { "disabled" });
        hub.push(Message::Dnd { on: cur }).await;
    }
    anyhow::bail!("gsettings monitor exited")
}

async fn poll(hub: Arc<ClipboardHub>, dnd: Arc<DndSync>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let Some(cur) = read_pc_dnd().await else { continue };
        let prev = dnd.last.load(Ordering::Relaxed);
        if prev != cur as i8 {
            dnd.last.store(cur as i8, Ordering::Relaxed);
            if prev != -1 {
                tracing::info!("🌙 PC DND changed → {} (push)", if cur { "enabled" } else { "disabled" });
            }
            hub.push(Message::Dnd { on: cur }).await;
        }
    }
}

async fn set_pc_dnd(on: bool) {
    let val = if on { "false" } else { "true" };
    let ok = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.notifications", "show-banners", val])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        tracing::warn!("gsettings unavailable — DND sync limited (non-GNOME desktop?)");
    }
}

async fn read_pc_dnd() -> Option<bool> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Some(s.trim() == "false")
}
