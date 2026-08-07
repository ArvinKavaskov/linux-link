mod battery;
mod ble;
mod clipboard;
mod clipwatch;
mod control;
mod discovery;
mod display;
mod dnd;
mod events;
mod files;
mod identity;
mod lock;
mod mdns;
mod media;
mod mic;
mod netwatch;
mod notifications;
mod pairing;
mod protocol;
mod server;
mod shortcuts;
mod speaker;
mod status;
mod sync;
mod volume;
mod webcam;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

pub const SERVICE_UUID: &str = "4c4c0001-6c69-6e75-786c-696e6b000001";
pub const DEFAULT_PORT: u16 = 47100;

#[derive(Parser)]
#[command(name = "linkd", version, about = "Linux Link daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Run {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        no_ble: bool,
    },
    Pair {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        no_ble: bool,
    },
    Status,
    SendUrl {
        url: Option<String>,
        #[arg(long, default_value = "")]
        title: String,
    },
    PhoneVolume {
        action: String,
        value: Option<i32>,
    },
    Media { action: String },
    SendFile {
        /// Omit it and pass `--pick` to choose the file in a dialog instead.
        path: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Opens the desktop's file chooser. This is what the keyboard
        /// shortcut uses: a shortcut cannot type a path.
        #[arg(long)]
        pick: bool,
    },
    PairLive,
    /// Asks the tablet to become a second screen, without touching it.
    Screen {
        /// Fingerprint of one device; omit it to offer the screen to every
        /// connected device.
        #[arg(long)]
        to: Option<String>,
    },
    ProximityLock { state: String },
    Battery,
    /// Removes a paired device (full fingerprint or the short form shown by
    /// `linkd status`).
    Forget { fingerprint: String },
    /// Global keyboard shortcuts: `install`, `remove` or `status`.
    Shortcuts { action: Option<String> },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run { port: DEFAULT_PORT, no_ble: false }) {
        Command::Run { port, no_ble } => run(port, no_ble, false).await,
        Command::Pair { port, no_ble } => run(port, no_ble, true).await,
        Command::Status => status(),
        Command::SendUrl { url, title } => send_url(url, title).await,
        Command::PhoneVolume { action, value } => {
            control::phone_volume(&action, value.unwrap_or(0)).await?;
            println!("Phone volume: {action}");
            Ok(())
        }
        Command::Media { action } => {
            control::phone_media(&action).await?;
            println!("Phone media: {action}");
            Ok(())
        }
        Command::SendFile { path, to, pick } => send_file(path, to, pick).await,
        Command::PairLive => control::pair_live().await,
        Command::Screen { to } => {
            control::second_screen(to.as_deref()).await?;
            println!("Second screen offered — accept it on the tablet.");
            Ok(())
        }
        Command::ProximityLock { state } => {
            let on = matches!(state.as_str(), "on" | "true" | "1");
            control::proximity_lock(on).await?;
            println!("Proximity lock: {}", if on { "enabled" } else { "disabled" });
            Ok(())
        }
        Command::Battery => {
            battery::print_last();
            Ok(())
        }
        Command::Forget { fingerprint } => {
            // Through the daemon when it is up, so it can drop the live
            // connection; straight to the file when it is not.
            match control::forget(&fingerprint).await {
                Ok(name) => println!("Device forgotten: {name}"),
                Err(_) => {
                    let mut peers = identity::TrustedPeers::load()?;
                    match peers.forget(&fingerprint)? {
                        Some(peer) => println!("Device forgotten: {}", peer.name),
                        None => anyhow::bail!("no device matches {fingerprint}"),
                    }
                }
            }
            Ok(())
        }
        Command::Shortcuts { action } => {
            let desktop = shortcuts::desktop_name(shortcuts::detect());
            match action.as_deref().unwrap_or("status") {
                "install" | "add" => {
                    shortcuts::install()?;
                    println!("Keyboard shortcuts registered in {desktop}:");
                    print!("{}", shortcuts::manual_help());
                    Ok(())
                }
                "remove" | "uninstall" => {
                    shortcuts::remove()?;
                    println!("Keyboard shortcuts removed from {desktop}.");
                    Ok(())
                }
                "status" => {
                    println!("Desktop   : {desktop}");
                    println!(
                        "Shortcuts : {}",
                        if shortcuts::installed() { "installed" } else { "not installed" }
                    );
                    print!("{}", shortcuts::manual_help());
                    Ok(())
                }
                other => anyhow::bail!("unknown action: {other} (install, remove or status)"),
            }
        }
    }
}

/// Sends one or more files to the phone, either by path or through the
/// desktop's file chooser.
async fn send_file(path: Option<String>, to: Option<String>, pick: bool) -> Result<()> {
    let paths: Vec<String> = match (path, pick) {
        (Some(p), _) => vec![p],
        (None, true) => pick_files()?,
        (None, false) => anyhow::bail!("give a file path, or --pick to choose one"),
    };
    if paths.is_empty() {
        return Ok(()); // The user cancelled the dialog — not an error.
    }
    for p in paths {
        let abs = std::fs::canonicalize(&p).with_context(|| format!("file not found: {p}"))?;
        control::send_file(abs.to_string_lossy().as_ref(), to.as_deref()).await?;
        println!("Offered to phone: {p}");
    }
    Ok(())
}

/// Opens whatever file chooser the desktop provides. Returns an empty list when
/// the user cancels, which every one of these signals with a non-zero exit.
fn pick_files() -> Result<Vec<String>> {
    let candidates: [(&str, &[&str]); 3] = [
        ("zenity", &["--file-selection", "--multiple", "--separator=\n", "--title=Send to phone"]),
        ("kdialog", &["--getopenfilename", "--multiple", "--separate-output"]),
        ("qarma", &["--file-selection", "--multiple", "--separator=\n", "--title=Send to phone"]),
    ];
    for (cmd, args) in candidates {
        let out = match std::process::Command::new(cmd).args(args).output() {
            Ok(o) => o,
            Err(_) => continue, // Not installed; try the next one.
        };
        if !out.status.success() {
            return Ok(Vec::new()); // Cancelled.
        }
        return Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect());
    }
    anyhow::bail!("no file chooser found — install zenity or kdialog")
}

async fn send_url(url: Option<String>, title: String) -> Result<()> {
    let url = match url {
        Some(u) => u,
        None => read_clipboard().unwrap_or_default(),
    };
    let url = url.trim();
    if url.is_empty() {
        anyhow::bail!("no URL provided (neither as an argument nor in the clipboard)");
    }
    control::send_url(url, title.trim()).await?;
    println!("Sent to phone: {url}");
    Ok(())
}

fn read_clipboard() -> Option<String> {
    let try_cmd = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd).args(args).output().ok().and_then(|o| {
            if o.status.success() { String::from_utf8(o.stdout).ok() } else { None }
        })
    };
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        try_cmd("wl-paste", &["-n"])
    } else {
        try_cmd("xclip", &["-selection", "clipboard", "-o"])
    }
}

async fn run(port: u16, no_ble: bool, pairing_mode: bool) -> Result<()> {
    let identity = std::sync::Arc::new(identity::Identity::load_or_create()?);
    tracing::info!(
        "Identity: {} (fingerprint {})",
        identity.device_name,
        &identity.fingerprint()[..16]
    );

    let initial_token = if pairing_mode {
        let token = pairing::new_token();
        pairing::print_qr(&identity, port, &token)?;
        Some(token)
    } else {
        None
    };
    let pairing = pairing::Pairing::new(initial_token);

    // mDNS + BLE, held so they can be republished after a suspend or an IP
    // change — that used to be the number one cause of "I had to re-pair".
    let advertiser = netwatch::Advertiser::start(identity.clone(), port, !no_ble).await;
    netwatch::spawn(advertiser);

    // Answers the phone's broadcast probe. This is what finds the PC again in
    // milliseconds when the router hands out a new lease.
    discovery::spawn(identity.clone(), port);

    let clipboard = clipboard::ClipboardHub::new();
    let pending_files = files::PendingFiles::new();
    clipboard.set_pending(pending_files.clone()).await;
    let battery = battery::BatteryStore::new();
    let dnd_sync = dnd::DndSync::new();
    let notifier = notifications::Notifier::new(clipboard.clone()).await;
    let proximity = lock::ProximityLock::new();
    if proximity.is_enabled() {
        tracing::info!("Proximity lock active (phone presence)");
    }
    let fast_presence = proximity.is_enabled();
    control::serve(
        clipboard.clone(),
        pending_files.clone(),
        proximity.clone(),
        identity.clone(),
        port,
        pairing.clone(),
    );
    media::spawn_watcher(clipboard.clone());
    status::spawn(clipboard.clone(), battery.clone(), proximity.clone());
    lock::spawn(clipboard.clone(), proximity);
    dnd::spawn_watcher(clipboard.clone(), dnd_sync.clone());
    server::serve(
        server::Services {
            identity,
            pairing,
            notifier,
            clipboard,
            pending: pending_files,
            battery,
            dnd: dnd_sync,
        },
        port,
        fast_presence,
    )
    .await
}

fn status() -> Result<()> {
    let identity = identity::Identity::load_or_create()?;
    println!("This PC      : {}", identity.device_name);
    println!("Fingerprint  : {}", identity.fingerprint());
    let peers = identity::TrustedPeers::load()?;
    if peers.peers.is_empty() {
        println!("No paired devices. Run `linkd pair` to pair a phone.");
    } else {
        println!("Paired devices:");
        for p in &peers.peers {
            println!("  - {} ({})", p.name, &p.fingerprint[..16]);
        }
    }
    Ok(())
}
