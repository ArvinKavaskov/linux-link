use crate::clipboard::ClipboardHub;
use crate::protocol::Message;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::zvariant::Value;

pub struct Notifier {
    conn: Option<zbus::Connection>,
    ids: Mutex<HashMap<String, u32>>,
    reply_targets: Mutex<HashMap<u32, (String, String)>>,
    open_targets: Mutex<HashMap<u32, String>>,
    hub: Arc<ClipboardHub>,
}

impl Notifier {
    pub async fn new(hub: Arc<ClipboardHub>) -> Arc<Self> {
        let conn = match zbus::Connection::session().await {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("D-Bus session bus unavailable ({e}) — desktop notifications disabled");
                None
            }
        };
        let notifier = Arc::new(Self {
            conn,
            ids: Mutex::new(HashMap::new()),
            reply_targets: Mutex::new(HashMap::new()),
            open_targets: Mutex::new(HashMap::new()),
            hub,
        });
        if notifier.conn.is_some() {
            let n = notifier.clone();
            tokio::spawn(async move { n.listen_actions().await });
        }
        notifier
    }

    pub async fn show(&self, key: &str, app: &str, title: &str, body: &str, can_reply: bool) {
        let Some(conn) = &self.conn else { return };
        let replaces_id = { self.ids.lock().await.get(key).copied().unwrap_or(0) };

        let summary = if title.is_empty() { app.to_string() } else { format!("{app} — {title}") };
        let actions: Vec<&str> = if can_reply { vec!["reply", "Reply"] } else { vec![] };
        let hints: HashMap<String, Value> = HashMap::new();
        let result = conn
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Linux Link",
                    replaces_id,
                    "phone",
                    summary.as_str(),
                    body,
                    actions,
                    hints,
                    8000i32,
                ),
            )
            .await;

        match result {
            Ok(reply) => match reply.body().deserialize::<u32>() {
                Ok(id) => {
                    self.ids.lock().await.insert(key.to_string(), id);
                    if can_reply {
                        self.reply_targets
                            .lock()
                            .await
                            .insert(id, (key.to_string(), summary.clone()));
                    }
                }
                Err(e) => tracing::warn!("unreadable Notify reply: {e}"),
            },
            Err(e) => tracing::warn!("failed to display the notification: {e}"),
        }
    }

    pub async fn show_handoff(&self, url: &str, title: &str) {
        let Some(conn) = &self.conn else { return };
        let summary = if title.is_empty() { "Page from the phone".to_string() }
            else { format!("Open: {title}") };
        let actions: Vec<&str> = vec!["open", "Open", "default", "Open"];
        let hints: HashMap<String, Value> = HashMap::new();
        let result = conn
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Linux Link", 0u32, "web-browser",
                    summary.as_str(), url, actions, hints, 0i32,
                ),
            )
            .await;
        match result {
            Ok(reply) => if let Ok(id) = reply.body().deserialize::<u32>() {
                self.open_targets.lock().await.insert(id, url.to_string());
            },
            Err(e) => tracing::warn!("handoff notification failed: {e}"),
        }
    }

    pub async fn dismiss(&self, key: &str) {
        let Some(conn) = &self.conn else { return };
        let Some(id) = self.ids.lock().await.remove(key) else { return };
        self.reply_targets.lock().await.remove(&id);
        if let Err(e) = conn
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "CloseNotification",
                &(id,),
            )
            .await
        {
            tracing::debug!("CloseNotification({id}): {e}");
        }
    }

    async fn listen_actions(self: Arc<Self>) {
        let Some(conn) = &self.conn else { return };
        let proxy = match zbus::Proxy::new(
            conn,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("cannot listen for actions: {e}");
                return;
            }
        };
        let mut stream = match proxy.receive_signal("ActionInvoked").await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("cannot subscribe to ActionInvoked: {e}");
                return;
            }
        };
        tracing::info!("Quick reply active (“Reply” button on notifications)");
        while let Some(signal) = stream.next().await {
            let Ok((id, action)) = signal.body().deserialize::<(u32, String)>() else { continue };
            match action.as_str() {
                "reply" => {
                    let target = { self.reply_targets.lock().await.get(&id).cloned() };
                    if let Some((key, label)) = target {
                        let me = self.clone();
                        tokio::spawn(async move { me.prompt_and_send(key, label).await });
                    }
                }
                "open" | "default" => {
                    let url = { self.open_targets.lock().await.get(&id).cloned() };
                    if let Some(url) = url {
                        tracing::info!("↗ Opening the page in the browser");
                        let _ = tokio::process::Command::new("xdg-open").arg(&url).spawn();
                    }
                }
                _ => {}
            }
        }
    }

    async fn prompt_and_send(&self, key: String, label: String) {
        let output = tokio::process::Command::new("zenity")
            .args([
                "--entry",
                "--title=Linux Link — Quick reply",
                &format!("--text=Reply to: {label}"),
                "--width=420",
            ])
            .output()
            .await;
        let text = match output {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            }
            Ok(_) => return,
            Err(e) => {
                tracing::warn!("zenity unavailable ({e}) — install it: sudo apt install zenity");
                return;
            }
        };
        if text.is_empty() {
            return;
        }
        tracing::info!("↩ Reply sent to the phone ({} characters)", text.chars().count());
        self.hub.push(Message::NotificationReply { key, text }).await;
    }
}
