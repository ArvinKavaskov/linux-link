//! Tells us the moment the clipboard changes, without polling.
//!
//! v2 ran `wl-paste` twice every two seconds — around 1.0 process spawns per
//! second doing nothing but confirming that nothing had happened. That single
//! loop was the largest share of the daemon's idle CPU.
//!
//! Both display servers can tell us instead of being asked:
//!
//!  * **Wayland** — `wl-paste --watch` keeps one long-lived process that prints
//!    a line each time the selection owner changes. One process, asleep.
//!  * **X11** — the XFIXES extension delivers `SelectionNotify` events for the
//!    CLIPBOARD selection. One socket, asleep.
//!
//! Either way the result is the same: zero work while the user is not copying
//! anything, and a wake-up within a millisecond when they are.

use tokio::sync::mpsc;

/// A tick means "the clipboard changed, go and read it".
pub type Ticks = mpsc::Receiver<()>;

/// Starts the right watcher for the current session.
///
/// Returns `None` when neither display server can be reached, in which case the
/// caller should fall back to polling.
pub fn spawn(wayland: bool) -> Option<Ticks> {
    let (tx, rx) = mpsc::channel(1);
    if wayland {
        spawn_wayland(tx)?;
    } else {
        spawn_x11(tx)?;
    }
    Some(rx)
}

// ------------------------------------------------------------------ Wayland

fn spawn_wayland(tx: mpsc::Sender<()>) -> Option<()> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    // `echo` ignores the clipboard content piped to it and prints one empty
    // line per change, which is all we need — the content is fetched properly
    // afterwards so that images and huge pastes go through the normal path.
    let mut child = tokio::process::Command::new("wl-paste")
        .args(["--watch", "echo", ""])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| tracing::warn!("wl-paste --watch unavailable: {e}"))
        .ok()?;

    let stdout = child.stdout.take()?;
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(_)) => {
                    // Full channel already means "a read is pending".
                    let _ = tx.try_send(());
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!("wl-paste --watch: {e}");
                    break;
                }
            }
        }
        let _ = child.kill().await;
        tracing::warn!("clipboard watcher stopped — no more clipboard sync");
    });
    tracing::info!("Clipboard: event-driven (wl-paste --watch)");
    Some(())
}

// ---------------------------------------------------------------------- X11

fn spawn_x11(tx: mpsc::Sender<()>) -> Option<()> {
    // x11rb's event loop is blocking, so it gets its own OS thread rather than
    // a tokio task. One thread parked on a socket costs nothing.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();
    std::thread::Builder::new()
        .name("clipwatch-x11".into())
        .spawn(move || match x11_loop(&ready_tx, &tx) {
            Ok(()) => tracing::warn!("X11 clipboard watcher ended"),
            Err(e) => {
                let _ = ready_tx.send(false);
                tracing::warn!("X11 clipboard watcher: {e}");
            }
        })
        .ok()?;

    // Wait for the thread to confirm XFIXES is actually usable before telling
    // the caller it can stop polling. A second is plenty for a local socket.
    match ready_rx.recv_timeout(std::time::Duration::from_secs(1)) {
        Ok(true) => {
            tracing::info!("Clipboard: event-driven (X11 XFIXES)");
            Some(())
        }
        _ => None,
    }
}

fn x11_loop(ready: &std::sync::mpsc::Sender<bool>, tx: &mpsc::Sender<()>) -> anyhow::Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xfixes::{self, ConnectionExt as _, SelectionEventMask};
    use x11rb::protocol::xproto::ConnectionExt as _;
    use x11rb::protocol::Event;

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    // XFIXES has to be version-negotiated before any of its requests work.
    xfixes::query_version(&conn, 5, 0)?.reply()?;

    let clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    conn.xfixes_select_selection_input(
        root,
        clipboard,
        SelectionEventMask::SET_SELECTION_OWNER
            | SelectionEventMask::SELECTION_WINDOW_DESTROY
            | SelectionEventMask::SELECTION_CLIENT_CLOSE,
    )?;
    conn.flush()?;
    let _ = ready.send(true);

    loop {
        match conn.wait_for_event()? {
            Event::XfixesSelectionNotify(_) => {
                let _ = tx.try_send(());
            }
            _ => continue,
        }
    }
}
