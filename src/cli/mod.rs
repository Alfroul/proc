// v0.6.0 阶段 5 #6：原 `src/cli.rs` 拆为 `src/cli/{mod.rs, def.rs, ...}`,
// 每个文件持一类 CLI 子命令的 dispatch 实现。`def.rs` 保留 clap derive 定义。

pub mod def;
pub mod diag;
pub mod dns;
pub mod docker_cmd;
// v0.7 阶段 3：`proc completions --shell <SHELL>` 在线生成补全脚本。
pub mod completions;
pub mod eject;
pub mod export;
pub mod flows;
pub mod handles;
pub mod kill;
pub mod ls;
pub mod mcp_cmd;
pub mod monitor;
pub mod port;
pub mod priority;
pub mod record;
pub mod smart;
pub mod throttle;

// 让旧路径 `proc::cli::Cli` / `proc::cli::Command` / `proc::cli::DockerSub` 继续 work。
pub use def::{Cli, Command, DockerSub, McpSub};

/// CLI dispatch 总入口 — `main.rs` 收到 `cli_args.command` 后转交给它。
///
/// 每个子命令的 `run_*` 实现位于对应子模块（`ls::run_ls` / `kill::run_kill` ...），
/// 此处仅做 match dispatch。本函数与 `def.rs::Command` 同步演进。
pub fn run_subcommand(cmd: &Command) {
    match cmd {
        Command::Ls {
            sort,
            limit,
            filter,
        } => ls::run_ls(sort, limit, filter),
        Command::Kill { pid, force } => kill::run_kill(*pid, *force),
        Command::Pkill {
            name,
            force,
            dry_run,
        } => kill::run_pkill(name, *force, *dry_run),
        Command::Tree => ls::run_tree(),
        Command::Port {
            port,
            kill: do_kill,
            stats,
        } => port::run_port(port, do_kill, stats),
        Command::Eject { drive, find_locks } => eject::run_eject(drive, find_locks),
        Command::Who { target_path } => handles::run_who(target_path),
        Command::Handles { pid, file } => handles::run_handles(pid, file),
        Command::Priority { pid, set } => priority::run_priority(*pid, set),
        Command::Affinity { pid, set } => priority::run_affinity(*pid, set),
        // v0.7 阶段 6：`proc throttle <pid> on|off` —— Windows 11 EcoQoS。
        Command::Throttle { pid, state } => throttle::run_throttle(*pid, state),
        Command::Monitor {
            add,
            remove,
            port,
            pid,
            command,
        } => monitor::run_monitor(*add, remove, port, pid, command),
        Command::Docker { sub } => docker_cmd::run_docker(sub),
        Command::Smart { device } => smart::run_smart(device.as_deref()),
        Command::Dns { tail, since } => dns::run_dns(*tail, since.as_deref()),
        // v0.7 阶段 8：`proc flows` —— eBPF flow graph CLI 视图（ADR-0016）。
        Command::Flows {
            limit,
            json,
            filter,
        } => flows::run_flows(limit, *json, filter.as_deref()),
        Command::Record { output } => record::run_record(output),
        Command::Replay { file, info } => record::run_replay(file, *info),
        Command::Export {
            format,
            output,
            sort,
            limit,
        } => export::run_export(format, output, sort, limit),
        Command::Diag { json } => diag::run_diag(*json),
        // v0.7.0 阶段 2：`proc mcp serve` 启动 stdio MCP server。
        // `sub=None` 时 fall back 到 `serve`，让 `proc mcp` 等价 `proc mcp serve`。
        Command::Mcp { sub } => match sub {
            Some(s) => mcp_cmd::run_mcp(s),
            None => mcp_cmd::run_mcp(&McpSub::Serve),
        },
        // v0.7.0 阶段 3：`proc completions --shell <SHELL>` 在线生成补全脚本。
        Command::Completions { shell } => completions::run_completions(*shell),
    }
}
