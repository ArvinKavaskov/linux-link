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

fn sanitize_name(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| *c != '\0' && *c != '/').collect();
    match cleaned.as_str() {
        "" | "." | ".." => "file".to_string(),
        _ => cleaned,
    }
}

pub fn unique_dest(name: &str) -> PathBuf {
    let dir = download_dir();
    let name = sanitize_name(name);
    let candidate = dir.join(&name);
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

#[cfg(test)]
mod tests {
    use super::sanitize_name;

    #[test]
    fn network_names_cannot_leave_the_download_directory() {
        assert_eq!(sanitize_name("../../.bashrc"), ".bashrc");
        assert_eq!(sanitize_name("/home/user/.ssh/authorized_keys"), "authorized_keys");
        assert_eq!(sanitize_name("a/b/c.txt"), "c.txt");
        assert_eq!(sanitize_name(""), "file");
        assert_eq!(sanitize_name(".."), "file");
        assert_eq!(sanitize_name("."), "file");
        assert_eq!(sanitize_name("/"), "file");
        assert_eq!(sanitize_name("Photo 2026-08-06.jpg"), "Photo 2026-08-06.jpg");
        assert_eq!(sanitize_name("rapport.final.pdf"), "rapport.final.pdf");
    }
}
