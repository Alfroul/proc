//! v0.6.0 阶段 5：录屏回放状态机封装，从 `App` 上帝对象拆出。
//!
//! 持 2 个字段（`replay_player` + `timeline_state`），暴露 `start` /
//! `current_frame` / `frame_mode` / `handle_key` / `tick`。
//!
//! `handle_key` / `tick` 返回 [`ReplayAction`] 枚举：副作用（写 `should_quit` /
//! 把当前帧应用到 panels）通过 action 让 [`crate::app::App`] 派发，避免
//! controller 反向依赖 App 的 15+ panel 字段（参考 `InspectorController`）。

use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::AppMode;
use crate::record::Player;

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

/// 时间线游标 + 播放状态。`half_tick` 在 `Half` 速度下隔帧推进用。
#[derive(Debug, Clone)]
pub struct TimelineState {
    pub current_frame: usize,
    pub total_frames: usize,
    pub speed: ReplaySpeed,
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
        }
    }

    /// 进入回放模式：保存 player + 初始化 timeline（停在首帧，未播放）。
    /// 调用方负责把首帧应用到 panels（[`crate::app::App::apply_replay_frame`]）。
    pub fn start(&mut self, player: Player) {
        let total = player.total_frames();
        self.replay_player = Some(player);
        self.timeline_state = Some(TimelineState {
            current_frame: 0,
            total_frames: total,
            speed: ReplaySpeed::Normal,
            playing: false,
            half_tick: 0,
        });
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

    /// 回放模式键盘路由。原 `App::handle_replay_key` 整体迁过来。
    ///
    /// 返回 [`ReplayAction`]；副作用（`q` 退出 / 应用帧到 panels）由 App 派发。
    pub fn handle_key(&mut self, key: KeyEvent) -> ReplayAction {
        let Some(ts) = self.timeline_state.as_mut() else {
            return ReplayAction::Noop;
        };
        match key.code {
            KeyCode::Char('q') => return ReplayAction::Quit,
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

    /// 每 `REFRESH_INTERVAL` 调一次：按 `speed` 步进 `current_frame`。
    /// 推进了 → [`ReplayAction::ApplyFrame`]；未推进（paused / half-tick 间歇
    /// / 已到末尾）→ [`ReplayAction::Noop`]。到末尾自动暂停。
    pub fn tick(&mut self) -> ReplayAction {
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
        let last = ts.total_frames.saturating_sub(1);
        ts.current_frame = (ts.current_frame + step).min(last);
        if ts.current_frame >= last {
            ts.playing = false;
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
    /// Left / Right / Home / End / tick 自动步进。
    ApplyFrame,
}
