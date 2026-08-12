
use anyhow::{Context, Result};
use bluer::adv::Advertisement;

pub struct AdvertisingGuard {
    _handle: bluer::adv::AdvertisementHandle,
    _session: bluer::Session,
}

pub async fn advertise(_port: u16) -> Result<AdvertisingGuard> {
    let session = bluer::Session::new().await.context("BlueZ session")?;
    let adapter = session.default_adapter().await.context("Bluetooth adapter")?;
    adapter.set_powered(true).await?;

    let uuid: bluer::Uuid = crate::SERVICE_UUID.parse().context("invalid service UUID")?;
    let adv = Advertisement {
        advertisement_type: bluer::adv::Type::Peripheral,
        service_uuids: vec![uuid].into_iter().collect(),
        discoverable: Some(true),
        ..Default::default()
    };
    let handle = adapter.advertise(adv).await.context("registering the BLE advertisement")?;
    tracing::info!("BLE advertisement up (service UUID — the phone wakes on it)");
    Ok(AdvertisingGuard { _handle: handle, _session: session })
}
