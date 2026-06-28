
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'proc' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'proc'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'proc' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('ls', 'ls', [CompletionResultType]::ParameterValue, '列出进程')
            [CompletionResult]::new('tree', 'tree', [CompletionResultType]::ParameterValue, '进程树')
            [CompletionResult]::new('port', 'port', [CompletionResultType]::ParameterValue, '端口映射')
            [CompletionResult]::new('kill', 'kill', [CompletionResultType]::ParameterValue, '终止进程')
            [CompletionResult]::new('pkill', 'pkill', [CompletionResultType]::ParameterValue, '按名称终止进程')
            [CompletionResult]::new('eject', 'eject', [CompletionResultType]::ParameterValue, 'U盘助手')
            [CompletionResult]::new('who', 'who', [CompletionResultType]::ParameterValue, '反查「谁占用这个文件 / 目录」（阶段 4 A1）')
            [CompletionResult]::new('handles', 'handles', [CompletionResultType]::ParameterValue, '枚举指定进程的所有句柄（阶段 4 A1）')
            [CompletionResult]::new('priority', 'priority', [CompletionResultType]::ParameterValue, '查询 / 设置进程优先级（阶段 4 A4）')
            [CompletionResult]::new('affinity', 'affinity', [CompletionResultType]::ParameterValue, '查询 / 设置进程 CPU affinity（阶段 4 A4）')
            [CompletionResult]::new('monitor', 'monitor', [CompletionResultType]::ParameterValue, '进程监控')
            [CompletionResult]::new('docker', 'docker', [CompletionResultType]::ParameterValue, 'Docker 监控')
            [CompletionResult]::new('smart', 'smart', [CompletionResultType]::ParameterValue, 'SMART 磁盘健康(阶段 5 B3)')
            [CompletionResult]::new('dns', 'dns', [CompletionResultType]::ParameterValue, 'DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。')
            [CompletionResult]::new('record', 'record', [CompletionResultType]::ParameterValue, '录制系统快照')
            [CompletionResult]::new('replay', 'replay', [CompletionResultType]::ParameterValue, '回放录制文件')
            [CompletionResult]::new('export', 'export', [CompletionResultType]::ParameterValue, '导出当前进程快照')
            [CompletionResult]::new('diag', 'diag', [CompletionResultType]::ParameterValue, 'v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics （avg/max/polls/drops），用户报 bug 时附上。')
            [CompletionResult]::new('mcp', 'mcp', [CompletionResultType]::ParameterValue, 'MCP server mode (stdio transport)')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Generate shell completions')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'proc;ls' {
            [CompletionResult]::new('--sort', '--sort', [CompletionResultType]::ParameterName, '排序字段: cpu, mem, name, pid, disk_read, disk_write, net_sent, net_recv')
            [CompletionResult]::new('--limit', '--limit', [CompletionResultType]::ParameterName, '限制显示数量')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;tree' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;port' {
            [CompletionResult]::new('--port', '--port', [CompletionResultType]::ParameterName, '查询指定端口号')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--kill', '--kill', [CompletionResultType]::ParameterName, '终止占用端口的进程')
            [CompletionResult]::new('--stats', '--stats', [CompletionResultType]::ParameterName, '输出 TCP 传输质量摘要（阶段 5 D2：重传 / RST / 失败连接计数）')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;kill' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, '强制终止（进程树）')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;pkill' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, '强制终止（进程树）')
            [CompletionResult]::new('--dry-run', '--dry-run', [CompletionResultType]::ParameterName, '仅显示匹配的进程，不终止')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;eject' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--find-locks', '--find-locks', [CompletionResultType]::ParameterName, '仅查看占用，不终止')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;who' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;handles' {
            [CompletionResult]::new('--pid', '--pid', [CompletionResultType]::ParameterName, '目标进程 PID（与 --file 互斥）')
            [CompletionResult]::new('--file', '--file', [CompletionResultType]::ParameterName, '反查模式：列出占用此路径的所有 PID')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;priority' {
            [CompletionResult]::new('--set', '--set', [CompletionResultType]::ParameterName, '设置优先级（idle / belownormal / normal / abovenormal / high / realtime）')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;affinity' {
            [CompletionResult]::new('--set', '--set', [CompletionResultType]::ParameterName, '设置 affinity mask（16 进制，如 0xFF）')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;monitor' {
            [CompletionResult]::new('--remove', '--remove', [CompletionResultType]::ParameterName, '删除监控 (按 ID)')
            [CompletionResult]::new('--port', '--port', [CompletionResultType]::ParameterName, '监控端口号')
            [CompletionResult]::new('--pid', '--pid', [CompletionResultType]::ParameterName, '监控进程 PID')
            [CompletionResult]::new('--command', '--command', [CompletionResultType]::ParameterName, '监控命令（带自动重启）')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--add', '--add', [CompletionResultType]::ParameterName, '添加监控')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('ps', 'ps', [CompletionResultType]::ParameterValue, '列出所有容器（默认）')
            [CompletionResult]::new('inspect', 'inspect', [CompletionResultType]::ParameterValue, '查看指定容器详情')
            [CompletionResult]::new('top', 'top', [CompletionResultType]::ParameterValue, '容器内进程列表（docker top）')
            [CompletionResult]::new('logs', 'logs', [CompletionResultType]::ParameterValue, '容器日志（跟随或一次性）')
            [CompletionResult]::new('images', 'images', [CompletionResultType]::ParameterValue, '列出本地镜像')
            [CompletionResult]::new('volumes', 'volumes', [CompletionResultType]::ParameterValue, '列出 volume')
            [CompletionResult]::new('image-rm', 'image-rm', [CompletionResultType]::ParameterValue, '删除镜像')
            [CompletionResult]::new('volume-rm', 'volume-rm', [CompletionResultType]::ParameterValue, '删除 volume')
            [CompletionResult]::new('compose', 'compose', [CompletionResultType]::ParameterValue, 'docker-compose 薄封装（需宿主机装 docker-compose）')
            [CompletionResult]::new('events', 'events', [CompletionResultType]::ParameterValue, '监听容器事件流（Ctrl+C 停止）')
            [CompletionResult]::new('exec', 'exec', [CompletionResultType]::ParameterValue, 'exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。 TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'proc;docker;ps' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;inspect' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;top' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;logs' {
            [CompletionResult]::new('--tail', '--tail', [CompletionResultType]::ParameterName, '从末尾开始显示的行数（如 "100"、"all"）；默认 "all"')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--follow', '--follow', [CompletionResultType]::ParameterName, '跟随模式（默认 false，输出后退出）')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;images' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;volumes' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;image-rm' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, '强制删除（即便 in_use）')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;volume-rm' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, '强制删除（即便 in_use）')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;compose' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;events' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;exec' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;docker;help' {
            [CompletionResult]::new('ps', 'ps', [CompletionResultType]::ParameterValue, '列出所有容器（默认）')
            [CompletionResult]::new('inspect', 'inspect', [CompletionResultType]::ParameterValue, '查看指定容器详情')
            [CompletionResult]::new('top', 'top', [CompletionResultType]::ParameterValue, '容器内进程列表（docker top）')
            [CompletionResult]::new('logs', 'logs', [CompletionResultType]::ParameterValue, '容器日志（跟随或一次性）')
            [CompletionResult]::new('images', 'images', [CompletionResultType]::ParameterValue, '列出本地镜像')
            [CompletionResult]::new('volumes', 'volumes', [CompletionResultType]::ParameterValue, '列出 volume')
            [CompletionResult]::new('image-rm', 'image-rm', [CompletionResultType]::ParameterValue, '删除镜像')
            [CompletionResult]::new('volume-rm', 'volume-rm', [CompletionResultType]::ParameterValue, '删除 volume')
            [CompletionResult]::new('compose', 'compose', [CompletionResultType]::ParameterValue, 'docker-compose 薄封装（需宿主机装 docker-compose）')
            [CompletionResult]::new('events', 'events', [CompletionResultType]::ParameterValue, '监听容器事件流（Ctrl+C 停止）')
            [CompletionResult]::new('exec', 'exec', [CompletionResultType]::ParameterValue, 'exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。 TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'proc;docker;help;ps' {
            break
        }
        'proc;docker;help;inspect' {
            break
        }
        'proc;docker;help;top' {
            break
        }
        'proc;docker;help;logs' {
            break
        }
        'proc;docker;help;images' {
            break
        }
        'proc;docker;help;volumes' {
            break
        }
        'proc;docker;help;image-rm' {
            break
        }
        'proc;docker;help;volume-rm' {
            break
        }
        'proc;docker;help;compose' {
            break
        }
        'proc;docker;help;events' {
            break
        }
        'proc;docker;help;exec' {
            break
        }
        'proc;docker;help;help' {
            break
        }
        'proc;smart' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;dns' {
            [CompletionResult]::new('--since', '--since', [CompletionResultType]::ParameterName, '输出过去 N 时间的事件（如 "1h"、"30m"）；当前不持久化，本参数留 TODO')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--tail', '--tail', [CompletionResultType]::ParameterName, '跟随模式：流式输出新事件，Ctrl+C 退出')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;record' {
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, '输出文件路径（默认: ~/.config/proc/recordings/recording_{timestamp}.prec）')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, '输出文件路径（默认: ~/.config/proc/recordings/recording_{timestamp}.prec）')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;replay' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;export' {
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, '输出格式：json | csv')
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, '输出文件路径（不指定则输出到 stdout）')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, '输出文件路径（不指定则输出到 stdout）')
            [CompletionResult]::new('--sort', '--sort', [CompletionResultType]::ParameterName, '排序字段：cpu | mem | name | pid | disk_read | disk_write | net_sent | net_recv')
            [CompletionResult]::new('--limit', '--limit', [CompletionResultType]::ParameterName, '限制导出数量')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;diag' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('--json', '--json', [CompletionResultType]::ParameterName, '输出 JSON（默认 human-readable 表格）')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;mcp' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('serve', 'serve', [CompletionResultType]::ParameterValue, '启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'proc;mcp;serve' {
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;mcp;help' {
            [CompletionResult]::new('serve', 'serve', [CompletionResultType]::ParameterValue, '启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'proc;mcp;help;serve' {
            break
        }
        'proc;mcp;help;help' {
            break
        }
        'proc;completions' {
            [CompletionResult]::new('-s', '-s', [CompletionResultType]::ParameterName, 'Target shell')
            [CompletionResult]::new('--shell', '--shell', [CompletionResultType]::ParameterName, 'Target shell')
            [CompletionResult]::new('--path', '--path', [CompletionResultType]::ParameterName, '工作路径')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'proc;help' {
            [CompletionResult]::new('ls', 'ls', [CompletionResultType]::ParameterValue, '列出进程')
            [CompletionResult]::new('tree', 'tree', [CompletionResultType]::ParameterValue, '进程树')
            [CompletionResult]::new('port', 'port', [CompletionResultType]::ParameterValue, '端口映射')
            [CompletionResult]::new('kill', 'kill', [CompletionResultType]::ParameterValue, '终止进程')
            [CompletionResult]::new('pkill', 'pkill', [CompletionResultType]::ParameterValue, '按名称终止进程')
            [CompletionResult]::new('eject', 'eject', [CompletionResultType]::ParameterValue, 'U盘助手')
            [CompletionResult]::new('who', 'who', [CompletionResultType]::ParameterValue, '反查「谁占用这个文件 / 目录」（阶段 4 A1）')
            [CompletionResult]::new('handles', 'handles', [CompletionResultType]::ParameterValue, '枚举指定进程的所有句柄（阶段 4 A1）')
            [CompletionResult]::new('priority', 'priority', [CompletionResultType]::ParameterValue, '查询 / 设置进程优先级（阶段 4 A4）')
            [CompletionResult]::new('affinity', 'affinity', [CompletionResultType]::ParameterValue, '查询 / 设置进程 CPU affinity（阶段 4 A4）')
            [CompletionResult]::new('monitor', 'monitor', [CompletionResultType]::ParameterValue, '进程监控')
            [CompletionResult]::new('docker', 'docker', [CompletionResultType]::ParameterValue, 'Docker 监控')
            [CompletionResult]::new('smart', 'smart', [CompletionResultType]::ParameterValue, 'SMART 磁盘健康(阶段 5 B3)')
            [CompletionResult]::new('dns', 'dns', [CompletionResultType]::ParameterValue, 'DNS 查询日志（阶段 8 D3）。仅 Windows 支持；隐私：仅内存，不持久化。')
            [CompletionResult]::new('record', 'record', [CompletionResultType]::ParameterValue, '录制系统快照')
            [CompletionResult]::new('replay', 'replay', [CompletionResultType]::ParameterValue, '回放录制文件')
            [CompletionResult]::new('export', 'export', [CompletionResultType]::ParameterValue, '导出当前进程快照')
            [CompletionResult]::new('diag', 'diag', [CompletionResultType]::ParameterValue, 'v0.6.0 阶段 3：worker 诊断 — 输出所有后台 worker 的 metrics （avg/max/polls/drops），用户报 bug 时附上。')
            [CompletionResult]::new('mcp', 'mcp', [CompletionResultType]::ParameterValue, 'MCP server mode (stdio transport)')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Generate shell completions')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'proc;help;ls' {
            break
        }
        'proc;help;tree' {
            break
        }
        'proc;help;port' {
            break
        }
        'proc;help;kill' {
            break
        }
        'proc;help;pkill' {
            break
        }
        'proc;help;eject' {
            break
        }
        'proc;help;who' {
            break
        }
        'proc;help;handles' {
            break
        }
        'proc;help;priority' {
            break
        }
        'proc;help;affinity' {
            break
        }
        'proc;help;monitor' {
            break
        }
        'proc;help;docker' {
            [CompletionResult]::new('ps', 'ps', [CompletionResultType]::ParameterValue, '列出所有容器（默认）')
            [CompletionResult]::new('inspect', 'inspect', [CompletionResultType]::ParameterValue, '查看指定容器详情')
            [CompletionResult]::new('top', 'top', [CompletionResultType]::ParameterValue, '容器内进程列表（docker top）')
            [CompletionResult]::new('logs', 'logs', [CompletionResultType]::ParameterValue, '容器日志（跟随或一次性）')
            [CompletionResult]::new('images', 'images', [CompletionResultType]::ParameterValue, '列出本地镜像')
            [CompletionResult]::new('volumes', 'volumes', [CompletionResultType]::ParameterValue, '列出 volume')
            [CompletionResult]::new('image-rm', 'image-rm', [CompletionResultType]::ParameterValue, '删除镜像')
            [CompletionResult]::new('volume-rm', 'volume-rm', [CompletionResultType]::ParameterValue, '删除 volume')
            [CompletionResult]::new('compose', 'compose', [CompletionResultType]::ParameterValue, 'docker-compose 薄封装（需宿主机装 docker-compose）')
            [CompletionResult]::new('events', 'events', [CompletionResultType]::ParameterValue, '监听容器事件流（Ctrl+C 停止）')
            [CompletionResult]::new('exec', 'exec', [CompletionResultType]::ParameterValue, 'exec 进容器（阶段 9 E2）。CLI 模式直接 exec docker，等价 `docker exec -it`。 TUI 内按 `e` 进入嵌入式 PTY 视图（详见 src/tui/container_exec_view.rs）。')
            break
        }
        'proc;help;docker;ps' {
            break
        }
        'proc;help;docker;inspect' {
            break
        }
        'proc;help;docker;top' {
            break
        }
        'proc;help;docker;logs' {
            break
        }
        'proc;help;docker;images' {
            break
        }
        'proc;help;docker;volumes' {
            break
        }
        'proc;help;docker;image-rm' {
            break
        }
        'proc;help;docker;volume-rm' {
            break
        }
        'proc;help;docker;compose' {
            break
        }
        'proc;help;docker;events' {
            break
        }
        'proc;help;docker;exec' {
            break
        }
        'proc;help;smart' {
            break
        }
        'proc;help;dns' {
            break
        }
        'proc;help;record' {
            break
        }
        'proc;help;replay' {
            break
        }
        'proc;help;export' {
            break
        }
        'proc;help;diag' {
            break
        }
        'proc;help;mcp' {
            [CompletionResult]::new('serve', 'serve', [CompletionResultType]::ParameterValue, '启动 MCP server（stdio transport），阻塞直到 client 关闭流。 接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。')
            break
        }
        'proc;help;mcp;serve' {
            break
        }
        'proc;help;completions' {
            break
        }
        'proc;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
