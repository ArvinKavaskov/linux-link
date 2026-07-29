//! Global keyboard shortcuts, registered in the desktop the user actually runs.
//!
//! There is no cross-desktop API for this — a program cannot simply "grab
//! Super+Shift+V" on Wayland, and it should not be able to. What it can do is
//! write the shortcut into the desktop's own configuration, which is exactly
//! what the user would do by hand in the settings panel. So we do that, per
//! desktop, and we do it reversibly.
//!
//! GNOME (and Zorin, Cinnamon, Budgie): a custom keybinding in
//! `org.gnome.settings-daemon.plugins.media-keys`. The bindings live at
//! individual dconf paths listed in a `custom-keybindings` array; we use named
//! paths of our own (`…/custom-keybindings/linuxlink-clipboard/`) rather than
//! the usual `custom0`, `custom1`… so we can never overwrite a shortcut the
//! user created themselves.
//!
//! KDE Plasma: a `.desktop` entry carrying `X-KDE-Shortcuts`. kglobalaccel
//! picks those up on its own.
//!
//! Hyprland: a `bind =` line in `hyprland.conf`, between two markers so we can
//! take it out again.

use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

/// One shortcut: an id used in config paths, a label, the keys, the command.
struct Shortcut {
    id: &'static str,
    name: &'static str,
    /// GNOME/GTK syntax.
    binding: &'static str,
    /// KDE syntax.
    kde: &'static str,
    /// Hyprland syntax: modifiers, then key.
    hypr: (&'static str, &'static str),
    /// `{bin}` is replaced by the absolute path to the binary directory.
    command: &'static str,
}

const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        id: "linuxlink-clipboard",
        name: "Linux Link: send the clipboard to the phone",
        binding: "<Super><Shift>v",
        kde: "Meta+Shift+V",
        hypr: ("SUPER SHIFT", "V"),
        command: "{bin}/linkd send-url",
    },
    Shortcut {
        id: "linuxlink-sendfile",
        name: "Linux Link: send a file to the phone",
        binding: "<Super><Shift>b",
        kde: "Meta+Shift+B",
        hypr: ("SUPER SHIFT", "B"),
        command: "{bin}/linkd send-file --pick",
    },
    Shortcut {
        id: "linuxlink-media",
        name: "Linux Link: play/pause on the phone",
        binding: "<Super><Shift>space",
        kde: "Meta+Shift+Space",
        hypr: ("SUPER SHIFT", "SPACE"),
        command: "{bin}/linkd media play_pause",
    },
];

const HYPR_BEGIN: &str = "# >>> Linux Link shortcuts >>>";
const HYPR_END: &str = "# <<< Linux Link shortcuts <<<";

#[derive(PartialEq, Clone, Copy)]
pub enum Desktop {
    Gnome,
    Kde,
    Hyprland,
    Unknown,
}

pub fn detect() -> Desktop {
    let d = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if d.contains("hyprland") {
        Desktop::Hyprland
    } else if d.contains("kde") || d.contains("plasma") {
        Desktop::Kde
    } else if d.contains("gnome")
        || d.contains("zorin")
        || d.contains("unity")
        || d.contains("cinnamon")
        || d.contains("budgie")
        || d.contains("pantheon")
    {
        Desktop::Gnome
    } else if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        Desktop::Hyprland
    } else {
        Desktop::Unknown
    }
}

pub fn desktop_name(d: Desktop) -> &'static str {
    match d {
        Desktop::Gnome => "GNOME",
        Desktop::Kde => "KDE Plasma",
        Desktop::Hyprland => "Hyprland",
        Desktop::Unknown => "this desktop",
    }
}

fn bin_dir() -> String {
    // The shortcut has to survive `$PATH` not being set the way our shell sees
    // it — dconf-launched commands get a very bare environment.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")).join(".local/bin")
        })
        .to_string_lossy()
        .into_owned()
}

fn command_of(s: &Shortcut) -> String {
    s.command.replace("{bin}", &bin_dir())
}

pub fn install() -> Result<()> {
    match detect() {
        Desktop::Gnome => gnome_install(),
        Desktop::Kde => kde_install(),
        Desktop::Hyprland => hyprland_install(),
        Desktop::Unknown => {
            anyhow::bail!(
                "unrecognised desktop — add the shortcuts by hand:\n{}",
                manual_help()
            )
        }
    }
}

pub fn remove() -> Result<()> {
    match detect() {
        Desktop::Gnome => gnome_remove(),
        Desktop::Kde => kde_remove(),
        Desktop::Hyprland => hyprland_remove(),
        Desktop::Unknown => Ok(()),
    }
}

/// True when our shortcuts appear to be registered right now.
pub fn installed() -> bool {
    match detect() {
        Desktop::Gnome => gnome_list()
            .map(|list| SHORTCUTS.iter().all(|s| list.iter().any(|p| p.contains(s.id))))
            .unwrap_or(false),
        Desktop::Kde => SHORTCUTS.iter().all(|s| kde_file(s.id).exists()),
        Desktop::Hyprland => std::fs::read_to_string(hypr_conf())
            .map(|c| c.contains(HYPR_BEGIN))
            .unwrap_or(false),
        Desktop::Unknown => false,
    }
}

pub fn manual_help() -> String {
    let mut out = String::new();
    for s in SHORTCUTS {
        out.push_str(&format!("  {}  →  {}\n", s.kde, command_of(s)));
    }
    out
}

// ------------------------------------------------------------------ GNOME

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const CUSTOM_ROOT: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

fn gsettings(args: &[&str]) -> Result<String> {
    let out = Command::new("gsettings").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("gsettings {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The `custom-keybindings` array, as a list of dconf paths.
fn gnome_list() -> Result<Vec<String>> {
    let raw = gsettings(&["get", MEDIA_KEYS, "custom-keybindings"])?;
    Ok(parse_gvariant_list(&raw))
}

/// `['/a/', '/b/']` → `["/a/", "/b/"]`. `@as []` for an empty array.
fn parse_gvariant_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches("@as")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn format_gvariant_list(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    format!("[{}]", items.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", "))
}

fn gnome_install() -> Result<()> {
    let mut list = gnome_list().unwrap_or_default();
    for s in SHORTCUTS {
        let path = format!("{CUSTOM_ROOT}/{}/", s.id);
        if !list.iter().any(|p| p == &path) {
            list.push(path.clone());
        }
        let target = format!("{CUSTOM_SCHEMA}:{path}");
        gsettings(&["set", &target, "name", s.name])?;
        gsettings(&["set", &target, "command", &command_of(s)])?;
        gsettings(&["set", &target, "binding", s.binding])?;
    }
    gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &format_gvariant_list(&list)])?;
    Ok(())
}

fn gnome_remove() -> Result<()> {
    let list = gnome_list().unwrap_or_default();
    let kept: Vec<String> = list
        .into_iter()
        .filter(|p| !SHORTCUTS.iter().any(|s| p.contains(s.id)))
        .collect();
    gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &format_gvariant_list(&kept)])?;
    // Also clear the values, so a reinstall does not inherit a stale command.
    for s in SHORTCUTS {
        let target = format!("{CUSTOM_SCHEMA}:{CUSTOM_ROOT}/{}/", s.id);
        let _ = gsettings(&["reset", &target, "binding"]);
        let _ = gsettings(&["reset", &target, "command"]);
        let _ = gsettings(&["reset", &target, "name"]);
    }
    Ok(())
}

// -------------------------------------------------------------------- KDE

fn kde_file(id: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".local/share/applications")
        .join(format!("{id}.desktop"))
}

fn kde_install() -> Result<()> {
    for s in SHORTCUTS {
        let path = kde_file(s.id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={}\n\
             Exec={}\n\
             Icon=linux-link\n\
             Terminal=false\n\
             NoDisplay=true\n\
             X-KDE-Shortcuts={}\n",
            s.name,
            command_of(s),
            s.kde,
        );
        std::fs::write(&path, entry)?;
    }
    // kglobalaccel only rescans when the menu cache is rebuilt.
    let _ = Command::new("kbuildsycoca6").status();
    let _ = Command::new("kbuildsycoca5").status();
    Ok(())
}

fn kde_remove() -> Result<()> {
    for s in SHORTCUTS {
        let _ = std::fs::remove_file(kde_file(s.id));
    }
    let _ = Command::new("kbuildsycoca6").status();
    let _ = Command::new("kbuildsycoca5").status();
    Ok(())
}

// --------------------------------------------------------------- Hyprland

fn hypr_conf() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/hypr/hyprland.conf")
}

fn hyprland_install() -> Result<()> {
    let path = hypr_conf();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let stripped = strip_block(&existing);
    let mut block = String::from(HYPR_BEGIN);
    block.push('\n');
    for s in SHORTCUTS {
        block.push_str(&format!(
            "bind = {}, {}, exec, {}\n",
            s.hypr.0,
            s.hypr.1,
            command_of(s)
        ));
    }
    block.push_str(HYPR_END);
    block.push('\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, format!("{}\n{block}", stripped.trim_end()))?;
    let _ = Command::new("hyprctl").arg("reload").status();
    Ok(())
}

fn hyprland_remove() -> Result<()> {
    let path = hypr_conf();
    let Ok(existing) = std::fs::read_to_string(&path) else { return Ok(()) };
    std::fs::write(&path, strip_block(&existing))?;
    let _ = Command::new("hyprctl").arg("reload").status();
    Ok(())
}

/// Removes a previous block of ours, markers included, leaving the rest of the
/// user's config exactly as it was.
fn strip_block(conf: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in conf.lines() {
        if line.trim() == HYPR_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == HYPR_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_gvariant_lists() {
        assert!(parse_gvariant_list("@as []").is_empty());
        assert_eq!(
            parse_gvariant_list("['/a/custom0/', '/a/custom1/']"),
            vec!["/a/custom0/".to_string(), "/a/custom1/".to_string()]
        );
        assert_eq!(format_gvariant_list(&[]), "@as []");
        assert_eq!(
            format_gvariant_list(&["/a/".to_string(), "/b/".to_string()]),
            "['/a/', '/b/']"
        );
    }

    #[test]
    fn strips_only_our_block() {
        let conf = "keep = 1\n# >>> Linux Link shortcuts >>>\nbind = x\n# <<< Linux Link shortcuts <<<\nkeep = 2\n";
        assert_eq!(strip_block(conf), "keep = 1\nkeep = 2\n");
    }
}
