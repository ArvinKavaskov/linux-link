use crate::clipboard::ClipboardHub;
use crate::protocol::Message;
use std::sync::Arc;
use tokio::process::Command;

pub async fn apply(action: &str) {
    let arg = match action {
        "play_pause" => "play-pause",
        "next" => "next",
        "previous" => "previous",
        other => {
            tracing::warn!("unknown media action: {other}");
            return;
        }
    };
    match Command::new("playerctl").arg(arg).status().await {
        Ok(s) if s.success() => tracing::info!("⏯ PC media: {action}"),
        Ok(_) => tracing::warn!("no active media player on the PC"),
        Err(e) => tracing::warn!("playerctl unavailable ({e}) — install playerctl"),
    }
}

pub fn spawn_watcher(hub: Arc<ClipboardHub>) {
    tokio::spawn(async move {
        let mut last: Option<(String, String, bool)> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let info = read_now_playing().await;
            if info != last {
                last = info.clone();
                let (title, artist, playing) = info.unwrap_or_default();
                hub.push(Message::MediaInfo { title, artist, playing }).await;
            }
        }
    });
}

async fn read_now_playing() -> Option<(String, String, bool)> {
    let out = Command::new("playerctl")
        .args(["metadata", "--format", "{{status}}\t{{title}}\t{{artist}}"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.splitn(3, '\t');
    let status = parts.next().unwrap_or("").to_string();
    let title = parts.next().unwrap_or("").to_string();
    let artist = parts.next().unwrap_or("").to_string();
    if title.is_empty() {
        return None;
    }
    Some((title, artist, status.eq_ignore_ascii_case("Playing")))
}
