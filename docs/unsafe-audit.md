# proc unsafe 审计（v0.26）

> **口径**：代码级 unsafe = `unsafe {` / `unsafe (` block **191** + `unsafe fn` **2** + `unsafe extern "system" fn` **5** = **198**；粗 grep `unsafe` 命中 205，差值 7 是注释文本（`// unsafe scope ...` 类）。统计于 2026-08-31（[v0.26 stage-1 附录 C](stages/v0.26-stage-1.md)），本文件是它的成文与判读。
>
> **为什么有这么多 unsafe**：proc 是 Windows 系统工具——ETW 事件回调、NT API（`NtQuerySystemInformation` / `NtQueryObject`）、`ReadProcessMemory`、句柄枚举、`SetProcessMitigationPolicy` 都没有 safe 绑定可用（windows-rs 覆盖之外的动态加载 NT 函数尤其如此）。unsafe 集中度是**领域属性**，不是代码卫生问题——判断依据是分布（见下表）：全部集中在 FFI 边界层，业务逻辑层（panel / controller / agent / mcp handler 主体）零 unsafe。

## 1. 分布（27 文件，按数量降序取头部）

| 文件 | unsafe | SAFETY 注释 | 模块性质 |
|---|---|---|---|
| `src/collect.rs` | 26 | 0（本 cycle 补 1） | sysinfo 采集 + ToolHelp 补漏快照 |
| `src/schannel_etw/provider.rs` | 20 | 17 | Schannel ETW（TLS SNI） |
| `src/dns_log/etw.rs` | 20 | 17 | DNS ETW provider |
| `src/inspect/handles.rs` | 17 | 0（本 cycle 补 2） | NT 句柄枚举 |
| `src/estats.rs` | 14 | 0（本 cycle 补 1） | TCP 传输质量（GetPerTcpConnectionEStats） |
| `src/process_control.rs` | 12 | 0 | 优先级 / affinity / kill |
| `src/net_flow/windows.rs` | 11 | 0 | GetExtendedTcp/UdpTable |
| `src/disk_io_etw/provider.rs` | 11 | 9 | Disk IO ETW provider |
| `src/gpu.rs` | 9 | 0 | NVML / AMD ADL |
| `src/security/privilege.rs` | 8 | 0 | token 特权查询 |
| 其余 17 文件 | 各 1-7 | 部分 | dll 检查 / restricted spawn / path rules / eject / env / memory / throttle 等 |

**头部 9 文件占 140/198（71%）**——全部是「与内核 / 驱动 / 系统服务对话」的采集与安全边界。

## 2. SAFETY 注释覆盖现状与判读

SAFETY 注释 59 处（30%），分布极不均匀：

- **覆盖 80-100% 的组**：ETW 三 provider（schannel 17/20、dns 17/20、disk_io 9+2/16）+ security 组（restricted_spawn 6/6、path_rules 4/4、self_mitigation 1/1、eject/device 3/3）。共同点：**回调与 token 操作**——ETW 是 `unsafe extern "system" fn` 回调直接吃 raw 指针，restricted spawn 是特权变更，写注释的优先级判断正确。
- **零覆盖的组（NT API 调用层）**：collect 26 / handles 17 / estats 14 / process_control 12 / net_flow 11 / gpu 9 / privilege 8——共 97 处。**为什么**：这一层多数是「调一次系统 API、指针只活在一个表达式内」的 thin wrapper，风险密度低于回调层；但其中三个模式值得显式 invariant 声明（本 cycle 已挑代表处补注释，见 §4）：
  1. **两段式 sizing + 灵活数组成员**（`GetTcpTable2` / `NtQuerySystemInformation`：先问大小再填充，头部 + 变长数组切片——必须先做字节预算校验再 `from_raw_parts`）
  2. **动态加载 NT 函数 + transmute**（`GetProcAddress` → fn 指针：签名正确性全靠人工对表）
  3. **`mem::zeroed` 的 Win32 POD**（全零是否合法表示需逐 struct 判断）

## 3. lint 与编译期防线

- **edition 2024**（`Cargo.toml:4`）：`unsafe_op_in_unsafe_fn` 默认启用。项目仅有的 2 个 `unsafe fn`（`src/inspect/env.rs:128/148`，ReadProcessMemory 封装）体内已显式包 unsafe block——**现状合规零变更**（双档 clippy `-D warnings` 全过即机器证据）。
- **miri**：workflow 存在（`.github/workflows/miri.yml`）但历史 run 全红（E0433 编译配置问题，非 unsafe 判定失败）——workflow 修复是 v0.27 候选（[stage-3 doc「已知阻塞」段](stages/v0.26-stage-3.md)）；修好前 miri 不构成有效防线，此处如实声明。
- **回归护栏**：unsafe 密集模块的行为契约由集成测试锚定（如 estats 差分精确断言、handles 枚举降级路径、ETW provider 回调解析 fixture）。

## 4. 本 cycle 补的 SAFETY 注释（4 处，纯注释零行为变化）

| 位置 | 模式 | 声明的 invariant |
|---|---|---|
| `src/inspect/handles.rs`（`nt_query_system_handles`） | 动态加载 NT 函数调用 | buffer 长度与传入 size 一致；STATUS_INFO_LENGTH_MISMATCH 扩容重试上限 64MB；成功时 ReturnLength ≤ buffer.len() 才 truncate |
| `src/inspect/handles.rs`（`collect` 灵活数组） | 灵活数组成员 | 头部 cast 前已校验 buffer ≥ header 大小；遍历前已校验 `header + count × entry_size ≤ buffer.len()`（expected_bytes 预算） |
| `src/estats.rs`（`enable_estats_for_all` rows 切片） | 两段式 sizing + 灵活数组 | dwNumEntries 来自本次成功调用；切片前显式校验 `buf.len() ≥ header + n × row_size` |
| `src/collect.rs`（ToolHelp 快照循环） | Win32 迭代 API | PROCESSENTRY32 按文档预置 dwSize 后传入；Process32First/Next 只写该结构 |

其余 93 处 NT API 层 unsafe 维持现状——全量补注释预估 ~90+ 处 × 每处需逐一核对文档 invariant，属独立专项（v0.27+ 候选，见 [tech-debt](tech-debt.md) v0.26 追踪段），不在展示冲刺 cycle 内赶工（注释写错比没有更伤）。

## 5. 结论

1. **198 处 unsafe 全部位于 FFI 边界层**，业务逻辑层零 unsafe——集中度是系统工具的领域属性。
2. **高风险区（回调 / 特权变更）SAFETY 覆盖 80-100%**，低风险 thin-wrapper 层零覆盖——优先级判断与实际风险排序一致。
3. **编译期防线合规**（edition 2024 lint + clippy -D warnings 双档）；miri 防线如实声明为不可用（workflow 待修）。
4. 补注释策略：挑代表性模式（本 cycle 4 处）而不是撒胡椒面——每处注释声明可核对的 invariant。
