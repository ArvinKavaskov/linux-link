//! BLE presence beacon — the thing that lets Android wake our app.
//!
//! The advertisement carries exactly one fact: the Linux Link service UUID.
//! That is all the CompanionDeviceManager filter on the phone matches on, and
//! it is all that fits: a 128-bit UUID list is 18 bytes, the mandatory flags
//! are 3 more, and a legacy BLE advertisement tops out at 31. Earlier versions
//! also tried to pack the PC's IP and port into service data; that pushed the
//! payload to 45 bytes, every controller rejected it, and nothing on the
//! Android side ever read it — the phone finds our address over UDP/mDNS in
//! milliseconds anyway.

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
