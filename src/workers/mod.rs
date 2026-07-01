//! v0.6.0 阶段 5：所有后台 worker 句柄的统一持有者。
//!
//! 见 CONTEXT.md「WorkerManager」。从 `App` 上帝对象拆出，避免新功能
//! 不断往 `App` 塞 worker 字段。

pub mod manager;
/// v0.11.0 阶段 1：Worker Restart Policy 纯状态机（ADR-0019）。
pub mod restart;

pub use manager::WorkerManager;
pub use restart::{MAX_RETRIES, RESET_WINDOW, RestartState, RestartStatus, backoff_for};
