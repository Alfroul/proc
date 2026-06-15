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
        /// 排序字段: cpu, mem, name, pid
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
        /// 监听容器事件
        #[arg(long)]
        watch: bool,

        /// 查看指定容器详情
        #[arg(long)]
        container: Option<String>,
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

        /// 排序字段：cpu | mem | name | pid
        #[arg(long, default_value = "cpu")]
        sort: String,

        /// 限制导出数量
        #[arg(long)]
        limit: Option<usize>,
    },
}
