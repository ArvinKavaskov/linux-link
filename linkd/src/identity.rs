use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

pub struct Identity {
    pub device_name: String,
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
}

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("configuration directory not found")?
        .join("linux-link");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl Identity {
    pub fn load_or_create() -> Result<Self> {
        let dir = config_dir()?;
        let cert_path = dir.join("cert.der");
        let key_path = dir.join("key.der");
        let name_path = dir.join("device_name");

        let device_name = if name_path.exists() {
            fs::read_to_string(&name_path)?.trim().to_string()
        } else {
            let name = hostname();
            fs::write(&name_path, &name)?;
            name
        };

        if cert_path.exists() && key_path.exists() {
            return Ok(Self {
                device_name,
                cert_der: fs::read(&cert_path)?,
                key_der: fs::read(&key_path)?,
            });
        }

        tracing::info!("First run: generating identity…");
        let cert = rcgen::generate_simple_self_signed(vec![device_name.clone()])?;
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();
        fs::write(&cert_path, &cert_der)?;
        fs::write(&key_path, &key_der)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self { device_name, cert_der, key_der })
    }

    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.cert_der)
    }
}

pub fn fingerprint_of(cert_der: &[u8]) -> String {
    hex::encode(Sha256::digest(cert_der))
}

fn hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "linux-pc".to_string())
}

#[derive(Serialize, Deserialize, Default)]
pub struct TrustedPeers {
    pub peers: Vec<Peer>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Peer {
    pub name: String,
    pub fingerprint: String,
}

impl TrustedPeers {
    pub fn load() -> Result<Self> {
        let path = config_dir()?.join("peers.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_dir()?.join("peers.json");
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        self.peers.iter().any(|p| p.fingerprint == fingerprint)
    }

    pub fn add(&mut self, peer: Peer) -> Result<()> {
        if !self.is_trusted(&peer.fingerprint) {
            self.peers.push(peer);
            self.save()?;
        }
        Ok(())
    }

    /// Removes a device. Returns the peer it was, if it was there at all — the
    /// caller needs the full fingerprint to also drop a live connection.
    ///
    /// Accepts a fingerprint prefix so the settings window and the command line
    /// can both work with the short form shown to the user.
    pub fn forget(&mut self, fingerprint: &str) -> Result<Option<Peer>> {
        let Some(pos) = self.peers.iter().position(|p| p.fingerprint.starts_with(fingerprint))
        else {
            return Ok(None);
        };
        let peer = self.peers.remove(pos);
        self.save()?;
        Ok(Some(peer))
    }
}
