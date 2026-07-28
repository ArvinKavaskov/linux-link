use crate::files::PendingFiles;
use crate::protocol::Message;
use sha2::{Digest, Sha256};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};

const MAX_LEN: usize = 256 * 1024;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Wayland,
    X11,
    None,
}

struct Subscriber {
    id: u64,
    fp: String,
    name: String,
    tx: mpsc::Sender<Message>,
}

pub struct ClipboardHub {
    backend: Backend,
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
    last_text: Mutex<String>,
    last_image_hash: Mutex<String>,
    pending: Mutex<Option<Arc<PendingFiles>>>,
}

impl ClipboardHub {
    pub async fn set_pending(&self, pending: Arc<PendingFiles>) {
        *self.pending.lock().await = Some(pending);
    }

    pub fn new() -> Arc<Self> {
        let backend = detect_backend();
        match backend {
            Backend::Wayland => tracing::info!("Clipboard: Wayland backend (wl-clipboard)"),
            Backend::X11 => tracing::info!("Clipboard: X11 backend (xclip)"),
            Backend::None => tracing::warn!(
                "Clipboard unavailable: install `wl-clipboard` (Wayland) or `xclip` (X11)"
            ),
        }
        let hub = Arc::new(Self {
            backend,
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            last_text: Mutex::new(String::new()),
            last_image_hash: Mutex::new(String::new()),
            pending: Mutex::new(None),
        });
        if backend != Backend::None {
            let h = hub.clone();
            tokio::spawn(async move { h.watch().await });
        }
        hub
    }

    pub async fn subscribe(&self, fp: String, name: String) -> (u64, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel(16);
        let last = self.last_text.lock().await.clone();
        if !last.is_empty() {
            let _ = tx.try_send(Message::Clipboard { text: last });
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscribers.lock().await.push(Subscriber { id, fp, name: name.clone(), tx });
        tracing::info!("Push channel opened: {name} (subscriber #{id})");
        (id, rx)
    }

    pub async fn unsubscribe(&self, id: u64) {
        self.subscribers.lock().await.retain(|s| s.id != id);
        tracing::info!("Push channel closed (subscriber #{id})");
    }

    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.lock().await.len()
    }

    pub async fn connected(&self) -> Vec<(String, String)> {
        let subs = self.subscribers.lock().await;
        let mut out: Vec<(String, String)> = Vec::new();
        for s in subs.iter() {
            if !out.iter().any(|(fp, _)| *fp == s.fp) {
                out.push((s.fp.clone(), s.name.clone()));
            }
        }
        out
    }

    pub async fn set_from_peer(&self, text: &str) {
        if text.len() > MAX_LEN {
            tracing::warn!("received clipboard too large ({} bytes), ignored", text.len());
            return;
        }
        *self.last_text.lock().await = text.to_string();
        self.set_clipboard(text).await;
        self.push(Message::Clipboard { text: text.to_string() }).await;
    }

    pub async fn push(&self, msg: Message) {
        let subs = self.subscribers.lock().await;
        for s in subs.iter() {
            let _ = s.tx.try_send(msg.clone());
        }
    }

    pub async fn push_to(&self, fp: &str, msg: Message) {
        let subs = self.subscribers.lock().await;
        for s in subs.iter().filter(|s| s.fp == fp) {
            let _ = s.tx.try_send(msg.clone());
        }
    }

    async fn watch(self: Arc<Self>) {
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            if let Some(bytes) = self.get_clipboard_image().await {
                let hash = hex::encode(Sha256::digest(&bytes));
                let mut last = self.last_image_hash.lock().await;
                if *last != hash {
                    *last = hash;
                    drop(last);
                    self.offer_clipboard_image(bytes).await;
                }
                continue;
            }

            let Some(current) = self.get_clipboard().await else { continue };
            if current.is_empty() || current.len() > MAX_LEN {
                continue;
            }
            {
                let mut last = self.last_text.lock().await;
                if *last == current {
                    continue;
                }
                *last = current.clone();
            }
            tracing::info!("📋 PC clipboard changed ({} characters) → push", current.chars().count());
            self.push(Message::Clipboard { text: current }).await;
        }
    }

    async fn get_clipboard_image(&self) -> Option<Vec<u8>> {
        let output = match self.backend {
            Backend::Wayland => Command::new("wl-paste").args(["-t", "image/png"]).output().await,
            Backend::X11 => {
                Command::new("xclip")
                    .args(["-selection", "clipboard", "-t", "image/png", "-o"])
                    .output()
                    .await
            }
            Backend::None => return None,
        };
        match output {
            Ok(o) if o.status.success() && !o.stdout.is_empty() => Some(o.stdout),
            _ => None,
        }
    }

    async fn offer_clipboard_image(&self, bytes: Vec<u8>) {
        let Some(pending) = self.pending.lock().await.clone() else { return };
        let tmp = std::env::temp_dir().join(format!("linuxlink-clip-{}.png", self.next_id.fetch_add(1, Ordering::Relaxed)));
        if tokio::fs::write(&tmp, &bytes).await.is_err() {
            return;
        }
        if let Ok((id, name, size)) = pending.offer(tmp).await {
            tracing::info!("🖼 PC clipboard image → phone ({size} bytes)");
            self.push(Message::FileOffer { id, name, size, clipboard: true }).await;
        }
    }

    pub async fn set_image(&self, bytes: &[u8]) {
        *self.last_image_hash.lock().await = hex::encode(Sha256::digest(bytes));
        let child = match self.backend {
            Backend::Wayland => Command::new("wl-copy").args(["-t", "image/png"]).stdin(Stdio::piped()).spawn(),
            Backend::X11 => Command::new("xclip")
                .args(["-selection", "clipboard", "-t", "image/png", "-i"])
                .stdin(Stdio::piped())
                .spawn(),
            Backend::None => return,
        };
        if let Ok(mut c) = child {
            if let Some(mut stdin) = c.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(bytes).await;
            }
            let _ = c.wait().await;
        }
    }

    async fn get_clipboard(&self) -> Option<String> {
        let output = match self.backend {
            Backend::Wayland => Command::new("wl-paste").arg("-n").output().await,
            Backend::X11 => {
                Command::new("xclip").args(["-selection", "clipboard", "-o"]).output().await
            }
            Backend::None => return None,
        };
        match output {
            Ok(o) if o.status.success() => String::from_utf8(o.stdout).ok(),
            _ => None,
        }
    }

    async fn set_clipboard(&self, text: &str) {
        let child = match self.backend {
            Backend::Wayland => Command::new("wl-copy").stdin(Stdio::piped()).spawn(),
            Backend::X11 => Command::new("xclip")
                .args(["-selection", "clipboard", "-i"])
                .stdin(Stdio::piped())
                .spawn(),
            Backend::None => return,
        };
        match child {
            Ok(mut c) => {
                if let Some(mut stdin) = c.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(text.as_bytes()).await;
                }
                let _ = c.wait().await;
            }
            Err(e) => tracing::warn!("could not set clipboard: {e}"),
        }
    }
}

fn detect_backend() -> Backend {
    let has = |cmd: &str| {
        std::process::Command::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if std::env::var("WAYLAND_DISPLAY").is_ok() && has("wl-paste") {
        Backend::Wayland
    } else if std::env::var("DISPLAY").is_ok() && has("xclip") {
        Backend::X11
    } else {
        Backend::None
    }
}
