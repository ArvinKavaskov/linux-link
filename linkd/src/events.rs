//! A one-line "something changed" bus.
//!
//! Several background tasks used to re-derive the daemon's state on a timer:
//! the status file was rewritten every two seconds and the proximity lock
//! re-counted subscribers every two seconds, whether or not anything had
//! happened. Both now park here instead and are woken by whatever actually
//! caused the change.
//!
//! A `watch` channel rather than a `Notify` on purpose: `notify_one` wakes a
//! single waiter and `notify_waiters` can be missed by a task that is between
//! two awaits. `watch` marks each receiver's last-seen value, so no consumer
//! can sleep through an update.

use tokio::sync::watch;

static BUS: std::sync::OnceLock<(watch::Sender<u64>, watch::Receiver<u64>)> =
    std::sync::OnceLock::new();

fn bus() -> &'static (watch::Sender<u64>, watch::Receiver<u64>) {
    BUS.get_or_init(|| watch::channel(0))
}

/// A device connected or left, the battery moved, a setting was toggled…
pub fn poke() {
    bus().0.send_modify(|v| *v = v.wrapping_add(1));
}

/// Wait on the returned receiver with `rx.changed().await`.
pub fn subscribe() -> watch::Receiver<u64> {
    bus().1.clone()
}
