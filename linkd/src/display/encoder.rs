
use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::has_cmd;

pub enum CaptureSource {
    PipeWire { node: u32, negotiate: Option<(u32, u32)> },
    WlrOutput { name: String },
    X11Region { x: u32, y: u32, width: u32, height: u32 },
}

pub struct Encoder {
    child: Child,
    pub units: mpsc::Receiver<Vec<u8>>,
}

impl Encoder {
    pub async fn start(source: &CaptureSource, fps: u32, bitrate_kbps: u32) -> Result<Self> {
        let (program, args) = build_command(source, fps, bitrate_kbps)?;
        tracing::info!("display encoder: {program} {}", args.join(" "));
        let mut child = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawning {program} (is it installed?)"))?;

        let stdout = child.stdout.take().context("encoder stdout")?;
        let stderr = child.stderr.take().context("encoder stderr")?;

        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut reader = tokio::io::BufReader::new(stderr);
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
            if !buf.is_empty() {
                let text = String::from_utf8_lossy(&buf);
                let tail: Vec<&str> = text.lines().rev().take(6).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                tracing::debug!("encoder stderr:\n{}", tail.join("\n"));
            }
        });

        let (tx, rx) = mpsc::channel::<Vec<u8>>(4);
        tokio::spawn(pump(stdout, tx));

        Ok(Self { child, units: rx })
    }

    pub async fn shutdown(mut self) {
        let _ = self.child.kill().await;
    }
}

async fn pump(mut stdout: tokio::process::ChildStdout, tx: mpsc::Sender<Vec<u8>>) {
    let mut splitter = AnnexBSplitter::new();
    let mut assembler = AuAssembler::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = tokio::time::timeout(
            std::time::Duration::from_millis(10),
            stdout.read(&mut buf),
        )
        .await;
        let n = match read {
            Err(_idle) => {
                let tail = splitter.finish().and_then(|nal| assembler.push(nal));
                for au in tail.into_iter().chain(assembler.finish()) {
                    if tx.try_send(au).is_err() && tx.is_closed() {
                        return;
                    }
                }
                continue;
            }
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => n,
        };
        for nal in splitter.push(&buf[..n]) {
            if let Some(au) = assembler.push(nal) {
                if tx.try_send(au).is_err() && tx.is_closed() {
                    return;
                }
            }
        }
    }
    if let Some(nal) = splitter.finish() {
        if let Some(au) = assembler.push(nal) {
            let _ = tx.try_send(au);
        }
    }
    if let Some(au) = assembler.finish() {
        let _ = tx.try_send(au);
    }
}

fn build_command(source: &CaptureSource, fps: u32, bitrate_kbps: u32) -> Result<(String, Vec<String>)> {
    let fps = fps.clamp(24, 60);
    match source {
        CaptureSource::PipeWire { node, negotiate } => {
            if !has_cmd("gst-launch-1.0") {
                anyhow::bail!(
                    "gst-launch-1.0 not found — install the GStreamer tools \
                     (gstreamer1.0-tools / gstreamer1-plugins / gstreamer)"
                );
            }
            let enc = gst_encoder(bitrate_kbps, fps)?;
            let mut args: Vec<String> = vec![
                "-q".into(),
                "pipewiresrc".into(),
                format!("path={node}"),
                "always-copy=true".into(),
                "!".into(),
            ];
            if let Some((w, h)) = negotiate {
                args.push(format!("video/x-raw,width={w},height={h}"));
                args.push("!".into());
            }
            args.extend([
                "videoconvert".into(),
                "n-threads=4".into(),
                "!".into(),
                "video/x-raw,format=I420".into(),
                "!".into(),
            ]);
            args.extend(enc);
            args.extend([
                "!".into(),
                "h264parse".into(),
                "config-interval=-1".into(),
                "!".into(),
                "video/x-h264,stream-format=byte-stream".into(),
                "!".into(),
                "fdsink".into(),
                "fd=1".into(),
                "sync=false".into(),
            ]);
            Ok(("gst-launch-1.0".into(), args))
        }
        CaptureSource::WlrOutput { name } => {
            if !has_cmd("wf-recorder") {
                anyhow::bail!("wf-recorder not found — install it (pacman -S wf-recorder)");
            }
            Ok((
                "wf-recorder".into(),
                vec![
                    "-o".into(),
                    name.clone(),
                    "-c".into(),
                    "libx264".into(),
                    "-p".into(),
                    "preset=superfast".into(),
                    "-p".into(),
                    "tune=zerolatency".into(),
                    "-p".into(),
                    "x264-params=intra-refresh=1:keyint=60".into(),
                    "-x".into(),
                    "yuv420p".into(),
                    "-m".into(),
                    "h264".into(),
                    "-f".into(),
                    "/dev/stdout".into(),
                    "-y".into(),
                ],
            ))
        }
        CaptureSource::X11Region { x, y, width, height } => {
            if !has_cmd("ffmpeg") {
                anyhow::bail!("ffmpeg not found — install it (it is also needed for the webcam)");
            }
            let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
            Ok((
                "ffmpeg".into(),
                vec![
                    "-loglevel".into(),
                    "error".into(),
                    "-f".into(),
                    "x11grab".into(),
                    "-framerate".into(),
                    fps.to_string(),
                    "-video_size".into(),
                    format!("{width}x{height}"),
                    "-i".into(),
                    format!("{display}+{x},{y}"),
                    "-c:v".into(),
                    "libx264".into(),
                    "-preset".into(),
                    "superfast".into(),
                    "-tune".into(),
                    "zerolatency".into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-b:v".into(),
                    format!("{bitrate_kbps}k"),
                    "-x264-params".into(),
                    format!(
                        "repeat-headers=1:keyint={fps}:intra-refresh=1:\
vbv-maxrate={bitrate_kbps}:vbv-bufsize={}",
                        (bitrate_kbps * 2 / fps.max(1)).max(64)
                    ),
                    "-f".into(),
                    "h264".into(),
                    "-".into(),
                ],
            ))
        }
    }
}

fn gst_encoder(bitrate_kbps: u32, fps: u32) -> Result<Vec<String>> {
    if gst_has_element("x264enc") {
        let vbv_ms = (2_000 / fps.max(1)).max(16);
        return Ok(vec![
            "x264enc".into(),
            "tune=zerolatency".into(),
            "speed-preset=superfast".into(),
            format!("bitrate={bitrate_kbps}"),
            "intra-refresh=true".into(),
            format!("key-int-max={fps}"),
            format!("vbv-buf-capacity={vbv_ms}"),
            "byte-stream=true".into(),
        ]);
    }
    if gst_has_element("openh264enc") {
        return Ok(vec![
            "openh264enc".into(),
            "usage-type=screen".into(),
            format!("bitrate={}", bitrate_kbps * 1000),
            "gop-size=120".into(),
        ]);
    }
    if gst_has_element("vah264enc") {
        return Ok(vec![
            "vah264enc".into(),
            format!("bitrate={bitrate_kbps}"),
            "key-int-max=120".into(),
        ]);
    }
    anyhow::bail!(
        "no H.264 encoder element found — install gstreamer1.0-plugins-ugly \
         (x264enc) or gst-plugins-ugly"
    )
}

fn gst_has_element(name: &str) -> bool {
    std::process::Command::new("gst-inspect-1.0")
        .args(["--exists", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub struct AnnexBSplitter {
    buf: Vec<u8>,
}

impl AnnexBSplitter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        while let Some(first) = find_start_code(&self.buf, 0) {
            let Some(second) = find_start_code(&self.buf, first + 3) else { break };
            out.push(self.buf[first..second].to_vec());
            self.buf.drain(..second);
        }
        out
    }

    pub fn finish(&mut self) -> Option<Vec<u8>> {
        let first = find_start_code(&self.buf, 0)?;
        if self.buf.len() > first + 4 {
            let tail = self.buf[first..].to_vec();
            self.buf.clear();
            Some(tail)
        } else {
            None
        }
    }
}

fn find_start_code(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            return Some(if i > from && buf[i - 1] == 0 { i - 1 } else { i });
        }
        i += 1;
    }
    None
}

fn nal_type(nal: &[u8]) -> u8 {
    let offset = if nal.len() > 3 && nal[2] == 1 { 3 } else { 4 };
    nal.get(offset).map(|b| b & 0x1F).unwrap_or(0)
}

fn is_vcl(t: u8) -> bool {
    (1..=5).contains(&t)
}

fn starts_new_picture(nal: &[u8]) -> bool {
    let offset = if nal.len() > 3 && nal[2] == 1 { 3 } else { 4 };
    match nal.get(offset + 1) {
        Some(b) => b & 0x80 != 0,
        None => true,
    }
}

pub struct AuAssembler {
    pending: Vec<u8>,
    has_vcl: bool,
}

impl AuAssembler {
    pub fn new() -> Self {
        Self { pending: Vec::new(), has_vcl: false }
    }

    pub fn push(&mut self, nal: Vec<u8>) -> Option<Vec<u8>> {
        let t = nal_type(&nal);
        let flush = self.has_vcl
            && ((is_vcl(t) && starts_new_picture(&nal)) || matches!(t, 6..=9));
        let out = if flush {
            self.has_vcl = false;
            Some(std::mem::take(&mut self.pending))
        } else {
            None
        };
        self.has_vcl |= is_vcl(t);
        self.pending.extend_from_slice(&nal);
        out
    }

    pub fn finish(&mut self) -> Option<Vec<u8>> {
        if self.has_vcl {
            self.has_vcl = false;
            Some(std::mem::take(&mut self.pending))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nal(t: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1, t];
        v.resize(4 + len, 0xAA);
        v
    }

    #[test]
    fn splitter_reassembles_across_arbitrary_chunk_cuts() {
        let mut stream = Vec::new();
        let sps = nal(0x67, 8);
        let pps = nal(0x68, 4);
        let idr = nal(0x65, 900);
        let p = nal(0x41, 300);
        for n in [&sps, &pps, &idr, &p] {
            stream.extend_from_slice(n);
        }

        let mut splitter = AnnexBSplitter::new();
        let mut nals = Vec::new();
        for chunk in stream.chunks(7) {
            nals.extend(splitter.push(chunk));
        }
        if let Some(tail) = splitter.finish() {
            nals.push(tail);
        }
        assert_eq!(nals.len(), 4);
        assert_eq!(nals[0], sps);
        assert_eq!(nals[1], pps);
        assert_eq!(nals[2], idr);
        assert_eq!(nals[3], p);
    }

    #[test]
    fn assembler_glues_parameter_sets_to_their_frame() {
        let mut asm = AuAssembler::new();
        let mut units = Vec::new();
        for n in [nal(0x67, 8), nal(0x68, 4), nal(0x65, 100), nal(0x41, 50), nal(0x41, 50)] {
            if let Some(au) = asm.push(n) {
                units.push(au);
            }
        }
        if let Some(au) = asm.finish() {
            units.push(au);
        }
        assert_eq!(units.len(), 3);
        assert_eq!(nal_type(&units[0]), 7);
        assert!(units[0].len() > 900 / 8);
        assert_eq!(nal_type(&units[1]), 1);
        assert_eq!(nal_type(&units[2]), 1);
    }

    #[test]
    fn sliced_frames_stay_one_access_unit() {
        let mut asm = AuAssembler::new();
        let mut units = Vec::new();
        for n in [
            nal(0x67, 8),
            nal(0x68, 4),
            vec![0, 0, 0, 1, 0x65, 0x88, 0xAA],
            vec![0, 0, 0, 1, 0x65, 0x08, 0xAA],
            vec![0, 0, 0, 1, 0x41, 0x9A, 0xAA],
            vec![0, 0, 0, 1, 0x41, 0x1A, 0xAA],
        ] {
            if let Some(au) = asm.push(n) {
                units.push(au);
            }
        }
        if let Some(au) = asm.finish() {
            units.push(au);
        }
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].windows(5).filter(|w| *w == [0, 0, 0, 1, 0x65]).count(), 2);
        assert_eq!(units[1].windows(5).filter(|w| *w == [0, 0, 0, 1, 0x41]).count(), 2);
    }

    #[test]
    fn three_byte_start_codes_are_understood() {
        let mut splitter = AnnexBSplitter::new();
        let mut data = vec![0, 0, 1, 0x67, 0xFF];
        data.extend_from_slice(&[0, 0, 1, 0x65, 0xEE, 0xEE]);
        data.extend_from_slice(&[0, 0, 1, 0x41]);
        let nals = splitter.push(&data);
        assert_eq!(nals.len(), 2);
        assert_eq!(nal_type(&nals[0]), 7);
        assert_eq!(nal_type(&nals[1]), 5);
    }
}
