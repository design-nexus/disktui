mod omarchy;

use ratatui::style::{Color, Style};

pub use omarchy::{find as find_omarchy, load_user_theme, spawn_watcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
}

/// Semantic colors shared by every screen.
#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub bg: Color,
    pub bg_deep: Color,
    pub bg_raised: Color,
    pub border: Color,
    pub selection: Color,
    pub selection_fg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub muted: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub red: Color,
    pub yellow: Color,
    pub green: Color,
    pub cyan: Color,
    pub blue: Color,
    pub magenta: Color,
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::default().bg(self.bg).fg(self.fg)
    }

    pub fn status(&self) -> Style {
        if self.kind == Kind::Light && !matches!(self.fg, Color::Reset) {
            Style::default().bg(self.fg).fg(self.bg)
        } else {
            Style::default().bg(self.bg_deep).fg(self.fg)
        }
    }

    pub fn selected(&self) -> Style {
        Style::default().bg(self.selection).fg(self.selection_fg)
    }

    pub fn dimmed(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Partition-bar colors. Index 0 is the accent so the theme stays recognizable.
    pub fn segment(&self, index: usize) -> Color {
        match index % 6 {
            0 => self.accent,
            1 => self.blue,
            2 => self.green,
            3 => self.yellow,
            4 => self.magenta,
            _ => self.cyan,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Choice {
    pub id: &'static str,
    pub label: &'static str,
}

pub static CHOICES: &[Choice] = &[
    Choice {
        id: "omarchy",
        label: "Follow Omarchy",
    },
    Choice {
        id: "custom",
        label: "Custom file",
    },
    Choice {
        id: "terminal",
        label: "Terminal",
    },
    Choice {
        id: "catppuccin-mocha",
        label: "Catppuccin Mocha",
    },
    Choice {
        id: "catppuccin-macchiato",
        label: "Catppuccin Macchiato",
    },
    Choice {
        id: "catppuccin-frappe",
        label: "Catppuccin Frappé",
    },
    Choice {
        id: "catppuccin-latte",
        label: "Catppuccin Latte",
    },
    Choice {
        id: "tokyo-night",
        label: "Tokyo Night",
    },
    Choice {
        id: "dracula",
        label: "Dracula",
    },
    Choice {
        id: "gruvbox-dark",
        label: "Gruvbox Dark",
    },
    Choice {
        id: "gruvbox-light",
        label: "Gruvbox Light",
    },
    Choice {
        id: "nord",
        label: "Nord",
    },
    Choice {
        id: "rose-pine",
        label: "Rosé Pine",
    },
];

pub fn known(id: &str) -> bool {
    CHOICES.iter().any(|c| c.id == id) || id == "custom"
}

pub fn by_id(id: &str) -> Option<Theme> {
    Some(match id {
        "terminal" => terminal(),
        "catppuccin-mocha" => mocha(),
        "catppuccin-macchiato" => macchiato(),
        "catppuccin-frappe" => frappe(),
        "catppuccin-latte" => latte(),
        "tokyo-night" => tokyo_night(),
        "dracula" => dracula(),
        "gruvbox-dark" => gruvbox_dark(),
        "gruvbox-light" => gruvbox_light(),
        "nord" => nord(),
        "rose-pine" => rose_pine(),
        _ => return None,
    })
}

pub fn resolve(id: &str, omarchy: Option<&Theme>, custom: Option<&Theme>) -> Theme {
    if id == "omarchy" {
        if let Some(theme) = omarchy {
            return theme.clone();
        }
    }
    if id == "custom" {
        if let Some(theme) = custom {
            return theme.clone();
        }
    }
    by_id(id).unwrap_or_else(mocha)
}

pub fn fallback_id(omarchy_installed: bool) -> &'static str {
    if omarchy_installed {
        "omarchy"
    } else {
        "catppuccin-mocha"
    }
}

fn terminal() -> Theme {
    Theme {
        id: "terminal".into(),
        name: "Terminal".into(),
        kind: Kind::Dark,
        bg: Color::Reset,
        bg_deep: Color::Reset,
        bg_raised: Color::Reset,
        border: Color::DarkGray,
        selection: Color::DarkGray,
        selection_fg: Color::White,
        fg: Color::Reset,
        fg_dim: Color::Gray,
        muted: Color::DarkGray,
        accent: Color::Cyan,
        on_accent: Color::Black,
        red: Color::Red,
        yellow: Color::Yellow,
        green: Color::Green,
        cyan: Color::Cyan,
        blue: Color::Blue,
        magenta: Color::Magenta,
    }
}

/// bg, deep, raised, border, selection, fg, dim, muted, accent, red, yellow, green, cyan, blue, magenta
fn pal(id: &'static str, name: &'static str, kind: Kind, c: [&str; 15]) -> Theme {
    let accent = hex(c[8]);
    let fg = hex(c[5]);
    let bg = hex(c[0]);
    let selection = hex(c[4]);
    Theme {
        id: id.into(),
        name: name.into(),
        kind,
        bg,
        bg_deep: hex(c[1]),
        bg_raised: hex(c[2]),
        border: hex(c[3]),
        selection,
        selection_fg: readable(selection, fg, bg),
        fg,
        fg_dim: hex(c[6]),
        muted: hex(c[7]),
        accent,
        on_accent: on_color(accent),
        red: hex(c[9]),
        yellow: hex(c[10]),
        green: hex(c[11]),
        cyan: hex(c[12]),
        blue: hex(c[13]),
        magenta: hex(c[14]),
    }
}

pub fn hex(s: &str) -> Color {
    parse_hex(s).unwrap_or(Color::White)
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

pub fn luminance(color: Color) -> f32 {
    match color {
        Color::Rgb(r, g, b) => 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32,
        Color::Black => 0.0,
        Color::White => 255.0,
        Color::Yellow | Color::Cyan | Color::Green | Color::Gray => 170.0,
        _ => 50.0,
    }
}

pub fn on_color(bg: Color) -> Color {
    if luminance(bg) > 150.0 {
        Color::Black
    } else {
        Color::Rgb(255, 255, 255)
    }
}

/// Pick the foreground that stays readable on `bg`.
pub fn readable(bg: Color, prefer: Color, alt: Color) -> Color {
    let prefer_gap = (luminance(bg) - luminance(prefer)).abs();
    let alt_gap = (luminance(bg) - luminance(alt)).abs();
    if prefer_gap >= 90.0 || prefer_gap >= alt_gap {
        prefer
    } else {
        alt
    }
}

fn mocha() -> Theme {
    pal(
        "catppuccin-mocha",
        "Catppuccin Mocha",
        Kind::Dark,
        [
            "1e1e2e", "11111b", "313244", "45475a", "45475a", "cdd6f4", "bac2de", "6c7086",
            "cba6f7", "f38ba8", "f9e2af", "a6e3a1", "94e2d5", "89b4fa", "f5c2e7",
        ],
    )
}
fn macchiato() -> Theme {
    pal(
        "catppuccin-macchiato",
        "Catppuccin Macchiato",
        Kind::Dark,
        [
            "24273a", "181926", "363a4f", "494d64", "494d64", "cad3f5", "b8c0e0", "6e738d",
            "c6a0f6", "ed8796", "eed49f", "a6da95", "8bd5ca", "8aadf4", "f5bde6",
        ],
    )
}
fn frappe() -> Theme {
    pal(
        "catppuccin-frappe",
        "Catppuccin Frappé",
        Kind::Dark,
        [
            "303446", "232634", "414559", "51576d", "51576d", "c6d0f5", "b5bfe2", "737994",
            "ca9ee6", "e78284", "e5c890", "a6d189", "81c8be", "8caaee", "f4b8e4",
        ],
    )
}
fn latte() -> Theme {
    pal(
        "catppuccin-latte",
        "Catppuccin Latte",
        Kind::Light,
        [
            "eff1f5", "e6e9ef", "ccd0da", "acb0be", "bcc0cc", "4c4f69", "5c5f77", "8c8fa1",
            "8839ef", "d20f39", "df8e1d", "40a02b", "179299", "1e66f5", "ea76cb",
        ],
    )
}
fn tokyo_night() -> Theme {
    pal(
        "tokyo-night",
        "Tokyo Night",
        Kind::Dark,
        [
            "1a1b26", "16161e", "292e42", "3b4261", "33467c", "c0caf5", "a9b1d6", "565f89",
            "7aa2f7", "f7768e", "e0af68", "9ece6a", "7dcfff", "7aa2f7", "bb9af7",
        ],
    )
}
fn dracula() -> Theme {
    pal(
        "dracula",
        "Dracula",
        Kind::Dark,
        [
            "282a36", "21222c", "343746", "44475a", "44475a", "f8f8f2", "f8f8f2", "6272a4",
            "bd93f9", "ff5555", "f1fa8c", "50fa7b", "8be9fd", "8be9fd", "ff79c6",
        ],
    )
}
fn gruvbox_dark() -> Theme {
    pal(
        "gruvbox-dark",
        "Gruvbox Dark",
        Kind::Dark,
        [
            "282828", "1d2021", "3c3836", "504945", "504945", "ebdbb2", "d5c4a1", "928374",
            "fe8019", "fb4934", "fabd2f", "b8bb26", "8ec07c", "83a598", "d3869b",
        ],
    )
}
fn gruvbox_light() -> Theme {
    pal(
        "gruvbox-light",
        "Gruvbox Light",
        Kind::Light,
        [
            "fbf1c7", "f2e5bc", "ebdbb2", "d5c4a1", "d5c4a1", "3c3836", "504945", "7c6f64",
            "d65d0e", "cc241d", "d79921", "98971a", "689d6a", "458588", "b16286",
        ],
    )
}
fn nord() -> Theme {
    pal(
        "nord",
        "Nord",
        Kind::Dark,
        [
            "2e3440", "242933", "3b4252", "434c5e", "434c5e", "d8dee9", "e5e9f0", "4c566a",
            "88c0d0", "bf616a", "ebcb8b", "a3be8c", "8fbcbb", "81a1c1", "b48ead",
        ],
    )
}
fn rose_pine() -> Theme {
    pal(
        "rose-pine",
        "Rosé Pine",
        Kind::Dark,
        [
            "191724", "16141f", "1f1d2e", "524f67", "403d52", "e0def4", "908caa", "6e6a86",
            "c4a7e7", "eb6f92", "f6c177", "31748f", "9ccfd8", "9ccfd8", "c4a7e7",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_the_built_in_set() {
        for id in [
            "omarchy",
            "custom",
            "terminal",
            "catppuccin-mocha",
            "catppuccin-macchiato",
            "catppuccin-frappe",
            "catppuccin-latte",
            "tokyo-night",
            "dracula",
            "gruvbox-dark",
            "gruvbox-light",
            "nord",
            "rose-pine",
        ] {
            assert!(known(id), "missing {id}");
            if id != "omarchy" && id != "custom" {
                assert_eq!(by_id(id).unwrap().id, id);
            }
        }
    }

    #[test]
    fn omarchy_wins_when_present() {
        let mut custom = mocha();
        custom.name = "Omarchy · catppuccin".into();
        let theme = resolve("omarchy", Some(&custom), None);
        assert_eq!(theme.name, "Omarchy · catppuccin");
        let fallback = resolve("omarchy", None, None);
        assert_eq!(fallback.id, "catppuccin-mocha");
    }

    #[test]
    fn light_selection_keeps_contrast() {
        let theme = latte();
        let gap = (luminance(theme.selection) - luminance(theme.selection_fg)).abs();
        assert!(gap >= 90.0, "gap {gap}");
    }
}
