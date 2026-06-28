use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::tui::theme;

/// v0.7.0 阶段 1 TD-8：worker 名截到 10 列宽（超出加 `…`），
/// 防止 `dns_log_worker`（14 字符）/ `docker_logs`（11 字符）撑爆 help_panel
/// Workers 区段的列对齐。
fn truncate_worker_name(name: &str) -> String {
    const MAX: usize = 10;
    if name.chars().count() <= MAX {
        return name.to_string();
    }
    let mut s: String = name.chars().take(MAX - 1).collect();
    s.push('…');
    s
}

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
            // v0.7 阶段 3：命令面板 —— 替代记忆 N 个键位，fuzzy 搜「kill」/「port」直达。
            ("Ctrl+P", "命令面板（fuzzy 搜命令，替代键位记忆）"),
        ],
    },
    HelpSection {
        title: "命令面板（Ctrl+P 触发）",
        rows: &[
            ("Esc", "关闭面板"),
            ("↑↓", "选择匹配项"),
            ("Enter", "执行选中命令"),
            ("Ctrl+U", "清空输入"),
            ("Backspace", "删除最后一个字符"),
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
            (
                "←→",
                "切换排序字段（持久化：CPU/内存/PID/名称/安全/磁盘读写）",
            ),
            ("v", "切换视图（列表 / 树 / 应用分组）"),
            ("/", "进入搜索（子串匹配，v0.6 行为）"),
            (":", "进入搜索（过滤表达式，v0.7 阶段 4 新增）"),
            ("S", "直达安全分排序（可疑进程排最前）"),
            ("y", "详情页: 复制进程信息到剪贴板（vim yank）"),
            ("F5", "详情页: 强制刷新 Inspector 数据"),
            ("w", "详情页: 把当前进程加入监控"),
            ("v", "详情页 Env Tab: 切换 secret 脱敏（录屏中强制 mask）"),
            ("o", "树视图: 选中孤儿进程"),
            ("z", "树视图: 选中僵尸/残存进程"),
            ("f", "树视图: 进入过滤搜索"),
        ],
    },
    HelpSection {
        title: "过滤表达式（按 : 进入，v0.7 阶段 4）",
        rows: &[
            (
                "字段",
                "cpu / mem / pid / name / user / cmd / disk_read / disk_write / net_sent / net_recv / security_score",
            ),
            ("操作符", "=  !=  >  <  >=  <=  =~（正则）"),
            ("组合", "AND  OR  NOT  ( ) — 关键字大小写敏感"),
            ("单位", "b/kb/mb/gb/tb（1024 进制字节）/ %（百分比）"),
            ("正则", "/pattern/i — i 后缀大小写不敏感"),
            ("示例1", "cpu > 5 AND name =~ /chrome/i"),
            ("示例2", "mem > 500mb OR security_score < 80"),
            (
                "示例3",
                "NOT (user = root) AND (cpu > 50 OR disk_read > 1mb)",
            ),
            (
                "错误提示",
                "parse 失败时 status_message 显示错误，保留上次成功 AST 继续过滤",
            ),
        ],
    },
    HelpSection {
        title: "端口 / 网络",
        rows: &[
            ("g", "循环切换视图（端口 / 进程 / 远程）"),
            ("Enter", "展开进程详情或触发远程诊断"),
            ("d", "远程视图: 网络诊断工具箱"),
            ("x", "触发远程诊断（Ping/DNS/Whois/Traceroute/端口探测）"),
            ("c", "复制选中行信息到剪贴板"),
            ("a", "异常检测（CLOSE_WAIT 堆积等 6 种模式）"),
            ("f", "进入过滤搜索"),
            ("s", "切换排序字段"),
            ("/", "搜索（端口 / IP / 进程名）"),
            ("k", "终止占用端口的进程"),
            ("D", "DNS 查询日志子视图（仅内存）"),
            ("F", "eBPF Flow 子视图（Linux + ebpf feature）"),
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
            ("⚠THERMAL", "侧边栏标识：CPU 因过热降频（参考降频检测）"),
            ("⚠POWER", "侧边栏标识：CPU 因功耗墙降频（参考降频检测）"),
            (
                "温度色阶",
                "CPU/GPU 温度颜色：< 70°C 绿 / 70-79 黄 / 80-89 橙 / ≥ 90 红",
            ),
        ],
    },
    HelpSection {
        title: "Docker 面板",
        rows: &[
            ("Enter", "查看容器详情"),
            ("Shift+R", "重启容器 / 刷新镜像或卷列表"),
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

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
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

    // v0.6.0 阶段 3：动态 Workers 区段（avg/max/polls/drops）。
    // 帮助页是用户报 bug 时第一个看的地方，放这里最合适。
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Workers (后台 worker 状态)",
        theme::style_selected(),
    )));
    lines.push(Line::from(Span::styled(
        "   name       badge  avg     max     polls   drops",
        theme::style_muted(),
    )));
    for entry in app.worker_metrics() {
        let s = &entry.stats;
        let badge_style = if s.health_badge() == "✓" {
            theme::style_normal()
        } else {
            theme::style_danger()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {:<10} ", truncate_worker_name(entry.name)),
                theme::style_info(),
            ),
            Span::styled(s.health_badge().to_string(), badge_style),
            Span::styled(
                format!(
                    "  {:>5}μs {:>5}μs {:>7} {:>5}",
                    s.avg_us, s.max_us, s.poll_count, s.channel_full,
                ),
                theme::style_normal(),
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "   按 D 关闭崩溃 banner（若有）",
        theme::style_muted(),
    )));

    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(inner.height as usize);
    let scroll = (app.help_scroll.min(max_scroll)) as u16;

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

    #[test]
    fn truncate_worker_name_short_unchanged() {
        assert_eq!(truncate_worker_name("port"), "port");
        assert_eq!(truncate_worker_name("docker"), "docker");
    }

    #[test]
    fn truncate_worker_name_exactly_10_unchanged() {
        assert_eq!(truncate_worker_name("0123456789"), "0123456789");
    }

    #[test]
    fn truncate_worker_name_long_truncates_with_ellipsis() {
        // v0.7.0 阶段 1 TD-8：worker 名 14 字符（如 dns_log_worker）必须
        // 截到 10 列宽，否则撑爆 Workers 区段的列对齐。
        assert_eq!(truncate_worker_name("dns_log_worker"), "dns_log_w…");
        assert_eq!(truncate_worker_name("docker_logs"), "docker_lo…");
        // 截完宽度恰好 10（9 字符 + …）。
        assert_eq!(truncate_worker_name("dns_log_worker").chars().count(), 10);
    }
}
