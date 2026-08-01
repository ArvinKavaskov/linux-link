//! Minimal xdg-desktop-portal ScreenCast client.
//!
//! Used on KDE, where the compositor has no direct "give me this output as
//! PipeWire" API we can reach without speaking a private Wayland protocol.
//! The portal shows a picker the first time; we ask it to persist the choice
//! (`persist_mode=2`) and store the returned restore token, so every later
//! connection starts silently.
//!
//! The portal's calling convention is indirect: each method returns a
//! `Request` object path, and the actual result arrives as a `Response`
//! signal on that path. The path is predictable from our unique bus name and
//! the handle token, so we subscribe before calling — the classic dance.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use zbus::zvariant::{self, OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.ScreenCast";

/// Opens a screencast session; the user picks the (virtual) monitor the first
/// time. Returns the PipeWire node id, a restore token to persist (if the
/// portal granted one), and the D-Bus connection — the caller must keep that
/// connection alive, because the portal session (and with it the PipeWire
/// stream) is torn down the moment it closes.
pub async fn open_screencast(
    restore_token: Option<String>,
) -> Result<(u32, Option<String>, zbus::Connection)> {
    let conn = zbus::Connection::session().await?;
    let portal = zbus::Proxy::new(&conn, DEST, PATH, IFACE).await?;

    // CreateSession
    let pending = PendingRequest::subscribe(&conn, "linuxlink_c").await?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("handle_token", "linuxlink_c".into());
    opts.insert("session_handle_token", "linuxlink_s".into());
    let ret: zvariant::OwnedObjectPath = portal.call("CreateSession", &(opts)).await?;
    let results = pending.wait(&conn, ret, "CreateSession").await?;
    let session: String = results
        .get("session_handle")
        .and_then(value_as_string)
        .context("no session_handle in portal response")?;
    let session_path = zvariant::ObjectPath::try_from(session)?;

    // SelectSources
    let pending = PendingRequest::subscribe(&conn, "linuxlink_ss").await?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("handle_token", "linuxlink_ss".into());
    opts.insert("types", Value::U32(1)); // MONITOR
    opts.insert("multiple", Value::Bool(false));
    opts.insert("cursor_mode", Value::U32(2)); // embedded
    opts.insert("persist_mode", Value::U32(2)); // persist until revoked
    if let Some(t) = &restore_token {
        opts.insert("restore_token", t.as_str().into());
    }
    let ret: zvariant::OwnedObjectPath = portal.call("SelectSources", &(&session_path, opts)).await?;
    pending.wait(&conn, ret, "SelectSources").await?;

    // Start — this is where the picker appears (first time only).
    let pending = PendingRequest::subscribe(&conn, "linuxlink_st").await?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("handle_token", "linuxlink_st".into());
    let ret: zvariant::OwnedObjectPath = portal.call("Start", &(&session_path, "", opts)).await?;
    let results = pending
        .wait(&conn, ret, "Start")
        .await
        .context("portal Start (was the screen picker cancelled?)")?;

    let node = extract_first_stream_node(&results).context("no stream in portal response")?;
    let token = results.get("restore_token").and_then(value_as_string);
    Ok((node, token, conn))
}

/// A subscription to the Response signal of a portal request, opened *before*
/// the method call so the response cannot be missed.
struct PendingRequest {
    stream: zbus::proxy::SignalStream<'static>,
    predicted: String,
}

impl PendingRequest {
    async fn subscribe(conn: &zbus::Connection, handle_token: &str) -> Result<Self> {
        let unique = conn
            .unique_name()
            .context("no unique bus name")?
            .trim_start_matches(':')
            .replace('.', "_");
        let predicted = format!("/org/freedesktop/portal/desktop/request/{unique}/{handle_token}");
        let proxy =
            zbus::Proxy::new(conn, DEST, predicted.clone(), "org.freedesktop.portal.Request").await?;
        let stream = proxy.receive_signal("Response").await?;
        Ok(Self { stream, predicted })
    }

    async fn wait(
        mut self,
        conn: &zbus::Connection,
        returned: zvariant::OwnedObjectPath,
        what: &str,
    ) -> Result<HashMap<String, OwnedValue>> {
        let timeout = std::time::Duration::from_secs(120);
        let msg = if returned.as_str() == self.predicted {
            tokio::time::timeout(timeout, self.stream.next()).await.ok().flatten()
        } else {
            // Older portals return a different request path; follow it.
            let proxy =
                zbus::Proxy::new(conn, DEST, returned, "org.freedesktop.portal.Request").await?;
            let mut stream = proxy.receive_signal("Response").await?;
            tokio::time::timeout(timeout, stream.next()).await.ok().flatten()
        };
        let msg = msg.with_context(|| format!("portal {what}: no response"))?;
        let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize()?;
        if code != 0 {
            anyhow::bail!("portal {what} refused (code {code})");
        }
        Ok(results)
    }
}

fn value_as_string(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.to_string()),
        _ => None,
    }
}

/// results["streams"] is a(ua{sv}); we want the first u.
fn extract_first_stream_node(results: &HashMap<String, OwnedValue>) -> Option<u32> {
    let streams = results.get("streams")?;
    if let Value::Array(arr) = &**streams {
        for item in arr.iter() {
            if let Value::Structure(s) = item {
                if let Some(Value::U32(node)) = s.fields().first() {
                    return Some(*node);
                }
            }
        }
    }
    None
}
