use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static THEME_INDEX: AtomicUsize = AtomicUsize::new(0);

/// Marks whether the persisted theme index has been loaded from disk.
/// Ensures `init_persisted_theme` runs only once per process.
static THEME_INIT: OnceLock<()> = OnceLock::new();

struct ThemeColors {
    name: &'static str,
    bg_primary: Color,
    bg_sidebar: Color,
    text_primary: Color,
    accent: Color,
    danger: Color,
    warning: Color,
    success: Color,
    info: Color,
    muted: Color,
}

const THEMES: [ThemeColors; 10] = [
    ThemeColors {
        name: "Dark",
        bg_primary: Color::Rgb(30, 30, 46),
        bg_sidebar: Color::Rgb(24, 24, 37),
        text_primary: Color::White,
        accent: Color::Cyan,
        danger: Color::Red,
        warning: Color::Yellow,
        success: Color::Green,
        info: Color::Blue,
        muted: Color::DarkGray,
    },
    ThemeColors {
        name: "Catppuccin",
        bg_primary: Color::Rgb(30, 30, 46),
        bg_sidebar: Color::Rgb(24, 24, 37),
        text_primary: Color::Rgb(205, 214, 244),
        accent: Color::Rgb(137, 180, 250),
        danger: Color::Rgb(243, 139, 168),
        warning: Color::Rgb(249, 226, 175),
        success: Color::Rgb(166, 227, 161),
        info: Color::Rgb(116, 199, 236),
        muted: Color::Rgb(88, 91, 112),
    },
    ThemeColors {
        name: "Dracula",
        bg_primary: Color::Rgb(40, 42, 54),
        bg_sidebar: Color::Rgb(34, 36, 46),
        text_primary: Color::Rgb(248, 248, 242),
        accent: Color::Rgb(189, 147, 249),
        danger: Color::Rgb(255, 85, 85),
        warning: Color::Rgb(241, 250, 140),
        success: Color::Rgb(80, 250, 123),
        info: Color::Rgb(98, 214, 247),
        muted: Color::Rgb(98, 98, 142),
    },
    ThemeColors {
        name: "Gruvbox",
        bg_primary: Color::Rgb(40, 40, 40),
        bg_sidebar: Color::Rgb(29, 32, 33),
        text_primary: Color::Rgb(235, 219, 178),
        accent: Color::Rgb(214, 93, 14),
        danger: Color::Rgb(204, 36, 29),
        warning: Color::Rgb(250, 189, 47),
        success: Color::Rgb(152, 151, 26),
        info: Color::Rgb(69, 133, 136),
        muted: Color::Rgb(146, 131, 116),
    },
    ThemeColors {
        name: "One Dark",
        bg_primary: Color::Rgb(40, 44, 52),
        bg_sidebar: Color::Rgb(33, 37, 43),
        text_primary: Color::Rgb(171, 178, 191),
        accent: Color::Rgb(97, 175, 239),
        danger: Color::Rgb(224, 108, 117),
        warning: Color::Rgb(229, 192, 123),
        success: Color::Rgb(152, 195, 121),
        info: Color::Rgb(86, 182, 194),
        muted: Color::Rgb(92, 99, 112),
    },
    ThemeColors {
        name: "Rose Pine",
        bg_primary: Color::Rgb(25, 23, 36),
        bg_sidebar: Color::Rgb(31, 29, 46),
        text_primary: Color::Rgb(224, 222, 244),
        accent: Color::Rgb(196, 167, 231),
        danger: Color::Rgb(235, 111, 146),
        warning: Color::Rgb(233, 185, 110),
        success: Color::Rgb(46, 194, 126),
        info: Color::Rgb(110, 193, 228),
        muted: Color::Rgb(110, 106, 134),
    },
    ThemeColors {
        name: "Nord",
        bg_primary: Color::Rgb(46, 52, 64),
        bg_sidebar: Color::Rgb(40, 45, 56),
        text_primary: Color::Rgb(216, 222, 233),
        accent: Color::Rgb(136, 192, 208),
        danger: Color::Rgb(191, 97, 106),
        warning: Color::Rgb(235, 203, 139),
        success: Color::Rgb(163, 190, 140),
        info: Color::Rgb(129, 161, 193),
        muted: Color::Rgb(76, 86, 106),
    },
    ThemeColors {
        name: "Solarized",
        bg_primary: Color::Rgb(0, 43, 54),
        bg_sidebar: Color::Rgb(7, 54, 66),
        text_primary: Color::Rgb(131, 148, 150),
        accent: Color::Rgb(38, 139, 210),
        danger: Color::Rgb(220, 50, 47),
        warning: Color::Rgb(181, 137, 0),
        success: Color::Rgb(133, 153, 0),
        info: Color::Rgb(42, 161, 152),
        muted: Color::Rgb(88, 110, 117),
    },
    ThemeColors {
        name: "Tokyo Night",
        bg_primary: Color::Rgb(26, 27, 38),
        bg_sidebar: Color::Rgb(22, 23, 33),
        text_primary: Color::Rgb(169, 177, 214),
        accent: Color::Rgb(122, 162, 247),
        danger: Color::Rgb(247, 118, 142),
        warning: Color::Rgb(224, 175, 104),
        success: Color::Rgb(158, 206, 106),
        info: Color::Rgb(125, 207, 255),
        muted: Color::Rgb(82, 86, 114),
    },
    ThemeColors {
        name: "Light",
        bg_primary: Color::Rgb(250, 250, 252),
        bg_sidebar: Color::Rgb(240, 240, 245),
        text_primary: Color::Rgb(40, 40, 50),
        accent: Color::Rgb(0, 102, 204),
        danger: Color::Rgb(200, 30, 30),
        warning: Color::Rgb(180, 120, 0),
        success: Color::Rgb(0, 140, 60),
        info: Color::Rgb(70, 110, 200),
        muted: Color::Rgb(110, 110, 125),
    },
];

fn theme_path() -> Option<std::path::PathBuf> {
    Some(crate::dirs_config_dir().join("theme.txt"))
}

fn load_theme() -> Option<usize> {
    let raw = std::fs::read_to_string(theme_path()?).ok()?;
    let trimmed = raw.trim();
    let idx: usize = trimmed.parse().ok()?;
    (idx < THEMES.len()).then_some(idx)
}

fn save_theme(idx: usize) {
    if let Some(p) = theme_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(p, idx.to_string());
    }
}

/// Load the persisted theme index exactly once at startup. Safe to call from
/// any code path before the first render — subsequent calls are no-ops.
pub fn init_persisted_theme() {
    if THEME_INIT.get().is_some() {
        return;
    }
    if THEME_INIT.set(()).is_ok()
        && let Some(idx) = load_theme()
    {
        THEME_INDEX.store(idx, Ordering::Relaxed);
    }
}

fn current() -> &'static ThemeColors {
    &THEMES[THEME_INDEX.load(Ordering::Relaxed) % THEMES.len()]
}

pub fn cycle_theme() {
    init_persisted_theme();
    let idx = THEME_INDEX.load(Ordering::Relaxed);
    let new = (idx + 1) % THEMES.len();
    THEME_INDEX.store(new, Ordering::Relaxed);
    save_theme(new);
}

pub fn theme_name() -> &'static str {
    current().name
}

pub fn theme_count() -> usize {
    THEMES.len()
}

pub fn bg_primary() -> Color {
    current().bg_primary
}
pub fn bg_sidebar() -> Color {
    current().bg_sidebar
}
pub fn text_primary() -> Color {
    current().text_primary
}
pub fn accent() -> Color {
    current().accent
}
pub fn danger() -> Color {
    current().danger
}
pub fn warning() -> Color {
    current().warning
}
pub fn success() -> Color {
    current().success
}
pub fn info() -> Color {
    current().info
}
pub fn muted() -> Color {
    current().muted
}

pub fn style_selected() -> Style {
    Style::new().fg(accent()).add_modifier(Modifier::BOLD)
}
pub fn style_header() -> Style {
    Style::new()
        .fg(current().text_primary)
        .add_modifier(Modifier::BOLD)
}
pub fn style_normal() -> Style {
    Style::new().fg(current().text_primary)
}
pub fn style_muted() -> Style {
    Style::new().fg(current().muted)
}
pub fn style_danger() -> Style {
    Style::new().fg(danger())
}
pub fn style_warning() -> Style {
    Style::new().fg(warning())
}
pub fn style_success() -> Style {
    Style::new().fg(success())
}
pub fn style_info() -> Style {
    Style::new().fg(info())
}

pub fn risk_color(level: &str) -> Color {
    match level {
        "critical" => danger(),
        "warning" => warning(),
        "safe" => success(),
        _ => muted(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_includes_light() {
        assert_eq!(THEMES.len(), 10);
        assert_eq!(THEMES[9].name, "Light");
    }

    #[test]
    fn cycle_theme_wraps_within_bounds() {
        for _ in 0..(THEMES.len() * 3) {
            cycle_theme();
        }
        let idx = THEME_INDEX.load(Ordering::Relaxed);
        assert!(idx < THEMES.len());
    }

    /// Parse validation — does not touch disk to keep parallel tests deterministic.
    fn parse_index(raw: &str) -> Option<usize> {
        let idx: usize = raw.trim().parse().ok()?;
        (idx < THEMES.len()).then_some(idx)
    }

    #[test]
    fn parse_index_accepts_valid() {
        assert_eq!(parse_index("0"), Some(0));
        assert_eq!(parse_index("9"), Some(9));
        assert_eq!(parse_index("  3  "), Some(3));
    }

    #[test]
    fn parse_index_rejects_out_of_range() {
        assert_eq!(parse_index("999"), None);
    }

    #[test]
    fn parse_index_rejects_garbage() {
        assert_eq!(parse_index("garbage"), None);
        assert_eq!(parse_index(""), None);
    }
}
