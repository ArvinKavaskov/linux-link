use tokio::process::Command;

const SINK: &str = "@DEFAULT_SINK@";
const STEP: &str = "5%";

pub async fn apply(action: &str, value: i32) {
    let result = match action {
        "up" => pactl(&["set-sink-volume", SINK, &format!("+{STEP}")]).await,
        "down" => pactl(&["set-sink-volume", SINK, &format!("-{STEP}")]).await,
        "mute" => pactl(&["set-sink-mute", SINK, "toggle"]).await,
        "set" => {
            let clamped = value.clamp(0, 150);
            pactl(&["set-sink-volume", SINK, &format!("{clamped}%")]).await
        }
        other => {
            tracing::warn!("unknown volume action: {other}");
            return;
        }
    };
    match result {
        Ok(true) => tracing::info!("🔊 PC volume: {action}"),
        Ok(false) => tracing::warn!("pactl rejected the volume command"),
        Err(e) => tracing::warn!("pactl unavailable ({e}) — install pulseaudio-utils"),
    }
}

async fn pactl(args: &[&str]) -> std::io::Result<bool> {
    Ok(Command::new("pactl").args(args).status().await?.success())
}
