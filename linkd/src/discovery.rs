//! Tiny UDP beacon so a phone can find this PC again after its IP changed.
//!
//! mDNS is the nice way to do this, but plenty of routers and "guest" or
//! "AP isolation" Wi-Fi profiles drop multicast, and Android's NsdManager can
//! take a couple of seconds to resolve. A one-datagram broadcast probe answers
//! in a few milliseconds and works everywhere broadcast works, which in
//! practice is everywhere the two devices can reach each other at all.
//!
//! Protocol: the phone sends `LINUXLINK?v1` to UDP 47101 (broadcast), we reply
//! with a small JSON object. Nothing secret goes out — the fingerprint is
//! public information, and it is what lets the phone recognise *its* PC among
//! several on the same network.
//!
//! Cost when idle: one socket blocked in `recv_from`. No timer, no wakeups.

use crate::identity::Identity;
use anyhow::Result;
use std::sync::Arc;
use tokio::net::UdpSocket;

pub const DISCOVERY_PORT: u16 = 47101;
const PROBE: &[u8] = b"LINUXLINK?v1";

pub async fn serve(identity: Arc<Identity>, quic_port: u16) -> Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await?;
    socket.set_broadcast(true)?;
    tracing::info!("Discovery beacon listening on UDP {DISCOVERY_PORT}");

    let reply = serde_json::json!({
        "v": 1,
        "name": identity.device_name,
        "fp": identity.fingerprint(),
        "port": quic_port,
    })
    .to_string();

    let mut buf = [0u8; 64];
    loop {
        let (n, from) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("discovery socket: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        if &buf[..n] != PROBE {
            continue;
        }
        tracing::debug!("discovery probe from {from}");
        let _ = socket.send_to(reply.as_bytes(), from).await;
    }
}

pub fn spawn(identity: Arc<Identity>, quic_port: u16) {
    tokio::spawn(async move {
        if let Err(e) = serve(identity, quic_port).await {
            tracing::warn!("discovery beacon unavailable: {e:#}");
        }
    });
}
