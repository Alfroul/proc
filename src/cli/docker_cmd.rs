//! `proc docker {ps,inspect,top,logs,images,volumes,image-rm,volume-rm,compose,events,exec}`
//! — 11 个 docker 子命令 dispatch（E1/E2/E3/E4）。

use colored::Colorize;

use crate::cli::def::DockerSub;
use crate::docker;
use crate::format::format_bytes;
use crate::shutdown;

pub fn run_docker(sub: &DockerSub) {
    let monitor = match docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            eprintln!("{}", "请确认 Docker 正在运行".yellow());
            std::process::exit(1);
        }
    };

    match sub {
        DockerSub::Ps => run_docker_ps(&monitor),
        DockerSub::Inspect { name } => run_docker_inspect(&monitor, name),
        DockerSub::Top { name } => run_docker_top(&monitor, name),
        DockerSub::Logs { name, follow, tail } => {
            run_docker_logs(&monitor, name, *follow, tail.as_deref())
        }
        DockerSub::Images => run_docker_images(&monitor),
        DockerSub::Volumes => run_docker_volumes(&monitor),
        DockerSub::ImageRm { id, force } => run_docker_image_rm(&monitor, id, *force),
        DockerSub::VolumeRm { name, force } => run_docker_volume_rm(&monitor, name, *force),
        DockerSub::Compose { args } => run_docker_compose(args),
        DockerSub::Events => run_docker_events(&monitor),
        DockerSub::Exec { container, cmd } => run_docker_exec(&monitor, container, cmd),
    }
}

fn run_docker_ps(monitor: &docker::DockerMonitor) {
    match monitor.list_containers(true) {
        Ok(containers) => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["状态", "名称", "镜像", "健康", "运行时长"]);
            for c in &containers {
                let status_icon = match c.state.as_str() {
                    "running" => "▲ 运行",
                    "exited" | "dead" => "■ 停止",
                    _ => &c.state,
                };
                let uptime = c
                    .running_since
                    .map(|s| {
                        let elapsed = s.elapsed().unwrap_or(std::time::Duration::ZERO);
                        let secs = elapsed.as_secs();
                        if secs < 60 {
                            format!("{}秒", secs)
                        } else if secs < 3600 {
                            format!("{}分", secs / 60)
                        } else if secs < 86400 {
                            format!("{}时", secs / 3600)
                        } else {
                            format!("{}天", secs / 86400)
                        }
                    })
                    .unwrap_or_else(|| "-".to_string());
                table.add_row(vec![
                    status_icon.to_string(),
                    c.name.clone(),
                    c.image.clone(),
                    c.health.to_string(),
                    uptime,
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取容器列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_inspect(monitor: &docker::DockerMonitor, name: &str) {
    let container = monitor.list_containers(true).ok().and_then(|cs| {
        cs.into_iter()
            .find(|c| c.name == name || c.id.starts_with(name))
    });

    let Some(c) = container else {
        eprintln!("{}", format!("容器 '{}' 未找到", name).red());
        std::process::exit(1);
    };
    println!("{}", format!("容器: {} ({})", c.name, c.id).cyan());
    println!("镜像: {}", c.image);
    println!("状态: {}", c.status);
    println!("健康: {}", c.health);

    match monitor.inspect_health(name) {
        Ok(health) => println!("健康详情: {}", health),
        Err(e) => println!("{} 健康检查失败: {}", "⚠".yellow(), e),
    }

    match monitor.get_stats(name) {
        Ok(stats) => {
            println!("CPU:  {:.1}%", stats.cpu_percent);
            println!(
                "内存: {} / {}",
                format_bytes(stats.memory_usage),
                format_bytes(stats.memory_limit)
            );
            println!(
                "网络: ↓{} ↑{}",
                format_bytes(stats.network_in),
                format_bytes(stats.network_out)
            );
        }
        Err(e) => println!("{} 获取统计失败: {}", "⚠".yellow(), e),
    }
}

fn run_docker_top(monitor: &docker::DockerMonitor, name: &str) {
    match monitor.container_top(name) {
        Ok(procs) => {
            if procs.is_empty() {
                println!("{}", "容器内无进程（可能未运行）".yellow());
                return;
            }
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["PID", "USER", "START", "TIME", "CMD"]);
            for p in &procs {
                table.add_row(vec![
                    p.pid.clone(),
                    p.user.clone(),
                    p.started.clone(),
                    p.cpu_time.clone(),
                    p.command.clone(),
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取进程列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_logs(monitor: &docker::DockerMonitor, name: &str, follow: bool, tail: Option<&str>) {
    if follow {
        // follow 模式：用 logs_worker 同样的策略（spawn thread + runtime）。
        let docker_client = monitor.docker();
        let worker =
            docker::logs_worker::spawn(docker_client, name.to_string(), tail.map(str::to_string));
        println!("{}", format!("跟随 {} 日志（Ctrl+C 停止）", name).cyan());
        loop {
            if shutdown::requested() {
                println!();
                return;
            }
            for chunk in worker.drain() {
                for line in chunk.lines {
                    let prefix = if line.is_stderr { "[stderr] " } else { "" };
                    println!("{}{}", prefix, line.message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    match monitor.collect_logs(name, tail) {
        Ok(logs) => {
            for line in logs {
                let prefix = if line.is_stderr { "[stderr] " } else { "" };
                println!("{}{}", prefix, line.message);
            }
        }
        Err(e) => {
            eprintln!("{} {}", "获取日志失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_images(monitor: &docker::DockerMonitor) {
    match monitor.list_images() {
        Ok(images) => {
            if images.is_empty() {
                println!("{}", "暂无镜像".yellow());
                return;
            }
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["ID", "Tags", "大小", "容器数", "创建"]);
            for img in &images {
                let tags = if img.repo_tags.is_empty() {
                    "<none>".to_string()
                } else {
                    img.repo_tags.join(", ")
                };
                table.add_row(vec![
                    img.short_id.clone(),
                    tags,
                    format_bytes(img.size),
                    img.containers.to_string(),
                    format!("{}", img),
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取镜像列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_volumes(monitor: &docker::DockerMonitor) {
    match monitor.list_volumes() {
        Ok(volumes) => {
            if volumes.is_empty() {
                println!("{}", "暂无 volume".yellow());
                return;
            }
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["名称", "驱动", "挂载点", "大小", "使用"]);
            for v in &volumes {
                let size = if v.size > 0 {
                    format_bytes(v.size)
                } else {
                    "-".to_string()
                };
                let used = if v.in_use { "使用中" } else { "未使用" };
                table.add_row(vec![
                    v.name.clone(),
                    v.driver.clone(),
                    v.mountpoint.clone(),
                    size,
                    used.to_string(),
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取 volume 列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_image_rm(monitor: &docker::DockerMonitor, id: &str, force: bool) {
    match monitor.remove_image(id, force) {
        Ok(()) => println!("{}", format!("镜像 {} 已删除", id).green()),
        Err(e) => {
            eprintln!("{} {}", "删除失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_volume_rm(monitor: &docker::DockerMonitor, name: &str, force: bool) {
    match monitor.remove_volume(name, force) {
        Ok(()) => println!("{}", format!("volume {} 已删除", name).green()),
        Err(e) => {
            eprintln!("{} {}", "删除失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_compose(args: &[String]) {
    use std::process::Command;
    let bin = std::env::var("PROC_DOCKER_COMPOSE").unwrap_or_else(|_| "docker-compose".to_string());
    let status = Command::new(&bin).args(args).status().unwrap_or_else(|e| {
        eprintln!(
            "{} 调用 {} 失败: {}（请确认已安装 docker-compose）",
            "错误:".red(),
            bin,
            e
        );
        std::process::exit(127);
    });
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn run_docker_events(monitor: &docker::DockerMonitor) {
    let docker_client = monitor.docker();
    let receiver = docker::events::spawn_event_watcher(docker_client);
    println!("{}", "监听 Docker 事件中... (Ctrl+C 停止)".cyan());

    loop {
        if shutdown::requested() {
            println!("{}", "停止事件监听".yellow());
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        while let Some(event) = receiver.try_recv() {
            let name = event
                .container_name
                .as_deref()
                .unwrap_or(&event.container_id);
            let style = match event.action.as_str() {
                "die" | "stop" => "red",
                "start" => "green",
                _ => "yellow",
            };
            let styled = match style {
                "red" => format!("{} {} ({})", event.action, name, event.container_id).red(),
                "green" => format!("{} {} ({})", event.action, name, event.container_id).green(),
                _ => format!("{} {} ({})", event.action, name, event.container_id).yellow(),
            };
            println!("{}", styled);
        }
    }
}

/// CLI `proc docker exec <container> [cmd...]`（阶段 9 E2）。
///
/// 直接 spawn `docker exec -it <container> <cmd>`，docker CLI 接管 stdio，
/// 用户的终端 = 远端 PTY（无需 proc 自身的 PTY 桥接）。
///
/// TUI 内按 `e` 走另一条路：[`crate::tui::container_exec_view`] 嵌入式 PTY 视图。
fn run_docker_exec(monitor: &docker::DockerMonitor, container: &str, cmd: &[String]) {
    use std::process::Command;

    // 容器存在性检查：友好错误优于 docker CLI 的晦涩报错。
    let containers = monitor.list_containers(true).unwrap_or_default();
    let found = containers
        .iter()
        .find(|c| c.name == container || c.id.starts_with(container));
    let Some(found) = found else {
        eprintln!("{}", format!("容器 '{}' 未找到", container).red());
        std::process::exit(1);
    };

    // cmd 为空时根据 image 推断 shell；非空时透传用户命令。
    let inferred_shell = if cmd.is_empty() {
        docker::exec::detect_default_shell(&found.image)
    } else {
        ""
    };

    let mut command = Command::new("docker");
    command.arg("exec").arg("-it").arg(container);
    if cmd.is_empty() {
        for token in inferred_shell.split_whitespace() {
            command.arg(token);
        }
    } else {
        for token in cmd {
            command.arg(token);
        }
    }

    match command.status() {
        Ok(status) => {
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Err(e) => {
            eprintln!("{} {}", "exec 失败（确认 PATH 有 docker）:".red(), e);
            std::process::exit(1);
        }
    }
}
