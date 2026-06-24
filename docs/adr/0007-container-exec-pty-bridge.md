# ADR-0007: 容器 exec 走 spawn `docker exec -it` + portable-pty

- **Status**: Accepted
- **Date**: 2026-06-20
- **Phase**: v0.5.0 阶段 9

## 背景

阶段 9 要在 Docker 面板按 `e` 进入嵌入式 PTY 模式（像 lazydocker 那样在 TUI 内 exec 进容器）。stage-9.md 要求用 `portable-pty` + `vt100` crate。

## 选项

| 方案 | 描述 | 优点 | 缺点 |
|---|---|---|---|
| A. **本地 spawn `docker exec -it <container> <shell>` 子进程 + portable-pty master** | docker CLI 处理所有 daemon 通信 | 实现最简、daemon 连接差异（命名管道 / TCP / unix socket）由 docker CLI 处理 | 依赖 PATH 有 docker；spawn 延迟 ~50ms |
| B. bollard exec Attached 流 + 自己处理 stdio | 全程 HTTP，无 docker CLI 依赖 | 与 proc 既有的 bollard 用法一致 | **不引入 portable-pty 违背 stage-9.md 明确要求**；ANSI 流处理复杂 |
| C. portable-pty master/slave pair + bollard exec 双向中转 | 融合 A 和 B 的"优点" | 看似最优 | **技术上不成立**：PTY slave 端需要子进程才有意义，bollard exec 不是子进程 |

## 决策

采用方案 A（spawn `docker exec -it` 子进程）。理由：

1. **方案 C 不可行**：PTY slave 端必须有子进程，bollard exec 走 HTTP 不是子进程，"双向中转"技术上不成立
2. **方案 B 违背 stage-9.md**：明确要求引入 portable-pty，方案 B 不引入
3. **方案 A 让 docker CLI 处理所有差异**：Docker Desktop 命名管道 / WSL Docker TCP / Linux unix socket，proc 不感知
4. **CLI 透传也简单**：CLI `proc docker exec <container>` 直接 spawn 等价 `docker exec -it`

## 后果

- 正面：~400 行实现完整 PTY exec 模式（ContainerExec + container_exec_view + App 字段 + handle_key）
- 正面：`detect_default_shell(image)` 纯函数按镜像推断 shell（alpine→/bin/sh，ubuntu→/bin/bash）
- 已知限制：需要 PATH 有 docker 二进制（与既有 `proc docker compose` 一致）
- 已知限制：Windows ConPTY 需 Windows 10 1809+
- 已知限制：exec 模式下 Ctrl+C 走 KeyEvent 转发容器（raw mode 下 crossterm 不传 SIGINT）
- 已知限制：**v0.6.0 阶段 2 子进程权限剥离**需扩展到 docker exec 子进程（restricted_spawn），否则 elevated proc 的 docker exec 持 SE_DEBUG

## 参考

- v0.5.0 阶段 9 落地：CHANGELOG.md
- 相关代码：`src/docker/exec.rs::ContainerExec` / `src/tui/container_exec_view.rs`
- v0.6.0 阶段 2 安全扩展：plan.md
