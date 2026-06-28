#compdef proc

autoload -U is-at-least

_proc() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
'-V[Print version]' \
'--version[Print version]' \
":: :_proc_commands" \
"*::: :->proc" \
&& ret=0
    case $state in
    (proc)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-command-$line[1]:"
        case $line[1] in
            (ls)
_arguments "${_arguments_options[@]}" : \
'--sort=[排序字段\: cpu, mem, name, pid, disk_read, disk_write, net_sent, net_recv]:SORT:_default' \
'--limit=[限制显示数量]:LIMIT:_default' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(tree)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(port)
_arguments "${_arguments_options[@]}" : \
'--port=[查询指定端口号]:PORT:_default' \
'--path=[工作路径]:PATH:_default' \
'--kill[终止占用端口的进程]' \
'--stats[输出 TCP 传输质量摘要（阶段 5 D2：重传 / RST / 失败连接计数）]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(kill)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'--force[强制终止（进程树）]' \
'-h[Print help]' \
'--help[Print help]' \
':pid -- 目标进程 PID:_default' \
&& ret=0
;;
(pkill)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'--force[强制终止（进程树）]' \
'--dry-run[仅显示匹配的进程，不终止]' \
'-h[Print help]' \
'--help[Print help]' \
':name -- 进程名（如 chrome.exe），精确匹配，大小写不敏感:_default' \
&& ret=0
;;
(eject)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'--find-locks[仅查看占用，不终止]' \
'-h[Print help]' \
'--help[Print help]' \
'::drive -- 驱动器号 (如 E\:):_default' \
&& ret=0
;;
(who)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':target_path -- 文件 / 目录路径（位置参数；不与全局 --path 冲突）:_files' \
&& ret=0
;;
(handles)
_arguments "${_arguments_options[@]}" : \
'--pid=[目标进程 PID（与 --file 互斥）]:PID:_default' \
'--file=[反查模式：列出占用此路径的所有 PID]:FILE:_files' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(priority)
_arguments "${_arguments_options[@]}" : \
'--set=[设置优先级（idle / belownormal / normal / abovenormal / high / realtime）]:SET:_default' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':pid -- 目标进程 PID:_default' \
&& ret=0
;;
(affinity)
_arguments "${_arguments_options[@]}" : \
'--set=[设置 affinity mask（16 进制，如 0xFF）]:SET:_default' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':pid -- 目标进程 PID:_default' \
&& ret=0
;;
(monitor)
_arguments "${_arguments_options[@]}" : \
'--remove=[删除监控 (按 ID)]:REMOVE:_default' \
'--port=[监控端口号]:PORT:_default' \
'--pid=[监控进程 PID]:PID:_default' \
'--command=[监控命令（带自动重启）]:COMMAND:_default' \
'--path=[工作路径]:PATH:_default' \
'--add[添加监控]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(docker)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
":: :_proc__subcmd__docker_commands" \
"*::: :->docker" \
&& ret=0

    case $state in
    (docker)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-docker-command-$line[1]:"
        case $line[1] in
            (ps)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':name -- 容器名 / 短 ID:_default' \
&& ret=0
;;
(top)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':name -- 容器名 / 短 ID:_default' \
&& ret=0
;;
(logs)
_arguments "${_arguments_options[@]}" : \
'--tail=[从末尾开始显示的行数（如 "100"、"all"）；默认 "all"]:TAIL:_default' \
'--path=[工作路径]:PATH:_default' \
'--follow[跟随模式（默认 false，输出后退出）]' \
'-h[Print help]' \
'--help[Print help]' \
':name -- 容器名 / 短 ID:_default' \
&& ret=0
;;
(images)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(volumes)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(image-rm)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'--force[强制删除（即便 in_use）]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- 镜像 ID / tag:_default' \
&& ret=0
;;
(volume-rm)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'--force[强制删除（即便 in_use）]' \
'-h[Print help]' \
'--help[Print help]' \
':name -- volume 名称:_default' \
&& ret=0
;;
(compose)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
'*::args -- 转发给 docker-compose 的参数（up / down / ps / -d / -f xxx 等）:_default' \
&& ret=0
;;
(events)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(exec)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':container -- 容器名 / 短 ID:_default' \
'*::cmd -- 命令 + 参数（如 `bash`、`/bin/sh -c "echo hi"`）；省略则根据 image 推断 shell:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_proc__subcmd__docker__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-docker-help-command-$line[1]:"
        case $line[1] in
            (ps)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(top)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(logs)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(images)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(volumes)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(image-rm)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(volume-rm)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(compose)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(events)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(exec)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(smart)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
'::device -- 指定设备(如 /dev/sda、PhysicalDrive0)。省略则列出所有磁盘。:_default' \
&& ret=0
;;
(dns)
_arguments "${_arguments_options[@]}" : \
'--since=[输出过去 N 时间的事件（如 "1h"、"30m"）；当前不持久化，本参数留 TODO]:SINCE:_default' \
'--path=[工作路径]:PATH:_default' \
'--tail[跟随模式：流式输出新事件，Ctrl+C 退出]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(record)
_arguments "${_arguments_options[@]}" : \
'-o+[输出文件路径（默认\: ~/.config/proc/recordings/recording_{timestamp}.prec）]:OUTPUT:_files' \
'--output=[输出文件路径（默认\: ~/.config/proc/recordings/recording_{timestamp}.prec）]:OUTPUT:_files' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(replay)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
':file -- 录制文件路径:_files' \
&& ret=0
;;
(export)
_arguments "${_arguments_options[@]}" : \
'--format=[输出格式：json | csv]:FORMAT:_default' \
'-o+[输出文件路径（不指定则输出到 stdout）]:OUTPUT:_files' \
'--output=[输出文件路径（不指定则输出到 stdout）]:OUTPUT:_files' \
'--sort=[排序字段：cpu | mem | name | pid | disk_read | disk_write | net_sent | net_recv]:SORT:_default' \
'--limit=[限制导出数量]:LIMIT:_default' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(diag)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'--json[输出 JSON（默认 human-readable 表格）]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(mcp)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
":: :_proc__subcmd__mcp_commands" \
"*::: :->mcp" \
&& ret=0

    case $state in
    (mcp)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-mcp-command-$line[1]:"
        case $line[1] in
            (serve)
_arguments "${_arguments_options[@]}" : \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_proc__subcmd__mcp__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-mcp-help-command-$line[1]:"
        case $line[1] in
            (serve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(completions)
_arguments "${_arguments_options[@]}" : \
'-s+[Target shell]:SHELL:(bash elvish fish powershell zsh)' \
'--shell=[Target shell]:SHELL:(bash elvish fish powershell zsh)' \
'--path=[工作路径]:PATH:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_proc__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-help-command-$line[1]:"
        case $line[1] in
            (ls)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(tree)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(port)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(kill)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(pkill)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(eject)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(who)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(handles)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(priority)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(affinity)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(monitor)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(docker)
_arguments "${_arguments_options[@]}" : \
":: :_proc__subcmd__help__subcmd__docker_commands" \
"*::: :->docker" \
&& ret=0

    case $state in
    (docker)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-help-docker-command-$line[1]:"
        case $line[1] in
            (ps)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(top)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(logs)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(images)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(volumes)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(image-rm)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(volume-rm)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(compose)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(events)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(exec)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(smart)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(dns)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(record)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(replay)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(export)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diag)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(mcp)
_arguments "${_arguments_options[@]}" : \
":: :_proc__subcmd__help__subcmd__mcp_commands" \
"*::: :->mcp" \
&& ret=0

    case $state in
    (mcp)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:proc-help-mcp-command-$line[1]:"
        case $line[1] in
            (serve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(completions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
}

(( $+functions[_proc_commands] )) ||
_proc_commands() {
    local commands; commands=(
'ls:列出进程' \
'tree:进程树' \
'port:端口映射' \
'kill:终止进程' \
'pkill:按名称终止进程' \
'eject:U盘助手' \
'who:反查「谁占用这个文件 / 目录」（阶段 4 A1）' \
'handles:枚举指定进程的所有句柄（阶段 4 A1）' \
'priority:查询 / 设置进程优先级（阶段 4 A4）' \
'affinity:查询 / 设置进程 CPU affinity（阶段 4 A4）' \
'monitor:进程监控' \
'docker:Docker 监控' \
'smart:SMART 磁盘健康(阶段 5 B3)' \
'dns:DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。' \
'record:录制系统快照' \
'replay:回放录制文件' \
'export:导出当前进程快照' \
'diag:v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics （avg/max/polls/drops），用户报 bug 时附上。' \
'mcp:MCP server mode (stdio transport)' \
'completions:Generate shell completions' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'proc commands' commands "$@"
}
(( $+functions[_proc__subcmd__affinity_commands] )) ||
_proc__subcmd__affinity_commands() {
    local commands; commands=()
    _describe -t commands 'proc affinity commands' commands "$@"
}
(( $+functions[_proc__subcmd__completions_commands] )) ||
_proc__subcmd__completions_commands() {
    local commands; commands=()
    _describe -t commands 'proc completions commands' commands "$@"
}
(( $+functions[_proc__subcmd__diag_commands] )) ||
_proc__subcmd__diag_commands() {
    local commands; commands=()
    _describe -t commands 'proc diag commands' commands "$@"
}
(( $+functions[_proc__subcmd__dns_commands] )) ||
_proc__subcmd__dns_commands() {
    local commands; commands=()
    _describe -t commands 'proc dns commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker_commands] )) ||
_proc__subcmd__docker_commands() {
    local commands; commands=(
'ps:列出所有容器（默认）' \
'inspect:查看指定容器详情' \
'top:容器内进程列表（docker top）' \
'logs:容器日志（跟随或一次性）' \
'images:列出本地镜像' \
'volumes:列出 volume' \
'image-rm:删除镜像' \
'volume-rm:删除 volume' \
'compose:docker-compose 薄封装（需宿主机装 docker-compose）' \
'events:监听容器事件流（Ctrl+C 停止）' \
'exec:exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 \`docker exec -it\`。 TUI 内按 \`e\` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'proc docker commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__compose_commands] )) ||
_proc__subcmd__docker__subcmd__compose_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker compose commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__events_commands] )) ||
_proc__subcmd__docker__subcmd__events_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker events commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__exec_commands] )) ||
_proc__subcmd__docker__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker exec commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help_commands] )) ||
_proc__subcmd__docker__subcmd__help_commands() {
    local commands; commands=(
'ps:列出所有容器（默认）' \
'inspect:查看指定容器详情' \
'top:容器内进程列表（docker top）' \
'logs:容器日志（跟随或一次性）' \
'images:列出本地镜像' \
'volumes:列出 volume' \
'image-rm:删除镜像' \
'volume-rm:删除 volume' \
'compose:docker-compose 薄封装（需宿主机装 docker-compose）' \
'events:监听容器事件流（Ctrl+C 停止）' \
'exec:exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 \`docker exec -it\`。 TUI 内按 \`e\` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'proc docker help commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__compose_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__compose_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help compose commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__events_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__events_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help events commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__exec_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help exec commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__help_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help help commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__image-rm_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__image-rm_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help image-rm commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__images_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__images_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help images commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__inspect_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help inspect commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__logs_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__logs_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help logs commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__ps_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__ps_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help ps commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__top_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__top_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help top commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__volume-rm_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__volume-rm_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help volume-rm commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__help__subcmd__volumes_commands] )) ||
_proc__subcmd__docker__subcmd__help__subcmd__volumes_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker help volumes commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__image-rm_commands] )) ||
_proc__subcmd__docker__subcmd__image-rm_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker image-rm commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__images_commands] )) ||
_proc__subcmd__docker__subcmd__images_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker images commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__inspect_commands] )) ||
_proc__subcmd__docker__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker inspect commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__logs_commands] )) ||
_proc__subcmd__docker__subcmd__logs_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker logs commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__ps_commands] )) ||
_proc__subcmd__docker__subcmd__ps_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker ps commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__top_commands] )) ||
_proc__subcmd__docker__subcmd__top_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker top commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__volume-rm_commands] )) ||
_proc__subcmd__docker__subcmd__volume-rm_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker volume-rm commands' commands "$@"
}
(( $+functions[_proc__subcmd__docker__subcmd__volumes_commands] )) ||
_proc__subcmd__docker__subcmd__volumes_commands() {
    local commands; commands=()
    _describe -t commands 'proc docker volumes commands' commands "$@"
}
(( $+functions[_proc__subcmd__eject_commands] )) ||
_proc__subcmd__eject_commands() {
    local commands; commands=()
    _describe -t commands 'proc eject commands' commands "$@"
}
(( $+functions[_proc__subcmd__export_commands] )) ||
_proc__subcmd__export_commands() {
    local commands; commands=()
    _describe -t commands 'proc export commands' commands "$@"
}
(( $+functions[_proc__subcmd__handles_commands] )) ||
_proc__subcmd__handles_commands() {
    local commands; commands=()
    _describe -t commands 'proc handles commands' commands "$@"
}
(( $+functions[_proc__subcmd__help_commands] )) ||
_proc__subcmd__help_commands() {
    local commands; commands=(
'ls:列出进程' \
'tree:进程树' \
'port:端口映射' \
'kill:终止进程' \
'pkill:按名称终止进程' \
'eject:U盘助手' \
'who:反查「谁占用这个文件 / 目录」（阶段 4 A1）' \
'handles:枚举指定进程的所有句柄（阶段 4 A1）' \
'priority:查询 / 设置进程优先级（阶段 4 A4）' \
'affinity:查询 / 设置进程 CPU affinity（阶段 4 A4）' \
'monitor:进程监控' \
'docker:Docker 监控' \
'smart:SMART 磁盘健康(阶段 5 B3)' \
'dns:DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。' \
'record:录制系统快照' \
'replay:回放录制文件' \
'export:导出当前进程快照' \
'diag:v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics （avg/max/polls/drops），用户报 bug 时附上。' \
'mcp:MCP server mode (stdio transport)' \
'completions:Generate shell completions' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'proc help commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__affinity_commands] )) ||
_proc__subcmd__help__subcmd__affinity_commands() {
    local commands; commands=()
    _describe -t commands 'proc help affinity commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__completions_commands] )) ||
_proc__subcmd__help__subcmd__completions_commands() {
    local commands; commands=()
    _describe -t commands 'proc help completions commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__diag_commands] )) ||
_proc__subcmd__help__subcmd__diag_commands() {
    local commands; commands=()
    _describe -t commands 'proc help diag commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__dns_commands] )) ||
_proc__subcmd__help__subcmd__dns_commands() {
    local commands; commands=()
    _describe -t commands 'proc help dns commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker_commands] )) ||
_proc__subcmd__help__subcmd__docker_commands() {
    local commands; commands=(
'ps:列出所有容器（默认）' \
'inspect:查看指定容器详情' \
'top:容器内进程列表（docker top）' \
'logs:容器日志（跟随或一次性）' \
'images:列出本地镜像' \
'volumes:列出 volume' \
'image-rm:删除镜像' \
'volume-rm:删除 volume' \
'compose:docker-compose 薄封装（需宿主机装 docker-compose）' \
'events:监听容器事件流（Ctrl+C 停止）' \
'exec:exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 \`docker exec -it\`。 TUI 内按 \`e\` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。' \
    )
    _describe -t commands 'proc help docker commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__compose_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__compose_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker compose commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__events_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__events_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker events commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__exec_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker exec commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__image-rm_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__image-rm_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker image-rm commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__images_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__images_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker images commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__inspect_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker inspect commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__logs_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__logs_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker logs commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__ps_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__ps_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker ps commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__top_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__top_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker top commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__volume-rm_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__volume-rm_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker volume-rm commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__docker__subcmd__volumes_commands] )) ||
_proc__subcmd__help__subcmd__docker__subcmd__volumes_commands() {
    local commands; commands=()
    _describe -t commands 'proc help docker volumes commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__eject_commands] )) ||
_proc__subcmd__help__subcmd__eject_commands() {
    local commands; commands=()
    _describe -t commands 'proc help eject commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__export_commands] )) ||
_proc__subcmd__help__subcmd__export_commands() {
    local commands; commands=()
    _describe -t commands 'proc help export commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__handles_commands] )) ||
_proc__subcmd__help__subcmd__handles_commands() {
    local commands; commands=()
    _describe -t commands 'proc help handles commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__help_commands] )) ||
_proc__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'proc help help commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__kill_commands] )) ||
_proc__subcmd__help__subcmd__kill_commands() {
    local commands; commands=()
    _describe -t commands 'proc help kill commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__ls_commands] )) ||
_proc__subcmd__help__subcmd__ls_commands() {
    local commands; commands=()
    _describe -t commands 'proc help ls commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__mcp_commands] )) ||
_proc__subcmd__help__subcmd__mcp_commands() {
    local commands; commands=(
'serve:启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。' \
    )
    _describe -t commands 'proc help mcp commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__mcp__subcmd__serve_commands] )) ||
_proc__subcmd__help__subcmd__mcp__subcmd__serve_commands() {
    local commands; commands=()
    _describe -t commands 'proc help mcp serve commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__monitor_commands] )) ||
_proc__subcmd__help__subcmd__monitor_commands() {
    local commands; commands=()
    _describe -t commands 'proc help monitor commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__pkill_commands] )) ||
_proc__subcmd__help__subcmd__pkill_commands() {
    local commands; commands=()
    _describe -t commands 'proc help pkill commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__port_commands] )) ||
_proc__subcmd__help__subcmd__port_commands() {
    local commands; commands=()
    _describe -t commands 'proc help port commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__priority_commands] )) ||
_proc__subcmd__help__subcmd__priority_commands() {
    local commands; commands=()
    _describe -t commands 'proc help priority commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__record_commands] )) ||
_proc__subcmd__help__subcmd__record_commands() {
    local commands; commands=()
    _describe -t commands 'proc help record commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__replay_commands] )) ||
_proc__subcmd__help__subcmd__replay_commands() {
    local commands; commands=()
    _describe -t commands 'proc help replay commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__smart_commands] )) ||
_proc__subcmd__help__subcmd__smart_commands() {
    local commands; commands=()
    _describe -t commands 'proc help smart commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__tree_commands] )) ||
_proc__subcmd__help__subcmd__tree_commands() {
    local commands; commands=()
    _describe -t commands 'proc help tree commands' commands "$@"
}
(( $+functions[_proc__subcmd__help__subcmd__who_commands] )) ||
_proc__subcmd__help__subcmd__who_commands() {
    local commands; commands=()
    _describe -t commands 'proc help who commands' commands "$@"
}
(( $+functions[_proc__subcmd__kill_commands] )) ||
_proc__subcmd__kill_commands() {
    local commands; commands=()
    _describe -t commands 'proc kill commands' commands "$@"
}
(( $+functions[_proc__subcmd__ls_commands] )) ||
_proc__subcmd__ls_commands() {
    local commands; commands=()
    _describe -t commands 'proc ls commands' commands "$@"
}
(( $+functions[_proc__subcmd__mcp_commands] )) ||
_proc__subcmd__mcp_commands() {
    local commands; commands=(
'serve:启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'proc mcp commands' commands "$@"
}
(( $+functions[_proc__subcmd__mcp__subcmd__help_commands] )) ||
_proc__subcmd__mcp__subcmd__help_commands() {
    local commands; commands=(
'serve:启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'proc mcp help commands' commands "$@"
}
(( $+functions[_proc__subcmd__mcp__subcmd__help__subcmd__help_commands] )) ||
_proc__subcmd__mcp__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'proc mcp help help commands' commands "$@"
}
(( $+functions[_proc__subcmd__mcp__subcmd__help__subcmd__serve_commands] )) ||
_proc__subcmd__mcp__subcmd__help__subcmd__serve_commands() {
    local commands; commands=()
    _describe -t commands 'proc mcp help serve commands' commands "$@"
}
(( $+functions[_proc__subcmd__mcp__subcmd__serve_commands] )) ||
_proc__subcmd__mcp__subcmd__serve_commands() {
    local commands; commands=()
    _describe -t commands 'proc mcp serve commands' commands "$@"
}
(( $+functions[_proc__subcmd__monitor_commands] )) ||
_proc__subcmd__monitor_commands() {
    local commands; commands=()
    _describe -t commands 'proc monitor commands' commands "$@"
}
(( $+functions[_proc__subcmd__pkill_commands] )) ||
_proc__subcmd__pkill_commands() {
    local commands; commands=()
    _describe -t commands 'proc pkill commands' commands "$@"
}
(( $+functions[_proc__subcmd__port_commands] )) ||
_proc__subcmd__port_commands() {
    local commands; commands=()
    _describe -t commands 'proc port commands' commands "$@"
}
(( $+functions[_proc__subcmd__priority_commands] )) ||
_proc__subcmd__priority_commands() {
    local commands; commands=()
    _describe -t commands 'proc priority commands' commands "$@"
}
(( $+functions[_proc__subcmd__record_commands] )) ||
_proc__subcmd__record_commands() {
    local commands; commands=()
    _describe -t commands 'proc record commands' commands "$@"
}
(( $+functions[_proc__subcmd__replay_commands] )) ||
_proc__subcmd__replay_commands() {
    local commands; commands=()
    _describe -t commands 'proc replay commands' commands "$@"
}
(( $+functions[_proc__subcmd__smart_commands] )) ||
_proc__subcmd__smart_commands() {
    local commands; commands=()
    _describe -t commands 'proc smart commands' commands "$@"
}
(( $+functions[_proc__subcmd__tree_commands] )) ||
_proc__subcmd__tree_commands() {
    local commands; commands=()
    _describe -t commands 'proc tree commands' commands "$@"
}
(( $+functions[_proc__subcmd__who_commands] )) ||
_proc__subcmd__who_commands() {
    local commands; commands=()
    _describe -t commands 'proc who commands' commands "$@"
}

if [ "$funcstack[1]" = "_proc" ]; then
    _proc "$@"
else
    compdef _proc proc
fi
