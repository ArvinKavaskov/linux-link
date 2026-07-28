use crate::protocol::SyncEntry;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub fn folder_root(folder: &str) -> Option<PathBuf> {
    let dir = match folder {
        "Download" => dirs::download_dir(),
        "Documents" => dirs::document_dir(),
        "Pictures" => dirs::picture_dir(),
        _ => dirs::home_dir().map(|h| h.join("LinuxLink")),
    }?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

fn baseline_path(folder: &str) -> PathBuf {
    let safe: String = folder.chars().filter(|c| c.is_alphanumeric()).collect();
    let name = if safe.is_empty() { "LinuxLink".to_string() } else { safe };
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("linux-link")
        .join(format!("sync-baseline-{name}.json"))
}

#[derive(Serialize, Deserialize, Default)]
struct Baseline {
    files: HashMap<String, BaseEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct BaseEntry {
    size: u64,
    pc_mtime: i64,
    phone_mtime: i64,
}

impl Baseline {
    fn load(folder: &str) -> Self {
        std::fs::read_to_string(baseline_path(folder))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self, folder: &str) {
        if let Ok(j) = serde_json::to_string(self) {
            let p = baseline_path(folder);
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(p, j);
        }
    }
}

pub fn scan(root: &Path) -> HashMap<String, SyncEntry> {
    let mut out = HashMap::new();
    walk(root, root, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut HashMap<String, SyncEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if let Ok(ft) = e.file_type() {
            if ft.is_dir() {
                walk(root, &path, out);
            } else if ft.is_file() {
                if let Ok(meta) = e.metadata() {
                    let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    out.insert(rel.clone(), SyncEntry { path: rel, size: meta.len(), mtime });
                }
            }
        }
    }
}

#[derive(Default)]
pub struct Plan {
    pub push: Vec<SyncEntry>,
    pub pull: Vec<String>,
    pub del_phone: Vec<String>,
    pub del_pc: Vec<String>,
}

fn reconcile(
    pc: &HashMap<String, SyncEntry>,
    phone: &HashMap<String, SyncEntry>,
    base: &Baseline,
) -> Plan {
    let mut plan = Plan::default();
    let mut keys: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
    keys.extend(pc.keys());
    keys.extend(phone.keys());
    keys.extend(base.files.keys());

    for k in keys {
        let p = pc.get(k);
        let h = phone.get(k);
        let b = base.files.get(k);

        let pc_changed = match (p, b) {
            (Some(p), Some(b)) => p.size != b.size || p.mtime != b.pc_mtime,
            (Some(_), None) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        };
        let phone_changed = match (h, b) {
            (Some(h), Some(b)) => h.size != b.size || h.mtime != b.phone_mtime,
            (Some(_), None) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        };

        match (p, h) {
            (Some(p), Some(h)) => {
                if pc_changed && phone_changed {
                    if p.mtime >= h.mtime {
                        plan.push.push(p.clone());
                    } else {
                        plan.pull.push(k.clone());
                    }
                } else if pc_changed {
                    plan.push.push(p.clone());
                } else if phone_changed {
                    plan.pull.push(k.clone());
                }
            }
            (Some(p), None) => {
                if b.is_some() && !pc_changed {
                    plan.del_pc.push(k.clone());
                } else {
                    plan.push.push(p.clone());
                }
            }
            (None, Some(_)) => {
                if b.is_some() && !phone_changed {
                    plan.del_phone.push(k.clone());
                } else {
                    plan.pull.push(k.clone());
                }
            }
            (None, None) => {}
        }
    }
    plan
}

pub async fn handle(
    reader: &mut BufReader<quinn::RecvStream>,
    send: &mut quinn::SendStream,
    folder: &str,
    phone_index: Vec<SyncEntry>,
) -> Result<()> {
    let Some(root) = folder_root(folder) else {
        anyhow::bail!("unknown sync folder: {folder}");
    };
    let label = if folder.is_empty() { "LinuxLink" } else { folder };
    let pc_index = scan(&root);
    let phone: HashMap<String, SyncEntry> =
        phone_index.into_iter().map(|e| (e.path.clone(), e)).collect();
    let base = Baseline::load(folder);

    let plan = reconcile(&pc_index, &phone, &base);
    if !plan.push.is_empty() || !plan.pull.is_empty() || !plan.del_phone.is_empty() || !plan.del_pc.is_empty() {
        tracing::info!(
            "🔄 [{label}] {} to send, {} to receive, {} to delete on phone, {} to delete on PC",
            plan.push.len(), plan.pull.len(), plan.del_phone.len(), plan.del_pc.len()
        );
    }

    for rel in &plan.del_pc {
        let _ = std::fs::remove_file(root.join(rel));
    }

    let plan_msg = serde_json::json!({
        "type": "sync_plan",
        "pull": plan.pull,
        "push": plan.push,
        "del_phone": plan.del_phone,
    });
    write_line(send, &plan_msg.to_string()).await?;

    for entry in &plan.push {
        let path = root.join(&entry.path);
        let Ok(mut file) = tokio::fs::File::open(&path).await else { continue };
        let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
        let hdr = serde_json::json!({"type":"sync_file","path":entry.path,"size":size,"mtime":entry.mtime});
        write_line(send, &hdr.to_string()).await?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut remaining = size;
        while remaining > 0 {
            let n = file.read(&mut buf).await?;
            if n == 0 { break; }
            send.write_all(&buf[..n]).await?;
            remaining = remaining.saturating_sub(n as u64);
        }
    }
    write_line(send, "{\"type\":\"sync_push_end\"}").await?;

    loop {
        let line = read_line(reader).await?;
        let v: serde_json::Value = serde_json::from_str(&line)?;
        match v["type"].as_str() {
            Some("sync_file") => {
                let path = v["path"].as_str().unwrap_or("").to_string();
                let size = v["size"].as_u64().unwrap_or(0);
                let mut buf = vec![0u8; size as usize];
                reader.read_exact(&mut buf).await?;
                if safe_rel(&path) {
                    let dest = root.join(&path);
                    if let Some(parent) = dest.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    let _ = tokio::fs::write(&dest, &buf).await;
                }
            }
            Some("sync_pull_end") => break,
            _ => break,
        }
    }

    let line = read_line(reader).await?;
    let v: serde_json::Value = serde_json::from_str(&line)?;
    let final_phone: Vec<SyncEntry> = serde_json::from_value(v["files"].clone()).unwrap_or_default();
    let phone_final: HashMap<String, SyncEntry> =
        final_phone.into_iter().map(|e| (e.path.clone(), e)).collect();
    let pc_final = scan(&root);

    let mut new_base = Baseline::default();
    for (k, p) in &pc_final {
        if let Some(h) = phone_final.get(k) {
            new_base.files.insert(k.clone(), BaseEntry { size: p.size, pc_mtime: p.mtime, phone_mtime: h.mtime });
        }
    }
    new_base.save(folder);

    write_line(send, "{\"type\":\"sync_done\"}").await?;
    let _ = send.finish();
    Ok(())
}

fn safe_rel(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.split('/').any(|c| c == ".." || c == ".")
}

async fn write_line(send: &mut quinn::SendStream, line: &str) -> Result<()> {
    send.write_all(line.as_bytes()).await?;
    send.write_all(b"\n").await?;
    send.flush().await?;
    Ok(())
}

async fn read_line(reader: &mut BufReader<quinn::RecvStream>) -> Result<String> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        anyhow::bail!("sync stream closed");
    }
    Ok(line.trim().to_string())
}
