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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct Advertiser {
    identity: Arc<Identity>,
    port: u16,
    use_ble: bool,
    mdns: Mutex<Option<crate::mdns::MdnsGuard>>,
    ble: Mutex<Option<crate::ble::AdvertisingGuard>>,
    /// Bumped by every republish. A pending BLE retry loop quits the moment it
    /// no longer matches, so at most one retry loop is ever doing anything.
    generation: AtomicU64,
}

impl Advertiser {
    pub async fn start(identity: Arc<Identity>, port: u16, use_ble: bool) -> Arc<Self> {
        let me = Arc::new(Self {
            identity,
            port,
            use_ble,
            mdns: Mutex::new(None),
            ble: Mutex::new(None),
            generation: AtomicU64::new(0),
        });
        me.republish("startup").await;
        me
    }

    /// Drops the current advertisements and publishes fresh ones.
    pub async fn republish(self: &Arc<Self>, why: &str) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut slot = self.mdns.lock().await;
            *slot = None; // Drop unregisters the old record first.
            match crate::mdns::advertise(&self.identity, self.port) {
                Ok(g) => *slot = Some(g),
                Err(e) => tracing::warn!("mDNS unavailable: {e:#}"),
            }
        }
        if self.use_ble && !self.try_ble().await {
            // The classic failure is a race at login: linkd is up before
            // bluetoothd has finished bringing the controller up, the
            // RegisterAdvertisement call fails, and without a retry BLE would
            // stay dead until the next suspend or IP change. So keep knocking.
            tracing::warn!("BLE advertisement failed — will keep retrying; mDNS/UDP still work");
            self.spawn_ble_retry(generation);
        }
        tracing::info!("Advertisements published ({why})");
    }

    /// One BLE registration attempt. Holds the slot lock so an attempt and a
    /// concurrent republish can never interleave their guard swaps.
    async fn try_ble(&self) -> bool {
        let mut slot = self.ble.lock().await;
        *slot = None;
        match crate::ble::advertise(self.port).await {
            Ok(g) => {
                *slot = Some(g);
                true
            }
            Err(e) => {
                tracing::debug!("BLE attempt failed: {e:#}");
                false
            }
        }
    }

    /// Retries the BLE advertisement with a growing delay (2 s → 60 s), for as
    /// long as this republish generation is the current one. Costs one D-Bus
    /// call per attempt; a Bluetooth adapter that shows up an hour later still
    /// gets picked up.
    fn spawn_ble_retry(self: &Arc<Self>, generation: u64) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut wait = 2u64;
            loop {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                if me.generation.load(Ordering::SeqCst) != generation {
                    return; // A newer republish owns the slot now.
                }
                if me.try_ble().await {
                    tracing::info!("BLE advertisement up after retry");
                    return;
                }
                wait = (wait * 2).min(60);
            }
        });
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
