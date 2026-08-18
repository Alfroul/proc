//! v0.7.0 阶段 3 — TUI 命令面板（Ctrl+P）。
//!
//! 解决 v0.6 的键位爆炸：每个面板 5-10 个键位，6 面板 × 6 Tab × 17+ 子命令，
//! 加上 v0.7+ 还要继续加（PSI / EcoQoS / Flow 子视图 / FilterExpr）。命令面板
//! 用 fuzzy 搜索替代记忆键位。
//!
//! 设计原则：
//! - 模态浮层：激活时（`AppLayer::Palette`）拦截所有按键，不传给面板 / 搜索 / 详情页
//! - fuzzy 用 nucleo（Helix 编辑器 fuzzy 库，性能极佳）
//! - 不替代 `?` 帮助页：帮助页是"学习键位"，面板是"快速执行"
//!
//! 详见 `docs/adr/0010-shell-completion-and-palette.md`。

use crossterm::event::{KeyCode, KeyEvent};

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app_panel::{AppMode, InspectionTab};
use crate::collect::{ProcessViewMode, SortField};
use crate::tui::theme;

/// 最多同时显示的匹配项数（其余仍可被选中向下滚动，但视觉只显示前 N 个）。
const MAX_DISPLAY: usize = 10;

/// 一条可执行命令的元数据。
#[derive(Debug)]
pub struct CommandItem {
    /// 稳定 ID，用于测试 anchor（如 "switch_to_port_panel"）。
    pub id: &'static str,
    /// 用户看到的标签，fuzzy 匹配的目标。
    pub label: &'static str,
    /// 分组标签（"Panel" / "Sort" / "Theme" / "Action"），渲染时按组分块。
    pub category: &'static str,
    /// 提示键位（"3" / "k" / "Ctrl+P"），仅作辅助显示，不影响执行。
    pub hint: &'static str,
    /// 选中后按 Enter 触发的动作。
    pub action: CommandAction,
}

/// 命令面板可触发的所有动作。App::dispatch_command_action 实现具体副作用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAction {
    Quit,
    /// 切到指定面板（ProcessList / PortMap / UsbAssistant / MonitorPanel / DockerPanel）。
    SwitchPanel(AppMode),
    /// 进程列表视图模式（List / Tree / AppGroup）。
    SetProcessViewMode(ProcessViewMode),
    /// 进程列表排序字段。
    SortBy(SortField),
    /// 详情页 Tab 切换（仅在 ProcessDetail 模式生效）。
    SwitchInspectionTab(InspectionTab),
    /// 详情页 F5 强制刷新 Inspector 数据。
    RefreshInspector,
    /// 进入当前选中进程的详情页（同 Enter）。
    EnterDetail,
    /// 终止当前光标进程（同 `k`）。
    KillCursor,
    /// 强制终止（同 `K`，含子进程树）。
    ForceKillCursor,
    /// 全选可见进程（同 `a`）。
    SelectAllVisible,
    /// 循环切换主题（同 `t`）。
    CycleTheme,
    /// 直接设置第 N 个主题（0-9）。
    SetTheme(usize),
    /// 折叠/展开侧边栏（同 `c`）。
    ToggleSidebar,
    /// 打开帮助页（同 `?`）。
    ToggleHelp,
    /// 打开告警弹窗（同 `A`）。
    ToggleAlertPopup,
    /// 切换 VT100 录屏（同 `R`）。
    ToggleRecording,
    /// 关闭 worker 崩溃 banner（同 `D`）。
    DismissCrashes,
    /// Docker 开始监听容器事件流（同 DockerPanel `a`）。
    DockerStartEvents,
    /// Docker 停止选中容器（同 DockerPanel `s`）。
    DockerStopContainer,
    /// Docker 重启选中容器（同 DockerPanel `Shift+R`）。
    DockerRestartContainer,
}

/// 命令面板单次按键的处理结果。
#[derive(Debug)]
pub enum PaletteHandleResult {
    /// 继续留在 palette 模态（输入字符 / 上下选择）。
    Stay,
    /// 关闭 palette 不执行动作（Esc）。
    Close,
    /// 关闭 palette 并执行选中动作（Enter）。
    Execute(CommandAction),
}

/// 命令面板状态：输入框 + 全量命令清单 + 当前匹配项索引 + 选中位置。
///
/// 匹配规则：query 非空 → nucleo fuzzy 打分 + 按分数降序；query 空 → 全量原序。
pub struct CommandPalette {
    input: String,
    items: &'static [CommandItem],
    matched: Vec<usize>,
    selected: usize,
    // nucleo 状态：matcher 复用避免每次按键重建；pattern 按 query 重建（廉价）。
    matcher: Matcher,
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPalette {
    #[must_use]
    pub fn new() -> Self {
        let items = default_items();
        let matched = (0..items.len()).collect();
        Self {
            input: String::new(),
            items,
            matched,
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// 打开 palette 前清空状态：input / matched / selected 全部复位。
    pub fn reset(&mut self) {
        self.input.clear();
        self.matched = (0..self.items.len()).collect();
        self.selected = 0;
    }

    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    #[must_use]
    pub fn items(&self) -> &'static [CommandItem] {
        self.items
    }

    /// 当前匹配的命令项索引（按 fuzzy 分数降序）。query 为空时返回全量原序。
    #[must_use]
    pub fn matched_indices(&self) -> &[usize] {
        &self.matched
    }

    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 渲染用：当前选中的 item。query 空 + matched 空 → None（极少见，items 为空）。
    #[must_use]
    pub fn selected_item(&self) -> Option<&CommandItem> {
        self.matched
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    /// 处理按键。返回 [`PaletteHandleResult`] —— Stay 留在模态，Close 关闭不执行，
    /// Execute 关闭并执行。
    pub fn handle_key(&mut self, key: KeyEvent) -> PaletteHandleResult {
        match key.code {
            KeyCode::Esc => PaletteHandleResult::Close,
            KeyCode::Enter => {
                if let Some(item) = self.selected_item() {
                    PaletteHandleResult::Execute(item.action)
                } else {
                    PaletteHandleResult::Close
                }
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                PaletteHandleResult::Stay
            }
            KeyCode::Down => {
                if self.selected + 1 < self.matched.len() {
                    self.selected += 1;
                }
                PaletteHandleResult::Stay
            }
            KeyCode::Backspace => {
                // 简单 ASCII pop：query 都靠 char-by-char 推入，对应 char pop 即可。
                // 多字节字符走 chars().last() 拿到完整 scalar value。
                if let Some(ch) = self.input.pop() {
                    let _ = ch; // 仅保留 pop 的副作用
                    self.recompute_matches();
                    self.clamp_selected();
                }
                PaletteHandleResult::Stay
            }
            KeyCode::Char('u')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.input.clear();
                self.matched = (0..self.items.len()).collect();
                self.selected = 0;
                PaletteHandleResult::Stay
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.recompute_matches();
                self.clamp_selected();
                PaletteHandleResult::Stay
            }
            // 其它键（Tab / F1 / 方向键左右 / Home/End）当前 v1 不消费，留给 App 决定。
            _ => PaletteHandleResult::Stay,
        }
    }

    /// 重新跑 nucleo fuzzy 打分，按分数降序排列匹配项。query 空 → 全量原序。
    fn recompute_matches(&mut self) {
        if self.input.is_empty() {
            self.matched = (0..self.items.len()).collect();
            return;
        }
        let pattern = Pattern::parse(&self.input, CaseMatching::Smart, Normalization::Smart);
        // nucleo Utf32Str::Ascii 走 fast path：所有 label 都是 ASCII（const 静态字符串）。
        // 若将来 label 含 unicode，再用 Utf32Str::Utf32 + char buffer。
        let mut scored: Vec<(u32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let haystack = Utf32Str::Ascii(item.label.as_bytes());
                pattern.score(haystack, &mut self.matcher).map(|s| (s, i))
            })
            .collect();
        // 分数高的在前；同分保持原注册顺序（stable sort）。
        scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
        self.matched = scored.into_iter().map(|(_, i)| i).collect();
    }

    /// matched 长度缩减后保证 selected 仍在范围内。
    fn clamp_selected(&mut self) {
        if !self.matched.is_empty() && self.selected >= self.matched.len() {
            self.selected = self.matched.len() - 1;
        }
    }
}

/// v0.7 阶段 3：默认注册的命令清单（约 40 项）。
///
/// 覆盖：
/// - 6 个面板切换 + 3 个视图模式（List / Tree / AppGroup）
/// - 9 个排序字段
/// - 6 个详情页 Tab
/// - 10 个主题直跳 + 1 个循环切换
/// - 5 个全局 toggle（侧边栏 / 帮助 / 告警 / 录屏 / 退出）
/// - 3 个 Docker 操作（events / stop / restart）
/// - 4 个进程操作（详情 / kill / 强制 kill / 全选）
///
/// 新功能（v0.7+ 各阶段）落地时在此处追加条目即可。
#[must_use]
fn default_items() -> &'static [CommandItem] {
    // 静态切片：const 上下文不能 Vec! / Box::leak，必须用 const slice。
    // label 全部 ASCII 让 nucleo 走 Utf32Str::Ascii 的 fast path。
    const ITEMS: &[CommandItem] = &[
        // ── 面板切换 ───────────────────────────────────────────────
        CommandItem {
            id: "switch_to_process_list",
            label: "Switch to Process List",
            category: "Panel",
            hint: "1",
            action: CommandAction::SwitchPanel(AppMode::ProcessList),
        },
        CommandItem {
            id: "switch_to_process_tree",
            label: "Switch to Process Tree",
            category: "Panel",
            hint: "2",
            action: CommandAction::SetProcessViewMode(ProcessViewMode::Tree),
        },
        CommandItem {
            id: "switch_to_app_group",
            label: "Switch to App Group",
            category: "Panel",
            hint: "v",
            action: CommandAction::SetProcessViewMode(ProcessViewMode::AppGroup),
        },
        CommandItem {
            id: "switch_to_port_panel",
            label: "Switch to Port Panel",
            category: "Panel",
            hint: "3",
            action: CommandAction::SwitchPanel(AppMode::PortMap),
        },
        CommandItem {
            id: "switch_to_usb_panel",
            label: "Switch to USB Assistant",
            category: "Panel",
            hint: "4",
            action: CommandAction::SwitchPanel(AppMode::UsbAssistant),
        },
        CommandItem {
            id: "switch_to_monitor_panel",
            label: "Switch to Monitor Panel",
            category: "Panel",
            hint: "5",
            action: CommandAction::SwitchPanel(AppMode::MonitorPanel),
        },
        CommandItem {
            id: "switch_to_docker_panel",
            label: "Switch to Docker Panel",
            category: "Panel",
            hint: "6",
            action: CommandAction::SwitchPanel(AppMode::DockerPanel),
        },
        // v0.21：AI Agent 面板（ADR-0031）。入口仅命令面板——`A` 键已被
        // 「打开告警弹窗」占用（stage 1 实测确认，brainstorm 决策 8 注记更新）。
        CommandItem {
            id: "switch_to_agent_panel",
            label: "Switch to AI Agent Panel",
            category: "Panel",
            hint: "Ctrl+P",
            action: CommandAction::SwitchPanel(AppMode::Agent),
        },
        CommandItem {
            id: "switch_to_process_list_view",
            label: "Process List View (flat)",
            category: "Panel",
            hint: "v",
            action: CommandAction::SetProcessViewMode(ProcessViewMode::List),
        },
        // ── 排序字段 ───────────────────────────────────────────────
        CommandItem {
            id: "sort_by_cpu",
            label: "Sort by CPU",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::Cpu),
        },
        CommandItem {
            id: "sort_by_memory",
            label: "Sort by Memory",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::Memory),
        },
        CommandItem {
            id: "sort_by_pid",
            label: "Sort by PID",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::Pid),
        },
        CommandItem {
            id: "sort_by_name",
            label: "Sort by Name",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::Name),
        },
        CommandItem {
            id: "sort_by_security",
            label: "Sort by Security Score",
            category: "Sort",
            hint: "S",
            action: CommandAction::SortBy(SortField::Security),
        },
        CommandItem {
            id: "sort_by_disk_read",
            label: "Sort by Disk Read",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::DiskRead),
        },
        CommandItem {
            id: "sort_by_disk_write",
            label: "Sort by Disk Write",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::DiskWrite),
        },
        CommandItem {
            id: "sort_by_net_sent",
            label: "Sort by Net Sent",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::NetSent),
        },
        CommandItem {
            id: "sort_by_net_recv",
            label: "Sort by Net Recv",
            category: "Sort",
            hint: "←→",
            action: CommandAction::SortBy(SortField::NetRecv),
        },
        // ── 详情页 Tab ─────────────────────────────────────────────
        CommandItem {
            id: "inspector_tab_summary",
            label: "Inspector Tab: Summary",
            category: "Inspector",
            hint: "Tab",
            action: CommandAction::SwitchInspectionTab(InspectionTab::Summary),
        },
        CommandItem {
            id: "inspector_tab_env",
            label: "Inspector Tab: Env",
            category: "Inspector",
            hint: "Tab",
            action: CommandAction::SwitchInspectionTab(InspectionTab::Env),
        },
        CommandItem {
            id: "inspector_tab_network",
            label: "Inspector Tab: Network",
            category: "Inspector",
            hint: "Tab",
            action: CommandAction::SwitchInspectionTab(InspectionTab::Network),
        },
        CommandItem {
            id: "inspector_tab_dlls",
            label: "Inspector Tab: DLLs",
            category: "Inspector",
            hint: "Tab",
            action: CommandAction::SwitchInspectionTab(InspectionTab::Dlls),
        },
        CommandItem {
            id: "inspector_tab_handles",
            label: "Inspector Tab: Handles",
            category: "Inspector",
            hint: "Tab",
            action: CommandAction::SwitchInspectionTab(InspectionTab::Handles),
        },
        CommandItem {
            id: "inspector_tab_memory",
            label: "Inspector Tab: Memory",
            category: "Inspector",
            hint: "Tab",
            action: CommandAction::SwitchInspectionTab(InspectionTab::Memory),
        },
        CommandItem {
            id: "inspector_refresh",
            label: "Inspector: Force Refresh (F5)",
            category: "Inspector",
            hint: "F5",
            action: CommandAction::RefreshInspector,
        },
        // ── 进程操作 ───────────────────────────────────────────────
        CommandItem {
            id: "enter_detail",
            label: "Open Process Detail (Enter)",
            category: "Action",
            hint: "Enter",
            action: CommandAction::EnterDetail,
        },
        CommandItem {
            id: "kill_cursor",
            label: "Kill Cursor Process",
            category: "Action",
            hint: "k",
            action: CommandAction::KillCursor,
        },
        CommandItem {
            id: "force_kill_cursor",
            label: "Force Kill Cursor (with children)",
            category: "Action",
            hint: "K",
            action: CommandAction::ForceKillCursor,
        },
        CommandItem {
            id: "select_all_visible",
            label: "Select All Visible Processes",
            category: "Action",
            hint: "a",
            action: CommandAction::SelectAllVisible,
        },
        // ── 主题（10 项直跳 + 1 循环）──────────────────────────────
        CommandItem {
            id: "cycle_theme",
            label: "Cycle Theme",
            category: "Theme",
            hint: "t",
            action: CommandAction::CycleTheme,
        },
        CommandItem {
            id: "theme_0",
            label: "Theme: Dark",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(0),
        },
        CommandItem {
            id: "theme_1",
            label: "Theme: Catppuccin",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(1),
        },
        CommandItem {
            id: "theme_2",
            label: "Theme: Dracula",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(2),
        },
        CommandItem {
            id: "theme_3",
            label: "Theme: Gruvbox",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(3),
        },
        CommandItem {
            id: "theme_4",
            label: "Theme: One Dark",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(4),
        },
        CommandItem {
            id: "theme_5",
            label: "Theme: Nord",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(5),
        },
        CommandItem {
            id: "theme_6",
            label: "Theme: Solarized Dark",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(6),
        },
        CommandItem {
            id: "theme_7",
            label: "Theme: Tokyo Night",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(7),
        },
        CommandItem {
            id: "theme_8",
            label: "Theme: Gruvbox Light",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(8),
        },
        CommandItem {
            id: "theme_9",
            label: "Theme: Light",
            category: "Theme",
            hint: "t",
            action: CommandAction::SetTheme(9),
        },
        // ── 全局 toggle ────────────────────────────────────────────
        CommandItem {
            id: "toggle_sidebar",
            label: "Toggle Sidebar Expand",
            category: "Global",
            hint: "c",
            action: CommandAction::ToggleSidebar,
        },
        CommandItem {
            id: "toggle_help",
            label: "Toggle Help Page",
            category: "Global",
            hint: "?",
            action: CommandAction::ToggleHelp,
        },
        CommandItem {
            id: "toggle_alert_popup",
            label: "Toggle Alert Popup",
            category: "Global",
            hint: "A",
            action: CommandAction::ToggleAlertPopup,
        },
        CommandItem {
            id: "toggle_recording",
            label: "Toggle VT100 Recording",
            category: "Global",
            hint: "R",
            action: CommandAction::ToggleRecording,
        },
        CommandItem {
            id: "dismiss_crashes",
            label: "Dismiss Worker Crash Banner",
            category: "Global",
            hint: "D",
            action: CommandAction::DismissCrashes,
        },
        // ── Docker 操作 ────────────────────────────────────────────
        CommandItem {
            id: "docker_start_events",
            label: "Docker: Start Event Stream",
            category: "Docker",
            hint: "a",
            action: CommandAction::DockerStartEvents,
        },
        CommandItem {
            id: "docker_stop_container",
            label: "Docker: Stop Selected Container",
            category: "Docker",
            hint: "s",
            action: CommandAction::DockerStopContainer,
        },
        CommandItem {
            id: "docker_restart_container",
            label: "Docker: Restart Selected Container",
            category: "Docker",
            hint: "Shift+R",
            action: CommandAction::DockerRestartContainer,
        },
        // ── 退出 ───────────────────────────────────────────────────
        CommandItem {
            id: "quit",
            label: "Quit proc",
            category: "Global",
            hint: "q",
            action: CommandAction::Quit,
        },
    ];
    ITEMS
}

/// 渲染层用：可见匹配项的最大数量。超过此数的匹配仍可被选中（向下滚动），
/// 但视觉上前 N 个最相关。
#[must_use]
pub fn max_display() -> usize {
    MAX_DISPLAY
}

/// 渲染命令面板浮层。调用方保证仅在 `app.is_palette_open()` 时调用。
///
/// 布局：
/// ```text
/// ┌─────────────────────────────────────────────────┐
/// │ Ctrl+P 命令面板 — Esc 取消 / ↑↓ 选择 / Enter 执行│
/// ├─────────────────────────────────────────────────┤
/// │ ❯ <input>                                       │
/// ├─────────────────────────────────────────────────┤
/// │ ▶ [Panel]    Switch to Port Panel        (3)    │
/// │   [Panel]    Switch to Docker Panel      (6)    │
/// │   ...                                            │
/// ├─────────────────────────────────────────────────┤
/// │ hint: 切换到端口面板                            │
/// └─────────────────────────────────────────────────┘
/// ```
pub fn draw(f: &mut Frame, app: &crate::app::App) {
    let palette = &app.command_palette;
    // 浮层尺寸：70% 宽 / 14 行高，居中。
    let area = crate::tui::centered_rect(70, 14, f.area());
    // 先 Clear 再画 Block：清除底层的 panel 像素，避免透过来。
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Ctrl+P 命令面板 — Esc 取消 / ↑↓ 选择 / Enter 执行 ",
            theme::style_header(),
        ))
        .style(theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    // 三段：input(1) + 列表(剩余) + footer(1)
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);
    let input_area = chunks[0];
    let list_area = chunks[1];
    let footer_area = chunks[2];

    // ── 输入框 ──────────────────────────────────────────────
    let prompt = Span::styled("❯ ", theme::style_selected());
    let query = Span::styled(palette.input().to_string(), theme::style_normal());
    // 光标用 ▎ 符号简化（ratatui 0.29 不支持 inline cursor）。
    let cursor = Span::styled("▎", theme::style_danger());
    let input_line = Line::from(vec![prompt, query, cursor]);
    f.render_widget(Paragraph::new(input_line), input_area);

    // ── 匹配列表 ────────────────────────────────────────────
    let matched = palette.matched_indices();
    let items = palette.items();
    let selected = palette.selected();
    // 选中项保持可见：滚动窗口随 selected 移动。
    let visible_start = selected.saturating_sub(MAX_DISPLAY / 2);
    let visible_end = (visible_start + MAX_DISPLAY).min(matched.len());
    let mut lines: Vec<Line> = Vec::new();
    for vi in visible_start..visible_end {
        let item = &items[matched[vi]];
        let cursor_marker = if vi == selected { "▶ " } else { "  " };
        let cursor_span = Span::styled(
            cursor_marker,
            if vi == selected {
                theme::style_selected()
            } else {
                theme::style_muted()
            },
        );
        let cat_span = Span::styled(
            format!("[{}] ", item.category),
            if vi == selected {
                theme::style_info()
            } else {
                theme::style_muted()
            },
        );
        let label_span = Span::styled(
            item.label,
            if vi == selected {
                theme::style_selected()
            } else {
                theme::style_normal()
            },
        );
        // 右对齐 hint —— 简化：在 label 后加固定 padding + hint。
        let hint_span = Span::styled(
            format!("   ({})", item.hint),
            if vi == selected {
                theme::style_info()
            } else {
                theme::style_muted()
            },
        );
        lines.push(Line::from(vec![
            cursor_span,
            cat_span,
            label_span,
            hint_span,
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  无匹配项 — 按 Backspace 清空搜索",
            theme::style_muted(),
        )));
    }
    f.render_widget(Paragraph::new(lines), list_area);

    // ── footer：选中项 hint + 总匹配数 ─────────────────────
    let footer_text = if let Some(item) = palette.selected_item() {
        format!(" {} | 共 {} 条匹配", item.label, matched.len())
    } else {
        format!(" 共 {} 条匹配", matched.len())
    };
    f.render_widget(
        Paragraph::new(Span::styled(footer_text, theme::style_muted())),
        footer_area,
    );

    // 抑制未使用警告（Rect / area 在未来扩展布局调整时会用到）。
    let _ = (Constraint::Length(0), Rect::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn default_items_count_covers_main_actions() {
        // 至少覆盖 spec 要求的 ~50 命令的最低门坎：6 面板 + 9 排序 + 6 Tab + 10 主题
        // + 5 toggle + Docker/Action 杂项。
        let items = default_items();
        assert!(
            items.len() >= 40,
            "expected ≥40 palette items, got {}",
            items.len()
        );
    }

    #[test]
    fn default_items_have_unique_ids() {
        // 测试 anchor：id 必须唯一，否则将来自动化（录制 / LLM）会冲突。
        let items = default_items();
        let mut ids: Vec<&str> = items.iter().map(|i| i.id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate CommandItem.id found");
    }

    #[test]
    fn empty_query_matches_all_items() {
        let p = CommandPalette::new();
        assert_eq!(p.matched_indices().len(), p.items().len());
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn kill_query_matches_only_kill_actions() {
        let mut p = CommandPalette::new();
        // 输入 "kill" → 仅匹配 label 含 "kill"（大小写不敏感）的命令。
        for c in "kill".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        // KillCursor / ForceKillCursor 两条；其它 "Cycle Theme" / "Toggle Sidebar"
        // 等不含 "kill" 字面量的不匹配。
        let matched_labels: Vec<&str> = p
            .matched_indices()
            .iter()
            .map(|&i| p.items()[i].label)
            .collect();
        assert!(
            matched_labels
                .iter()
                .all(|l| l.to_lowercase().contains("kill"))
        );
        assert!(matched_labels.iter().any(|l| l.contains("Cursor")));
    }

    #[test]
    fn enter_returns_selected_action() {
        let mut p = CommandPalette::new();
        // 默认选中第 0 项（"Switch to Process List"），Enter 应返回对应 action。
        let first_action = p.items()[0].action;
        let result = p.handle_key(key(KeyCode::Enter));
        match result {
            PaletteHandleResult::Execute(a) => assert_eq!(a, first_action),
            other => panic!("expected Execute, got {other:?}"),
        }
    }

    #[test]
    fn esc_returns_close() {
        let mut p = CommandPalette::new();
        match p.handle_key(key(KeyCode::Esc)) {
            PaletteHandleResult::Close => {}
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn down_arrow_advances_selection_then_clamps() {
        let mut p = CommandPalette::new();
        let total = p.matched_indices().len();
        assert!(total > 1, "default items should have >1 entries");
        p.handle_key(key(KeyCode::Down));
        assert_eq!(p.selected(), 1);
        // 把光标推到底
        for _ in 0..total {
            p.handle_key(key(KeyCode::Down));
        }
        assert_eq!(p.selected(), total - 1);
    }

    #[test]
    fn up_arrow_does_not_underflow() {
        let mut p = CommandPalette::new();
        p.handle_key(key(KeyCode::Up));
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn backspace_pops_last_char() {
        let mut p = CommandPalette::new();
        for c in "abc".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(p.input(), "abc");
        p.handle_key(key(KeyCode::Backspace));
        assert_eq!(p.input(), "ab");
    }

    #[test]
    fn ctrl_u_clears_input() {
        let mut p = CommandPalette::new();
        for c in "xyz".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        p.handle_key(ctrl(KeyCode::Char('u')));
        assert!(p.input().is_empty());
        assert_eq!(p.matched_indices().len(), p.items().len());
    }

    #[test]
    fn reset_restores_empty_state() {
        let mut p = CommandPalette::new();
        for c in "kill".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        p.handle_key(key(KeyCode::Down));
        p.reset();
        assert!(p.input().is_empty());
        assert_eq!(p.matched_indices().len(), p.items().len());
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn query_typo_scores_lower_but_still_matches_some() {
        // nucleo fuzzy：缺字符 / 顺序错位也能匹配，但分数低。
        let mut p = CommandPalette::new();
        for c in "thm".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        // 应该至少匹配到 "Theme" 类的命令
        assert!(!p.matched_indices().is_empty());
        let matched_labels: Vec<&str> = p
            .matched_indices()
            .iter()
            .map(|&i| p.items()[i].label)
            .collect();
        assert!(
            matched_labels
                .iter()
                .any(|l| l.to_lowercase().contains("theme")),
            "expected Theme items in matches, got {matched_labels:?}"
        );
    }

    #[test]
    fn selected_index_clamps_after_backspace_shrinks_matches() {
        let mut p = CommandPalette::new();
        // 输入长 query 把匹配缩到 1-2 条，再 backspace 让匹配扩回去；
        // selected 不该越界。
        for c in "zzzz".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        // 无论是否匹配，selected 必须 ≤ matched.len()。
        assert!(p.selected() < p.matched_indices().len().max(1));
        p.handle_key(key(KeyCode::Backspace));
        p.handle_key(key(KeyCode::Backspace));
        p.handle_key(key(KeyCode::Backspace));
        p.handle_key(key(KeyCode::Backspace));
        assert!(p.selected() < p.matched_indices().len().max(1));
    }
}
