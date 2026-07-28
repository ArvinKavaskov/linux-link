use anyhow::{Context, Result};
use rand::Rng;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct PendingFiles {
    map: Mutex<HashMap<String, PathBuf>>,
}

impl PendingFiles {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn offer(&self, path: PathBuf) -> Result<(String, String, u64)> {
        let meta = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("file not found: {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let id: String = {
            let bytes: [u8; 8] = rand::thread_rng().gen();
            hex::encode(bytes)
        };
        self.map.lock().await.insert(id.clone(), path);
        Ok((id, name, meta.len()))
    }

    pub async fn take(&self, id: &str) -> Option<PathBuf> {
        self.map.lock().await.remove(id)
    }
}

pub fn download_dir() -> PathBuf {
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(if dirs::download_dir().is_some() { "" } else { "Downloads" });
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn unique_dest(name: &str) -> PathBuf {
    let dir = download_dir();
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    for i in 1..10_000 {
        let c = dir.join(format!("{stem} ({i}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    candidate
}
