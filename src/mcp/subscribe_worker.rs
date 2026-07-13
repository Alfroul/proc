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
//! ## v0.18 stage 2 实装范围（subscribe-push lifecycle 三步）
//!
//! 1. **subscribe**：client 调 `resources/subscribe { uri }`（rmcp SDK 自动 ACK）→
//!    `ServerHandler::subscribe` impl 从 `context.peer` 拿 `Peer<RoleServer>` 句柄 →
//!    调本 worker `subscribe(uri, peer)` → 写入 `subscribers` 注册表 + lazy spawn
//!    push task（如未 spawn）
//! 2. **push**：push task 1s tick snapshot 注册表 → 用 `tokio::task::JoinSet` 并发
//!    spawn 每个 `Arc<Peer>` 的 `notify_resource_updated` 调用 → `join_next().await`
//!    逐个判断 fail peer + 用 `Arc::ptr_eq` 一次性从 vec retain 精确清理
//! 3. **unsubscribe / 断开**：client 主动 `resources/unsubscribe { uri }` →
//!    worker 从注册表 remove（**stage 2 已知限制**——清空整个 vec，SSE multi-client
//!    下不影响其他 client 的 push 因为 push task 失败 cleanup 会兜底，详见 ADR-0027
//!    §6.3）；或网络断开 → `peer.notify_resource_updated(...)` 返 `Err(ServiceError)`
//!    → push task `Arc::ptr_eq` 精确从 vec 清理
//!
//! ## v0.19 cycle stage 2 实装范围（multi-client + 并发 push）
//!
//! - 注册表 value：`Vec<Peer<RoleServer>>` → `Vec<Arc<Peer<RoleServer>>>`
//!   （让 push task 失败 cleanup 走 `Arc::ptr_eq` 精确 retain 单个 peer）
//! - subscribe：`Arc::new(peer)` 包装后 push（**不做跨调用 dedup**——rmcp 0.11
//!   `Peer` 不暴露 public identity API，每次 `context.peer.clone()` 返新值；
//!   重复订阅依赖 push task 失败 cleanup 兜底）
//! - unsubscribe：清空 vec（**SSE multi-client 已知限制**——A client unsubscribe
//!   会清掉 B client 同 URI 订阅；stdio 单 client 场景正确；mcp-session-id-based
//!   精确 removal 留 v0.20+ cycle，详见 ADR-0027 §6.3 fallback 段）
//! - spawn_push_task：`tokio::task::JoinSet::spawn` 并发调 `peer.notify_resource_updated`
//!   + `join_next().await` 逐个判断 fail peer + 一次性 `Arc::ptr_eq` retain 精确清理
//!
//! ## 注册表设计
//!
//! v0.18 stage 2 落地：`HashMap<String /* uri */, Peer<RoleServer>>`（stdio
//! 单 client 假设——同 URI 单 client 订阅，第二次 subscribe 同 URI 覆盖第一次）。
//! v0.19 stage 1 Spike 类型升级：`HashMap<String, Vec<Peer<RoleServer>>>`
//! （同 URI 多 client 各占 vec 一位）。
//! v0.19 stage 2 进一步升级：`HashMap<String, Vec<Arc<Peer<RoleServer>>>>`
//! （让 push task 失败 cleanup 走 `Arc::ptr_eq` 精确 retain）。
//!
//! 详见 ADR-0027 §关键设计点 5（v0.18 stage 1 Spike 扩段）+ §6.3 multi-client
//! 注册表（v0.19 stage 1 Spike 扩段 + stage 2 实装 + 已知限制）+ brainstorm 决策 3。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rmcp::Peer;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::service::RoleServer;
use tokio::runtime::Handle as TokioHandle;
use tokio::task::JoinSet;

/// subscribe-push 注册表 key 类型别名（与 rmcp 0.11 内部 subscriber id 类型对齐）。
///
/// **stage 2 实装后**：注册表实际用 `String`（uri）作 key 而非 `SubscriberId`，
/// 因 rmcp 0.11 `SubscribeRequestParam` 只有 `uri` 字段无 subscriber_id（client
/// 不需要传 ID，server 用 uri 作 key）。本类型别名保留供未来 SSE multi-client
/// 落地时复用作 internal id。
pub type SubscriberId = u64;

/// v0.19 stage 2 multi-client 注册表类型别名（clippy type_complexity mitigate）。
///
/// `uri → Vec<Arc<Peer<RoleServer>>>` —— 同 URI 多 client 各占 vec 一位，
/// `Arc<Peer>` 让 push task 失败 cleanup 走 `Arc::ptr_eq` 精确 retain 单个 peer。
type SubscribeRegistry = HashMap<String, Vec<Arc<Peer<RoleServer>>>>;

/// subscribe-push worker lifecycle 容器（v0.18 stage 2 实装 / v0.19 stage 2 multi-client + JoinSet 改造）。
///
/// 持 `subscribers: Arc<Mutex<HashMap<String, Vec<Arc<Peer<RoleServer>>>>>>` 注册表 + lazy
/// spawn 1s tick push task。push task 持 `Arc::clone(&subscribers)`，1s tick 用 `JoinSet`
/// 并发调 `peer.notify_resource_updated`，peer 失败（client 断开）用 `Arc::ptr_eq` 精确从
/// vec retain 清理。
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
/// - **peer 断开自动清理**：push 失败时 `Arc::ptr_eq` 精确 retain 清理单个 peer
/// - **multi-client 支持**：注册表 value 是 `Vec<Arc<Peer>>`，同 URI 多 client
///   各占 vec 一位；JoinSet 并发 push 避免 1 个慢 peer 阻塞其他 peer
/// - **已知限制**：subscribe 跨调用不 dedup + unsubscribe 清空整个 vec（rmcp 0.11
///   `Peer` 不暴露 public identity API），mcp-session-id-based 精确 identity 留
///   v0.20+ cycle
pub struct SubscribePushWorker {
    /// 注册表：uri → Vec<Arc<Peer<RoleServer>>>（v0.19 stage 2 升级自 v0.18 的
    /// `HashMap<String, Peer>` → stage 1 Spike 的 `HashMap<String, Vec<Peer>>`
    /// → stage 2 的 `HashMap<String, Vec<Arc<Peer>>>`，让 push task 失败 cleanup
    /// 走 `Arc::ptr_eq` 精确 retain 单个 peer）。
    subscribers: Arc<Mutex<SubscribeRegistry>>,
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

    /// 注册 subscriber（v0.18 stage 2 实装 / v0.19 stage 2 Arc<Peer> 包装）。
    ///
    /// 把 `Peer<RoleServer>` 句柄 wrap 成 `Arc<Peer>` 后 push 到注册表 vec（key = uri），
    /// 如 push task 未 spawn 则 lazy spawn。后续 push task 1s tick 会调
    /// `peer.notify_resource_updated(uri)` 给 client 推送 `notifications/resources/updated` notification。
    ///
    /// **v0.19 stage 2 已知限制**：subscribe 跨调用不做 dedup（rmcp 0.11 `Peer` 不
    /// 暴露 public identity API，每次 `context.peer.clone()` 返新值）。同 client 重复
    /// subscribe 同 URI 会在 vec 中累积多个 `Arc<Peer>` 副本，push 时该 client 收到
    /// N 次重复 notification（client-side dedup 或 push task 失败 cleanup 兜底）。
    /// mcp-session-id-based subscribe dedup 留 v0.20+ cycle。
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
        // v0.19 stage 2：Arc::new 包装后 push。每次 subscribe 调用都会 wrap 一个新 Arc，
        // 即使同 client 多次订阅也累积多个 Arc<Peer> 副本（依赖 push task 失败 cleanup
        // 兜底；mcp-session-id-based dedup 留 v0.20+ cycle）。
        guard
            .entry(uri.to_string())
            .or_default()
            .push(Arc::new(peer));
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

    /// 注销 subscriber（v0.18 stage 2 实装 / v0.19 stage 2 已知限制）。
    ///
    /// 清空对应 uri 的整个 vec + vec 空时 remove 整个 entry。client 主动
    /// `resources/unsubscribe { uri }` 时调用。
    ///
    /// **v0.19 stage 2 已知限制**：MCP 协议 `UnsubscribeRequestParam` 只有 `uri`
    /// 字段，不携带 client/subscriber identity；rmcp 0.11 `Peer` 也不暴露 public
    /// identity API。无法精确识别「哪个 client 在 unsubscribe」，所以清空整个 vec。
    /// stdio 单 client 场景下正确（vec 长度 ≤ 1）；SSE multi-client 场景下 A client
    /// unsubscribe 会清掉 B client 同 URI 订阅——B 想继续订阅需 client-side 重 subscribe。
    ///
    /// **v0.20+ cycle 改进方向**：从 `RequestContext::extensions` 拿 `mcp-session-id`
    /// HTTP header（SSE 路径）作为 client identity，让 unsubscribe 走
    /// `vec.retain(|p| !Arc::ptr_eq(p, &target))` 精确移除单个 client。
    ///
    /// # Errors
    ///
    /// 注册表 mutex poisoned → 返 Err 含 poison 错误。
    pub fn unsubscribe(&self, uri: &str) -> Result<(), String> {
        let mut guard = self
            .subscribers
            .lock()
            .map_err(|e| format!("subscribers mutex poisoned: {e}"))?;
        // v0.19 stage 2：清空 vec（无 identity 检测，stage 2 已知限制——SSE multi-client
        // 下会清掉其他 client 同 URI 订阅；stdio 单 client 场景正确）。
        if let Some(vec) = guard.get_mut(uri) {
            vec.clear();
        }
        // vec 空时 remove 整个 entry（避免 push task 遍历空 vec 浪费 tick）
        guard.retain(|_, vec| !vec.is_empty());
        Ok(())
    }

    /// spawn 1s tick push task（v0.18 stage 2 实装 / v0.19 stage 2 JoinSet 并发改造）。
    ///
    /// 用 `tokio::spawn` 在当前 tokio runtime 上 spawn 一个 task，持
    /// `Arc::clone(&subscribers)`，1s tick：
    ///
    /// 1. snapshot 注册表为 `Vec<(String, Vec<Arc<Peer>>)>`（持锁窗口短，避免 push
    ///    期间阻塞 subscribe/unsubscribe）
    /// 2. 用 `tokio::task::JoinSet` 并发 spawn 每个 `Arc<Peer>::clone` 的
    ///    `notify_resource_updated` 调用（避免 1 个慢 peer 阻塞其他 peer push）
    /// 3. `join_next().await` 逐个判断每个 spawn 任务的返回值——失败的 peer
    ///    （client 断开）用 `Arc::ptr_eq` 精确从 vec retain 清理（drop Arc 让 push
    ///    task 下 tick 不再调）
    /// 4. vec 空时从 HashMap remove 整个 entry
    ///
    /// **JoinSet 相对 join_all / 顺序 for 的优势**（详见 ADR-0027 §6.3 + brainstorm FAQ Q5）：
    /// - **fail peer 一次性清理**：JoinSet 的 `join_next().await` 逐个判断 vs join_all
    ///   等所有 future 完成
    /// - **abort 能力**：`abort_all()` 在 worker shutdown 时主动取消（预留 graceful shutdown）
    /// - **backpressure**：可配 `JoinSet::build()` 限并发度（避免 100 个 client 同 URI
    ///   一次性 spawn 100 个 task；当前用默认无界，v0.20+ cycle 评估限并发）
    ///
    /// **Arc::ptr_eq 精确 cleanup**：stored Arc<Peer> 与 spawned Arc::clone(&stored Arc)
    /// 共享同一 allocation，`Arc::ptr_eq(p1, p2)` 返 true 即可精确识别 fail peer 并 retain
    /// 清理（vs Peer 不暴露 identity API 时无法精确 removal 的 stage 1 Spike 简化策略）。
    ///
    /// **必须在 tokio runtime 上下文调用**（`ServerHandler::subscribe` 方法是 async，
    /// 在 tokio context 内，OK）。
    ///
    /// # Errors
    ///
    /// 不在 tokio runtime 上下文 → 返 Err 含错误信息。
    pub fn spawn_push_task(&self) -> Result<(), String> {
        let subscribers = Arc::clone(&self.subscribers);

        // 检查是否在 tokio runtime 上下文（rmcp serve_server / serve_sse 已在 tokio runtime）
        let handle = TokioHandle::try_current()
            .map_err(|e| format!("spawn_push_task 必须在 tokio runtime 上下文: {e}"))?;

        handle.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                // 1. snapshot 注册表（持锁窗口短，避免 push 期间阻塞 subscribe/unsubscribe）。
                // snapshot 类型 `Vec<(String, Vec<Arc<Peer>>)>` —— Arc::clone 是 cheap clone
                // （仅增 refcount），snapshot 期间 vec 内 Arc<Peer> 不会被 drop。
                let snapshot: Vec<(String, Vec<Arc<Peer<RoleServer>>>)> = match subscribers.lock() {
                    Ok(g) => g.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    Err(_) => continue, // mutex poisoned, 下 tick 重试
                };

                // 2. JoinSet 并发 spawn 每个 Arc<Peer>::clone 的 notify_resource_updated。
                let mut join_set: JoinSet<(String, Arc<Peer<RoleServer>>, bool)> = JoinSet::new();
                for (uri, peers) in snapshot {
                    for peer_arc in peers {
                        let uri_clone = uri.clone();
                        // Arc::clone 让 spawned task 持同一 allocation 的 Arc，失败时
                        // Arc::ptr_eq(stored, spawned) 可精确识别同一 peer。
                        join_set.spawn(async move {
                            let params = ResourceUpdatedNotificationParam {
                                uri: uri_clone.clone(),
                            };
                            let is_err = peer_arc.notify_resource_updated(params).await.is_err();
                            (uri_clone, peer_arc, is_err)
                        });
                    }
                }

                // 3. join_next().await 逐个判断 + 失败的 peer 用 Arc::ptr_eq 精确 retain 清理。
                while let Some(res) = join_set.join_next().await {
                    if let Ok((uri, failed_arc, is_err)) = res {
                        if is_err {
                            // peer 断开 → Arc::ptr_eq 精确 retain 清理单个 peer
                            if let Ok(mut g) = subscribers.lock() {
                                if let Some(vec) = g.get_mut(&uri) {
                                    vec.retain(|p| !Arc::ptr_eq(p, &failed_arc));
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
        // v0.19 stage 2：静态断言注册表 value 类型从 Vec<Peer<RoleServer>> 升级为
        // Vec<Arc<Peer<RoleServer>>>（让 push task 失败 cleanup 走 Arc::ptr_eq 精确 retain）。
        // type alias `SubscribeRegistry` 拆复杂类型（clippy type_complexity mitigate）。
        // 与 brainstorm 决策 3 + ADR-0027 §6.3 multi-client 注册表 stage 2 实装对齐。
        let source =
            std::fs::read_to_string("src/mcp/subscribe_worker.rs").expect("source readable");
        assert!(
            source.contains("type SubscribeRegistry = HashMap<String, Vec<Arc<Peer<RoleServer>>>>"),
            "v0.19 stage 2 注册表应用 type alias SubscribeRegistry 拆复杂类型（clippy type_complexity mitigate）"
        );
        assert!(
            source.contains("subscribers: Arc<Mutex<SubscribeRegistry>>"),
            "v0.19 stage 2 subscribers 字段应用 SubscribeRegistry type alias"
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
        assert!(
            source.contains("JoinSet"),
            "v0.19 stage 2 push task 应用 JoinSet 并发（替代 stage 1 Spike 双层 for 循环）"
        );
        assert!(
            source.contains("Arc::ptr_eq"),
            "v0.19 stage 2 push task 失败 cleanup 应用 Arc::ptr_eq 精确 retain（替代 stage 1 Spike vec.clear()）"
        );
    }

    #[test]
    fn subscribe_worker_source_documents_known_limitations() {
        // v0.19 stage 2：静态断言 source 含已知限制 doc 注释（让 stage 3 Reviewer
        // + v0.20+ cycle 接力者一眼看到 SSE multi-client identity 的 stage 2 缺口）
        let source =
            std::fs::read_to_string("src/mcp/subscribe_worker.rs").expect("source readable");
        assert!(
            source.contains("stage 2 已知限制"),
            "stage 2 source 应含「已知限制」doc 注释（subscribe 不 dedup + unsubscribe 清空 vec）"
        );
        assert!(
            source.contains("mcp-session-id"),
            "stage 2 source 应含 mcp-session-id 改进方向（v0.20+ cycle 精确 identity）"
        );
    }
}
