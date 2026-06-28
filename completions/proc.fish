# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_proc_global_optspecs
	string join \n path= h/help V/version
end

function __fish_proc_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_proc_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_proc_using_subcommand
	set -l cmd (__fish_proc_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c proc -n "__fish_proc_needs_command" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_needs_command" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_needs_command" -s V -l version -d 'Print version'
complete -c proc -n "__fish_proc_needs_command" -f -a "ls" -d '列出进程'
complete -c proc -n "__fish_proc_needs_command" -f -a "tree" -d '进程树'
complete -c proc -n "__fish_proc_needs_command" -f -a "port" -d '端口映射'
complete -c proc -n "__fish_proc_needs_command" -f -a "kill" -d '终止进程'
complete -c proc -n "__fish_proc_needs_command" -f -a "pkill" -d '按名称终止进程'
complete -c proc -n "__fish_proc_needs_command" -f -a "eject" -d 'U盘助手'
complete -c proc -n "__fish_proc_needs_command" -f -a "who" -d '反查「谁占用这个文件 / 目录」（阶段 4 A1）'
complete -c proc -n "__fish_proc_needs_command" -f -a "handles" -d '枚举指定进程的所有句柄（阶段 4 A1）'
complete -c proc -n "__fish_proc_needs_command" -f -a "priority" -d '查询 / 设置进程优先级（阶段 4 A4）'
complete -c proc -n "__fish_proc_needs_command" -f -a "affinity" -d '查询 / 设置进程 CPU affinity（阶段 4 A4）'
complete -c proc -n "__fish_proc_needs_command" -f -a "monitor" -d '进程监控'
complete -c proc -n "__fish_proc_needs_command" -f -a "docker" -d 'Docker 监控'
complete -c proc -n "__fish_proc_needs_command" -f -a "smart" -d 'SMART 磁盘健康(阶段 5 B3)'
complete -c proc -n "__fish_proc_needs_command" -f -a "dns" -d 'DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。'
complete -c proc -n "__fish_proc_needs_command" -f -a "record" -d '录制系统快照'
complete -c proc -n "__fish_proc_needs_command" -f -a "replay" -d '回放录制文件'
complete -c proc -n "__fish_proc_needs_command" -f -a "export" -d '导出当前进程快照'
complete -c proc -n "__fish_proc_needs_command" -f -a "diag" -d 'v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics （avg/max/polls/drops），用户报 bug 时附上。'
complete -c proc -n "__fish_proc_needs_command" -f -a "mcp" -d 'MCP server mode (stdio transport)'
complete -c proc -n "__fish_proc_needs_command" -f -a "completions" -d 'Generate shell completions'
complete -c proc -n "__fish_proc_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c proc -n "__fish_proc_using_subcommand ls" -l sort -d '排序字段: cpu, mem, name, pid, disk_read, disk_write, net_sent, net_recv' -r
complete -c proc -n "__fish_proc_using_subcommand ls" -l limit -d '限制显示数量' -r
complete -c proc -n "__fish_proc_using_subcommand ls" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand ls" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand tree" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand tree" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand port" -l port -d '查询指定端口号' -r
complete -c proc -n "__fish_proc_using_subcommand port" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand port" -l kill -d '终止占用端口的进程'
complete -c proc -n "__fish_proc_using_subcommand port" -l stats -d '输出 TCP 传输质量摘要（阶段 5 D2：重传 / RST / 失败连接计数）'
complete -c proc -n "__fish_proc_using_subcommand port" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand kill" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand kill" -l force -d '强制终止（进程树）'
complete -c proc -n "__fish_proc_using_subcommand kill" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand pkill" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand pkill" -l force -d '强制终止（进程树）'
complete -c proc -n "__fish_proc_using_subcommand pkill" -l dry-run -d '仅显示匹配的进程，不终止'
complete -c proc -n "__fish_proc_using_subcommand pkill" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand eject" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand eject" -l find-locks -d '仅查看占用，不终止'
complete -c proc -n "__fish_proc_using_subcommand eject" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand who" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand who" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand handles" -l pid -d '目标进程 PID（与 --file 互斥）' -r
complete -c proc -n "__fish_proc_using_subcommand handles" -l file -d '反查模式：列出占用此路径的所有 PID' -r -F
complete -c proc -n "__fish_proc_using_subcommand handles" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand handles" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand priority" -l set -d '设置优先级（idle / belownormal / normal / abovenormal / high / realtime）' -r
complete -c proc -n "__fish_proc_using_subcommand priority" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand priority" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand affinity" -l set -d '设置 affinity mask（16 进制，如 0xFF）' -r
complete -c proc -n "__fish_proc_using_subcommand affinity" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand affinity" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand monitor" -l remove -d '删除监控 (按 ID)' -r
complete -c proc -n "__fish_proc_using_subcommand monitor" -l port -d '监控端口号' -r
complete -c proc -n "__fish_proc_using_subcommand monitor" -l pid -d '监控进程 PID' -r
complete -c proc -n "__fish_proc_using_subcommand monitor" -l command -d '监控命令（带自动重启）' -r
complete -c proc -n "__fish_proc_using_subcommand monitor" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand monitor" -l add -d '添加监控'
complete -c proc -n "__fish_proc_using_subcommand monitor" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "ps" -d '列出所有容器（默认）'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "inspect" -d '查看指定容器详情'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "top" -d '容器内进程列表（docker top）'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "logs" -d '容器日志（跟随或一次性）'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "images" -d '列出本地镜像'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "volumes" -d '列出 volume'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "image-rm" -d '删除镜像'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "volume-rm" -d '删除 volume'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "compose" -d 'docker-compose 薄封装（需宿主机装 docker-compose）'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "events" -d '监听容器事件流（Ctrl+C 停止）'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "exec" -d 'exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。 TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。'
complete -c proc -n "__fish_proc_using_subcommand docker; and not __fish_seen_subcommand_from ps inspect top logs images volumes image-rm volume-rm compose events exec help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from ps" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from ps" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from inspect" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from inspect" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from top" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from top" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from logs" -l tail -d '从末尾开始显示的行数（如 "100"、"all"）；默认 "all"' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from logs" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from logs" -l follow -d '跟随模式（默认 false，输出后退出）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from logs" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from images" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from images" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from volumes" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from volumes" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from image-rm" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from image-rm" -l force -d '强制删除（即便 in_use）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from image-rm" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from volume-rm" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from volume-rm" -l force -d '强制删除（即便 in_use）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from volume-rm" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from compose" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from compose" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from events" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from events" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from exec" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from exec" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "ps" -d '列出所有容器（默认）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "inspect" -d '查看指定容器详情'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "top" -d '容器内进程列表（docker top）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "logs" -d '容器日志（跟随或一次性）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "images" -d '列出本地镜像'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "volumes" -d '列出 volume'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "image-rm" -d '删除镜像'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "volume-rm" -d '删除 volume'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "compose" -d 'docker-compose 薄封装（需宿主机装 docker-compose）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "events" -d '监听容器事件流（Ctrl+C 停止）'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "exec" -d 'exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。 TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。'
complete -c proc -n "__fish_proc_using_subcommand docker; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c proc -n "__fish_proc_using_subcommand smart" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand smart" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand dns" -l since -d '输出过去 N 时间的事件（如 "1h"、"30m"）；当前不持久化，本参数留 TODO' -r
complete -c proc -n "__fish_proc_using_subcommand dns" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand dns" -l tail -d '跟随模式：流式输出新事件，Ctrl+C 退出'
complete -c proc -n "__fish_proc_using_subcommand dns" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand record" -s o -l output -d '输出文件路径（默认: ~/.config/proc/recordings/recording_{timestamp}.prec）' -r -F
complete -c proc -n "__fish_proc_using_subcommand record" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand record" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand replay" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand replay" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand export" -l format -d '输出格式：json | csv' -r
complete -c proc -n "__fish_proc_using_subcommand export" -s o -l output -d '输出文件路径（不指定则输出到 stdout）' -r -F
complete -c proc -n "__fish_proc_using_subcommand export" -l sort -d '排序字段：cpu | mem | name | pid | disk_read | disk_write | net_sent | net_recv' -r
complete -c proc -n "__fish_proc_using_subcommand export" -l limit -d '限制导出数量' -r
complete -c proc -n "__fish_proc_using_subcommand export" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand export" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand diag" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand diag" -l json -d '输出 JSON（默认 human-readable 表格）'
complete -c proc -n "__fish_proc_using_subcommand diag" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand mcp; and not __fish_seen_subcommand_from serve help" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand mcp; and not __fish_seen_subcommand_from serve help" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand mcp; and not __fish_seen_subcommand_from serve help" -f -a "serve" -d '启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。'
complete -c proc -n "__fish_proc_using_subcommand mcp; and not __fish_seen_subcommand_from serve help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c proc -n "__fish_proc_using_subcommand mcp; and __fish_seen_subcommand_from serve" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand mcp; and __fish_seen_subcommand_from serve" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "serve" -d '启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。'
complete -c proc -n "__fish_proc_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c proc -n "__fish_proc_using_subcommand completions" -s s -l shell -d 'Target shell' -r -f -a "bash\t''
elvish\t''
fish\t''
powershell\t''
zsh\t''"
complete -c proc -n "__fish_proc_using_subcommand completions" -l path -d '工作路径' -r
complete -c proc -n "__fish_proc_using_subcommand completions" -s h -l help -d 'Print help'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "ls" -d '列出进程'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "tree" -d '进程树'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "port" -d '端口映射'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "kill" -d '终止进程'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "pkill" -d '按名称终止进程'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "eject" -d 'U盘助手'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "who" -d '反查「谁占用这个文件 / 目录」（阶段 4 A1）'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "handles" -d '枚举指定进程的所有句柄（阶段 4 A1）'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "priority" -d '查询 / 设置进程优先级（阶段 4 A4）'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "affinity" -d '查询 / 设置进程 CPU affinity（阶段 4 A4）'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "monitor" -d '进程监控'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "docker" -d 'Docker 监控'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "smart" -d 'SMART 磁盘健康(阶段 5 B3)'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "dns" -d 'DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "record" -d '录制系统快照'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "replay" -d '回放录制文件'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "export" -d '导出当前进程快照'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "diag" -d 'v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics （avg/max/polls/drops），用户报 bug 时附上。'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "mcp" -d 'MCP server mode (stdio transport)'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "completions" -d 'Generate shell completions'
complete -c proc -n "__fish_proc_using_subcommand help; and not __fish_seen_subcommand_from ls tree port kill pkill eject who handles priority affinity monitor docker smart dns record replay export diag mcp completions help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "ps" -d '列出所有容器（默认）'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "inspect" -d '查看指定容器详情'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "top" -d '容器内进程列表（docker top）'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "logs" -d '容器日志（跟随或一次性）'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "images" -d '列出本地镜像'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "volumes" -d '列出 volume'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "image-rm" -d '删除镜像'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "volume-rm" -d '删除 volume'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "compose" -d 'docker-compose 薄封装（需宿主机装 docker-compose）'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "events" -d '监听容器事件流（Ctrl+C 停止）'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from docker" -f -a "exec" -d 'exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。 TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。'
complete -c proc -n "__fish_proc_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "serve" -d '启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。'
