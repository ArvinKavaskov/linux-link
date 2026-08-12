
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

    pub async fn republish(self: &Arc<Self>, why: &str) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut slot = self.mdns.lock().await;
            *slot = None;
            match crate::mdns::advertise(&self.identity, self.port) {
                Ok(g) => *slot = Some(g),
                Err(e) => tracing::warn!("mDNS unavailable: {e:#}"),
            }
        }
        if self.use_ble && !self.try_ble().await {
            tracing::warn!("BLE advertisement failed — will keep retrying; mDNS/UDP still work");
            self.spawn_ble_retry(generation);
        }
        tracing::info!("Advertisements published ({why})");
    }

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

    fn spawn_ble_retry(self: &Arc<Self>, generation: u64) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut wait = 2u64;
            loop {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                if me.generation.load(Ordering::SeqCst) != generation {
                    return;
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
