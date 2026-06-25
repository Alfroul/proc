use clap::{Parser, Subcommand};

/// proc — 交互式系统进程管理器
#[derive(Parser, Debug)]
#[command(name = "proc", version, about = "Rust 编写的交互式系统进程管理器")]
pub struct Cli {
    /// 工作路径
    #[arg(long, global = true)]
    pub path: Option<String>,

    /// 子命令
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 列出进程
    Ls {
        /// 排序字段: cpu, mem, name, pid, disk_read, disk_write, net_sent, net_recv
        #[arg(long, default_value = "cpu")]
        sort: String,

        /// 限制显示数量
        #[arg(long)]
        limit: Option<usize>,
    },

    /// 进程树
    Tree,

    /// 端口映射
    Port {
        /// 查询指定端口号
        #[arg(long)]
        port: Option<u16>,

        /// 终止占用端口的进程
        #[arg(long)]
        kill: bool,

        /// 输出 TCP 传输质量摘要（阶段 5 D2：重传 / RST / 失败连接计数）
        #[arg(long)]
        stats: bool,
    },

    /// 终止进程
    Kill {
        /// 目标进程 PID
        pid: u32,

        /// 强制终止（进程树）
        #[arg(long)]
        force: bool,
    },

    /// 按名称终止进程
    Pkill {
        /// 进程名（如 chrome.exe），精确匹配，大小写不敏感
        name: String,

        /// 强制终止（进程树）
        #[arg(long)]
        force: bool,

        /// 仅显示匹配的进程，不终止
        #[arg(long)]
        dry_run: bool,
    },

    /// U盘助手
    Eject {
        /// 驱动器号 (如 E:)
        drive: Option<String>,

        /// 仅查看占用，不终止
        #[arg(long)]
        find_locks: bool,
    },

    /// 反查「谁占用这个文件 / 目录」（阶段 4 A1）
    Who {
        /// 文件 / 目录路径（位置参数；不与全局 --path 冲突）
        target_path: std::path::PathBuf,
    },

    /// 枚举指定进程的所有句柄（阶段 4 A1）
    Handles {
        /// 目标进程 PID（与 --file 互斥）
        #[arg(long)]
        pid: Option<u32>,

        /// 反查模式：列出占用此路径的所有 PID
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },

    /// 查询 / 设置进程优先级（阶段 4 A4）
    Priority {
        /// 目标进程 PID
        pid: u32,

        /// 设置优先级（idle / belownormal / normal / abovenormal / high / realtime）
        #[arg(long)]
        set: Option<String>,
    },

    /// 查询 / 设置进程 CPU affinity（阶段 4 A4）
    Affinity {
        /// 目标进程 PID
        pid: u32,

        /// 设置 affinity mask（16 进制，如 0xFF）
        #[arg(long)]
        set: Option<String>,
    },

    /// 进程监控
    Monitor {
        /// 添加监控
        #[arg(long)]
        add: bool,

        /// 删除监控 (按 ID)
        #[arg(long)]
        remove: Option<u32>,

        /// 监控端口号
        #[arg(long)]
        port: Option<u16>,

        /// 监控进程 PID
        #[arg(long)]
        pid: Option<u32>,

        /// 监控命令（带自动重启）
        #[arg(long)]
        command: Option<String>,
    },

    /// Docker 监控
    Docker {
        #[command(subcommand)]
        sub: DockerSub,
    },

    /// SMART 磁盘健康(阶段 5 B3)
    Smart {
        /// 指定设备(如 /dev/sda、PhysicalDrive0)。省略则列出所有磁盘。
        device: Option<String>,
    },

    /// DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。
    Dns {
        /// 跟随模式：流式输出新事件，Ctrl+C 退出
        #[arg(long)]
        tail: bool,

        /// 输出过去 N 时间的事件（如 "1h"、"30m"）；当前不持久化，本参数留 TODO
        #[arg(long)]
        since: Option<String>,
    },

    /// 录制系统快照
    Record {
        /// 输出文件路径（默认: ~/.config/proc/recordings/recording_{timestamp}.prec）
        #[arg(short = 'o', long = "output")]
        output: Option<std::path::PathBuf>,
    },

    /// 回放录制文件
    Replay {
        /// 录制文件路径
        file: std::path::PathBuf,
    },

    /// 导出当前进程快照
    Export {
        /// 输出格式：json | csv
        #[arg(long, default_value = "json")]
        format: String,

        /// 输出文件路径（不指定则输出到 stdout）
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,

        /// 排序字段：cpu | mem | name | pid | disk_read | disk_write | net_sent | net_recv
        #[arg(long, default_value = "cpu")]
        sort: String,

        /// 限制导出数量
        #[arg(long)]
        limit: Option<usize>,
    },

    /// v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics
    /// （avg/max/polls/drops），用户报 bug 时附上。
    Diag {
        /// 输出 JSON（默认 human-readable 表格）
        #[arg(long)]
        json: bool,
    },
}

/// Docker 子命令（E3/E4/E1）。
#[derive(Subcommand, Debug)]
pub enum DockerSub {
    /// 列出所有容器（默认）
    Ps,

    /// 查看指定容器详情
    Inspect {
        /// 容器名 / 短 ID
        name: String,
    },

    /// 容器内进程列表（docker top）
    Top {
        /// 容器名 / 短 ID
        name: String,
    },

    /// 容器日志（跟随或一次性）
    Logs {
        /// 容器名 / 短 ID
        name: String,

        /// 跟随模式（默认 false，输出后退出）
        #[arg(long)]
        follow: bool,

        /// 从末尾开始显示的行数（如 "100"、"all"）；默认 "all"
        #[arg(long)]
        tail: Option<String>,
    },

    /// 列出本地镜像
    Images,

    /// 列出 volume
    Volumes,

    /// 删除镜像
    #[command(name = "image-rm")]
    ImageRm {
        /// 镜像 ID / tag
        id: String,

        /// 强制删除（即便 in_use）
        #[arg(long)]
        force: bool,
    },

    /// 删除 volume
    #[command(name = "volume-rm")]
    VolumeRm {
        /// volume 名称
        name: String,

        /// 强制删除（即便 in_use）
        #[arg(long)]
        force: bool,
    },

    /// docker-compose 薄封装（需宿主机装 docker-compose）
    Compose {
        /// 转发给 docker-compose 的参数（up / down / ps / -d / -f xxx 等）
        #[arg(num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// 监听容器事件流（Ctrl+C 停止）
    Events,

    /// exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。
    /// TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。
    Exec {
        /// 容器名 / 短 ID
        container: String,

        /// 命令 + 参数（如 `bash`、`/bin/sh -c "echo hi"`）；省略则根据 image 推断 shell
        #[arg(num_args = 0.., trailing_var_arg = true)]
        cmd: Vec<String>,
    },
}
