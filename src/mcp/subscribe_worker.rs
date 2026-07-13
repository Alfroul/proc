//! v0.18 cycle 项 3 — rmcp 0.11 subscribe-push worker lifecycle 容器。
//!
//! ## 背景
//!
//! v0.17 stage 4 落地的 [`crate::mcp::resources::ResourceRoute`] 是 polling-push
//! 设计（client 走 `resources/read` 主动拉），与 ADR-0027 §1 描述「subscribe-push」
//! 语义有差距（REVIEW-v0.17 §2 Findings P2-B1）。v0.18 cycle 项 3 补全真正的
//! subscribe-push worker lifecycle——client 订阅后 server 主动 push 增量。
//!
//! ## stage 1 Spike 调研结论（context7 rmcp 0.11 docs + 源码验证，2026-07-10）
//!
//! | 维度 | rmcp 0.11 原生 API | 结论 |
//! |---|---|---|
//! | server 端 capability 声明 | `ServerCapabilitiesBuilder::enable_resources_subscribe()` | 复用，server 启动时调一次 |
//! | client 端发起订阅 | `ServerSink::subscribe(SubscribeRequestParams)` — client 角色方法 | 不暴露给 server handler |
//! | server 端 subscribe hook | **`ServerHandler` trait 不暴露 `subscribe_resource` 方法**——SDK 自动 ACK | 自建 worker lifecycle 必要 |
//! | server 主动 push | `Peer::notify_resource_updated(ResourceUpdatedNotificationParam)` | 复用，worker 1s tick 调此 API |
//! | client 接收 push | `ClientHandler::on_resource_updated` — client 角色 handler | 自动接收 |
//! | Peer 句柄拿取路径 | `RequestContext<RoleServer>::peer` 是 `pub` 字段（`Peer<RoleServer>` = `ClientSink`）| server handler 直接 `context.peer.clone()` |
//!
//! **关键结论**：rmcp 0.11 没有「server 持注册表 + 1s tick push」的原生 helper，
//! proc 需自建 worker lifecycle（brainstorm 决策 3 拍板）。
//!
//! ## stage 2 Slice 实装范围（v0.18 落地）
//!
//! 1. **subscribe**：client 调 `resources/subscribe { uri }`（rmcp SDK 自动 ACK）→
//!    `ServerHandler::subscribe` impl 从 `context.peer` 拿 `Peer<RoleServer>` 句柄 →
//!    调本 worker `subscribe(uri, peer)` → 写入 `subscribers` 注册表 + lazy spawn
//!    push task（如未 spawn）
//! 2. **push**：push task 1s tick 遍历注册表 → 对每个 subscriber 调
//!    `peer.notify_resource_updated(ResourceUpdatedNotificationParam { uri })` →
//!    client 通过 `ClientHandler::on_resource_updated` 接收
//! 3. **unsubscribe / 断开**：client 主动 `resources/unsubscribe { uri }` →
//!    worker 从注册表 remove（drop Peer 让 push task 下 tick 不再调）；或网络断开
//!    → `peer.notify_resource_updated(...)` 返 `Err(ServiceError)` → push task
//!    自动从注册表移除（client 断开自动清理）
//!
//! ## v0.19 cycle stage 1 Spike 类型升级（multi-client 准备）
//!
//! v0.18 stage 2 落地的注册表是 `HashMap<String, Peer<RoleServer>>`（**stdio 单
//! client 假设**，REVIEW-v0.18 §4 Findings P2-S2）。v0.19 cycle SSE transport
//! 多 client 场景需升级为 `HashMap<String, Vec<Peer<RoleServer>>>`（同 URI 多
//! client 各占 vec 一位）。
//!
//! **stage 1 Spike 改动**（仅类型升级 + 业务最小适配，stage 2 实装完整 lifecycle）：
//!
//! - 注册表 value：`Peer<RoleServer>` → `Vec<Peer<RoleServer>>`
//! - subscribe 业务：`guard.entry(uri).or_default().push(peer)`（vec 追加）
//! - unsubscribe 业务：`vec.retain(...)` + vec 空时 remove 整个 entry
//! - spawn_push_task：双层 for 循环遍历（外层 uri / 内层 peers），stage 1 Spike
//!   简化策略——fail peer 时清空整个 vec（不精确），**stage 2 改造**为
//!   `Arc::ptr_eq` identity 检测 + `tokio::task::JoinSet` 并发 push + fail peer
//!   一次性精确 retain 清理（详见 ADR-0027 §6.3 multi-client 注册表）
//!
//! **stage 2 实装路径**（v0.19 cycle）：
//!
//! - `Arc::ptr_eq(p1, p2)` 做 vec 去重（Peer 内部 Arc 包装，cheap clone 后 pointer 一致）
//! - `JoinSet::spawn` 并发调 `peer.notify_resource_updated`
//! - `join_next().await` 逐个判断 fail peer + 一次性从 vec retain 清理
//!
//! ## 注册表设计
//!
//! v0.18 stage 2 落地：`HashMap<String /* uri */, Peer<RoleServer>>`（stdio
//! 单 client 假设——同 URI 单 client 订阅，第二次 subscribe 同 URI 覆盖第一次）。
//! v0.19 stage 1 Spike 类型升级：`HashMap<String, Vec<Peer<RoleServer>>>`
//! （同 URI 多 client 各占 vec 一位）。
//!
//! 详见 ADR-0027 §关键设计点 5（v0.18 stage 1 Spike 扩段）+ §6.3 multi-client
//! 注册表（v0.19 stage 1 Spike 扩段）+ brainstorm 决策 3 拍板「自建 worker lifecycle」。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rmcp::Peer;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::service::RoleServer;
use tokio::runtime::Handle as TokioHandle;

/// subscribe-push 注册表 key 类型别名（与 rmcp 0.11 内部 subscriber id 类型对齐）。
///
/// **stage 2 实装后**：注册表实际用 `String`（uri）作 key 而非 `SubscriberId`，
/// 因 rmcp 0.11 `SubscribeRequestParam` 只有 `uri` 字段无 subscriber_id（client
/// 不需要传 ID，server 用 uri 作 key）。本类型别名保留供未来 SSE multi-client
/// 落地时复用作 internal id。
pub type SubscriberId = u64;

/// subscribe-push worker lifecycle 容器（v0.18 stage 2 实装 / v0.19 stage 1 Spike 类型升级）。
///
/// 持 `subscribers: Arc<Mutex<HashMap<String, Vec<Peer<RoleServer>>>>>` 注册表 + lazy
/// spawn 1s tick push task。push task 持 `Arc::clone(&subscribers)`，1s tick
/// 遍历调 `peer.notify_resource_updated`，peer 失败（client 断开）从注册表移除。
///
/// **生命周期**：push task 在 tokio runtime 上 spawn，进程退出时 tokio runtime
/// shutdown 自动 cancel 所有 task（无 zombie task）。`shutdown` 方法保留 no-op
/// 占位，预留未来 graceful shutdown。
///
/// **设计要点**：
///
/// - **lazy spawn**：第一次 subscribe 时 spawn push task（避免无 subscriber 时空跑）
/// - **单 task 多 subscriber**：一个 push task 遍历所有 subscriber（不每个 subscribe
///   spawn 一个 task），与 brainstorm 决策 3 描述对齐
/// - **peer 断开自动清理**：push 失败时从注册表 remove（drop Peer 让 Arc 引用计数减 1）
/// - **v0.19 stage 1 Spike 类型升级**：注册表 value 从 `Peer` 升级为 `Vec<Peer>`
///   （同 URI 多 client 各占 vec 一位），适配 SSE multi-client 场景；stdio 路径
///   vec 长度始终 ≤ 1（单 client 假设保持）
pub struct SubscribePushWorker {
    /// 注册表：uri → Vec<Peer<RoleServer>>（v0.19 stage 1 Spike 升级自 v0.18 的
    /// `HashMap<String, Peer>`，适配 SSE multi-client 场景）。
    ///
    /// **stage 1 Spike 业务最小适配**：subscribe `entry().or_default().push(peer)` /
    /// unsubscribe `vec.retain(...)` 空时 remove 整个 entry / spawn_push_task 双层
    /// for 循环遍历 + fail peer 清空整个 vec（不精确，stage 2 改 Arc::ptr_eq 精确 retain）。
    subscribers: Arc<Mutex<HashMap<String, Vec<Peer<RoleServer>>>>>,
    /// push task 是否已 spawn（防重复 spawn，lazy spawn 第一次 subscribe 时）。
    task_spawned: Arc<Mutex<bool>>,
}

impl SubscribePushWorker {
    /// 创建空 worker（无 subscriber，push task 未 spawn）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(HashMap::new())),
            task_spawned: Arc::new(Mutex::new(false)),
        }
    }

    /// 注册 subscriber（v0.18 stage 2 实装 / v0.19 stage 1 Spike 类型适配）。
    ///
    /// 把 `Peer<RoleServer>` 句柄 push 到注册表 `Vec<Peer>`（key = uri），如 push task
    /// 未 spawn 则 lazy spawn。后续 push task 1s tick 会调 `peer.notify_resource_updated(uri)`
    /// 给 client 推送 `notifications/resources/updated` notification。
    ///
    /// **v0.19 stage 1 Spike 简化策略**：subscribe 不做 identity 去重（同 client 多次
    /// subscribe 同 URI 会 push 重复 vec 项）。**stage 2 改造**为 `Arc::ptr_eq(p1, p2)`
    /// 做 vec 去重（详见 ADR-0027 §6.3 multi-client 注册表）。
    ///
    /// # Errors
    ///
    /// - 注册表 mutex poisoned → 返 Err 含 poison 错误
    /// - push task spawn 失败（如不在 tokio runtime 上下文）→ 返 Err 含错误信息
    pub fn subscribe(&self, uri: &str, peer: Peer<RoleServer>) -> Result<(), String> {
        let mut guard = self
            .subscribers
            .lock()
            .map_err(|e| format!("subscribers mutex poisoned: {e}"))?;
        // v0.19 stage 1 Spike：vec push 无 identity 去重（stage 2 改 Arc::ptr_eq）
        guard.entry(uri.to_string()).or_default().push(peer);
        drop(guard);

        // lazy spawn push task（如未 spawn）
        let mut spawned = self
            .task_spawned
            .lock()
            .map_err(|e| format!("task_spawned mutex poisoned: {e}"))?;
        if !*spawned {
            self.spawn_push_task()?;
            *spawned = true;
        }
        Ok(())
    }

    /// 注销 subscriber（v0.18 stage 2 实装 / v0.19 stage 1 Spike 类型适配）。
    ///
    /// 从注册表 `Vec<Peer>` 清空对应 uri 的 vec（**stage 1 Spike 简化策略**——
    /// 清空整个 vec 不精确，stage 2 改 `Arc::ptr_eq` 精确 retain 单个 peer）。
    /// client 主动 `resources/unsubscribe { uri }` 时调用。
    ///
    /// **v0.19 stage 1 Spike 决策**：unsubscribe 清空整个 vec 是简化策略（与
    /// stage 1 Spike 「不动业务代码逻辑」原则对齐——identity 检测留 stage 2）。
    /// stdio 单 client 场景下 vec 长度 ≤ 1，清空整个 vec 等价 retain。
    /// SSE multi-client 场景下 stage 2 改为精确 retain + 空 vec 时 remove entry。
    ///
    /// # Errors
    ///
    /// 注册表 mutex poisoned → 返 Err 含 poison 错误。
    pub fn unsubscribe(&self, uri: &str) -> Result<(), String> {
        let mut guard = self
            .subscribers
            .lock()
            .map_err(|e| format!("subscribers mutex poisoned: {e}"))?;
        // v0.19 stage 1 Spike：清空 vec（无 identity 检测，stage 2 改 Arc::ptr_eq retain）
        if let Some(vec) = guard.get_mut(uri) {
            vec.clear();
        }
        // vec 空时 remove 整个 entry（避免 push task 遍历空 vec 浪费 tick）
        guard.retain(|_, vec| !vec.is_empty());
        Ok(())
    }

    /// spawn 1s tick push task（v0.18 stage 2 实装 / v0.19 stage 1 Spike 类型适配）。
    ///
    /// 用 `tokio::spawn` 在当前 tokio runtime 上 spawn 一个 task，持
    /// `Arc::clone(&subscribers)`，1s tick 双层 for 循环遍历注册表调
    /// `peer.notify_resource_updated`（外层 uri / 内层 peers）。peer 失败（client 断开）
    /// 清空对应 uri 的整个 vec（**stage 1 Spike 简化策略**——不精确，stage 2 改
    /// `Arc::ptr_eq` 精确 retain 单个 peer）。
    ///
    /// **stage 2 改造**（v0.19 cycle，详见 ADR-0027 §6.3）：
    /// - 用 `tokio::task::JoinSet` 并发 spawn 每个 peer 的 `notify_resource_updated`
    /// - `join_next().await` 逐个判断 fail peer + 一次性从 vec `Arc::ptr_eq` retain 清理
    /// - 可配 `JoinSet::build()` 限并发度（避免 100 个 client 同 URI 一次性 spawn 100 task）
    ///
    /// **必须在 tokio runtime 上下文调用**（`ServerHandler::subscribe` 方法是 async，
    /// 在 tokio context 内，OK）。
    ///
    /// # Errors
    ///
    /// 不在 tokio runtime 上下文 → 返 Err 含错误信息。
    pub fn spawn_push_task(&self) -> Result<(), String> {
        let subscribers = Arc::clone(&self.subscribers);

        // 检查是否在 tokio runtime 上下文（rmcp serve_server 已在 tokio runtime）
        let handle = TokioHandle::try_current()
            .map_err(|e| format!("spawn_push_task 必须在 tokio runtime 上下文: {e}"))?;

        handle.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                // snapshot 注册表（持锁窗口短，避免 push 期间阻塞 subscribe/unsubscribe）
                // v0.19 stage 1 Spike：snapshot 类型从 Vec<(String, Peer)> 升级为
                // Vec<(String, Vec<Peer>)>（适配 multi-client Vec<Peer> 注册表 value）
                let snapshot: Vec<(String, Vec<Peer<RoleServer>>)> = match subscribers.lock() {
                    Ok(g) => g.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    Err(_) => continue, // mutex poisoned, 下 tick 重试
                };
                // v0.19 stage 1 Spike：双层 for 循环遍历（stage 2 改 JoinSet 并发）
                for (uri, peers) in snapshot {
                    for peer in peers {
                        let params = ResourceUpdatedNotificationParam { uri: uri.clone() };
                        if peer.notify_resource_updated(params).await.is_err() {
                            // peer 断开 → 清空对应 uri 的 vec（stage 1 Spike 简化策略，
                            // stage 2 改 Arc::ptr_eq 精确 retain 单个 peer）
                            if let Ok(mut g) = subscribers.lock() {
                                if let Some(vec) = g.get_mut(&uri) {
                                    vec.clear();
                                }
                                // vec 空时 remove 整个 entry
                                g.retain(|_, v| !v.is_empty());
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }

    /// 触发 worker shutdown（v0.18 stage 2 保留 no-op 占位）。
    ///
    /// push task 在 tokio runtime 上 spawn，进程退出时 tokio runtime shutdown
    /// 自动 cancel 所有 task（无 zombie task）。本方法保留 no-op 占位，预留未来
    /// graceful shutdown（如 server 重载时主动停 push task）。
    pub fn shutdown(&mut self) {
        // no-op：tokio runtime shutdown 时 push task 自动 cancel
    }

    /// 返回注册表当前 subscriber 数量（所有 vec 长度之和，含同 URI 多 client）。
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map_or(0, |m| m.values().map(Vec::len).sum())
    }
}

impl Default for SubscribePushWorker {
    fn default() -> Self {
        Self::new()
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
    fn default_impl_matches_new() {
        let from_new = SubscribePushWorker::new();
        let from_default = SubscribePushWorker::default();
        assert_eq!(from_new.subscriber_count(), from_default.subscriber_count());
    }

    #[test]
    fn shutdown_is_noop_in_stage_2() {
        // stage 2：shutdown 是 no-op（tokio runtime shutdown 自动 cancel push task）
        let mut worker = SubscribePushWorker::new();
        worker.shutdown();
        assert_eq!(worker.subscriber_count(), 0);
    }

    #[test]
    fn unsubscribe_on_empty_registry_is_idempotent() {
        // stage 2：unsubscribe 不存在的 uri 不报错（idempotent）
        // 注意：真实 subscribe 业务需 Peer 实例（Peer::new 是 pub(crate) 无法在
        // proc 测试中构造），用 mcp-inspector manual 验证（stage 3 Review 时跑）
        let worker = SubscribePushWorker::new();
        assert_eq!(worker.subscriber_count(), 0);
        // unsubscribe 不存在的 uri 不报错
        worker.unsubscribe("proc://metrics/system").unwrap();
        worker.unsubscribe("proc://not/subscribed").unwrap();
        assert_eq!(worker.subscriber_count(), 0);
    }

    #[test]
    fn spawn_push_task_returns_err_without_tokio_runtime() {
        // stage 2：spawn_push_task 在无 tokio runtime 上下文会失败
        // （测试线程默认不在 tokio runtime，TokioHandle::try_current 返 Err）
        let worker = SubscribePushWorker::new();
        let result = worker.spawn_push_task();
        assert!(
            result.is_err(),
            "spawn_push_task 在无 tokio runtime 上下文应返 Err"
        );
    }

    #[test]
    fn subscribe_worker_source_uses_peer_role_server() {
        // v0.19 stage 1 Spike：静态断言注册表 value 类型从 Peer<RoleServer>
        // 升级为 Vec<Peer<RoleServer>>（适配 SSE multi-client 场景，与 brainstorm
        // 决策 3 + ADR-0027 §6.3 对齐）。stage 2 加 Arc::ptr_eq identity 检测 +
        // JoinSet 并发 push + fail peer 一次性精确 retain 清理。
        let source =
            std::fs::read_to_string("src/mcp/subscribe_worker.rs").expect("source readable");
        assert!(
            source.contains("subscribers: Arc<Mutex<HashMap<String, Vec<Peer<RoleServer>>>>>"),
            "v0.19 stage 1 Spike 注册表 value 应升级为 Vec<Peer<RoleServer>>"
        );
        assert!(
            source.contains("peer.notify_resource_updated"),
            "stage 2 push task 应调 peer.notify_resource_updated"
        );
        assert!(
            source.contains("TokioHandle::try_current"),
            "stage 2 spawn_push_task 应检查 tokio runtime 上下文"
        );
        assert!(
            source.contains("ResourceUpdatedNotificationParam"),
            "stage 2 push task 应构造 ResourceUpdatedNotificationParam"
        );
    }
}
