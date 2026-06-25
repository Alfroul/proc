//! `proc dns` — DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。

use colored::Colorize;

/// `proc dns` 子命令：流式输出 DNS 查询日志。仅 Windows 平台（其它平台
/// [`crate::dns_log::detect_collector`] 返回 None，给出降级提示）。
pub fn run_dns(tail: bool, since: Option<&str>) {
    if let Some(s) = since {
        // 隐私约束：DNS 查询不持久化；`--since` 需要从持久化源（Windows EventLog）
        // 读历史。本阶段未实现历史读取（需要单独的 Get-WinEvent 一次性查询路径），
        // 留作未来工作 —— stage-8.md §7 明确「需要持久化？本阶段不做，留 TODO」。
        eprintln!(
            "{}",
            "--since 暂未实现：DNS 日志仅内存缓冲，不持久化。请用 --tail 实时跟随。".yellow()
        );
        let _ = s;
        return;
    }

    let Some(collector) = crate::dns_log::detect_collector() else {
        eprintln!(
            "{}",
            "DNS 日志采集在此平台不可用（Windows 走 PowerShell Get-WinEvent，其它见 ADR-0006）"
                .yellow()
        );
        return;
    };

    println!("{}", "DNS 日志跟随中（仅内存 · Ctrl+C 退出）...".cyan());
    let mut collector = collector;

    // tail 模式：每 500ms drain collector，新事件打 stdout。
    // 非 tail 模式：drain 一次拿现有事件，然后退出（与 --since 互补）。
    let poll = std::time::Duration::from_millis(500);
    let mut printed_any = false;
    loop {
        let queries = collector.drain();
        for q in &queries {
            println!("{q}");
            printed_any = true;
        }
        if crate::shutdown::requested() {
            break;
        }
        if !tail {
            if !printed_any {
                eprintln!(
                    "{}",
                    "当前暂无 DNS 查询日志（启动浏览器或 curl 触发，或用 --tail 持续跟随）"
                        .yellow()
                );
            }
            break;
        }
        std::thread::sleep(poll);
    }
}
