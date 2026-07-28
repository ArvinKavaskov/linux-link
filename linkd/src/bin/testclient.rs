use anyhow::{Context as _, Result};
use clap::Parser;
use quinn::crypto::rustls::QuicClientConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

const ALPN: &[u8] = b"linuxlink/1";

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:47100")]
    addr: SocketAddr,
    #[arg(long)]
    token: Option<String>,
    #[arg(long, default_value = "test-phone")]
    name: String,
    #[arg(long, default_value_t = 3)]
    pings: u64,
    #[arg(long)]
    notify: Option<String>,
    #[arg(long)]
    send_clip: Option<String>,
    #[arg(long)]
    listen: Option<u64>,
    #[arg(long)]
    send_file: Option<String>,
    #[arg(long)]
    pull_to: Option<String>,
    #[arg(long)]
    sync_dir: Option<String>,
    #[arg(long)]
    battery: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let cert_path = std::env::temp_dir().join("linuxlink-testclient-cert.der");
    let key_path = std::env::temp_dir().join("linuxlink-testclient-key.der");
    let (cert_der, key_der) = if cert_path.exists() && key_path.exists() {
        (std::fs::read(&cert_path)?, std::fs::read(&key_path)?)
    } else {
        let cert = rcgen::generate_simple_self_signed(vec![cli.name.clone()])?;
        let c = cert.cert.der().to_vec();
        let k = cert.key_pair.serialize_der();
        std::fs::write(&cert_path, &c)?;
        std::fs::write(&key_path, &k)?;
        (c, k)
    };

    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert::new()))
    .with_client_auth_cert(
        vec![CertificateDer::from(cert_der)],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
    )?;
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    println!("QUIC connection to {} …", cli.addr);
    let conn = endpoint.connect(cli.addr, "linuxlink")?.await.context("QUIC connection")?;
    println!("Connected ({})", conn.remote_address());

    let (send, recv) = conn.open_bi().await?;
    let mut send = send;
    let mut reader = BufReader::new(recv);
    let mut line = String::new();

    let first = if let Some(token) = &cli.token {
        serde_json::json!({"type": "pair_request", "version": 1, "token": token, "device_name": cli.name})
    } else {
        serde_json::json!({"type": "hello", "version": 1, "device_name": cli.name})
    };
    send.write_all(format!("{first}\n").as_bytes()).await?;
    line.clear();
    reader.read_line(&mut line).await?;
    println!("← {}", line.trim());

    if line.contains("pair_rejected") || line.contains("not_trusted") {
        eprintln!("The PC refused the connection — check the token / pairing.");
        std::process::exit(1);
    }

    for seq in 0..cli.pings {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let ping = serde_json::json!({"type": "ping", "seq": seq, "sent_at_ms": now});
        send.write_all(format!("{ping}\n").as_bytes()).await?;
        line.clear();
        reader.read_line(&mut line).await?;
        let rtt = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64 - now;
        println!("← {} (round-trip {rtt} ms)", line.trim());
    }

    if let Some(spec) = &cli.notify {
        let (title, body) = spec.split_once(':').unwrap_or((spec.as_str(), ""));
        let notif = serde_json::json!({
            "type": "notification", "key": "testclient-1",
            "app": "TestApp", "title": title, "body": body,
            "can_reply": true,
        });
        send.write_all(format!("{notif}\n").as_bytes()).await?;
        line.clear();
        reader.read_line(&mut line).await?;
        println!("← {} (notification sent)", line.trim());
    }

    if let Some(text) = &cli.send_clip {
        let msg = serde_json::json!({"type": "clipboard", "text": text});
        send.write_all(format!("{msg}\n").as_bytes()).await?;
        line.clear();
        reader.read_line(&mut line).await?;
        println!("← {} (clipboard sent)", line.trim());
    }

    if let Some(secs) = cli.listen {
        let (sub_send, sub_recv) = conn.open_bi().await?;
        let mut sub_send = sub_send;
        let mut sub_reader = BufReader::new(sub_recv);
        let mut sub_line = String::new();
        sub_send.write_all(b"{\"type\":\"subscribe\"}\n").await?;
        sub_reader.read_line(&mut sub_line).await?;
        println!("Push channel open: {} — listening for {secs} s…", sub_line.trim());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            sub_line.clear();
            match tokio::time::timeout_at(deadline, sub_reader.read_line(&mut sub_line)).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(_)) => {
                    println!("⇐ push: {}", sub_line.trim());
                    sub_send.write_all(b"{\"type\":\"ok\"}\n").await?;
                }
                Ok(Err(e)) => return Err(e.into()),
            }
        }
        println!("End of listening.");
    }

    if let Some(spec) = &cli.battery {
        let charging = spec.ends_with('c');
        let level: i32 = spec.trim_end_matches('c').parse().unwrap_or(0);
        let msg = serde_json::json!({"type":"battery","level":level,"charging":charging});
        send.write_all(format!("{msg}\n").as_bytes()).await?;
        line.clear();
        reader.read_line(&mut line).await?;
        println!("← {} (battery sent: {level}%)", line.trim());
    }

    if let Some(path) = &cli.send_file {
        let data = std::fs::read(path)?;
        let name = std::path::Path::new(path)
            .file_name().unwrap().to_string_lossy().to_string();
        let (fsend, frecv) = conn.open_bi().await?;
        let mut fsend = fsend;
        let mut freader = BufReader::new(frecv);
        let header = serde_json::json!({"type":"file_start","name":name,"size":data.len()});
        fsend.write_all(format!("{header}\n").as_bytes()).await?;
        fsend.write_all(&data).await?;
        fsend.finish()?;
        let mut line = String::new();
        freader.read_line(&mut line).await?;
        println!("← {} (file sent: {} bytes)", line.trim(), data.len());
    }

    if let Some(dest) = &cli.pull_to {
        let (ssend, srecv) = conn.open_bi().await?;
        let mut ssend = ssend;
        let mut sreader = BufReader::new(srecv);
        let mut sline = String::new();
        ssend.write_all(b"{\"type\":\"subscribe\"}\n").await?;
        sreader.read_line(&mut sline).await?;
        println!("Waiting for a file offer from the PC…");
        let (id, name, size) = loop {
            sline.clear();
            sreader.read_line(&mut sline).await?;
            let v: serde_json::Value = serde_json::from_str(sline.trim())?;
            if v["type"] == "file_offer" {
                break (
                    v["id"].as_str().unwrap().to_string(),
                    v["name"].as_str().unwrap().to_string(),
                    v["size"].as_u64().unwrap(),
                );
            }
            ssend.write_all(b"{\"type\":\"ok\"}\n").await?;
        };
        println!("Offer received: {name} ({size} bytes) — pulling…");
        let (mut psend, precv) = conn.open_bi().await?;
        let mut preader = BufReader::new(precv);
        psend.write_all(format!("{{\"type\":\"file_pull\",\"id\":\"{id}\"}}\n").as_bytes()).await?;
        psend.finish()?;
        let mut data = vec![0u8; size as usize];
        preader.read_exact(&mut data).await?;
        std::fs::write(dest, &data)?;
        println!("File saved: {dest} ({} bytes)", data.len());
    }

    if let Some(dir) = &cli.sync_dir {
        run_sync(&conn, std::path::Path::new(dir)).await?;
    }

    send.finish()?;
    conn.close(0u32.into(), b"bye");
    endpoint.wait_idle().await;
    println!("Done.");
    Ok(())
}

async fn run_sync(conn: &quinn::Connection, root: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(root).ok();
    let (send, recv) = conn.open_bi().await?;
    let mut send = send;
    let mut reader = BufReader::new(recv);

    let index = scan_local(root);
    let hello = serde_json::json!({"type":"sync_index","files": index});
    send.write_all(format!("{hello}\n").as_bytes()).await?;

    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let plan: serde_json::Value = serde_json::from_str(line.trim())?;
    let pull: Vec<String> = serde_json::from_value(plan["pull"].clone()).unwrap_or_default();
    let del_phone: Vec<String> = serde_json::from_value(plan["del_phone"].clone()).unwrap_or_default();
    println!("Plan received: push_end to read, {} to send, {} to delete locally",
        pull.len(), del_phone.len());

    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        let v: serde_json::Value = serde_json::from_str(line.trim())?;
        match v["type"].as_str() {
            Some("sync_file") => {
                let path = v["path"].as_str().unwrap().to_string();
                let size = v["size"].as_u64().unwrap();
                let mut buf = vec![0u8; size as usize];
                reader.read_exact(&mut buf).await?;
                let dest = root.join(&path);
                if let Some(p) = dest.parent() { std::fs::create_dir_all(p).ok(); }
                std::fs::write(&dest, &buf).ok();
                println!("  ← received {path} ({size} B)");
            }
            Some("sync_push_end") => break,
            _ => break,
        }
    }

    for rel in &del_phone {
        std::fs::remove_file(root.join(rel)).ok();
        println!("  ✗ deleted {rel}");
    }

    for path in &pull {
        let data = std::fs::read(root.join(path)).unwrap_or_default();
        let hdr = serde_json::json!({"type":"sync_file","path":path,"size":data.len()});
        send.write_all(format!("{hdr}\n").as_bytes()).await?;
        send.write_all(&data).await?;
        println!("  → sent {path} ({} B)", data.len());
    }
    send.write_all(b"{\"type\":\"sync_pull_end\"}\n").await?;

    let final_index = scan_local(root);
    let idx2 = serde_json::json!({"type":"sync_index2","files": final_index});
    send.write_all(format!("{idx2}\n").as_bytes()).await?;

    line.clear();
    reader.read_line(&mut line).await?;
    println!("Sync: {}", line.trim());
    Ok(())
}

fn scan_local(root: &std::path::Path) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<serde_json::Value>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() { walk(root, &p, out); }
                else if let Ok(m) = e.metadata() {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
                    let mtime = m.modified().ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64).unwrap_or(0);
                    out.push(serde_json::json!({"path":rel,"size":m.len(),"mtime":mtime}));
                }
            }
        }
    }
    walk(root, root, &mut out);
    out
}

#[derive(Debug)]
struct AcceptAnyServerCert {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyServerCert {
    fn new() -> Self {
        Self { provider: Arc::new(rustls::crypto::ring::default_provider()) }
    }
}

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message, cert, dss, &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message, cert, dss, &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}
