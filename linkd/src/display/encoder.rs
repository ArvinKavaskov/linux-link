//! Turns a capture source into a stream of H.264 access units.
//!
//! We do not link against an encoder. GStreamer and FFmpeg are already on the
//! machines we target (the webcam feature needs ffmpeg, PipeWire desktops ship
//! the gst stack), and a subprocess writing Annex-B to its stdout is a process
//! boundary we can kill cleanly — no half-initialised VA-API context to unwind
//! when the tablet walks out of Wi-Fi range.
//!
//! The one real piece of work here is framing. The child gives us an Annex-B
//! byte stream cut wherever the pipe felt like it; Android's MediaCodec wants
//! whole access units. [`AnnexBSplitter`] finds NAL boundaries and
//! [`AuAssembler`] groups NALs so that SPS/PPS travel glued to the IDR frame
//! they describe — the first buffer the decoder sees is then always
//! self-contained.

use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::has_cmd;

/// What the backend managed to set up for us to point an encoder at.
pub enum CaptureSource {
    /// A PipeWire node (Mutter virtual monitor, or a portal stream on KDE).
    /// `negotiate` asks pipewiresrc to drive the format to this size — that is
    /// how Mutter decides how big the virtual monitor is.
    PipeWire { node: u32, negotiate: Option<(u32, u32)> },
    /// A named wlroots output, captured with wf-recorder (Hyprland, Sway).
    WlrOutput { name: String },
    /// A region of the X11 root window, captured with ffmpeg's x11grab.
    X11Region { x: u32, y: u32, width: u32, height: u32 },
}

pub struct Encoder {
    child: Child,
    /// Assembled access units, ready to length-prefix onto the wire.
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

        // The child's complaints are the only clue when a pipeline dies, and
        // they die for local reasons (missing plugin, refused caps). Keep the
        // last lines and log them when the stream ends.
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

        let (tx, rx) = mpsc::channel::<Vec<u8>>(16);
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
        let n = match stdout.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for nal in splitter.push(&buf[..n]) {
            if let Some(au) = assembler.push(nal) {
                // If the consumer lags, dropping old frames is the right call
                // for a live stream — never queue latency.
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

/// Picks the encoder command for a capture source, preferring elements that
/// actually exist on this machine.
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
            let enc = gst_encoder(bitrate_kbps)?;
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
                    "preset=ultrafast".into(),
                    "-p".into(),
                    "tune=zerolatency".into(),
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
                    "ultrafast".into(),
                    "-tune".into(),
                    "zerolatency".into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-b:v".into(),
                    format!("{bitrate_kbps}k"),
                    "-x264-params".into(),
                    format!("repeat-headers=1:keyint={}", fps * 2),
                    "-f".into(),
                    "h264".into(),
                    "-".into(),
                ],
            ))
        }
    }
}

/// The gst H.264 element chain, by decreasing preference. x264 in zerolatency
/// mode is the boring, universally packaged choice; openh264 covers Fedora
/// without RPM Fusion; vah264enc is the hardware path when present.
fn gst_encoder(bitrate_kbps: u32) -> Result<Vec<String>> {
    if gst_has_element("x264enc") {
        return Ok(vec![
            "x264enc".into(),
            "tune=zerolatency".into(),
            "speed-preset=ultrafast".into(),
            format!("bitrate={bitrate_kbps}"),
            "key-int-max=120".into(),
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

/// Cuts an Annex-B byte stream into NAL units (start codes stripped of
/// nothing — each returned unit *keeps* its start code, which is exactly what
/// MediaCodec wants to see).
pub struct AnnexBSplitter {
    buf: Vec<u8>,
}

impl AnnexBSplitter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feeds bytes in, returns every complete NAL unit found so far.
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            // The first start code marks where our current NAL begins…
            let Some(first) = find_start_code(&self.buf, 0) else { break };
            // …and the next one marks where it ends.
            let Some(second) = find_start_code(&self.buf, first + 3) else { break };
            out.push(self.buf[first..second].to_vec());
            self.buf.drain(..second);
        }
        out
    }

    /// The stream is over: whatever remains after the last start code is the
    /// final NAL.
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

/// Index of the next 3-byte start code (00 00 01), including the leading zero
/// of a 4-byte one (00 00 00 01) so the code travels with its NAL.
fn find_start_code(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            // Prefer to include a fourth leading zero if there is one.
            return Some(if i > from && buf[i - 1] == 0 { i - 1 } else { i });
        }
        i += 1;
    }
    None
}

fn nal_type(nal: &[u8]) -> u8 {
    // Skip the 3- or 4-byte start code.
    let offset = if nal.len() > 3 && nal[2] == 1 { 3 } else { 4 };
    nal.get(offset).map(|b| b & 0x1F).unwrap_or(0)
}

fn is_vcl(t: u8) -> bool {
    (1..=5).contains(&t)
}

/// Groups NALs into access units: parameter sets and SEI attach to the frame
/// that follows them, so every emitted unit decodes on its own terms.
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
        // A new frame, parameter set or AU delimiter after a frame closes the
        // current access unit.
        let flush = self.has_vcl && (is_vcl(t) || matches!(t, 6 | 7 | 8 | 9));
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

        // Feed in awkward 7-byte chunks, as the pipe might.
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
        // SPS+PPS+IDR together, then each P frame on its own.
        assert_eq!(units.len(), 3);
        assert_eq!(nal_type(&units[0]), 7);
        assert!(units[0].len() > 900 / 8); // contains the IDR too
        assert_eq!(nal_type(&units[1]), 1);
        assert_eq!(nal_type(&units[2]), 1);
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
