use crate::battery::BatteryStore;
use crate::clipboard::ClipboardHub;
use crate::dnd::DndSync;
use crate::files::{self, PendingFiles};
use crate::identity::{fingerprint_of, Identity, Peer, TrustedPeers};
use crate::notifications::Notifier;
use crate::protocol::{Message, ALPN, PROTOCOL_VERSION};
use anyhow::Result;
use quinn::crypto::rustls::QuicServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub async fn serve(
    identity: Arc<Identity>,
    port: u16,
    pairing: Arc<crate::pairing::Pairing>,
    notifier: Arc<Notifier>,
    clipboard: Arc<ClipboardHub>,
    pending: Arc<PendingFiles>,
    battery: Arc<BatteryStore>,
    dnd: Arc<DndSync>,
    fast_presence: bool,
) -> Result<()> {
    let endpoint = make_endpoint(&identity, port, fast_presence)?;
    tracing::info!("QUIC server listening on 0.0.0.0:{port} (ALPN {})", String::from_utf8_lossy(ALPN));

    while let Some(incoming) = endpoint.accept().await {
        let identity = identity.clone();
        let pairing = pairing.clone();
        let notifier = notifier.clone();
        let clipboard = clipboard.clone();
        let pending = pending.clone();
        let battery = battery.clone();
        let dnd = dnd.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    if let Err(e) =
                        handle_connection(conn, identity, pairing, notifier, clipboard, pending, battery, dnd).await
                    {
                        tracing::warn!("connection ended with error: {e:#}");
                    }
                }
                Err(e) => tracing::warn!("handshake failed: {e}"),
            }
        });
    }
    Ok(())
}

fn make_endpoint(identity: &Identity, port: u16, fast_presence: bool) -> Result<quinn::Endpoint> {
    let cert = CertificateDer::from(identity.cert_der.clone());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_der.clone()));

    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_client_cert_verifier(Arc::new(AcceptAnyClientCert::new()?))
    .with_single_cert(vec![cert], key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    // Every keep-alive packet wakes the phone's Wi-Fi radio, and the radio is
    // by far the most expensive thing we can ask a phone to do. One packet
    // every 3 s (the v2 setting) meant the radio never reached a deep sleep
    // state; 20 s is still far inside any NAT binding timeout and cuts those
    // wakeups by almost 7×.
    //
    // Proximity lock is the one feature that needs to *notice* the phone
    // leaving quickly, so when it is on we trade some battery for a 6 s beat.
    let (keep_alive, idle) = if fast_presence { (6, 20) } else { (20, 60) };
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(quinn::IdleTimeout::try_from(
        std::time::Duration::from_secs(idle),
    )?));
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(keep_alive)));
    server_config.transport_config(Arc::new(transport));
    tracing::info!("QUIC keep-alive {keep_alive}s, idle timeout {idle}s");
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    Ok(quinn::Endpoint::server(server_config, addr)?)
}

async fn handle_connection(
    conn: quinn::Connection,
    identity: Arc<Identity>,
    pairing: Arc<crate::pairing::Pairing>,
    notifier: Arc<Notifier>,
    clipboard: Arc<ClipboardHub>,
    pending: Arc<PendingFiles>,
    battery: Arc<BatteryStore>,
    dnd: Arc<DndSync>,
) -> Result<()> {
    let peer_fp = peer_fingerprint(&conn);
    let remote = conn.remote_address();
    tracing::info!("Connection from {remote} (client fingerprint: {})",
        peer_fp.as_deref().map(|f| &f[..16]).unwrap_or("none"));

    loop {
        let (send, recv) = match conn.accept_bi().await {
            Ok(s) => s,
            Err(quinn::ConnectionError::ApplicationClosed(_))
            | Err(quinn::ConnectionError::ConnectionClosed(_)) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let identity = identity.clone();
        let pairing = pairing.clone();
        let notifier = notifier.clone();
        let clipboard = clipboard.clone();
        let pending = pending.clone();
        let battery = battery.clone();
        let dnd = dnd.clone();
        let peer_fp = peer_fp.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(
                send,
                recv,
                &identity,
                &pairing,
                peer_fp.as_deref(),
                &notifier,
                &clipboard,
                &pending,
                &battery,
                &dnd,
            )
            .await
            {
                tracing::debug!("stream ended: {e:#}");
            }
        });
    }
}

async fn handle_stream(
    mut send: quinn::SendStream,
    recv: quinn::RecvStream,
    identity: &Identity,
    pairing: &Arc<crate::pairing::Pairing>,
    peer_fp: Option<&str>,
    notifier: &Notifier,
    clipboard: &Arc<ClipboardHub>,
    pending: &Arc<PendingFiles>,
    battery: &Arc<BatteryStore>,
    dnd: &Arc<DndSync>,
) -> Result<()> {
    let mut reader = BufReader::new(recv);
    let mut line = String::new();
    let mut session_trusted = false;

    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let msg: Message = match serde_json::from_str(line.trim()) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("invalid message: {e}");
                continue;
            }
        };

        let trusted = session_trusted
            || peer_fp.map(|fp| TrustedPeers::load().map(|t| t.is_trusted(fp)).unwrap_or(false))
                .unwrap_or(false);

        let reply = match msg {
            Message::PairRequest { version, token, device_name } => {
                if version != PROTOCOL_VERSION {
                    Message::PairRejected { reason: format!("version {version} not supported") }
                } else {
                    let expected = pairing.current().await;
                    match (expected, peer_fp) {
                        (Some(expected), Some(fp)) if constant_time_eq(&expected, &token) => {
                            let mut peers = TrustedPeers::load()?;
                            peers.add(Peer { name: device_name.clone(), fingerprint: fp.to_string() })?;
                            session_trusted = true;
                            pairing.notify_paired(&device_name);
                            pairing.end().await;
                            tracing::info!("✔ Device paired: {device_name} ({})", &fp[..16]);
                            Message::PairOk { device_name: identity.device_name.clone() }
                        }
                        (None, _) => Message::PairRejected {
                            reason: "the PC is not in pairing mode (\"Pair a device…\")".into(),
                        },
                        _ => Message::PairRejected { reason: "invalid token".into() },
                    }
                }
            }
            Message::Hello { version: _, device_name } if trusted => {
                tracing::info!("Hello from {device_name}");
                Message::HelloOk { device_name: identity.device_name.clone() }
            }
            Message::Ping { seq, sent_at_ms } if trusted => Message::Pong { seq, sent_at_ms },
            Message::Notification { key, app, title, body, can_reply } if trusted => {
                tracing::info!("🔔 {app}: {title}{}", if can_reply { " (replyable)" } else { "" });
                notifier.show(&key, &app, &title, &body, can_reply).await;
                Message::Ok
            }
            Message::NotificationDismissed { key } if trusted => {
                notifier.dismiss(&key).await;
                Message::Ok
            }
            Message::Clipboard { text } if trusted => {
                tracing::info!("📋 Clipboard received from phone ({} characters)", text.chars().count());
                clipboard.set_from_peer(&text).await;
                Message::Ok
            }
            Message::Handoff { url, title } if trusted => {
                tracing::info!("↘ Handoff phone→PC: {url}");
                notifier.show_handoff(&url, &title).await;
                Message::Ok
            }
            Message::Battery { level, charging } if trusted => {
                tracing::debug!("🔋 Phone battery: {level}%{}", if charging { " (charging)" } else { "" });
                battery.update(level, charging);
                Message::Ok
            }
            Message::Dnd { on } if trusted => {
                dnd.apply_from_phone(on).await;
                Message::Ok
            }
            Message::WebcamStart { width, height } if trusted => {
                match crate::webcam::WebcamFeed::start(width, height) {
                    Ok(feed) => {
                        receive_webcam(&mut reader, feed).await;
                    }
                    Err(e) => tracing::warn!("webcam: {e:#}"),
                }
                let _ = send.finish();
                return Ok(());
            }
            Message::MicStart { sample_rate, channels } if trusted => {
                match crate::mic::MicFeed::start(sample_rate, channels).await {
                    Ok(feed) => {
                        receive_mic(&mut reader, feed).await;
                    }
                    Err(e) => tracing::warn!("mic: {e:#}"),
                }
                let _ = send.finish();
                return Ok(());
            }
            Message::SpeakerStart { sample_rate, channels } if trusted => {
                match crate::speaker::SpeakerFeed::start(sample_rate, channels).await {
                    Ok(feed) => {
                        stream_speaker(&mut reader, &mut send, feed).await;
                    }
                    Err(e) => tracing::warn!("speaker: {e:#}"),
                }
                let _ = send.finish();
                return Ok(());
            }
            Message::DisplayStart { width, height, fps } if trusted => {
                match crate::display::DisplaySession::start(width, height, fps).await {
                    Ok(session) => {
                        stream_display(&mut reader, &mut send, session).await;
                    }
                    Err(e) => {
                        tracing::warn!("second screen: {e:#}");
                        let reason = serde_json::to_string(&format!("{e:#}")).unwrap_or_default();
                        let _ = send
                            .write_all(
                                format!("{{\"type\":\"display_error\",\"reason\":{reason}}}\n")
                                    .as_bytes(),
                            )
                            .await;
                    }
                }
                let _ = send.finish();
                return Ok(());
            }
            Message::SyncIndex { folder, files } if trusted => {
                if let Err(e) = crate::sync::handle(&mut reader, &mut send, &folder, files).await {
                    tracing::warn!("sync: {e:#}");
                }
                return Ok(());
            }
            Message::PcVolume { action, value } if trusted => {
                crate::volume::apply(&action, value).await;
                Message::Ok
            }
            Message::PcMedia { action } if trusted => {
                crate::media::apply(&action).await;
                Message::Ok
            }
            Message::FileStart { name, size, clipboard: is_clip } if trusted => {
                if is_clip {
                    tracing::info!("🖼 Clipboard image received from phone ({size} bytes)");
                    let mut buf = Vec::with_capacity(size as usize);
                    if read_exact_to_vec(&mut reader, &mut buf, size).await.is_ok() {
                        clipboard.set_image(&buf).await;
                    }
                    let _ = send.finish();
                    return Ok(());
                }
                let dest = files::unique_dest(&name);
                tracing::info!("📥 Receiving \"{name}\" ({size} bytes) → {}", dest.display());
                let res = receive_file(&mut reader, &dest, size).await;
                match res {
                    Ok(()) => {
                        notifier.show_handoff(
                            &format!("file://{}", dest.display()),
                            &format!("File received: {name}"),
                        ).await;
                        send.write_all(&Message::Ok.to_line()).await?;
                    }
                    Err(e) => {
                        tracing::warn!("reception failed: {e:#}");
                        let _ = tokio::fs::remove_file(&dest).await;
                    }
                }
                let _ = send.finish();
                return Ok(());
            }
            Message::FilePull { id } if trusted => {
                if let Some(path) = pending.take(&id).await {
                    tracing::info!("📤 Sending {} to phone", path.display());
                    if let Err(e) = send_file_bytes(&mut send, &path).await {
                        tracing::warn!("send failed: {e:#}");
                    }
                }
                let _ = send.finish();
                return Ok(());
            }
            Message::Subscribe if trusted => {
                send.write_all(&Message::Ok.to_line()).await?;
                send.flush().await?;
                let fp = peer_fp.map(|s| s.to_string()).unwrap_or_default();
                let name = peer_fp
                    .and_then(|fp| {
                        TrustedPeers::load()
                            .ok()
                            .and_then(|t| t.peers.into_iter().find(|p| p.fingerprint == fp).map(|p| p.name))
                    })
                    .unwrap_or_else(|| "Phone".to_string());
                let (id, mut rx) = clipboard.subscribe(fp, name).await;
                let result: Result<()> = async {
                    loop {
                        tokio::select! {
                            msg = rx.recv() => {
                                let Some(msg) = msg else { break };
                                send.write_all(&msg.to_line()).await?;
                                send.flush().await?;
                            }
                            n = reader.read_line(&mut line) => {
                                if n? == 0 {
                                    break;
                                }
                                line.clear();
                            }
                        }
                    }
                    Ok(())
                }
                .await;
                clipboard.unsubscribe(id).await;
                return result;
            }
            Message::Hello { .. }
            | Message::Ping { .. }
            | Message::Notification { .. }
            | Message::NotificationDismissed { .. }
            | Message::Clipboard { .. }
            | Message::Handoff { .. }
            | Message::PcVolume { .. }
            | Message::PcMedia { .. }
            | Message::Battery { .. }
            | Message::Dnd { .. }
            | Message::WebcamStart { .. }
            | Message::MicStart { .. }
            | Message::SpeakerStart { .. }
            | Message::DisplayStart { .. }
            | Message::SyncIndex { .. }
            | Message::FileStart { .. }
            | Message::FilePull { .. }
            | Message::Subscribe => Message::NotTrusted,
            other => {
                tracing::debug!("unexpected message on the server side: {other:?}");
                continue;
            }
        };
        send.write_all(&reply.to_line()).await?;
        send.flush().await?;
    }
}

async fn receive_webcam(reader: &mut BufReader<quinn::RecvStream>, mut feed: crate::webcam::WebcamFeed) {
    let mut len_buf = [0u8; 4];
    loop {
        if reader.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > 8 * 1024 * 1024 {
            break;
        }
        let mut frame = vec![0u8; len];
        if reader.read_exact(&mut frame).await.is_err() {
            break;
        }
        if feed.write_frame(&frame).await.is_err() {
            break;
        }
    }
}

/// Pumps PCM from parec to the phone. The direction is the mirror of the mic:
/// here the PC produces and the phone consumes. We also watch our receive side
/// — the phone closing its end (or sending anything at all) is the stop signal.
async fn stream_speaker(
    reader: &mut BufReader<quinn::RecvStream>,
    send: &mut quinn::SendStream,
    mut feed: crate::speaker::SpeakerFeed,
) {
    let Some(mut pcm) = feed.take_stdout() else { return };
    let mut buf = vec![0u8; 8192];
    let mut line = String::new();
    loop {
        tokio::select! {
            n = pcm.read(&mut buf) => {
                let n = match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if send.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            n = reader.read_line(&mut line) => {
                match n {
                    Ok(0) | Err(_) => break,
                    Ok(_) => line.clear(),
                }
            }
        }
    }
}

/// The second-screen pump: H.264 access units go down (length-prefixed),
/// input events come up (one JSON object per line). Either side going quiet
/// — pipe EOF, stream reset, tablet gone — ends the session, and
/// `DisplaySession::shutdown` folds the virtual monitor back up.
async fn stream_display(
    reader: &mut BufReader<quinn::RecvStream>,
    send: &mut quinn::SendStream,
    mut session: crate::display::DisplaySession,
) {
    let ready = format!(
        "{{\"type\":\"display_ready\",\"width\":{},\"height\":{}}}\n",
        session.width, session.height
    );
    if send.write_all(ready.as_bytes()).await.is_err() {
        session.shutdown().await;
        return;
    }
    let mut line = String::new();
    loop {
        tokio::select! {
            unit = session.units.recv() => {
                let Some(unit) = unit else { break };
                let len = (unit.len() as u32).to_be_bytes();
                if send.write_all(&len).await.is_err() {
                    break;
                }
                if send.write_all(&unit).await.is_err() {
                    break;
                }
            }
            n = reader.read_line(&mut line) => {
                match n {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        session.handle_input(&line).await;
                        line.clear();
                    }
                }
            }
        }
    }
    session.shutdown().await;
}

async fn receive_mic(reader: &mut BufReader<quinn::RecvStream>, mut feed: crate::mic::MicFeed) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if feed.write_pcm(&buf[..n]).await.is_err() {
            break;
        }
    }
}

async fn read_exact_to_vec(
    reader: &mut BufReader<quinn::RecvStream>,
    out: &mut Vec<u8>,
    size: u64,
) -> Result<()> {
    let mut remaining = size;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..want]).await?;
        if n == 0 {
            anyhow::bail!("stream cut off");
        }
        out.extend_from_slice(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(())
}

async fn receive_file(
    reader: &mut BufReader<quinn::RecvStream>,
    dest: &std::path::Path,
    size: u64,
) -> Result<()> {
    let mut file = tokio::fs::File::create(dest).await?;
    let mut remaining = size;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..want]).await?;
        if n == 0 {
            anyhow::bail!("stream cut off ({remaining} bytes missing)");
        }
        file.write_all(&buf[..n]).await?;
        remaining -= n as u64;
    }
    file.flush().await?;
    Ok(())
}

async fn send_file_bytes(send: &mut quinn::SendStream, path: &std::path::Path) -> Result<()> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        send.write_all(&buf[..n]).await?;
    }
    Ok(())
}

fn peer_fingerprint(conn: &quinn::Connection) -> Option<String> {
    let identity = conn.peer_identity()?;
    let certs = identity.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| fingerprint_of(c.as_ref()))
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Debug)]
struct AcceptAnyClientCert {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyClientCert {
    fn new() -> Result<Self> {
        Ok(Self { provider: Arc::new(rustls::crypto::ring::default_provider()) })
    }
}

impl rustls::server::danger::ClientCertVerifier for AcceptAnyClientCert {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}
