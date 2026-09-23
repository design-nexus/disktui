use std::path::{Path, PathBuf};

use notify::Watcher;
use ratatui::style::Color;
use serde::Deserialize;

use super::{hex, on_color, readable, Kind, Theme};

/// Newest Omarchy writes the live theme here. Older installs used the config dir.
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/state/omarchy/current"));
        dirs.push(home.join(".config/omarchy/current"));
    }
    dirs
}

pub fn find() -> Option<(String, Theme, PathBuf)> {
    for dir in candidate_dirs() {
        if let Some((name, theme)) = load_from(&dir) {
            return Some((name, theme, dir));
        }
    }
    None
}

pub fn user_theme_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("disktui").join("theme.toml"))
}

pub fn load_user_theme() -> Option<Theme> {
    let path = user_theme_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    theme_from_str(&text, "custom").ok().map(|mut theme| {
        theme.id = "custom".into();
        if !theme.name.starts_with("Custom") {
            theme.name = format!("Custom · {}", theme.name);
        }
        theme
    })
}

/// `dir` is the `current` directory that holds `theme/colors.toml` and `theme.name`.
pub fn load_from(dir: &Path) -> Option<(String, Theme)> {
    let colors_path = dir.join("theme").join("colors.toml");
    let text = std::fs::read_to_string(colors_path).ok()?;
    let name = std::fs::read_to_string(dir.join("theme.name"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let name = if name.is_empty() {
        "omarchy".into()
    } else {
        name
    };
    let theme = theme_from_str(&text, &name).ok()?;
    Some((name, theme))
}

pub fn theme_from_str(text: &str, name: &str) -> Result<Theme, String> {
    let raw: RawColors = toml::from_str(text).map_err(|e| e.to_string())?;
    let background = required(&raw.background, "background")?;
    let foreground = required(&raw.foreground, "foreground")?;
    let deep = first([
        &raw.dark_background,
        &raw.darker_background,
        &raw.background,
    ]);
    let raised = first([&raw.lighter_background, &raw.selection, &raw.background]);
    let border = first([&raw.muted, &raw.selection, &raw.dark_foreground]);
    let selection = first([&raw.selection, &raw.lighter_background, &raw.muted]);
    let dim = first([
        &raw.light_foreground,
        &raw.foreground,
        &raw.bright_foreground,
    ]);
    let muted = first([&raw.dark_foreground, &raw.muted, &raw.light_foreground]);
    let accent = first([&raw.accent, &raw.blue, &raw.cyan]);
    let red = first([&raw.red, &raw.bright_red, &raw.accent]);
    let yellow = first([&raw.yellow, &raw.bright_yellow, &raw.accent]);
    let green = first([&raw.green, &raw.bright_green, &raw.accent]);
    let cyan = first([&raw.cyan, &raw.bright_cyan, &raw.blue]);
    let blue = first([&raw.blue, &raw.bright_blue, &raw.accent]);
    let magenta = first([&raw.magenta, &raw.bright_magenta, &raw.accent]);
    let kind = if raw.mode.as_deref() == Some("light") {
        Kind::Light
    } else {
        Kind::Dark
    };
    let selection_fg = readable(selection, foreground, background);
    Ok(Theme {
        id: "omarchy".into(),
        name: format!("Omarchy · {name}"),
        kind,
        bg: background,
        bg_deep: deep,
        bg_raised: raised,
        border,
        selection,
        selection_fg,
        fg: foreground,
        fg_dim: dim,
        muted,
        accent,
        on_accent: on_color(accent),
        red,
        yellow,
        green,
        cyan,
        blue,
        magenta,
    })
}

fn required(value: &Option<String>, key: &str) -> Result<Color, String> {
    let raw = value.as_deref().ok_or_else(|| format!("missing {key}"))?;
    super::parse_hex(raw).ok_or_else(|| format!("bad color for {key}"))
}

fn first(values: [&Option<String>; 3]) -> Color {
    for value in values {
        if let Some(text) = value {
            if let Some(color) = super::parse_hex(text) {
                return color;
            }
        }
    }
    hex("888888")
}

#[derive(Debug, Deserialize)]
struct RawColors {
    mode: Option<String>,
    accent: Option<String>,
    selection: Option<String>,
    muted: Option<String>,
    background: Option<String>,
    dark_background: Option<String>,
    darker_background: Option<String>,
    lighter_background: Option<String>,
    foreground: Option<String>,
    dark_foreground: Option<String>,
    light_foreground: Option<String>,
    bright_foreground: Option<String>,
    red: Option<String>,
    yellow: Option<String>,
    green: Option<String>,
    cyan: Option<String>,
    blue: Option<String>,
    magenta: Option<String>,
    bright_red: Option<String>,
    bright_yellow: Option<String>,
    bright_green: Option<String>,
    bright_cyan: Option<String>,
    bright_blue: Option<String>,
    bright_magenta: Option<String>,
}

pub fn spawn_watcher(path: PathBuf, on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let (stx, srx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(stx) {
            Ok(watcher) => watcher,
            Err(_) => return,
        };
        if watcher
            .watch(&path, notify::RecursiveMode::Recursive)
            .is_err()
        {
            return;
        }
        while let Ok(event) = srx.recv() {
            if event.is_ok() {
                on_change();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys copied from the installed Catppuccin Omarchy theme.
    const CATPPUCCIN: &str = r##"
mode = "dark"

accent = "#89b4fa"
selection = "#45475a"
muted = "#585b70"

background = "#1e1e2e"
dark_background = "#161622"
darker_background = "#101019"
lighter_background = "#313244"

foreground = "#cdd6f4"
dark_foreground = "#6c7086"
light_foreground = "#bac2de"
bright_foreground = "#cdd6f4"

red = "#f38ba8"
yellow = "#f9e2af"
green = "#a6e3a1"
cyan = "#94e2d5"
blue = "#89b4fa"
magenta = "#f5c2e7"

bright_red = "#f38ba8"
bright_yellow = "#f9e2af"
bright_green = "#a6e3a1"
bright_cyan = "#94e2d5"
bright_blue = "#89b4fa"
bright_magenta = "#f5c2e7"
"##;

    #[test]
    fn parses_omarchy_catppuccin_colors() {
        let theme = theme_from_str(CATPPUCCIN, "catppuccin").unwrap();
        assert_eq!(theme.name, "Omarchy · catppuccin");
        assert_eq!(theme.kind, Kind::Dark);
        assert_eq!(theme.bg, Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(theme.accent, Color::Rgb(0x89, 0xb4, 0xfa));
        assert_eq!(theme.green, Color::Rgb(0xa6, 0xe3, 0xa1));
        let gap = (super::super::luminance(theme.selection)
            - super::super::luminance(theme.selection_fg))
        .abs();
        assert!(gap >= 90.0, "selection contrast {gap}");
    }

    #[test]
    fn light_mode_is_honored() {
        let theme = theme_from_str(
            "mode = \"light\"\nbackground = \"#ffffff\"\nforeground = \"#111111\"\n",
            "paper",
        )
        .unwrap();
        assert_eq!(theme.kind, Kind::Light);
        assert_eq!(theme.fg, Color::Rgb(0x11, 0x11, 0x11));
    }

    #[test]
    fn missing_background_is_an_error() {
        let err = theme_from_str("foreground = \"#ffffff\"\n", "x").unwrap_err();
        assert!(err.contains("background"), "{err}");
    }
}
