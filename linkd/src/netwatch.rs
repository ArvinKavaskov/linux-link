//! Keeps the PC findable, whatever the machine or the network does.
//!
//! Two things used to break the link permanently:
//!
//!   * **Suspend.** BlueZ drops advertisements across a sleep cycle and the
//!     mDNS daemon's socket comes back on a different interface state, so after
//!     a lid-close the PC was still listening but had stopped saying so.
//!   * **A new IP.** The BLE service data and the mDNS record both embed the
//!     address; a DHCP lease change made both of them lie.
//!
//! So we hold the two advertisement guards here and re-publish them whenever
//! logind tells us we just woke up, or whenever the primary IPv4 changes.
//!
//! Cost when idle: one D-Bus signal subscription (zero wakeups) plus a
//! `getifaddrs` call every 15 s, which is a few microseconds of user time.

use crate::identity::Identity;
use futures_util::StreamExt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct Advertiser {
    identity: Arc<Identity>,
    port: u16,
    use_ble: bool,
    mdns: Mutex<Option<crate::mdns::MdnsGuard>>,
    ble: Mutex<Option<crate::ble::AdvertisingGuard>>,
}

impl Advertiser {
    pub async fn start(identity: Arc<Identity>, port: u16, use_ble: bool) -> Arc<Self> {
        let me = Arc::new(Self {
            identity,
            port,
            use_ble,
            mdns: Mutex::new(None),
            ble: Mutex::new(None),
        });
        me.republish("startup").await;
        me
    }

    /// Drops the current advertisements and publishes fresh ones.
    pub async fn republish(&self, why: &str) {
        {
            let mut slot = self.mdns.lock().await;
            *slot = None; // Drop unregisters the old record first.
            match crate::mdns::advertise(&self.identity, self.port) {
                Ok(g) => *slot = Some(g),
                Err(e) => tracing::warn!("mDNS unavailable: {e:#}"),
            }
        }
        if self.use_ble {
            let mut slot = self.ble.lock().await;
            *slot = None;
            match crate::ble::advertise(self.port).await {
                Ok(g) => *slot = Some(g),
                Err(e) => tracing::warn!("BLE unavailable ({e:#}) — falling back to mDNS/UDP only"),
            }
        }
        tracing::info!("Advertisements published ({why})");
    }
}

/// Re-advertise on resume from suspend and whenever our IPv4 changes.
pub fn spawn(adv: Arc<Advertiser>) {
    let a = adv.clone();
    tokio::spawn(async move { watch_sleep(a).await });
    tokio::spawn(async move { watch_ip(adv).await });
}

async fn watch_sleep(adv: Arc<Advertiser>) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("system bus unavailable ({e}) — no suspend/resume handling");
            return;
        }
    };
    let proxy = match zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("logind unreachable ({e}) — no suspend/resume handling");
            return;
        }
    };
    let mut stream = match proxy.receive_signal("PrepareForSleep").await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cannot subscribe to PrepareForSleep: {e}");
            return;
        }
    };
    tracing::info!("Suspend/resume handling active (logind)");
    while let Some(sig) = stream.next().await {
        let Ok(going_to_sleep) = sig.body().deserialize::<bool>() else { continue };
        if going_to_sleep {
            tracing::info!("💤 Suspending — advertisements will be republished on resume");
            continue;
        }
        // Give the network stack a moment to bring interfaces back up.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        adv.republish("resume from suspend").await;
    }
}

async fn watch_ip(adv: Arc<Advertiser>) {
    let mut current = primary_ip();
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let now = primary_ip();
        if now == current {
            continue;
        }
        match (&current, &now) {
            (_, Some(ip)) => {
                tracing::info!("Address changed → {ip}, republishing");
                current = now;
                adv.republish("address change").await;
            }
            (_, None) => {
                tracing::info!("Network went away — waiting for it to come back");
                current = None;
            }
        }
    }
}

fn primary_ip() -> Option<IpAddr> {
    local_ip_address::local_ip().ok()
}
