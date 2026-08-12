
use tokio::sync::mpsc;

pub type Ticks = mpsc::Receiver<()>;

pub fn spawn(wayland: bool) -> Option<Ticks> {
    let (tx, rx) = mpsc::channel(1);
    if wayland {
        spawn_wayland(tx)?;
    } else {
        spawn_x11(tx)?;
    }
    Some(rx)
}

fn spawn_wayland(tx: mpsc::Sender<()>) -> Option<()> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

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

fn spawn_x11(tx: mpsc::Sender<()>) -> Option<()> {
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
