
use crate::clipboard::ClipboardHub;
use crate::protocol::Message;
use futures_util::{FutureExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

pub async fn apply(action: &str) {
    let member = match action {
        "play_pause" => "PlayPause",
        "next" => "Next",
        "previous" => "Previous",
        other => {
            tracing::warn!("unknown media action: {other}");
            return;
        }
    };
    if dbus_apply(member).await.is_ok() {
        tracing::info!("⏯ PC media: {action}");
        return;
    }
    let arg = match action {
        "play_pause" => "play-pause",
        "next" => "next",
        _ => "previous",
    };
    match tokio::process::Command::new("playerctl").arg(arg).status().await {
        Ok(s) if s.success() => tracing::info!("⏯ PC media: {action} (playerctl)"),
        _ => tracing::warn!("no media player responded to {action}"),
    }
}

async fn dbus_apply(member: &str) -> anyhow::Result<()> {
    let conn = Connection::session().await?;
    let target = pick_player(&conn).await.ok_or_else(|| anyhow::anyhow!("no player"))?;
    let proxy = zbus::Proxy::new(&conn, target.as_str(), MPRIS_PATH, PLAYER_IFACE).await?;
    proxy.call_method(member, &()).await?;
    Ok(())
}

pub fn spawn_watcher(hub: Arc<ClipboardHub>) {
    tokio::spawn(async move {
        if let Err(e) = watch(hub).await {
            tracing::warn!("media watcher stopped: {e} — now playing will not be mirrored");
        }
    });
}

async fn watch(hub: Arc<ClipboardHub>) -> anyhow::Result<()> {
    let conn = Connection::session().await?;

    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(MPRIS_PATH)?
        .build();
    let mut props = zbus::MessageStream::for_match_rule(rule, &conn, Some(8)).await?;

    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut names = dbus.receive_name_owner_changed().await?;

    tracing::info!("Media: event-driven (MPRIS over D-Bus)");
    let mut last: Option<(String, String, bool)> = None;
    publish(&hub, &mut last, read_now_playing(&conn).await).await;

    loop {
        tokio::select! {
            msg = props.next() => { if msg.is_none() { break; } }
            sig = names.next() => {
                match sig {
                    Some(s) => {
                        let is_player = s.args()
                            .map(|a| a.name().starts_with(MPRIS_PREFIX))
                            .unwrap_or(false);
                        if !is_player { continue; }
                    }
                    None => break,
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        while props.next().now_or_never().is_some() {}

        publish(&hub, &mut last, read_now_playing(&conn).await).await;
    }
    Ok(())
}

async fn publish(
    hub: &Arc<ClipboardHub>,
    last: &mut Option<(String, String, bool)>,
    info: Option<(String, String, bool)>,
) {
    if info == *last {
        return;
    }
    *last = info.clone();
    let (title, artist, playing) = info.unwrap_or_default();
    hub.push(Message::MediaInfo { title, artist, playing }).await;
}

async fn pick_player(conn: &Connection) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let names = dbus.list_names().await.ok()?;
    let mut fallback = None;
    for name in names {
        let name = name.as_str().to_string();
        if !name.starts_with(MPRIS_PREFIX) {
            continue;
        }
        match read_player(conn, &name).await {
            Some((_, _, true)) => return Some(name),
            Some(_) if fallback.is_none() => fallback = Some(name),
            _ => {}
        }
    }
    fallback
}

async fn read_now_playing(conn: &Connection) -> Option<(String, String, bool)> {
    let name = pick_player(conn).await?;
    read_player(conn, &name).await
}

async fn read_player(conn: &Connection, dest: &str) -> Option<(String, String, bool)> {
    let proxy = zbus::Proxy::new(conn, dest, MPRIS_PATH, PLAYER_IFACE).await.ok()?;
    let status: String = proxy.get_property("PlaybackStatus").await.unwrap_or_default();
    let meta: HashMap<String, OwnedValue> = proxy.get_property("Metadata").await.ok()?;
    let title = meta.get("xesam:title").and_then(first_string)?;
    if title.is_empty() {
        return None;
    }
    let artist = meta.get("xesam:artist").and_then(first_string).unwrap_or_default();
    Some((title, artist, status.eq_ignore_ascii_case("Playing")))
}

fn first_string(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        Value::Array(a) => a.iter().find_map(|x| match x {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        }),
        _ => None,
    }
}
