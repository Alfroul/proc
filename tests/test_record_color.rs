//! VT100 颜色编码 roundtrip 测试（阶段 2 / ADR 0003）。
//!
//! 覆盖 `pack_color` / `unpack_color` 的可变 32-bit 编码：
//!
//! - Reset / 16 基本色 / Indexed(u8) 走调色板路径
//! - Color::Rgb(r, g, b) 走 RGB 标记位路径
//!
//! 以及完整 `Buffer → VtFrame → bincode → VtFrame → Buffer` 的颜色一致性。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

use proc::record::vt100::{
    VT100_VERSION, VtFrame, VtFrameWidget, VtPlayer, VtRecorder, pack_color, unpack_color,
};

#[test]
fn test_pack_unpack_reset() {
    assert_eq!(unpack_color(pack_color(Color::Reset)), Color::Reset);
    // Reset 编码为调色板模式的 0
    assert_eq!(pack_color(Color::Reset), 0);
}

#[test]
fn test_pack_unpack_basic_16() {
    let palette = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    for c in palette {
        let packed = pack_color(c);
        // 基本色不使用 RGB 标记位
        assert_eq!(packed & 0x8000_0000, 0, "基本色 {c:?} 误入 RGB 模式");
        assert_eq!(unpack_color(packed), c);
    }
}

#[test]
fn test_pack_unpack_rgb() {
    // 涵盖 6 个内置主题实际使用的 RGB 颜色 + 边界值。
    let rgbs = [
        Color::Rgb(30, 30, 46),    // Dark / Catppuccin bg_primary
        Color::Rgb(137, 180, 250), // Catppuccin accent
        Color::Rgb(189, 147, 249), // Dracula accent
        Color::Rgb(136, 192, 208), // Nord accent
        Color::Rgb(247, 118, 142), // Tokyo Night danger
        Color::Rgb(0, 0, 0),       // 边界：纯黑
        Color::Rgb(255, 255, 255), // 边界：纯白
    ];
    for c in rgbs {
        let packed = pack_color(c);
        // RGB 必须落标记位
        assert_ne!(packed & 0x8000_0000, 0, "RGB {c:?} 缺少标记位");
        assert_eq!(unpack_color(packed), c);
    }
}

#[test]
fn test_pack_unpack_indexed() {
    assert_eq!(
        unpack_color(pack_color(Color::Indexed(0))),
        Color::Indexed(0)
    );
    assert_eq!(
        unpack_color(pack_color(Color::Indexed(100))),
        Color::Indexed(100)
    );
    assert_eq!(
        unpack_color(pack_color(Color::Indexed(255))),
        Color::Indexed(255)
    );
}

#[test]
fn test_rgb_preserves_theme_colors() {
    // 模拟 6 个内置主题的关键 RGB 颜色，确保序列化/反序列化后完全一致。
    let theme_colors = [
        Color::Rgb(30, 30, 46),
        Color::Rgb(24, 24, 37),
        Color::Rgb(205, 214, 244),
        Color::Rgb(137, 180, 250),
        Color::Rgb(243, 139, 168),
        Color::Rgb(40, 42, 54),
        Color::Rgb(189, 147, 249),
        Color::Rgb(255, 85, 85),
        Color::Rgb(46, 52, 64),
        Color::Rgb(136, 192, 208),
        Color::Rgb(0, 43, 54),
        Color::Rgb(38, 139, 210),
        Color::Rgb(26, 27, 38),
        Color::Rgb(122, 162, 247),
        Color::Rgb(247, 118, 142),
    ];
    for c in theme_colors {
        let packed = pack_color(c);
        let bytes = bincode::serialize(&packed).unwrap();
        let back: u32 = bincode::deserialize(&bytes).unwrap();
        let restored = unpack_color(back);
        assert_eq!(restored, c, "主题颜色 roundtrip 失败: {c:?}");
    }
}

#[test]
fn test_vt_frame_roundtrip_with_rgb() {
    // 构造 3x2 buffer，混合 RGB / 基本色 / Reset 颜色。
    let area = Rect::new(0, 0, 3, 2);
    let mut buf = Buffer::empty(area);

    if let Some(c) = buf.cell_mut((0, 0)) {
        c.set_symbol("A");
        c.set_fg(Color::Rgb(137, 180, 250)); // Catppuccin accent
        c.set_bg(Color::Rgb(30, 30, 46)); // Catppuccin bg
    }
    if let Some(c) = buf.cell_mut((1, 0)) {
        c.set_symbol("B");
        c.set_fg(Color::Red);
        c.set_bg(Color::Reset);
    }
    if let Some(c) = buf.cell_mut((2, 0)) {
        c.set_symbol("C");
        c.set_fg(Color::Indexed(100));
        c.set_bg(Color::Rgb(0, 43, 54)); // Solarized bg
    }
    // Row 1 全部 default（Reset）

    let frame = VtFrame::from_buffer(&buf, area, 12345);
    let bytes = bincode::serialize(&frame).unwrap();
    let frame2: VtFrame = bincode::deserialize(&bytes).unwrap();

    // 重新渲染到新 buffer 比对
    let mut out = Buffer::empty(area);
    VtFrameWidget::new(&frame2).render(area, &mut out);

    let c00 = out.cell((0, 0)).unwrap();
    assert_eq!(c00.symbol(), "A");
    assert_eq!(c00.fg, Color::Rgb(137, 180, 250));
    assert_eq!(c00.bg, Color::Rgb(30, 30, 46));

    let c10 = out.cell((1, 0)).unwrap();
    assert_eq!(c10.symbol(), "B");
    assert_eq!(c10.fg, Color::Red);
    assert_eq!(c10.bg, Color::Reset);

    let c20 = out.cell((2, 0)).unwrap();
    assert_eq!(c20.symbol(), "C");
    assert_eq!(c20.fg, Color::Indexed(100));
    assert_eq!(c20.bg, Color::Rgb(0, 43, 54));

    // Row 1 全部 Reset
    let c01 = out.cell((0, 1)).unwrap();
    assert_eq!(c01.fg, Color::Reset);
    assert_eq!(c01.bg, Color::Reset);
}

#[test]
fn test_v2_header_version() {
    // 常量必须是 2
    assert_eq!(VT100_VERSION, 2);

    // 写入的文件 header.version 必须是 2
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2_header.prec");
    let rec = VtRecorder::start(path.clone(), 80, 24).unwrap();
    rec.stop().unwrap();

    let player = VtPlayer::open(path).unwrap();
    assert_eq!(player.header().version, 2);
}
