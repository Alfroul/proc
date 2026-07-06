//! v0.6.0 阶段 5：录屏回放状态机封装，从 `App` 上帝对象拆出。
//!
//! 持 2 个字段（`replay_player` + `timeline_state`），暴露 `start` /
//! `current_frame` / `frame_mode` / `handle_key` / `tick`。
//!
//! `handle_key` / `tick` 返回 [`ReplayAction`] 枚举：副作用（写 `should_quit` /
//! 把当前帧应用到 panels）通过 action 让 [`crate::app::App`] 派发，避免
//! controller 反向依赖 App 的 15+ panel 字段（参考 `InspectorController`）。
//!
//! v0.14 stage 2：加 `bookmarks: Option<BookmarkFile>` + `bookmark_panel:
//! Option<BookmarkPanelState>`，`B` 键打开书签面板，Up/Down/Enter/`e`/`d`/Esc 控制。

use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::AppMode;
use crate::record::{BookmarkFile, BookmarkPanelState, Player};
use crate::replay::search::ReplaySearch;

/// 录屏回放速度档位。`Half` = 0.5x（每两个 tick 推进 1 帧），
/// `Normal` = 1x，`Double` = 2x，`Quad` = 4x。
#[derive(Debug, Clone, Copy)]
pub enum ReplaySpeed {
    Half,
    Normal,
    Double,
    Quad,
}

impl ReplaySpeed {
    #[must_use]
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Half => 0.5,
            Self::Normal => 1.0,
            Self::Double => 2.0,
            Self::Quad => 4.0,
        }
    }
}

/// v0.14 stage 4：录屏回放方向。`Forward` = 帧索引递增（默认），
/// `Reverse` = 帧索引递减（倒放）。
///
/// 设计取舍：作为独立枚举（而非扩 ReplaySpeed 8 档）—— speed 与 direction
/// 正交（任意 speed × 任意 direction 组合），独立字段更清晰，UI 渲染 / 配置
/// 序列化也直观（未来 `proc replay --speed 2x --reverse` CLI flag 时不动
/// ReplaySpeed 序列化契约）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayDirection {
    /// 默认方向：帧索引递增。
    #[default]
    Forward,
    /// 倒放：帧索引递减。
    Reverse,
}

impl ReplayDirection {
    #[must_use]
    pub fn is_reverse(self) -> bool {
        matches!(self, Self::Reverse)
    }

    /// v0.14 stage 4：UI icon（与 timeline 渲染共用同一字符表）。
    /// 暂停态由调用方决策（playing == false 时显示 ⏸，与本方法无关）。
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Self::Forward => "\u{25B6}", // ▶
            Self::Reverse => "\u{25C0}", // ◀
        }
    }

    /// 切方向（Forward ↔ Reverse）。
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::Forward => Self::Reverse,
            Self::Reverse => Self::Forward,
        }
    }
}

/// 时间线游标 + 播放状态。`half_tick` 在 `Half` 速度下隔帧推进用。
#[derive(Debug, Clone)]
pub struct TimelineState {
    pub current_frame: usize,
    pub total_frames: usize,
    pub speed: ReplaySpeed,
    /// v0.14 stage 4：回放方向。默认 Forward（与既有行为兼容）。
    pub direction: ReplayDirection,
    pub playing: bool,
    pub half_tick: u32,
}

/// 录屏回放状态机封装（v0.6.0 阶段 5 从 `App` 拆出）。
///
/// 字段名沿用 App 原名（`replay_player` / `timeline_state`），让搬迁只多一层
/// `.replay.` 前缀，TUI / main.rs 改造机械化（与 `InspectorController` 同款原则）。
pub struct ReplayController {
    pub replay_player: Option<Player>,
    pub timeline_state: Option<TimelineState>,
    /// v0.14 stage 2：书签 sidecar（start 时一次性 load；add/edit/delete 后写盘）。
    pub bookmarks: Option<BookmarkFile>,
    /// v0.14 stage 2：`B` 键打开的书签面板状态。None=未打开 / Some=打开中。
    pub bookmark_panel: Option<BookmarkPanelState>,
    /// v0.14 stage 3：时间轴搜索状态。始终存在（默认空 input / 无 expr），
    /// `is_active()` 决定是否生效。
    pub search: ReplaySearch,
    /// v0.14 stage 3：搜索输入态（`/` 进入 / Esc 或 Enter 退出）。
    /// 与 `search.is_active()` 不同：input 可在退出输入态后仍非空（n/N 跳转用）。
    pub search_input_active: bool,
}

impl Default for ReplayController {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            replay_player: None,
            timeline_state: None,
            bookmarks: None,
            bookmark_panel: None,
            search: ReplaySearch::new(),
            search_input_active: false,
        }
    }

    /// 进入回放模式：保存 player + 初始化 timeline（停在首帧，未播放）。
    /// 调用方负责把首帧应用到 panels（[`crate::app::App::apply_replay_frame`]）。
    pub fn start(&mut self, player: Player) {
        let total = player.total_frames();
        // v0.14 stage 2：load 书签 sidecar（不存在 / 损坏静默降级到空列表）
        let bookmarks = BookmarkFile::load_or_empty(player.path());
        self.replay_player = Some(player);
        self.timeline_state = Some(TimelineState {
            current_frame: 0,
            total_frames: total,
            speed: ReplaySpeed::Normal,
            direction: ReplayDirection::Forward,
            playing: false,
            half_tick: 0,
        });
        self.bookmarks = Some(bookmarks);
        self.bookmark_panel = None;
        // v0.14 stage 3：reset 搜索（cycle 内每次 start 重新开始）
        self.search.reset();
        self.search_input_active = false;
    }

    /// 取当前帧的克隆。返回 `None` 表示未启动 / 帧索引越界。调用方拿到后
    /// 释放借用，再 mutate panels / metrics（典型用法见
    /// [`crate::app::App::apply_replay_frame`]）。
    #[must_use]
    pub fn current_frame(&self) -> Option<crate::record::UiFrame> {
        let idx = self.timeline_state.as_ref()?.current_frame;
        self.replay_player.as_ref()?.frame_at(idx)
    }

    /// 当前帧对应的 `AppMode`（用于 TUI 渲染录制时活跃的面板）。
    /// 无录制数据时返回默认 `ProcessList`。
    #[must_use]
    pub fn frame_mode(&self) -> AppMode {
        let frame_index = self
            .timeline_state
            .as_ref()
            .map(|ts| ts.current_frame)
            .unwrap_or(0);
        if let Some(ref player) = self.replay_player
            && let Some(frame) = player.frame_at(frame_index)
        {
            return match frame.mode.as_str() {
                "ProcessTree" | "ProcessList" => AppMode::ProcessList,
                "PortMap" => AppMode::PortMap,
                "UsbAssistant" => AppMode::UsbAssistant,
                "MonitorPanel" => AppMode::MonitorPanel,
                "DockerPanel" => AppMode::DockerPanel,
                _ => AppMode::ProcessList,
            };
        }
        AppMode::ProcessList
    }

    /// v0.14 stage 2：当前书签 sidecar（start 时 load）。
    #[must_use]
    pub fn bookmarks(&self) -> Option<&BookmarkFile> {
        self.bookmarks.as_ref()
    }

    /// v0.14 stage 2：当前书签面板状态（B 键打开）。
    #[must_use]
    pub fn bookmark_panel(&self) -> Option<&BookmarkPanelState> {
        self.bookmark_panel.as_ref()
    }

    /// v0.14 stage 2：过滤后的书签索引列表（按 search_query 子串过滤 label / frame_idx 字符串）。
    /// 让 UI 渲染 / cursor 移动用同一份数据。同时返回列表让 UI 和 controller 跑同一份逻辑。
    #[must_use]
    pub fn filtered_bookmark_indices(&self) -> Vec<usize> {
        let Some(file) = self.bookmarks.as_ref() else {
            return Vec::new();
        };
        let Some(panel) = self.bookmark_panel.as_ref() else {
            return Vec::new();
        };
        filter_indices(file, panel)
    }

    /// 回放模式键盘路由。原 `App::handle_replay_key` 整体迁过来。
    ///
    /// 优先级（v0.14 stage 2 + stage 3 + stage 4）：
    /// 1. 书签面板激活 → 走 [`Self::handle_bookmark_panel_key`]
    /// 2. 搜索输入态激活 → 走搜索输入键位
    /// 3. 普通模式 → q / space / Left / Right / + / - / Home / End / B / `/` / `n` / `N` / `r`
    ///
    /// 返回 [`ReplayAction`]；副作用（`q` 退出 / 应用帧到 panels）由 App 派发。
    pub fn handle_key(&mut self, key: KeyEvent) -> ReplayAction {
        // v0.14 stage 2：书签面板激活时优先处理面板键位
        if self.bookmark_panel.is_some() {
            return self.handle_bookmark_panel_key(key);
        }
        // v0.14 stage 3：搜索输入态激活时优先处理输入键位
        if self.search_input_active {
            return self.handle_search_input_key(key);
        }
        let Some(ts) = self.timeline_state.as_mut() else {
            return ReplayAction::Noop;
        };
        match key.code {
            KeyCode::Char('q') => return ReplayAction::Quit,
            KeyCode::Char('B') => {
                // v0.14 stage 2：Shift+B 打开书签面板
                self.bookmark_panel = Some(BookmarkPanelState::new());
                return ReplayAction::BookmarkPanelToggled;
            }
            KeyCode::Char('r') => {
                // v0.14 stage 4：切播放方向（小写 r）。不与录制键 R 冲突——
                // 录制键 R 是 Shift+R 在 App 主路径触发 toggle_recording，
                // ReplayController 仅在回放路径激活，路由不到。
                ts.direction = ts.direction.toggle();
                return ReplayAction::DirectionToggled;
            }
            KeyCode::Char('/') => {
                // v0.14 stage 3：进入搜索输入态
                self.search_input_active = true;
                // 不清空 input — 让用户在既有搜索基础上修改（按 Esc 后 n/N 仍可用）
                return ReplayAction::SearchInputToggled;
            }
            KeyCode::Char('n') => {
                // v0.14 stage 3：跳转到下一命中帧
                if let Some(idx) = self.search.next_match() {
                    if let Some(ts) = self.timeline_state.as_mut() {
                        ts.current_frame = idx.min(ts.total_frames.saturating_sub(1));
                        ts.playing = false;
                        return ReplayAction::ApplyFrame;
                    }
                }
                return ReplayAction::Noop;
            }
            KeyCode::Char('N') => {
                // v0.14 stage 3：跳转到上一命中帧
                if let Some(idx) = self.search.prev_match() {
                    if let Some(ts) = self.timeline_state.as_mut() {
                        ts.current_frame = idx.min(ts.total_frames.saturating_sub(1));
                        ts.playing = false;
                        return ReplayAction::ApplyFrame;
                    }
                }
                return ReplayAction::Noop;
            }
            KeyCode::Char(' ') => {
                ts.playing = !ts.playing;
            }
            KeyCode::Left => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    ts.current_frame = ts.current_frame.saturating_sub(10);
                } else {
                    ts.current_frame = ts.current_frame.saturating_sub(1);
                }
                ts.playing = false;
                return ReplayAction::ApplyFrame;
            }
            KeyCode::Right => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    ts.current_frame =
                        (ts.current_frame + 10).min(ts.total_frames.saturating_sub(1));
                } else {
                    ts.current_frame =
                        (ts.current_frame + 1).min(ts.total_frames.saturating_sub(1));
                }
                ts.playing = false;
                return ReplayAction::ApplyFrame;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                ts.speed = match ts.speed {
                    ReplaySpeed::Half => ReplaySpeed::Normal,
                    ReplaySpeed::Normal => ReplaySpeed::Double,
                    ReplaySpeed::Double => ReplaySpeed::Quad,
                    ReplaySpeed::Quad => ReplaySpeed::Quad,
                };
            }
            KeyCode::Char('-') => {
                ts.speed = match ts.speed {
                    ReplaySpeed::Half => ReplaySpeed::Half,
                    ReplaySpeed::Normal => ReplaySpeed::Half,
                    ReplaySpeed::Double => ReplaySpeed::Normal,
                    ReplaySpeed::Quad => ReplaySpeed::Double,
                };
            }
            KeyCode::Home => {
                ts.current_frame = 0;
                ts.playing = false;
                return ReplayAction::ApplyFrame;
            }
            KeyCode::End => {
                ts.current_frame = ts.total_frames.saturating_sub(1);
                ts.playing = false;
                return ReplayAction::ApplyFrame;
            }
            _ => {}
        }
        ReplayAction::Noop
    }

    /// v0.14 stage 3：搜索输入态键位路由。
    /// - Esc / Enter → 退出输入态（保留 input + matches；n/N 仍可用）
    /// - Backspace → pop 末字符 + reparse + clear matches
    /// - 字符 → push + reparse + clear matches
    /// - 其他键 → 吞掉（避免意外触发 q / space 等）
    fn handle_search_input_key(&mut self, key: KeyEvent) -> ReplayAction {
        match key.code {
            KeyCode::Esc => {
                self.search_input_active = false;
                ReplayAction::SearchInputToggled
            }
            KeyCode::Enter => {
                // 退出输入态；触发一次 recompute（n/N 跳转用）
                self.search_input_active = false;
                self.recompute_search_matches();
                ReplayAction::SearchMatchesUpdated
            }
            KeyCode::Backspace => {
                self.search.pop_char();
                self.recompute_search_matches();
                ReplayAction::SearchMatchesUpdated
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.search.push_char(c);
                self.recompute_search_matches();
                ReplayAction::SearchMatchesUpdated
            }
            _ => ReplayAction::Noop,
        }
    }

    /// v0.14 stage 3：用 player 的 frame_at 重新计算命中帧列表。
    /// 借用拆分：先 take `&mut self.search`，再读 `&self.replay_player`，
    /// 最后把更新后的 search 放回（实际上 search 是字段，直接调即可——
    /// frame_at 通过 closure 注入）。
    fn recompute_search_matches(&mut self) {
        // 借用拆分：把 search 字段先取出（take + replace），让 self 只持 player 借用
        let mut search = std::mem::take(&mut self.search);
        let total = self
            .timeline_state
            .as_ref()
            .map(|ts| ts.total_frames)
            .unwrap_or(0);
        if let Some(player) = self.replay_player.as_ref() {
            search.recompute_matches(total, |idx| player.frame_at(idx));
        } else {
            search.matches.clear();
        }
        self.search = search;
    }

    /// v0.14 stage 2：书签面板激活时的键位路由。
    /// 把 panel 从 self 取出操作，最后再放回 — 避免 `&mut self.bookmark_panel` 与
    /// `&mut self.bookmarks` / `&self.replay_player` 借用冲突。
    fn handle_bookmark_panel_key(&mut self, key: KeyEvent) -> ReplayAction {
        let mut panel = match self.bookmark_panel.take() {
            Some(p) => p,
            None => return ReplayAction::Noop,
        };

        // 编辑模式优先（除 Enter / Esc 外字符都 push 到 editing_label）
        if panel.is_editing() {
            match key.code {
                KeyCode::Esc => {
                    panel.editing_label = None;
                    panel.editing_id = None;
                }
                KeyCode::Enter => {
                    if let (Some(file), Some(player)) =
                        (self.bookmarks.as_mut(), self.replay_player.as_ref())
                    {
                        if let Some((id, new_label)) = panel.end_edit() {
                            let trimmed = new_label.trim().to_string();
                            // 空 input 时保留原 label（lookup by id）；非空则替换。
                            let final_label = if trimmed.is_empty() {
                                file.bookmarks
                                    .iter()
                                    .find(|b| b.id == id)
                                    .map(|b| b.label.clone())
                                    .unwrap_or_else(|| format!("书签 #{id}"))
                            } else {
                                trimmed
                            };
                            file.edit_label(id, final_label);
                            file.write(player.path());
                        }
                    } else {
                        panel.end_edit();
                    }
                }
                KeyCode::Backspace => {
                    if let Some(s) = panel.editing_label.as_mut() {
                        s.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if !c.is_control()
                        && let Some(s) = panel.editing_label.as_mut()
                    {
                        s.push(c);
                    }
                }
                _ => {}
            }
            self.bookmark_panel = Some(panel);
            return ReplayAction::Noop;
        }

        // 非编辑模式：先算 indices（panel 还在本地变量里，不冲突）
        let indices = match self.bookmarks.as_ref() {
            Some(file) => filter_indices(file, &panel),
            None => Vec::new(),
        };
        let len = indices.len();
        let action = match key.code {
            KeyCode::Esc => {
                // 关闭面板 — 不放回（self.bookmark_panel 已 take 走，保持 None）
                ReplayAction::BookmarkPanelToggled
            }
            KeyCode::Up => {
                if len > 0 {
                    panel.cursor = panel.cursor.saturating_sub(1);
                }
                ReplayAction::Noop
            }
            KeyCode::Down => {
                if len > 0 {
                    panel.cursor = (panel.cursor + 1).min(len.saturating_sub(1));
                }
                ReplayAction::Noop
            }
            KeyCode::Enter => {
                // 跳转到 cursor 处书签的 frame_idx
                if len == 0 {
                    ReplayAction::Noop
                } else {
                    let cursor = panel.cursor.min(len - 1);
                    let bookmark_idx = indices[cursor];
                    let frame_idx = self
                        .bookmarks
                        .as_ref()
                        .and_then(|f| f.bookmarks.get(bookmark_idx))
                        .map(|b| b.frame_idx);
                    if let Some(idx) = frame_idx
                        && let Some(ts) = self.timeline_state.as_mut()
                    {
                        ts.current_frame = idx.min(ts.total_frames.saturating_sub(1));
                        ts.playing = false;
                        ReplayAction::ApplyFrame
                    } else {
                        ReplayAction::Noop
                    }
                }
            }
            KeyCode::Char('d') => {
                if len == 0 {
                    ReplayAction::Noop
                } else {
                    let cursor = panel.cursor.min(len - 1);
                    let bookmark_idx = indices[cursor];
                    if let (Some(file), Some(player)) =
                        (self.bookmarks.as_mut(), self.replay_player.as_ref())
                    {
                        if let Some(bm) = file.bookmarks.get(bookmark_idx).cloned() {
                            file.remove(bm.id);
                            file.write(player.path());
                            let new_len = file.bookmarks.len();
                            if new_len == 0 {
                                panel.cursor = 0;
                            } else if panel.cursor >= new_len {
                                panel.cursor = new_len - 1;
                            }
                        }
                    }
                    ReplayAction::BookmarkPanelToggled
                }
            }
            KeyCode::Char('e') => {
                if len == 0 {
                    ReplayAction::Noop
                } else {
                    let cursor = panel.cursor.min(len - 1);
                    let bookmark_idx = indices[cursor];
                    if let Some(file) = self.bookmarks.as_ref()
                        && let Some(bm) = file.bookmarks.get(bookmark_idx).cloned()
                    {
                        // 编辑模式：清空 input buffer（fresh replacement）。用户按 Enter
                        // 后若 input 为空则保留原 label（在 Enter 分支里 fallback）。
                        panel.start_edit(bm.id, "");
                    }
                    ReplayAction::Noop
                }
            }
            KeyCode::Backspace => {
                panel.search_query.pop();
                let new_len = match self.bookmarks.as_ref() {
                    Some(file) => filter_indices(file, &panel).len(),
                    None => 0,
                };
                if new_len == 0 {
                    panel.cursor = 0;
                } else if panel.cursor >= new_len {
                    panel.cursor = new_len - 1;
                }
                ReplayAction::Noop
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    panel.search_query.push(c);
                    let new_len = match self.bookmarks.as_ref() {
                        Some(file) => filter_indices(file, &panel).len(),
                        None => 0,
                    };
                    if new_len == 0 {
                        panel.cursor = 0;
                    } else if panel.cursor >= new_len {
                        panel.cursor = new_len - 1;
                    }
                }
                ReplayAction::Noop
            }
            _ => ReplayAction::Noop,
        };

        // 把 panel 放回（除非 Esc 关闭）
        if !matches!(key.code, KeyCode::Esc) {
            self.bookmark_panel = Some(panel);
        }
        action
    }

    /// 每 `REFRESH_INTERVAL` 调一次：按 `speed` 步进 `current_frame`。
    /// 推进了 → [`ReplayAction::ApplyFrame`]；未推进（paused / half-tick 间歇
    /// / 已到末尾）→ [`ReplayAction::Noop`]。到末尾自动暂停。
    pub fn tick(&mut self) -> ReplayAction {
        // v0.14 stage 2：书签面板打开时暂停自动步进（用户在选书签，不要 push 帧）
        if self.bookmark_panel.is_some() {
            return ReplayAction::Noop;
        }
        let Some(ts) = self.timeline_state.as_mut() else {
            return ReplayAction::Noop;
        };
        if !ts.playing || ts.total_frames == 0 {
            return ReplayAction::Noop;
        }
        // Compute frame step for this tick based on playback speed.
        // Half speed steps every other tick; the rest advance by N frames per tick.
        let step = match ts.speed {
            ReplaySpeed::Half => {
                ts.half_tick = (ts.half_tick + 1) % 2;
                usize::from(ts.half_tick == 0)
            }
            ReplaySpeed::Normal => 1,
            ReplaySpeed::Double => 2,
            ReplaySpeed::Quad => 4,
        };
        if step == 0 {
            return ReplayAction::Noop;
        }
        // v0.14 stage 4：方向分支。正向 saturating_add + clamp 到末帧；
        // 倒放 saturating_sub + clamp 到首帧。到边界自动暂停（对称）。
        match ts.direction {
            ReplayDirection::Forward => {
                let last = ts.total_frames.saturating_sub(1);
                ts.current_frame = (ts.current_frame + step).min(last);
                if ts.current_frame >= last {
                    ts.playing = false;
                }
            }
            ReplayDirection::Reverse => {
                ts.current_frame = ts.current_frame.saturating_sub(step);
                if ts.current_frame == 0 {
                    ts.playing = false;
                }
            }
        }
        ReplayAction::ApplyFrame
    }
}

/// `ReplayController::handle_key` / `tick` 的返回值。副作用通过此枚举让
/// App 派发，controller 不持 App 引用（参考 `InspectorAction`）。
#[derive(Debug)]
pub enum ReplayAction {
    /// 无副作用（速度调整 / space 暂停切换 / 未启动 / paused / half-tick 间歇）。
    Noop,
    /// `q` 键：App 收到后设 `should_quit = true`。
    Quit,
    /// timeline 推进了，App 需要把当前帧重新应用到 panels（见
    /// [`crate::app::App::apply_replay_frame`]）。触发点：
    /// Left / Right / Home / End / tick 自动步进 / 书签 Enter 跳转 / 搜索 n/N 跳转。
    ApplyFrame,
    /// v0.14 stage 2：书签面板打开 / 关闭（App 设 status_message 提示用户）。
    BookmarkPanelToggled,
    /// v0.14 stage 3：搜索输入态打开 / 关闭（App 设 status_message 提示用户）。
    SearchInputToggled,
    /// v0.14 stage 3：命中帧列表更新（用户输入变化或 recompute 触发）。
    /// App 设 status_message 提示命中数。
    SearchMatchesUpdated,
    /// v0.14 stage 4：方向切换（r 键）。App 设 status_message 提示当前方向。
    DirectionToggled,
}

/// v0.14 stage 2：按 search_query 过滤书签索引（label / frame_idx / id 三个字段子串匹配）。
/// 顶层函数让 controller 与 UI 共用同一份过滤逻辑。
#[must_use]
fn filter_indices(file: &BookmarkFile, panel: &BookmarkPanelState) -> Vec<usize> {
    let q = panel.search_query.trim().to_lowercase();
    if q.is_empty() {
        return (0..file.bookmarks.len()).collect();
    }
    file.bookmarks
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            b.label.to_lowercase().contains(&q)
                || b.frame_idx.to_string().contains(&q)
                || b.id.to_string().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}
