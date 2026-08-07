use crate::clipboard::ClipboardHub;
use crate::files::PendingFiles;
use crate::identity::Identity;
use crate::lock::ProximityLock;
use crate::pairing::Pairing;
use crate::protocol::Message;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub fn socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("linuxlink.sock")
}

pub fn serve(
    hub: Arc<ClipboardHub>,
    pending: Arc<PendingFiles>,
    prox: Arc<ProximityLock>,
    identity: Arc<Identity>,
    port: u16,
    pairing: Arc<Pairing>,
) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("control socket unavailable ({e}) — local actions disabled");
            return;
        }
    };
    tracing::info!("Control socket: {}", path.display());
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let hub = hub.clone();
                    let pending = pending.clone();
                    let prox = prox.clone();
                    let identity = identity.clone();
                    let pairing = pairing.clone();
                    tokio::spawn(handle(stream, hub, pending, prox, identity, port, pairing));
                }
                Err(e) => {
                    tracing::warn!("control socket accept: {e}");
                    break;
                }
            }
        }
    });
}

async fn handle(
    stream: UnixStream,
    hub: Arc<ClipboardHub>,
    pending: Arc<PendingFiles>,
    prox: Arc<ProximityLock>,
    identity: Arc<Identity>,
    port: u16,
    pairing: Arc<Pairing>,
) {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();

        if line == "PAIR" {
            handle_pair(&mut wr, &identity, port, &pairing).await;
        } else if let Some(rest) = line.strip_prefix("OPEN ") {
            let (url, title) = rest.split_once('\t').unwrap_or((rest, ""));
            if url.is_empty() {
                continue;
            }
            tracing::info!("↗ Handoff PC→phone: {url}");
            hub.push(Message::OpenUrl { url: url.to_string(), title: title.to_string() })
                .await;
        } else if let Some(rest) = line.strip_prefix("PHONEVOL ") {
            let (action, value) = rest.split_once(' ').unwrap_or((rest, "0"));
            let value: i32 = value.trim().parse().unwrap_or(0);
            tracing::info!("🔊 Phone volume: {action}");
            hub.push(Message::PhoneVolume { action: action.to_string(), value }).await;
        } else if let Some(rest) = line.strip_prefix("MEDIA ") {
            let action = rest.trim();
            tracing::info!("⏯ Phone media: {action}");
            hub.push(Message::PhoneMedia { action: action.to_string() }).await;
        } else if let Some(rest) = line.strip_prefix("SENDFILETO ") {
            let (fp, path) = match rest.split_once('\t') {
                Some(p) => p,
                None => continue,
            };
            if path.is_empty() {
                continue;
            }
            match pending.offer(PathBuf::from(path)).await {
                Ok((id, name, size)) => {
                    tracing::info!("📎 File offered to {}…: {name} ({size} bytes)", &fp[..fp.len().min(16)]);
                    hub.push_to(fp, Message::FileOffer { id, name, size, clipboard: false }).await;
                }
                Err(e) => tracing::warn!("send-file (targeted): {e:#}"),
            }
        } else if let Some(rest) = line.strip_prefix("SENDFILE ") {
            let path = PathBuf::from(rest.trim());
            match pending.offer(path).await {
                Ok((id, name, size)) => {
                    tracing::info!("📎 File offered to phone(s): {name} ({size} bytes)");
                    hub.push(Message::FileOffer { id, name, size, clipboard: false }).await;
                }
                Err(e) => tracing::warn!("send-file: {e:#}"),
            }
        } else if line == "SCREEN" || line.starts_with("SCREEN ") {
            // "SCREEN" invites every connected tablet, "SCREEN <fingerprint>"
            // just the one — the same shape as the file commands above.
            let fp = line["SCREEN".len()..].trim();
            tracing::info!("🖥 Second screen offered to {}", if fp.is_empty() { "phone(s)" } else { fp });
            if fp.is_empty() {
                hub.push(Message::DisplayInvite).await;
            } else {
                hub.push_to(fp, Message::DisplayInvite).await;
            }
        } else if let Some(rest) = line.strip_prefix("LOCKMODE ") {
            prox.set_enabled(rest.trim() == "on");
        } else if let Some(rest) = line.strip_prefix("FORGET ") {
            let reply = forget_device(rest.trim()).await;
            let _ = wr.write_all(reply.as_bytes()).await;
            let _ = wr.flush().await;
        } else if line == "PING" {
            let _ = wr.write_all(b"PONG\n").await;
            let _ = wr.flush().await;
        }
    }
}

/// Drops a device from the trusted list and cuts its live connection on the
/// spot. The phone keeps its certificate but it no longer buys anything:
/// no more automatic reconnection until the user pairs it again.
async fn forget_device(fingerprint: &str) -> String {
    if fingerprint.is_empty() {
        return "ERR no fingerprint\n".to_string();
    }
    let mut peers = match crate::identity::TrustedPeers::load() {
        Ok(p) => p,
        Err(e) => return format!("ERR {e}\n"),
    };
    match peers.forget(fingerprint) {
        Ok(Some(peer)) => {
            let cut = crate::server::kick(&peer.fingerprint);
            tracing::info!(
                "🗑 Device forgotten: {}{}",
                peer.name,
                if cut > 0 { " (live connection cut)" } else { "" }
            );
            crate::events::poke();
            format!("OK {}\n", peer.name)
        }
        Ok(None) => "ERR unknown device\n".to_string(),
        Err(e) => format!("ERR {e}\n"),
    }
}

async fn handle_pair(
    wr: &mut tokio::net::unix::OwnedWriteHalf,
    identity: &Identity,
    port: u16,
    pairing: &Arc<Pairing>,
) {
    let token = crate::pairing::new_token();
    let json = match crate::pairing::payload_json(identity, port, &token) {
        Ok(j) => j,
        Err(e) => {
            let _ = wr.write_all(format!("ERR {e}\n").as_bytes()).await;
            return;
        }
    };
    let mut rx = pairing.subscribe();
    pairing.begin(token).await;
    tracing::info!("🔗 Pairing window opened (live)");
    let _ = wr.write_all(format!("{json}\n").as_bytes()).await;
    let _ = wr.flush().await;

    let result = tokio::select! {
        r = rx.recv() => r.ok(),
        _ = tokio::time::sleep(Duration::from_secs(120)) => None,
    };
    pairing.end().await;
    let reply = match result {
        Some(name) => format!("PAIRED {name}\n"),
        None => "TIMEOUT\n".to_string(),
    };
    let _ = wr.write_all(reply.as_bytes()).await;
    let _ = wr.flush().await;
}

pub async fn send_url(url: &str, title: &str) -> Result<()> {
    send_line(&format!("OPEN {url}\t{title}")).await
}

pub async fn phone_volume(action: &str, value: i32) -> Result<()> {
    send_line(&format!("PHONEVOL {action} {value}")).await
}

pub async fn phone_media(action: &str) -> Result<()> {
    send_line(&format!("MEDIA {action}")).await
}

pub async fn send_file(path: &str, to: Option<&str>) -> Result<()> {
    match to {
        Some(fp) => send_line(&format!("SENDFILETO {fp}\t{path}")).await,
        None => send_line(&format!("SENDFILE {path}")).await,
    }
}

/// Offers the second screen to a tablet, or to every connected one.
pub async fn second_screen(to: Option<&str>) -> Result<()> {
    match to {
        Some(fp) => send_line(&format!("SCREEN {fp}")).await,
        None => send_line("SCREEN").await,
    }
}

pub async fn proximity_lock(on: bool) -> Result<()> {
    send_line(&format!("LOCKMODE {}", if on { "on" } else { "off" })).await
}

pub async fn pair_live() -> Result<()> {
    let path = socket_path();
    let stream = UnixStream::connect(&path).await.map_err(|e| {
        anyhow::anyhow!(
            "daemon unreachable on {} ({e}). Is the linkd service running?",
            path.display()
        )
    })?;
    let (rd, mut wr) = stream.into_split();
    wr.write_all(b"PAIR\n").await?;
    wr.flush().await?;

    let mut lines = BufReader::new(rd).lines();
    let payload = lines
        .next_line()
        .await?
        .context("no response from daemon")?;
    if let Some(err) = payload.strip_prefix("ERR ") {
        anyhow::bail!("{err}");
    }
    crate::pairing::print_qr_json(&payload)?;
    println!("Waiting for scan… (2 minutes max). Keep this window open.\n");

    match lines.next_line().await? {
        Some(l) if l.starts_with("PAIRED ") => {
            println!("✔ Device paired: {}", l.trim_start_matches("PAIRED ").trim());
            println!("It is active immediately, without restarting anything.");
        }
        _ => println!("⏱ Time elapsed — run \"Pair a device…\" again if needed."),
    }
    Ok(())
}

/// Asks the daemon to forget a device and returns the name it had.
pub async fn forget(fingerprint: &str) -> Result<String> {
    let path = socket_path();
    let stream = UnixStream::connect(&path).await.map_err(|e| {
        anyhow::anyhow!(
            "daemon unreachable on {} ({e}). Is the linkd service running?",
            path.display()
        )
    })?;
    let (rd, mut wr) = stream.into_split();
    wr.write_all(format!("FORGET {fingerprint}\n").as_bytes()).await?;
    wr.flush().await?;
    let reply = BufReader::new(rd)
        .lines()
        .next_line()
        .await?
        .context("no response from daemon")?;
    match reply.strip_prefix("OK ") {
        Some(name) => Ok(name.trim().to_string()),
        None => anyhow::bail!("{}", reply.trim_start_matches("ERR ").trim()),
    }
}

async fn send_line(line: &str) -> Result<()> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).await.map_err(|e| {
        anyhow::anyhow!(
            "daemon unreachable on {} ({e}). Is the linkd service running?",
            path.display()
        )
    })?;
    stream.write_all(format!("{line}\n").as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}
