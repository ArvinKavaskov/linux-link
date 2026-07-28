use anyhow::{Context, Result};
use bluer::adv::Advertisement;
use std::collections::BTreeMap;
use std::net::IpAddr;

pub struct AdvertisingGuard {
    _handle: bluer::adv::AdvertisementHandle,
    _session: bluer::Session,
}

pub async fn advertise(port: u16) -> Result<AdvertisingGuard> {
    let session = bluer::Session::new().await.context("BlueZ session")?;
    let adapter = session.default_adapter().await.context("Bluetooth adapter")?;
    adapter.set_powered(true).await?;

    let uuid: bluer::Uuid = crate::SERVICE_UUID.parse().context("invalid service UUID")?;

    let mut service_data: BTreeMap<bluer::Uuid, Vec<u8>> = BTreeMap::new();
    service_data.insert(uuid, service_payload(port));
    let full = Advertisement {
        advertisement_type: bluer::adv::Type::Peripheral,
        service_uuids: vec![uuid].into_iter().collect(),
        service_data,
        discoverable: Some(true),
        ..Default::default()
    };
    match adapter.advertise(full).await {
        Ok(handle) => {
            tracing::info!("Full BLE advertisement (UUID + IP:port in the service data)");
            return Ok(AdvertisingGuard { _handle: handle, _session: session });
        }
        Err(e) => {
            tracing::warn!("Full BLE advertisement rejected ({e}) — falling back to the minimal advertisement");
        }
    }

    let minimal = Advertisement {
        advertisement_type: bluer::adv::Type::Peripheral,
        service_uuids: vec![uuid].into_iter().collect(),
        discoverable: Some(true),
        ..Default::default()
    };
    let handle = adapter
        .advertise(minimal)
        .await
        .context("starting the minimal BLE advertisement")?;
    tracing::info!("Minimal BLE advertisement (UUID only — the app will use the last known IP/mDNS)");
    Ok(AdvertisingGuard { _handle: handle, _session: session })
}

fn service_payload(port: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(6);
    if let Ok(IpAddr::V4(ip)) = local_ip_address::local_ip() {
        data.extend_from_slice(&ip.octets());
    } else {
        data.extend_from_slice(&[0, 0, 0, 0]);
    }
    data.extend_from_slice(&port.to_be_bytes());
    data
}
