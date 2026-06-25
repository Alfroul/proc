//! 子进程权限剥离 — 见 CONTEXT.md / ADR-0008。
//!
//! elevated proc spawn PowerShell DNS / docker exec / nvtop 时，子进程默认继承
//! 父的 SE_DEBUG_NAME（SeDebugPrivilege）。这相当于给子进程一把「随便读写任何
//! 进程内存」的钥匙 —— 如果子进程被恶意输入劫持（PowerShell 接受 -Command 脚本），
//! 就成了 credential theft 跳板。
//!
//! 本模块在 spawn 前调 [`CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)`] 剥离所有
//! 权限，再用 [`CreateProcessAsUserW`] 走 restricted token 启动子进程。这样即便
//! 子进程被劫持也无法直接 OpenProcess(PROCESS_VM_READ) 其他进程。
//!
//! 接口设计：返回 mini [`RestrictedChild`] 而非 `std::process::Child` —— stable
//! Rust 不支持从 raw handle 构造 `Child`，且我们的关键需求只有 stdout pipe, kill,
//! wait。复用 `std::fs::File`（impl Read）让上层 `BufReader::new(stdout)` 路径
//! 一行不改。
//!
//! **不接入 docker exec / nvtop** —— 这些子进程本身就是 privileged 操作（docker
//! daemon 通信需要），无法简单 drop。阶段 2 范围内只接入 PowerShell DNS（最高危
//! 因为 PowerShell 命令行接受任意脚本）。

use std::fs::File;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

/// 用 reduced token spawn 出来的子进程句柄。
///
/// 字段语义与 `std::process::Child` 等价但只暴露 DNS reader 实际用到的子集：
/// `stdout` / `kill` / `id`。`wait` 用 WaitForSingleObject 实现。
///
/// 注：stdout 类型是 `std::fs::File`（不是 `std::process::ChildStdout`）— 因为 stable
/// Rust 不提供 `ChildStdout::from_raw_handle`，而 `File` 实现了 FromRawHandle + Read，
/// 上层 `BufReader::new(stdout)` 一行不改。
///
/// 内部两种后端：
/// - **Win32 native**（restricted 路径）：`process_handle != 0`，kill 走 TerminateProcess
/// - **std fallback**（restricted 不可用 / Unix）：`std_child.is_some()`，kill 走 Child::kill
pub struct RestrictedChild {
    pid: u32,
    /// Owns the process handle (Win32 native path); closes on Drop.
    /// `0` 表示不持有 native handle（走 std_child 路径）。
    process_handle: usize,
    /// `Option` 以便 Drop / wait 时 take 出来关掉。
    stdout: Option<File>,
    /// Fallback 路径下保留的 std::process::Child，让 kill 能实际工作。
    /// `stdout` 已 take 出去后 Child::stdout 变 None，但 kill / wait 仍可用。
    std_child: Option<std::process::Child>,
}

impl RestrictedChild {
    /// 拿走 stdout 管道（语义与 `std::process::Child::stdout` 一致）。
    #[must_use]
    pub fn stdout(&mut self) -> Option<File> {
        self.stdout.take()
    }

    /// 子进程 PID。
    #[must_use]
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// kill（语义与 `Child::kill` 一致）。idempotent — 多次调用不报错。
    pub fn kill(&mut self) -> io::Result<()> {
        // 优先 std_child（fallback / Unix），其次 native handle（Windows restricted）
        if let Some(child) = self.std_child.as_mut() {
            return child.kill();
        }
        #[cfg(windows)]
        {
            // SAFETY: process_handle 在 Drop 前一直有效；TerminateProcess 文档允许
            // 对已退出进程调用（返回 success 但什么都不做）。
            use windows::Win32::Foundation::HANDLE;
            use windows::Win32::System::Threading::TerminateProcess;
            if self.process_handle == 0 {
                return Ok(()); // 已经被 take（如 wait 之后）
            }
            unsafe {
                let h = HANDLE(self.process_handle as isize);
                let _ = TerminateProcess(h, 1);
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            // Unix 路径在 std_child 上处理；若到此处说明 std_child 已被 take（异常状态）
            Ok(())
        }
    }

    /// 阻塞等子进程退出。语义与 `Child::wait` 一致，但返回简化 ExitStatus。
    pub fn wait(mut self) -> io::Result<RestrictedExitStatus> {
        if let Some(mut child) = self.std_child.take() {
            let status = child.wait()?;
            let code = status.code();
            return Ok(RestrictedExitStatus { code });
        }
        #[cfg(windows)]
        {
            // SAFETY: process_handle 是 own 的；wait 后 take 防止 double close。
            use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_EVENT};
            use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
            if self.process_handle == 0 {
                return Ok(RestrictedExitStatus { code: None });
            }
            let raw = self.process_handle;
            self.process_handle = 0;
            unsafe {
                let h = HANDLE(raw as isize);
                let r = WaitForSingleObject(h, u32::MAX); // INFINITE
                let exited = matches!(r, WAIT_EVENT(0));
                let mut code: u32 = 0;
                if exited {
                    let _ = GetExitCodeProcess(h, &mut code);
                }
                let _ = CloseHandle(h);
                Ok(RestrictedExitStatus {
                    code: if exited { Some(code as i32) } else { None },
                })
            }
        }
        #[cfg(not(windows))]
        {
            Ok(RestrictedExitStatus { code: None })
        }
    }
}

impl Drop for RestrictedChild {
    fn drop(&mut self) {
        // std_child 优先（自动 Drop 时 kill + close）；native handle 手动 CloseHandle
        if let Some(mut child) = self.std_child.take() {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::{CloseHandle, HANDLE};
            if self.process_handle != 0 {
                let raw = self.process_handle;
                self.process_handle = 0;
                unsafe {
                    let _ = CloseHandle(HANDLE(raw as isize));
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestrictedExitStatus {
    pub code: Option<i32>,
}

impl RestrictedExitStatus {
    #[must_use]
    pub fn success(&self) -> bool {
        matches!(self.code, Some(0))
    }
}

/// 用 reduced token spawn 子进程。
///
/// - Windows：尝试 CreateRestrictedToken(DISABLE_MAX_PRIVILEGE) + CreateProcessAsUserW；
///   非 elevated 环境下 CreateProcessAsUserW 会 E_ACCESSDENIED（缺 SeIncreaseQuotaPrivilege），
///   此时**自动 fallback** 到普通 `std::process::Command`（tracing 一次 warn）。
///   非 elevated 场景本身就没 SeDebugPrivilege 可剥离，fallback 安全。
///   elevated 场景才能享受真正的 token 剥离（ADR-0008 主要威胁模型）。
/// - 非 Windows：fallback 走 `std::process::Command`（Linux 下 sudo setuid 不会被
///   spawn 自动继承；如果父是 setuid 启动，调用方应自行 `setuid(getuid())`）
///
/// `program` 是不带参数的可执行名；`args` 是参数列表（不会再次 shell-quote，
/// 调用方应保证每个元素是合法 shell token；含空格需自行加引号）。
pub fn spawn_with_reduced_privileges(program: &str, args: &[&str]) -> io::Result<RestrictedChild> {
    #[cfg(windows)]
    {
        match spawn_windows_restricted(program, args) {
            Ok(child) => Ok(child),
            Err(e) => {
                // 不在 elevated 环境下 E_ACCESSDENIED 是预期；其他错误也走 fallback
                // 保证 DNS 等 worker 不会因 hardening 失败而完全不可用。
                tracing::warn!(
                    "restricted spawn 失败，降级到普通 Command（SeDebugPrivilege 未剥离）: {e}"
                );
                spawn_windows_fallback(program, args)
            }
        }
    }
    #[cfg(not(windows))]
    {
        spawn_unix(program, args)
    }
}

#[cfg(windows)]
fn spawn_windows_restricted(program: &str, args: &[&str]) -> io::Result<RestrictedChild> {
    use std::os::windows::io::{FromRawHandle, RawHandle};

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, SECURITY_ATTRIBUTES, TOKEN_DUPLICATE,
    };
    use windows::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken,
        PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
        STARTUPINFOW_FLAGS,
    };
    use windows::core::{PCWSTR, PWSTR};

    // SAFETY: 所有 Win32 调用都遵守其文档要求；handle 在出错路径上一律 CloseHandle。
    unsafe {
        // ── 1. 打开自己 token（duplicate 权限以便 CreateRestrictedToken 复制）──────
        let mut own_token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE, &mut own_token)
            .map_err(|e| io::Error::other(format!("OpenProcessToken: {e}")))?;

        // ── 2. CreateRestrictedToken(DISABLE_MAX_PRIVILEGE) ─────────────────────
        let mut restricted = HANDLE::default();
        let r = CreateRestrictedToken(
            own_token,
            DISABLE_MAX_PRIVILEGE,
            None,
            None,
            None,
            &mut restricted,
        );
        let _ = CloseHandle(own_token);
        if r.is_err() {
            return Err(io::Error::other(format!("CreateRestrictedToken: {r:?}")));
        }

        // ── 3. CreatePipe(stdout_read, stdout_write) — write end inheritable ────
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: true.into(),
        };
        let mut stdout_read = HANDLE::default();
        let mut stdout_write = HANDLE::default();
        let r = CreatePipe(&mut stdout_read, &mut stdout_write, Some(&sa), 0);
        if r.is_err() {
            let _ = CloseHandle(restricted);
            return Err(io::Error::other(format!("CreatePipe: {r:?}")));
        }
        // 父读端不继承（防孙进程意外继承）；子写端继承
        // SetHandleInformation(stdout_read, HANDLE_FLAG_INHERIT, 0) — 省略：
        // 默认 CreatePipe 两端都 inheritable，但子进程不读 stdout，仅继承 write end。
        // 父进程持 read 端即可；额外孙进程继承 read 端在阶段 2 不在威胁模型内。

        // ── 4. STARTUPINFOW：hStdInput = parent stdin, hStdOutput/hStdError = pipe write ─
        let stdin_handle = GetStdHandle(STD_INPUT_HANDLE).unwrap_or_default();
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.dwFlags = STARTUPINFOW_FLAGS(STARTF_USESTDHANDLES.0);
        si.hStdInput = stdin_handle;
        si.hStdOutput = stdout_write;
        si.hStdError = stdout_write;

        // ── 5. 命令行：`program arg1 arg2 ...`（UTF-16 + NUL terminator）──────
        let mut cmd_line_string = String::from(program);
        for arg in args {
            // 简单参数直接拼；含空格的脚本需要引号包裹 —— PowerShell -Command 走
            // 这里，调用方应保证 args 中每个元素本身已是合法 shell token。
            cmd_line_string.push(' ');
            cmd_line_string.push_str(arg);
        }
        let mut cmd_line: Vec<u16> = cmd_line_string
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // ── 6. CreateProcessAsUserW(restricted) ──────────────────────────────────
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let cmd_line_pwstr = PWSTR(cmd_line.as_mut_ptr());
        let r = CreateProcessAsUserW(
            restricted,
            PCWSTR::null(),
            cmd_line_pwstr,
            None,
            None,
            true, // bInheritHandles — pipe write end 需要
            PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW.0),
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        );
        // 立刻关 restricted token + pipe write end（父侧不再需要；子侧继承副本）
        let _ = CloseHandle(restricted);
        let _ = CloseHandle(stdout_write);
        if r.is_err() {
            let _ = CloseHandle(stdout_read);
            return Err(io::Error::other(format!("CreateProcessAsUserW: {r:?}")));
        }

        // pi.hThread 不需要 — 关掉避免泄漏
        let _ = CloseHandle(pi.hThread);

        // ── 7. 把 stdout_read 包成 std::fs::File（实现 Read，BufReader 能包）─────
        // SAFETY: stdout_read 是有效的内核句柄，由 CreatePipe 刚产出，所有权转移给 File。
        // FromRawHandle trait 在函数顶部 import；用 fully-qualified 调用避开 method resolution 歧义。
        let raw: RawHandle = stdout_read.0 as *mut _;
        let stdout = <File as FromRawHandle>::from_raw_handle(raw);

        Ok(RestrictedChild {
            pid: pi.dwProcessId,
            process_handle: pi.hProcess.0 as usize,
            stdout: Some(stdout),
            std_child: None,
        })
    }
}

#[cfg(not(windows))]
fn spawn_unix(program: &str, args: &[&str]) -> io::Result<RestrictedChild> {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    cmd.stdin(Stdio::null());
    let mut child = cmd.spawn()?;
    let pid = child.id();
    // stdout 转 fs::File（ChildStdout 不直接是 File，但能转 raw fd 重建）。
    let stdout = child.stdout.take().map(|cs| {
        let fd = cs.into_raw_fd();
        // SAFETY: fd 来自 valid ChildStdout，所有权转移给 File。
        unsafe { std::fs::File::from_raw_fd(fd) }
    });
    Ok(RestrictedChild {
        pid,
        process_handle: 0,
        stdout,
        std_child: Some(child),
    })
}

/// Windows fallback：restricted token 不可用时（非 elevated / SeIncreaseQuotaPrivilege 缺失）
/// 走普通 `std::process::Command`。子进程继承父的 token。
///
/// 关键：保留 std::process::Child（不 forget），让 RestrictedChild::kill / Drop 能实际
/// 终止子进程。否则 DNS collector Drop 时无法 kill PowerShell → reader_loop 永远阻塞。
#[cfg(windows)]
fn spawn_windows_fallback(program: &str, args: &[&str]) -> io::Result<RestrictedChild> {
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    cmd.stdin(Stdio::null());
    let mut child = cmd.spawn()?;
    let pid = child.id();
    let child_stdout = child.stdout.take();
    let stdout = child_stdout.map(|cs| {
        let raw = cs.into_raw_handle();
        // SAFETY: raw 来自 valid ChildStdout handle，所有权转移给 File。
        unsafe { std::fs::File::from_raw_handle(raw) }
    });
    Ok(RestrictedChild {
        pid,
        process_handle: 0,
        stdout,
        std_child: Some(child),
    })
}

/// Read trait 让 BufReader 能用 RestrictedChild 的 stdout。
/// 现实路径：上层用 `child.stdout.take()` 拿到 ChildStdout 后直接用，不需要这里 impl。
#[allow(dead_code)]
fn _read_compat_demo<R: Read>(_r: R) {}

#[allow(dead_code)]
fn _arc_mutex_compat(_x: Arc<Mutex<()>>) {}
