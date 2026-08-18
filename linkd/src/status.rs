use crate::battery::BatteryStore;
use crate::clipboard::ClipboardHub;
use crate::identity::TrustedPeers;
use crate::lock::ProximityLock;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Serialize)]
pub struct DeviceStatus {
    pub name: String,
    pub fingerprint: String,
    pub connected: bool,
    pub battery: i32,
    pub charging: bool,
}

#[derive(Serialize)]
pub struct Status {
    pub connected: bool,
    pub device_count: usize,
    pub device_name: String,
    pub devices: Vec<DeviceStatus>,
    pub battery: i32,
    pub charging: bool,
    pub proximity: bool,
}

pub fn status_file() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("linux-link");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("status.json")
}

pub fn spawn(
    clipboard: Arc<ClipboardHub>,
    battery: Arc<BatteryStore>,
    proximity: Arc<ProximityLock>,
) {
    tokio::spawn(async move {
        let mut rx = crate::events::subscribe();
        let mut last_json = String::new();
        loop {
            let (level, charging) = match battery.snapshot() {
                Some(s) => (s.level, s.charging),
                None => (-1, false),
            };

            let connected = clipboard.connected().await;
            let connected_fps: std::collections::HashSet<String> =
                connected.iter().map(|(fp, _)| fp.clone()).collect();
            let peers = TrustedPeers::load().map(|t| t.peers).unwrap_or_default();
            let devices: Vec<DeviceStatus> = peers
                .iter()
                .map(|p| {
                    let b = battery.of(&p.fingerprint);
                    DeviceStatus {
                        name: p.name.clone(),
                        fingerprint: p.fingerprint.clone(),
                        connected: connected_fps.contains(&p.fingerprint),
                        battery: b.map(|s| s.level).unwrap_or(-1),
                        charging: b.map(|s| s.charging).unwrap_or(false),
                    }
                })
                .collect();
            let device_count = connected_fps.len();
            let device_name = connected
                .first()
                .map(|(_, n)| n.clone())
                .or_else(|| peers.first().map(|p| p.name.clone()))
                .unwrap_or_default();

            let status = Status {
                connected: device_count > 0,
                device_count,
                device_name,
                devices,
                battery: level,
                charging,
                proximity: proximity.is_enabled(),
            };
            if let Ok(json) = serde_json::to_string(&status) {
                if json != last_json {
                    let _ = tokio::fs::write(status_file(), &json).await;
                    last_json = json;
                }
            }

            let _ = tokio::time::timeout(Duration::from_secs(30), rx.changed()).await;
        }
    });
}
