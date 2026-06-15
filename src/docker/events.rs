use std::sync::mpsc;
use std::time::SystemTime;

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
    pub fn try_recv(&self) -> Option<DockerEvent> {
        self.rx.try_recv().ok()
    }
}

/// 启动后台事件监听线程
///
/// 使用独立的 tokio runtime，避免与 TUI 主循环冲突。
/// 线程在连接断开或接收端被 drop 时自动退出。
pub fn spawn_event_watcher(docker: bollard::Docker) -> DockerEventReceiver {
    let (tx, rx) = mpsc::channel();

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

            let mut stream = docker.events(Some(options));

            while let Ok(Some(event)) = stream.try_next().await {
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

                if tx.send(docker_event).is_err() {
                    break;
                }
            }
        });
    });

    DockerEventReceiver { rx }
}

/// 格式化事件描述（用于 CLI 输出）
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
