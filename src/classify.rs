use std::collections::HashMap;
use std::os::windows::ffi::OsStringExt;
use std::sync::OnceLock;

use crate::collect::ProcessInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessClass {
    UserApp,
    SystemProcess,
    WindowsService,
    Kernel,
    Unknown,
}

impl ProcessClass {
    pub fn label(&self) -> &str {
        match self {
            Self::UserApp => "用户",
            Self::SystemProcess => "系统",
            Self::WindowsService => "服务",
            Self::Kernel => "内核",
            Self::Unknown => "未知",
        }
    }
}

const SYSTEM_PROCESS_NAMES: &[&str] = &[
    "csrss.exe",
    "smss.exe",
    "wininit.exe",
    "lsass.exe",
    "svchost.exe",
    "services.exe",
    "winlogon.exe",
    "dwm.exe",
    "conhost.exe",
    "LogonUI.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "taskhostw.exe",
    "ctfmon.exe",
    "dllhost.exe",
    "audiodg.exe",
    "WUDFHost.exe",
    "WerFault.exe",
    "RuntimeBroker.exe",
    "SearchIndexer.exe",
    "SecurityHealthService.exe",
    "MsMpEng.exe",
    "NisSrv.exe",
    "MpCmdRun.exe",
    "registry",
    "secure system",
];

pub const EXPECTED_ORPHAN_NAMES: &[&str] = &["explorer.exe"];

static SERVICE_CACHE: OnceLock<HashMap<u32, String>> = OnceLock::new();

fn build_service_cache() -> HashMap<u32, String> {
    let mut cache = HashMap::new();

    use windows::Win32::System::Services::{
        ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW, OpenSCManagerW, SC_ENUM_PROCESS_INFO,
        SC_MANAGER_ENUMERATE_SERVICE, SERVICE_STATE_ALL, SERVICE_WIN32,
    };
    use windows::core::PCWSTR;

    unsafe {
        let scm = match OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ENUMERATE_SERVICE)
        {
            Ok(scm) => scm,
            Err(_) => return cache,
        };

        let mut bytes_needed: u32 = 0;
        let mut services_returned: u32 = 0;
        let mut resume_handle: u32 = 0;

        let _ = EnumServicesStatusExW(
            scm,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_STATE_ALL,
            Some(&mut []),
            &mut bytes_needed,
            &mut services_returned,
            Some(&mut resume_handle),
            PCWSTR::null(),
        );

        let mut buffer: Vec<u8> = vec![0u8; bytes_needed as usize];

        let result = EnumServicesStatusExW(
            scm,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_STATE_ALL,
            Some(&mut buffer),
            &mut bytes_needed,
            &mut services_returned,
            None,
            PCWSTR::null(),
        );

        if result.is_ok() || services_returned > 0 {
            let services = buffer.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW;
            for i in 0..services_returned as usize {
                let svc = &*services.add(i);
                let pid = svc.ServiceStatusProcess.dwProcessId;
                if pid > 0 {
                    let name_ptr = svc.lpServiceName;
                    if !name_ptr.is_null() {
                        let name = widestring_to_string(name_ptr);
                        cache.insert(pid, name);
                    }
                }
            }
        }

        let _ = windows::Win32::System::Services::CloseServiceHandle(scm);
    }

    cache
}

fn widestring_to_string(ptr: windows::core::PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let ptr_raw = ptr.as_ptr();
    let len = unsafe {
        let mut len = 0;
        while *ptr_raw.add(len) != 0 {
            len += 1;
        }
        len
    };
    if len == 0 {
        return String::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr_raw, len) };
    std::ffi::OsString::from_wide(slice)
        .to_string_lossy()
        .into_owned()
}

fn get_service_cache() -> &'static HashMap<u32, String> {
    SERVICE_CACHE.get_or_init(build_service_cache)
}

pub fn classify_process(proc: &ProcessInfo) -> ProcessClass {
    if proc.pid == 4 || proc.pid == 0 {
        return ProcessClass::Kernel;
    }

    let name_lower = proc.name.to_lowercase();
    if SYSTEM_PROCESS_NAMES.iter().any(|&s| s == name_lower) {
        return ProcessClass::SystemProcess;
    }

    let cache = get_service_cache();
    if cache.contains_key(&proc.pid) {
        return ProcessClass::WindowsService;
    }

    ProcessClass::UserApp
}

pub fn classify_batch(processes: &[ProcessInfo]) -> Vec<(ProcessClass, &ProcessInfo)> {
    processes.iter().map(|p| (classify_process(p), p)).collect()
}

pub fn classify_count(processes: &[ProcessInfo]) -> ClassifyCount {
    let mut count = ClassifyCount::default();
    for p in processes {
        match classify_process(p) {
            ProcessClass::UserApp => count.user += 1,
            ProcessClass::SystemProcess => count.system += 1,
            ProcessClass::WindowsService => count.service += 1,
            ProcessClass::Kernel => count.kernel += 1,
            ProcessClass::Unknown => count.unknown += 1,
        }
    }
    count
}

#[derive(Debug, Default)]
pub struct ClassifyCount {
    pub user: usize,
    pub system: usize,
    pub service: usize,
    pub kernel: usize,
    pub unknown: usize,
}
