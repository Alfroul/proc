use std::collections::HashMap;

use crate::collect::ProcessInfo;

pub struct VersionInfo {
    pub product_name: Option<String>,
    pub company_name: Option<String>,
    pub file_description: Option<String>,
}

#[derive(Clone)]
pub struct AppGroup {
    pub display_name: String,
    pub exe_dir: String,
    pub processes: Vec<AppGroupProcess>,
    pub total_cpu: f32,
    pub total_memory: u64,
}

#[derive(Clone)]
pub struct AppGroupProcess {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
    pub role_hint: Option<String>,
}

/// Visual item in the flat list for cursor navigation.
#[derive(Clone, Copy)]
pub enum AppGroupItem {
    Header { group_idx: usize },
    Child { group_idx: usize, child_idx: usize },
}

/// Build the flat visual item list from groups and expanded state.
#[must_use]
pub fn build_visual_items(groups: &[AppGroup], expanded: Option<usize>) -> Vec<AppGroupItem> {
    let mut items = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        items.push(AppGroupItem::Header { group_idx: gi });
        if expanded == Some(gi) {
            for ci in 0..group.processes.len() {
                items.push(AppGroupItem::Child {
                    group_idx: gi,
                    child_idx: ci,
                });
            }
        }
    }
    items
}

// ── Version info query ──

#[cfg(target_os = "windows")]
#[must_use]
pub fn query_version_info(exe_path: &str) -> Option<VersionInfo> {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    use windows::core::PCWSTR;

    let _ = BOOL; // only used in some configs
    unsafe {
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let pw = PCWSTR::from_raw(wide.as_ptr());

        let size = GetFileVersionInfoSizeW(pw, None);
        if size == 0 {
            return None;
        }

        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(pw, 0, buf.len() as u32, buf.as_mut_ptr() as *mut _).is_err() {
            return None;
        }

        // Query translation table
        let trans_query: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;

        let ok = VerQueryValueW(
            buf.as_ptr() as *const _,
            PCWSTR::from_raw(trans_query.as_ptr()),
            &mut ptr,
            &mut len,
        );
        if ok == BOOL(0) || len < 4 {
            return None;
        }

        let lang = *(ptr as *const u16);
        let cp = *((ptr as *const u16).add(1));
        let lang_cp = format!("{:04x}{:04x}", lang, cp);

        Some(VersionInfo {
            product_name: ver_string(&buf, &lang_cp, "ProductName"),
            company_name: ver_string(&buf, &lang_cp, "CompanyName"),
            file_description: ver_string(&buf, &lang_cp, "FileDescription"),
        })
    }
}

#[cfg(target_os = "windows")]
fn ver_string(buf: &[u8], lang_cp: &str, key: &str) -> Option<String> {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Storage::FileSystem::VerQueryValueW;
    use windows::core::PCWSTR;

    unsafe {
        let query = format!("\\StringFileInfo\\{}\\{}\0", lang_cp, key);
        let qwide: Vec<u16> = query.encode_utf16().collect();

        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;

        let ok = VerQueryValueW(
            buf.as_ptr() as *const _,
            PCWSTR::from_raw(qwide.as_ptr()),
            &mut ptr,
            &mut len,
        );
        if ok == BOOL(0) || len == 0 {
            return None;
        }

        let slice = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        String::from_utf16(slice)
            .ok()
            .map(|s| s.trim_end_matches('\0').trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

#[cfg(not(target_os = "windows"))]
pub fn query_version_info(_exe_path: &str) -> Option<VersionInfo> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!("query_version_info is not supported on this platform; app grouping falls back to file name")
    });
    None
}

// ── Role hint inference ──

fn infer_role_hint(proc: &ProcessInfo) -> Option<String> {
    let name_lower = proc.name.to_lowercase();

    // Chromium/Electron --type= parameter
    for arg in proc.cmd.iter() {
        if let Some(rest) = arg.strip_prefix("--type=") {
            return Some(match rest {
                "renderer" => "renderer".to_string(),
                "gpu-process" => "gpu".to_string(),
                "utility" => "utility".to_string(),
                "broker" => "broker".to_string(),
                other => other.to_string(),
            });
        }
    }

    // Name-based heuristics
    if name_lower.contains("renderer") {
        return Some("renderer".to_string());
    }
    if name_lower.contains("gpu") || name_lower.contains("d3d") || name_lower.contains("dxva") {
        return Some("gpu".to_string());
    }
    if name_lower.contains("helper") || name_lower.contains("service") {
        return Some("helper".to_string());
    }
    if name_lower.contains("crashpad") || name_lower.contains("crash") {
        return Some("crashpad".to_string());
    }
    if name_lower.contains("watchdog") || name_lower.contains("monitor") {
        return Some("watchdog".to_string());
    }

    None
}

// ── Special case handling (Tier 4) ──

/// Returns a special group key for processes that need custom grouping.
/// Returns None for normal processes.
fn special_group_key(proc: &ProcessInfo) -> Option<String> {
    let name_lower = proc.name.to_lowercase();

    // vmmem / vmmemWSL → WSL group
    if name_lower == "vmmem.exe" || name_lower == "vmmemwsl.exe" || name_lower == "vmmem" {
        return Some("__wsl__".to_string());
    }

    // svchost.exe → group by -k parameter
    if name_lower == "svchost.exe" || name_lower == "svchost" {
        // Look for -k <group> or -k:<group> in command line
        let mut svc_group = None;
        for (i, arg) in proc.cmd.iter().enumerate() {
            let arg_lower = arg.to_lowercase();
            if arg_lower == "-k" || arg_lower == "/k" {
                // Next arg is the group name
                if let Some(next) = proc.cmd.get(i + 1) {
                    svc_group = Some(next.clone());
                }
                break;
            } else if arg_lower.starts_with("-k:")
                || arg_lower.starts_with("/k:")
                || arg_lower.starts_with("-k=")
                || arg_lower.starts_with("/k=")
            {
                let val = arg
                    .split_once(&[':', '='][..])
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim();
                if !val.is_empty() {
                    svc_group = Some(val.to_string());
                }
                break;
            }
        }
        return Some(format!(
            "__svchost_{}__",
            svc_group.unwrap_or_else(|| "default".to_string())
        ));
    }

    // java.exe / javaw.exe → parse -jar / main class
    if name_lower == "java.exe"
        || name_lower == "javaw.exe"
        || name_lower == "java"
        || name_lower == "javaw"
    {
        // Scan for -jar followed by arg
        let mut jar_name: Option<String> = None;
        let mut found_jar = false;
        for arg in proc.cmd.iter() {
            if found_jar {
                jar_name = Some(arg.clone());
                break;
            }
            if arg == "-jar" {
                found_jar = true;
            }
        }

        if let Some(jar) = jar_name {
            let name = std::path::Path::new(&jar)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(jar);
            return Some(format!("__java_{}__", name));
        }

        // Try last non-flag arg as main class
        for arg in proc.cmd.iter().rev() {
            if !arg.starts_with('-') && !arg.is_empty() {
                return Some(format!("__java_{}__", arg));
            }
        }

        return Some("__java_unknown__".to_string());
    }

    // python.exe / pythonw.exe → parse script path
    if name_lower.starts_with("python") {
        let script = proc.cmd.iter().find(|a| {
            !a.starts_with('-')
                && (a.ends_with(".py")
                    || a.ends_with(".pyw")
                    || std::path::Path::new(a).is_absolute())
        });
        if let Some(s) = script {
            let name = std::path::Path::new(s)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| s.clone());
            return Some(format!("__python_{}__", name));
        }
        return Some("__python_unknown__".to_string());
    }

    None
}

fn special_display_name(key: &str) -> Option<String> {
    if key == "__wsl__" {
        return Some("WSL".to_string());
    }
    if let Some(rest) = key
        .strip_prefix("__svchost_")
        .and_then(|s| s.strip_suffix("__"))
    {
        return Some(format!("Service: {}", rest));
    }
    if let Some(rest) = key
        .strip_prefix("__java_")
        .and_then(|s| s.strip_suffix("__"))
    {
        return Some(format!("Java: {}", rest));
    }
    if let Some(rest) = key
        .strip_prefix("__python_")
        .and_then(|s| s.strip_suffix("__"))
    {
        return Some(format!("Python: {}", rest));
    }
    None
}

// ── Main grouping function ──

pub fn compute_groups(
    procs: &[ProcessInfo],
    cache: &mut HashMap<String, Option<VersionInfo>>,
) -> Vec<AppGroup> {
    if procs.is_empty() {
        return Vec::new();
    }

    // Build pid→index map and parent→children map for Tier 3
    let mut ppid_children: HashMap<u32, Vec<u32>> = HashMap::new();
    for proc in procs {
        if let Some(ppid) = proc.parent_pid {
            ppid_children.entry(ppid).or_default().push(proc.pid);
        }
    }

    // Query version info for new exe paths
    let mut queried: std::collections::HashSet<String> = std::collections::HashSet::new();
    for proc in procs {
        if let Some(ref exe) = proc.exe
            && !queried.contains(exe.as_ref())
            && !cache.contains_key(exe.as_ref())
        {
            queried.insert((*exe).to_string());
            cache.insert((*exe).to_string(), query_version_info(exe));
        }
    }

    // Phase 1: Assign each process to a group key
    // Key → (group_key, display_name_override, dir)
    struct GroupMeta {
        key: String,
        display_override: Option<String>,
        exe_dir: String,
    }

    let mut proc_group_meta: Vec<GroupMeta> = Vec::with_capacity(procs.len());

    for proc in procs {
        // Tier 4: Check special cases first
        if let Some(skey) = special_group_key(proc) {
            let dname = special_display_name(&skey);
            proc_group_meta.push(GroupMeta {
                key: skey,
                display_override: dname,
                exe_dir: String::new(),
            });
            continue;
        }

        // Tier 1: Group by exe directory
        let exe_dir = proc
            .exe
            .as_ref()
            .and_then(|e| std::path::Path::new(&**e).parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        // Use ProductName for the key if available (Tier 2)
        let vinfo = proc
            .exe
            .as_ref()
            .and_then(|e| cache.get(&**e).and_then(|opt| opt.as_ref()));
        let tier2_key = vinfo
            .and_then(|v| v.product_name.as_ref())
            .filter(|pn| !pn.is_empty())
            .map(|pn| format!("__product_{}__", pn.to_lowercase()));

        let key = tier2_key.unwrap_or_else(|| format!("__dir_{}__", exe_dir.to_lowercase()));

        proc_group_meta.push(GroupMeta {
            key,
            display_override: None,
            exe_dir,
        });
    }

    // Phase 2: Build groups
    let mut group_map: HashMap<String, (Vec<usize>, Option<String>, String)> = HashMap::new();
    for (i, meta) in proc_group_meta.iter().enumerate() {
        let entry = group_map.entry(meta.key.clone()).or_insert_with(|| {
            (
                Vec::new(),
                meta.display_override.clone(),
                meta.exe_dir.clone(),
            )
        });
        entry.0.push(i);
        // If display override not set yet, take it
        if entry.1.is_none() && meta.display_override.is_some() {
            entry.1 = meta.display_override.clone();
        }
    }

    // Tier 3: Orphan processes whose parent is already in a group
    // Find single-process groups and check if their parent is in another group
    let mut reassign: Vec<(usize, String)> = Vec::new(); // (proc_idx, target_key)
    {
        let mut proc_to_key: HashMap<u32, String> = HashMap::new();
        for (i, meta) in proc_group_meta.iter().enumerate() {
            proc_to_key.insert(procs[i].pid, meta.key.clone());
        }

        for (key, (indices, _, _)) in &group_map {
            if indices.len() != 1 {
                continue;
            }
            let proc = &procs[indices[0]];
            if let Some(ppid) = proc.parent_pid
                && let Some(parent_key) = proc_to_key.get(&ppid)
                && parent_key != key
            {
                reassign.push((indices[0], parent_key.clone()));
            }
        }
    }

    for (proc_idx, target_key) in reassign {
        let old_key = &proc_group_meta[proc_idx].key;
        if let Some((indices, _, _)) = group_map.get_mut(old_key) {
            indices.retain(|&i| i != proc_idx);
        }
        if let Some((indices, _, _)) = group_map.get_mut(&target_key) {
            indices.push(proc_idx);
        }
    }

    // Remove empty groups
    group_map.retain(|_, (indices, _, _)| !indices.is_empty());

    // Phase 3: Build AppGroup structs with display names
    let mut groups: Vec<AppGroup> = group_map
        .into_iter()
        .map(|(_key, (indices, display_override, exe_dir))| {
            let processes: Vec<AppGroupProcess> = indices
                .iter()
                .map(|&i| {
                    let proc = &procs[i];
                    AppGroupProcess {
                        pid: proc.pid,
                        name: (*proc.name).to_string(),
                        cpu_usage: proc.cpu_usage,
                        memory: proc.memory,
                        role_hint: infer_role_hint(proc),
                    }
                })
                .collect();

            let total_cpu: f32 = processes.iter().map(|p| p.cpu_usage).sum();
            let total_memory: u64 = processes.iter().map(|p| p.memory).sum();

            // Display name: override > ProductName > FileDescription > dir name > exe name
            let display_name = display_override.unwrap_or_else(|| {
                // Try ProductName from version info
                let first_exe = indices.iter().find_map(|&i| procs[i].exe.as_ref());
                let vinfo = first_exe.and_then(|e| cache.get(&**e).and_then(|opt| opt.as_ref()));

                vinfo
                    .and_then(|v| v.product_name.clone())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        vinfo
                            .and_then(|v| v.file_description.clone())
                            .filter(|s| !s.is_empty())
                    })
                    .or_else(|| {
                        // Directory name
                        let dir_name = std::path::Path::new(&exe_dir)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string());
                        dir_name.filter(|s| !s.is_empty())
                    })
                    .or_else(|| {
                        // Exe file name (without extension)
                        processes.first().map(|p| {
                            std::path::Path::new(&p.name)
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| p.name.clone())
                        })
                    })
                    .unwrap_or_else(|| "Unknown".to_string())
            });

            AppGroup {
                display_name,
                exe_dir,
                processes,
                total_cpu,
                total_memory,
            }
        })
        .collect();

    // Sort by total_cpu descending
    groups.sort_by(|a, b| {
        b.total_cpu
            .partial_cmp(&a.total_cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    groups
}
