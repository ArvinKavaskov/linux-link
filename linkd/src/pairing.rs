use crate::identity::Identity;
use anyhow::Result;
use qrcode::render::unicode;
use qrcode::QrCode;
use rand::Rng;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

pub fn new_token() -> String {
    let bytes: [u8; 16] = rand::thread_rng().gen();
    hex::encode(bytes)
}

#[derive(Serialize)]
struct QrPayload<'a> {
    v: u32,
    name: &'a str,
    addrs: Vec<String>,
    port: u16,
    fp: String,
    token: &'a str,
}

pub fn payload_json(identity: &Identity, port: u16, token: &str) -> Result<String> {
    let payload = QrPayload {
        v: crate::protocol::PROTOCOL_VERSION,
        name: &identity.device_name,
        addrs: local_addresses(),
        port,
        fp: identity.fingerprint(),
        token,
    };
    Ok(serde_json::to_string(&payload)?)
}

pub fn print_qr_json(json: &str) -> Result<()> {
    let code = QrCode::new(json.as_bytes())?;
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();
    println!("\n=== Pairing mode ===\n");
    println!("{image}\n");
    println!("Scan this QR code with the Linux Link app on your phone.\n");
    Ok(())
}

pub fn print_qr(identity: &Identity, port: u16, token: &str) -> Result<()> {
    let json = payload_json(identity, port, token)?;
    print_qr_json(&json)?;
    println!("(The code is only valid while this daemon is running.)\n");
    println!("Manual entry if needed:");
    println!("  token       : {token}");
    println!("  fingerprint : {}\n", identity.fingerprint());
    Ok(())
}

fn local_addresses() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(ip) = local_ip_address::local_ip() {
        out.push(ip.to_string());
    }
    out
}

pub struct Pairing {
    token: Mutex<Option<String>>,
    events: broadcast::Sender<String>,
}

impl Pairing {
    pub fn new(initial: Option<String>) -> Arc<Self> {
        let (events, _) = broadcast::channel(4);
        Arc::new(Self {
            token: Mutex::new(initial),
            events,
        })
    }

    pub async fn current(&self) -> Option<String> {
        self.token.lock().await.clone()
    }

    pub async fn begin(&self, token: String) {
        *self.token.lock().await = Some(token);
    }

    pub async fn end(&self) {
        *self.token.lock().await = None;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.events.subscribe()
    }

    pub fn notify_paired(&self, name: &str) {
        let _ = self.events.send(name.to_string());
    }
}
