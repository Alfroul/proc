use ratatui::style::{Color, Modifier, Style};
use std::sync::atomic::{AtomicUsize, Ordering};

static THEME_INDEX: AtomicUsize = AtomicUsize::new(0);

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

const THEMES: [ThemeColors; 6] = [
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
];

fn current() -> &'static ThemeColors {
    &THEMES[THEME_INDEX.load(Ordering::Relaxed) % THEMES.len()]
}

pub fn cycle_theme() {
    let idx = THEME_INDEX.load(Ordering::Relaxed);
    THEME_INDEX.store((idx + 1) % THEMES.len(), Ordering::Relaxed);
}

pub fn theme_name() -> &'static str {
    current().name
}

pub fn bg_primary() -> Color { current().bg_primary }
pub fn bg_sidebar() -> Color { current().bg_sidebar }
pub fn text_primary() -> Color { current().text_primary }
pub fn accent() -> Color { current().accent }
pub fn danger() -> Color { current().danger }
pub fn warning() -> Color { current().warning }
pub fn success() -> Color { current().success }
pub fn info() -> Color { current().info }
pub fn muted() -> Color { current().muted }

pub fn style_selected() -> Style {
    Style::new().fg(accent()).add_modifier(Modifier::BOLD)
}
pub fn style_header() -> Style {
    Style::new().fg(current().text_primary).add_modifier(Modifier::BOLD)
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
