//! v0.6.0 阶段 5：录屏回放状态机封装。
//!
//! 见 CONTEXT.md「ReplayController」。从 `App` 上帝对象拆出，避免
//! `replay_tick` / `replay_load_current_frame` 深度耦合 App 15+ 字段的逻辑
//! 散落在 `App` 上帝对象中。

pub mod controller;
pub mod search;

pub use controller::{ReplayAction, ReplayController, ReplayDirection, ReplaySpeed, TimelineState};
pub use search::ReplaySearch;
