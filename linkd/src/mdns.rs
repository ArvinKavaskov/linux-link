use crate::identity::Identity;
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};

pub struct MdnsGuard {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for MdnsGuard {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

pub fn advertise(identity: &Identity, port: u16) -> Result<MdnsGuard> {
    let daemon = ServiceDaemon::new()?;
    let service_type = "_linuxlink._udp.local.";
    let instance = identity.device_name.clone();
    let host = format!("{}.local.", identity.device_name);

    let props = [
        ("v", "1"),
        ("fp", &identity.fingerprint()[..]),
    ];

    let info = ServiceInfo::new(service_type, &instance, &host, (), port, &props[..])?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info)?;
    tracing::info!("mDNS advertisement active ({fullname})");
    Ok(MdnsGuard { daemon, fullname })
}
