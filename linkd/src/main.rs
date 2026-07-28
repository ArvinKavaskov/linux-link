mod battery;
mod ble;
mod clipboard;
mod control;
mod dnd;
mod files;
mod identity;
mod lock;
mod mdns;
mod media;
mod mic;
mod notifications;
mod pairing;
mod protocol;
mod server;
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
        path: String,
        #[arg(long)]
        to: Option<String>,
    },
    PairLive,
    ProximityLock { state: String },
    Battery,
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
        Command::SendFile { path, to } => {
            let abs = std::fs::canonicalize(&path)
                .with_context(|| format!("file not found: {path}"))?;
            control::send_file(abs.to_string_lossy().as_ref(), to.as_deref()).await?;
            println!("Offered to phone: {path}");
            Ok(())
        }
        Command::PairLive => control::pair_live().await,
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
    }
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

    let _mdns = match mdns::advertise(&identity, port) {
        Ok(g) => Some(g),
        Err(e) => {
            tracing::warn!("mDNS unavailable: {e:#}");
            None
        }
    };

    let _ble = if no_ble {
        None
    } else {
        match ble::advertise(port).await {
            Ok(guard) => {
                tracing::info!("BLE advertising active (service {SERVICE_UUID})");
                Some(guard)
            }
            Err(e) => {
                tracing::warn!("BLE unavailable ({e:#}) — falling back to mDNS only");
                None
            }
        }
    };

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
    server::serve(identity, port, pairing, notifier, clipboard, pending_files, battery, dnd_sync).await
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
