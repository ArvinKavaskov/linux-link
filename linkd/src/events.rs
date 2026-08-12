
use tokio::sync::watch;

static BUS: std::sync::OnceLock<(watch::Sender<u64>, watch::Receiver<u64>)> =
    std::sync::OnceLock::new();

fn bus() -> &'static (watch::Sender<u64>, watch::Receiver<u64>) {
    BUS.get_or_init(|| watch::channel(0))
}

pub fn poke() {
    bus().0.send_modify(|v| *v = v.wrapping_add(1));
}

pub fn subscribe() -> watch::Receiver<u64> {
    bus().1.clone()
}
