use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct BatteryState {
    pub level: i32,
    pub charging: bool,
}

pub struct BatteryStore {
    last: Mutex<Option<BatteryState>>,
    by_device: Mutex<std::collections::HashMap<String, BatteryState>>,
}

impl BatteryStore {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            last: Mutex::new(load()),
            by_device: Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn update(&self, fingerprint: &str, level: i32, charging: bool) {
        let state = BatteryState { level, charging };
        *self.last.lock().unwrap() = Some(state);
        if !fingerprint.is_empty() {
            self.by_device
                .lock()
                .unwrap()
                .insert(fingerprint.to_string(), state);
        }
        save(&state);
        crate::events::poke();
    }

    pub fn snapshot(&self) -> Option<BatteryState> {
        *self.last.lock().unwrap()
    }

    pub fn of(&self, fingerprint: &str) -> Option<BatteryState> {
        self.by_device.lock().unwrap().get(fingerprint).copied()
    }
}

fn state_file() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("linux-link");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("battery.json")
}

fn load() -> Option<BatteryState> {
    std::fs::read_to_string(state_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save(state: &BatteryState) {
    if let Ok(json) = serde_json::to_string(state) {
        let _ = std::fs::write(state_file(), json);
    }
}

pub fn print_last() {
    match load() {
        Some(s) => {
            let icon = if s.charging { "⚡" } else { "🔋" };
            println!("{icon} Phone: {}%{}", s.level, if s.charging { " (charging)" } else { "" });
        }
        None => println!("Battery level unknown (the phone has not connected yet)."),
    }
}
