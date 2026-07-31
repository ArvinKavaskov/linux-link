use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const ALPN: &[u8] = b"linuxlink/1";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    PairRequest {
        version: u32,
        token: String,
        device_name: String,
    },
    PairOk { device_name: String },
    PairRejected { reason: String },
    Ping { seq: u64, sent_at_ms: u64 },
    Pong { seq: u64, sent_at_ms: u64 },
    Hello { version: u32, device_name: String },
    HelloOk { device_name: String },
    NotTrusted,
    Notification {
        key: String,
        app: String,
        title: String,
        body: String,
        #[serde(default)]
        can_reply: bool,
    },
    NotificationDismissed { key: String },
    Clipboard { text: String },
    NotificationReply { key: String, text: String },
    Handoff { url: String, title: String },
    OpenUrl { url: String, title: String },
    PcVolume {
        action: String,
        #[serde(default)]
        value: i32,
    },
    PhoneVolume {
        action: String,
        #[serde(default)]
        value: i32,
    },
    PcMedia { action: String },
    PhoneMedia { action: String },
    MediaInfo {
        title: String,
        artist: String,
        playing: bool,
    },
    Subscribe,
    FileStart {
        name: String,
        size: u64,
        #[serde(default)]
        clipboard: bool,
    },
    FilePull { id: String },
    FileOffer {
        id: String,
        name: String,
        size: u64,
        #[serde(default)]
        clipboard: bool,
    },
    Battery { level: i32, charging: bool },
    Dnd { on: bool },
    WebcamStart { width: u32, height: u32 },
    MicStart { sample_rate: u32, channels: u32 },
    SpeakerStart { sample_rate: u32, channels: u32 },
    SyncIndex {
        #[serde(default)]
        folder: String,
        files: Vec<SyncEntry>,
    },
    Ok,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SyncEntry {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

impl Message {
    pub fn to_line(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec(self).expect("JSON serialization");
        v.push(b'\n');
        v
    }
}
