use std::sync::mpsc::{self, TrySendError};
use std::time::SystemTime;

/// Bounded channel capacity for the Docker events backpressure queue.
/// See ADR-0006 for rationale (≈6 s buffer at 10 events/sec).
const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Docker 容器事件
#[derive(Debug, Clone)]
pub struct DockerEvent {
    pub action: String,
    pub container_id: String,
    pub container_name: Option<String>,
    pub timestamp: SystemTime,
}

/// 事件接收器（非阻塞）
pub struct DockerEventReceiver {
    rx: mpsc::Receiver<DockerEvent>,
}

impl DockerEventReceiver {
    #[must_use]
    pub fn try_recv(&self) -> Option<DockerEvent> {
        self.rx.try_recv().ok()
    }
}

/// 启动后台事件监听线程
///
/// 使用独立的 tokio runtime，避免与 TUI 主循环冲突。
/// 连接断开时尝试重连（指数退避，上限 60s），超过 10 次放弃。
/// 接收端被 drop 时线程自动退出。
#[must_use]
pub fn spawn_event_watcher(docker: bollard::Docker) -> DockerEventReceiver {
    let (tx, rx) = mpsc::sync_channel(EVENT_CHANNEL_CAPACITY);

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(_) => return,
        };

        rt.block_on(async move {
            use bollard::system::EventsOptions;
            use futures_util::stream::TryStreamExt;
            use std::collections::HashMap;

            let mut filters = HashMap::new();
            filters.insert("type".to_string(), vec!["container".to_string()]);
            filters.insert(
                "event".to_string(),
                vec![
                    "die".to_string(),
                    "stop".to_string(),
                    "start".to_string(),
                    "health_status".to_string(),
                ],
            );

            let options: EventsOptions<String> = EventsOptions {
                filters,
                ..Default::default()
            };

            const MAX_RECONNECT_ATTEMPTS: u32 = 10;
            let mut attempts: u32 = 0;

            'outer: loop {
                let mut stream = docker.events(Some(options.clone()));
                loop {
                    match stream.try_next().await {
                        Ok(Some(event)) => {
                            let container_id = event
                                .actor
                                .as_ref()
                                .and_then(|a| a.id.clone())
                                .unwrap_or_default();

                            let container_name = event
                                .actor
                                .as_ref()
                                .and_then(|a| a.attributes.as_ref())
                                .and_then(|attrs| attrs.get("name"))
                                .cloned();

                            let docker_event = DockerEvent {
                                action: event.action.unwrap_or_default(),
                                container_id,
                                container_name,
                                timestamp: SystemTime::now(),
                            };

                            // ADR-0006: bounded sync_channel. On Full we drop
                            // the new event (keep history). On Disconnected
                            // (consumer dropped) we exit the watcher.
                            match tx.try_send(docker_event) {
                                Ok(()) => {}
                                Err(TrySendError::Full(_)) => {
                                    tracing::warn!(
                                        "Docker 事件通道已满（{}），丢弃新事件",
                                        EVENT_CHANNEL_CAPACITY
                                    );
                                }
                                Err(TrySendError::Disconnected(_)) => break 'outer,
                            }
                        }
                        Ok(None) => break, // stream ended cleanly — try reconnect
                        Err(e) => {
                            attempts += 1;
                            tracing::warn!(
                                attempt = attempts,
                                error = %e,
                                "Docker 事件流断开，尝试重连",
                            );
                            if attempts >= MAX_RECONNECT_ATTEMPTS {
                                tracing::warn!(
                                    attempts = attempts,
                                    "Docker 事件流重连失败超过上限，停止监听",
                                );
                                break 'outer;
                            }
                            // 指数退避：1, 2, 4, ..., 上限 60s
                            let backoff_secs = std::cmp::min(1u64 << (attempts - 1).min(6), 60);
                            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                            break; // re-create stream
                        }
                    }
                }

                // 接收端 drop 时，下一次 tx.send 业务事件会返回 Err → break 'outer。
                // 这里无需主动探测：重连 + 退避循环本身有限（最多 MAX_RECONNECT_ATTEMPTS），
                // 即便消费者已死也只会浪费几次退避时间，不会泄漏线程。
            }
        });
    });

    DockerEventReceiver { rx }
}

/// 格式化事件描述（用于 CLI 输出）
#[must_use]
pub fn format_event_description(event: &DockerEvent) -> String {
    let name = event
        .container_name
        .as_deref()
        .unwrap_or(&event.container_id);
    match event.action.as_str() {
        "die" | "stop" => format!("容器 {} 已停止", name),
        "start" => format!("容器 {} 已启动", name),
        "health_status" => format!("容器 {} 健康状态变化", name),
        _ => format!("容器 {} 事件: {}", name, event.action),
    }
}
