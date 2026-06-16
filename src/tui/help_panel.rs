use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::tui::theme;

struct HelpSection {
    title: &'static str,
    rows: &'static [(&'static str, &'static str)],
}

const SECTIONS: &[HelpSection] = &[
    HelpSection {
        title: "全局",
        rows: &[
            (
                "1-6",
                "切换面板（1 进程列表 / 2 进程树 / 3 端口 / 4 U盘 / 5 监控 / 6 Docker）",
            ),
            ("t", "切换主题（持久化）"),
            ("?", "打开/关闭本帮助页"),
            ("q", "退出 proc"),
            ("R", "切换 VT100 录制（开始/停止）"),
            ("A", "打开告警弹窗"),
        ],
    },
    HelpSection {
        title: "进程列表",
        rows: &[
            ("Space", "多选 / 取消多选"),
            ("a", "全选可见进程"),
            ("k", "终止进程"),
            ("K", "强制终止（含子进程树）"),
            ("Enter", "查看进程详情"),
            ("←→", "切换排序字段"),
            ("v", "切换视图（列表 / 树 / 应用分组）"),
            ("/", "进入搜索"),
            ("S", "按安全分排序"),
            ("c", "详情页: 复制进程信息到剪贴板"),
            ("w", "详情页: 把当前进程加入监控"),
            ("o", "树视图: 选中孤儿进程"),
            ("z", "树视图: 选中僵尸/残存进程"),
            ("f", "树视图: 进入过滤搜索"),
        ],
    },
    HelpSection {
        title: "端口 / 网络",
        rows: &[
            ("g", "循环切换视图（端口 / 进程 / 远程）"),
            ("Enter", "展开进程详情或触发远程诊断"),
            ("d", "远程视图: 网络诊断工具箱"),
            ("a", "异常检测（CLOSE_WAIT 堆积等 6 种模式）"),
            ("f", "进入过滤搜索"),
            ("s", "切换排序字段"),
            ("/", "搜索（端口 / IP / 进程名）"),
            ("k", "终止占用端口的进程"),
        ],
    },
    HelpSection {
        title: "U 盘助手",
        rows: &[
            ("Enter", "选择设备"),
            ("Tab", "在设备 / 句柄列表之间切换"),
            ("k", "终止安全进程"),
            ("r", "刷新设备列表"),
            ("w", "持续监测模式（每 5s 扫描）"),
        ],
    },
    HelpSection {
        title: "监控面板",
        rows: &[
            ("a", "添加监控（按 PID / 端口 / 命令）"),
            ("d", "删除选中监控"),
            ("s", "暂停 / 恢复监控"),
            ("r", "手动重启监控目标"),
        ],
    },
    HelpSection {
        title: "Docker 面板",
        rows: &[
            ("Enter", "查看容器详情"),
            ("r", "重启容器"),
            ("s", "停止容器"),
            ("a", "开始监听事件流"),
        ],
    },
    HelpSection {
        title: "录制 / 回放",
        rows: &[
            ("Space", "回放: 播放 / 暂停"),
            ("←→", "回放: 逐帧后退 / 前进"),
            ("Shift+←→", "回放: 一次跳 10 帧"),
            ("+/-", "回放: 调速（0.5x/1x/2x/4x）"),
            ("Home / End", "回放: 跳到开头 / 结尾"),
        ],
    },
    HelpSection {
        title: "帮助页",
        rows: &[
            ("Esc / q / ?", "返回进程列表"),
            ("↑↓ / PageUp/Down", "上下滚动"),
            ("Home / End", "跳到顶部 / 底部"),
        ],
    },
];

pub fn draw(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" ? 帮助 — Esc/q 返回 ", theme::style_header()))
        .style(theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for (i, section) in SECTIONS.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            format!(" {}", section.title),
            theme::style_selected(),
        )));
        for (key, desc) in section.rows {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {:<14}", format!("[{}]", key)),
                    theme::style_info(),
                ),
                Span::styled(format!("  {}", desc), theme::style_normal()),
            ]));
        }
    }

    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(inner.height as usize);
    let scroll = (_app.help_scroll.min(max_scroll)) as u16;

    let paragraph = Paragraph::new(lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false })
        .style(theme::style_normal());
    f.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_non_empty() {
        assert!(!SECTIONS.is_empty());
        for s in SECTIONS {
            assert!(!s.rows.is_empty(), "section {} has no rows", s.title);
        }
    }

    #[test]
    fn every_shortcut_has_a_label() {
        for s in SECTIONS {
            for (k, d) in s.rows {
                assert!(!k.is_empty(), "empty key in {}", s.title);
                assert!(!d.is_empty(), "empty desc for {} in {}", k, s.title);
            }
        }
    }
}
