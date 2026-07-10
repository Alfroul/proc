//! v0.18 cycle 项 3 — rmcp 0.11 subscribe-push worker lifecycle 容器。
//!
//! ## 背景
//!
//! v0.17 stage 4 落地的 [`crate::mcp::resources::ResourceRoute`] 是 polling-push
//! 设计（client 走 `resources/read` 主动拉），与 ADR-0027 §1 描述「subscribe-push」
//! 语义有差距（REVIEW-v0.17 §2 Findings P2-B1）。v0.18 cycle 项 3 补全真正的
//! subscribe-push worker lifecycle——client 订阅后 server 主动 push 增量。
//!
//! ## stage 1 Spike 调研结论（context7 rmcp 0.11 docs 验证，2026-07-10）
//!
//! | 维度 | rmcp 0.11 原生 API | 结论 |
//! |---|---|---|
//! | server capability 声明 | `ServerCapabilitiesBuilder::enable_resources_subscribe()` | 复用，server 启动时调一次 |
//! | client 端发起订阅 | `ServerSink::subscribe(SubscribeRequestParams)` — client 角色方法 | 不暴露给 server handler |
//! | server 端 subscribe hook | **`ServerHandler` trait 不暴露 `subscribe_resource` 方法**——SDK 自动 ACK | 自建 worker lifecycle 必要 |
//! | server 主动 push | `Peer::notify_resource_updated(ResourceUpdatedNotificationParam)` | 复用，worker 1s tick 调此 API |
//! | client 接收 push | `ClientHandler::on_resource_updated` — client 角色 handler | 自动接收 |
//!
//! **关键结论**：rmcp 0.11 没有「server 持注册表 + 1s tick push」的原生 helper，
//! proc 需自建 worker lifecycle（brainstorm 决策 3 拍板）。
//!
//! ## stage 1 Spike 落地范围（本文件）
//!
//! - [`SubscriberId`] type alias（与 rmcp 0.11 内部 subscriber id 类型对齐）
//! - [`SubscribePushWorker`] struct 声明（含 `subscribers` 注册表 stub）
//! - `new` / `spawn_push_task` / `subscribe` / `unsubscribe` / `shutdown` 方法签名
//!   （方法体 stub 返占位值或 Err，stage 2 实装业务逻辑）
//!
//! ## stage 2 Slice 实装范围
//!
//! 1. **subscribe**：client 调 `resources/subscribe`（rmcp SDK 自动 ACK）→ proc 通过
//!    自定义 hook 拿到 Peer 句柄 → worker 持 Peer + 记 SubscriberId 到注册表
//! 2. **push**：worker 1s tick 遍历注册表 → 对每个 subscriber 调
//!    `peer.notify_resource_updated(ResourceUpdatedNotificationParam { uri })`
//! 3. **unsubscribe / 断开**：client 主动 `resources/unsubscribe` 或网络断开 →
//!    worker 检测 `peer.notify_resource_updated` 返 `Err(ServiceError)` → 从注册表移除
//!
//! 详见 ADR-0027 §关键设计点 5（v0.18 stage 1 Spike 扩段）+ brainstorm 决策 3
//! 拍板「自建 worker lifecycle」。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// subscribe-push 注册表 key 类型别名。
///
/// 与 rmcp 0.11 内部 subscriber id 类型对齐（`u64`）。stage 2 实装时通过
/// rmcp 0.11 `RequestContext::extensions` 或 `SubscribeRequestParams::subscriber_id`
/// 拿到（具体 API stage 2 调研），与 `Peer::notify_resource_updated` 配对——
/// peer 失败时按 SubscriberId 从注册表移除。
pub type SubscriberId = u64;

/// subscribe-push worker lifecycle 容器（v0.18 stage 1 Spike 骨架）。
///
/// **stage 1 Spike**：仅声明 struct + `subscribers` 注册表 stub，方法体返占位值
/// 或 Err。stage 2 实装真正的 lifecycle（注册表 add/remove + 1s tick push +
/// client 断开自动清理）。
///
/// **设计要点**：
///
/// - **`subscribers: Arc<Mutex<HashMap<SubscriberId, Peer<RoleServer)>>>`** 注册表
///   ——stage 1 Spike 暂用占位 value 类型 `()` 让结构编译过；stage 2 替换为
///   `Peer<RoleServer>`（rmcp 0.11 server-side client 句柄，通过 `extensions` 拿到）
/// - **三步 lifecycle**：subscribe → push → unsubscribe / 断开 cancel（与 brainstorm
///   决策 3 描述对齐）
/// - **push task spawn 暂不实装**：stage 1 Spike 仅声明 `spawn_push_task` 方法
///   签名，stage 2 实装 tokio task 1s tick 遍历注册表调 `peer.notify_resource_updated`
#[derive(Debug, Default)]
pub struct SubscribePushWorker {
    /// 注册表 stub：SubscriberId → 占位 value（stage 2 替换为 `Peer<RoleServer>`）。
    ///
    /// stage 1 Spike 用 `Arc<Mutex<HashMap<SubscriberId, ()>>>` 让结构编译过，
    /// stage 2 实装时改为 `Arc<Mutex<HashMap<SubscriberId, Peer<RoleServer>>>>`。
    subscribers: Arc<Mutex<HashMap<SubscriberId, ()>>>,

    /// worker shutdown signal sender（stage 2 实装时填充）。
    /// stage 1 Spike 暂用 `()` 占位，stage 2 替换为 `Option<oneshot::Sender<()>>`。
    _shutdown_tx: (),
}

impl SubscribePushWorker {
    /// 创建空 worker（stage 1 Spike stub）。
    ///
    /// stage 2 实装时此处 spawn push task（持 subscribers Arc clone + 1s tick
    /// 遍历调 `peer.notify_resource_updated`）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(HashMap::new())),
            _shutdown_tx: (),
        }
    }

    /// 注册 subscriber（stage 1 Spike stub）。
    ///
    /// stage 2 实装时此处：(1) 从 `RequestContext::extensions` 拿 Peer 句柄 →
    /// (2) 持 Peer + 记 SubscriberId 到 `subscribers` 注册表 → (3) 返 Ok。
    ///
    /// **当前 stage 1 Spike 仅返 Err "v0.18-stage-2 未实装"**——保留签名让 stage 2
    /// 直接填充业务逻辑。与 ADR-0027 §关键设计点 5 + brainstorm 决策 3 对齐。
    ///
    /// # Errors
    ///
    /// stage 1 Spike 永远返 Err。stage 2 实装后返 Ok 包含 SubscriberId 让 client
    /// 后续 unsubscribe。
    pub fn subscribe(&self, uri: &str, subscriber_id: SubscriberId) -> Result<(), String> {
        // stage 1 Spike stub：保留签名让 stage 2 直接填充业务逻辑
        let _ = (uri, subscriber_id, &self.subscribers);
        Err("v0.18-stage-2 未实装：subscribe-push worker lifecycle 留 stage 2 Slice".to_string())
    }

    /// 注销 subscriber（stage 1 Spike stub）。
    ///
    /// stage 2 实装时此处：从 `subscribers` 注册表 remove 该 SubscriberId →
    /// drop Peer 句柄让 push task 自动检测 channel closed 退出。
    ///
    /// **当前 stage 1 Spike 仅返 Err "v0.18-stage-2 未实装"**——保留签名让 stage 2
    /// 直接填充业务逻辑。
    ///
    /// # Errors
    ///
    /// stage 1 Spike 永远返 Err。
    pub fn unsubscribe(&self, subscriber_id: SubscriberId) -> Result<(), String> {
        // stage 1 Spike stub：保留签名让 stage 2 直接填充业务逻辑
        let _ = (subscriber_id, &self.subscribers);
        Err("v0.18-stage-2 未实装：subscribe-push worker lifecycle 留 stage 2 Slice".to_string())
    }

    /// spawn 1s tick push task（stage 1 Spike stub）。
    ///
    /// stage 2 实装时此处：spawn tokio task 持 `subscribers` Arc clone →
    /// 每 1s 遍历注册表 → 对每个 subscriber 调 `peer.notify_resource_updated` →
    /// push 失败（peer 断开）从注册表移除。task 与 `shutdown_rx` 配对让 worker
    /// shutdown 时干净退出。
    ///
    /// **当前 stage 1 Spike 仅返 Err "v0.18-stage-2 未实装"**——保留签名让 stage 2
    /// 直接填充业务逻辑。
    ///
    /// # Errors
    ///
    /// stage 1 Spike 永远返 Err。
    pub fn spawn_push_task(&self) -> Result<(), String> {
        // stage 1 Spike stub：保留签名让 stage 2 直接填充业务逻辑
        let _ = &self.subscribers;
        Err("v0.18-stage-2 未实装：subscribe-push worker lifecycle 留 stage 2 Slice".to_string())
    }

    /// 触发 worker shutdown（stage 1 Spike stub）。
    ///
    /// stage 2 实装时此处：drop `_shutdown_tx` 让 push task 自动退出（cancel
    /// via oneshot channel）+ 等 task join（避免 zombie task）。
    ///
    /// **当前 stage 1 Spike 是 no-op**——stage 1 没有 spawn task，shutdown 无需操作。
    pub fn shutdown(&mut self) {
        // stage 1 Spike no-op：stage 2 实装时此处 drop shutdown_tx + 等 task join
    }

    /// 返回注册表当前 subscriber 数量（stage 1 Spike 测试 helper）。
    ///
    /// stage 1 Spike 注册表永远空（subscribe stub 返 Err），此方法返 0。
    /// stage 2 实装后返真实数量。
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().map_or(0, |m| m.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_worker_has_empty_subscribers() {
        let worker = SubscribePushWorker::new();
        assert_eq!(worker.subscriber_count(), 0);
    }

    #[test]
    fn subscribe_stub_returns_err() {
        // stage 1 Spike：subscribe 永远返 Err
        let worker = SubscribePushWorker::new();
        let result = worker.subscribe("proc://metrics/system", 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("v0.18-stage-2"));
    }

    #[test]
    fn unsubscribe_stub_returns_err() {
        // stage 1 Spike：unsubscribe 永远返 Err
        let worker = SubscribePushWorker::new();
        let result = worker.unsubscribe(1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("v0.18-stage-2"));
    }

    #[test]
    fn spawn_push_task_stub_returns_err() {
        // stage 1 Spike：spawn_push_task 永远返 Err
        let worker = SubscribePushWorker::new();
        let result = worker.spawn_push_task();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("v0.18-stage-2"));
    }

    #[test]
    fn shutdown_is_noop_in_stage_1() {
        // stage 1 Spike：shutdown 是 no-op，不应 panic
        let mut worker = SubscribePushWorker::new();
        worker.shutdown();
        assert_eq!(worker.subscriber_count(), 0);
    }

    #[test]
    fn default_impl_matches_new() {
        // Default impl 与 new() 行为一致（都返空注册表）
        let from_new = SubscribePushWorker::new();
        let from_default = SubscribePushWorker::default();
        assert_eq!(from_new.subscriber_count(), from_default.subscriber_count());
    }
}
