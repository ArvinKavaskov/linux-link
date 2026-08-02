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

/// Every address this PC can plausibly be reached on, best guess first.
///
/// We used to hand the phone a single IPv4. On a machine with both Ethernet
/// and Wi-Fi up — or on a laptop that gets a new lease five minutes later —
/// that one address is a coin flip. Giving the phone the whole list costs a
/// few bytes in the QR code and saves a re-pairing.
fn local_addresses() -> Vec<String> {
    // Real, phone-reachable interfaces only. A Docker bridge or a VPN tunnel
    // has an address too, and with the hard cap below it would evict the LAN
    // address from the QR — pairing then fails with the PC one metre away.
    let mut reachable: Vec<String> = Vec::new();
    if let Ok(list) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in list {
            if name == "lo" || ip.is_loopback() || !ip.is_ipv4() || is_virtual_interface(&name) {
                continue;
            }
            let s = ip.to_string();
            if !reachable.contains(&s) {
                reachable.push(s);
            }
        }
    }

    // local_ip() follows the default route, so when it points at one of our
    // reachable interfaces it is the best first guess. When it does not (VPN
    // holding the default route), it would send the phone into a tunnel it
    // cannot enter — ignore it and trust the interface list.
    if let Ok(primary) = local_ip_address::local_ip() {
        let p = primary.to_string();
        if let Some(pos) = reachable.iter().position(|a| a == &p) {
            reachable.swap(0, pos);
        }
    }

    // Hard cap: every extra address grows the QR payload, and past ~230 bytes
    // the code jumps a version, the modules shrink and phones stop scanning it.
    // Two is plenty — the UDP beacon handles the rest.
    reachable.truncate(2);
    reachable
}

/// Interfaces a phone on the same Wi-Fi can never reach: container bridges,
/// VM networks, VPN tunnels. Matching by name prefix is what everyone does,
/// because that is all the information there is.
fn is_virtual_interface(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "docker", "br-", "veth", "virbr", "vnet", "tun", "tap", "wg",
        "tailscale", "zt", "vmnet", "vboxnet", "lxcbr", "lxdbr", "podman",
        "cni", "flannel", "ppp",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
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
        // A new device in the trusted list changes the status file.
        crate::events::poke();
    }
}

#[cfg(test)]
mod tests {
    use super::is_virtual_interface;

    #[test]
    fn keeps_real_interfaces_and_drops_virtual_ones() {
        for real in ["enp7s0", "eth0", "wlan0", "wlp3s0", "eno1", "enx089798f1a6c7"] {
            assert!(!is_virtual_interface(real), "{real} wrongly filtered");
        }
        for fake in ["docker0", "br-a1b2c3", "veth12ab", "virbr0", "tun0", "tap0", "wg0", "tailscale0", "vboxnet0", "ppp0"] {
            assert!(is_virtual_interface(fake), "{fake} wrongly kept");
        }
    }
}
