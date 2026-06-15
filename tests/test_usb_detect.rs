use proc::classify::ProcessClass;
use proc::eject::classify::{self, HandleRisk};
use proc::eject::device::format_size;
use proc::eject::locks::HandleLock;

fn make_lock(pid: u32, name: &str, process_class: ProcessClass) -> HandleLock {
    HandleLock {
        pid,
        process_name: name.to_string(),
        exe_path: None,
        process_class,
        port_info: Vec::new(),
    }
}

#[test]
fn test_detect_removable_devices_no_panic() {
    let result = proc::eject::device::detect_removable_devices();
    assert!(result.is_ok());
}

#[test]
fn test_classify_handle_system_pid() {
    let lock = make_lock(4, "System", ProcessClass::Kernel);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Critical);
}

#[test]
fn test_classify_handle_explorer() {
    let lock = make_lock(1234, "explorer.exe", ProcessClass::UserApp);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Warning);
}

#[test]
fn test_classify_handle_search_indexer() {
    let lock = make_lock(5678, "SearchIndexer.exe", ProcessClass::UserApp);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Warning);
}

#[test]
fn test_classify_handle_msmpeng() {
    let lock = make_lock(9999, "MsMpEng.exe", ProcessClass::UserApp);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Warning);
}

#[test]
fn test_classify_handle_user_app() {
    let lock = make_lock(8080, "notepad.exe", ProcessClass::UserApp);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Safe);
}

#[test]
fn test_classify_handle_system_process() {
    let lock = make_lock(600, "svchost.exe", ProcessClass::SystemProcess);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Warning);
}

#[test]
fn test_classify_handle_kernel_process() {
    let lock = make_lock(0, "Idle", ProcessClass::Kernel);
    let risk = classify::classify_handle(&lock);
    assert_eq!(risk, HandleRisk::Critical);
}

#[test]
fn test_handle_risk_label() {
    assert_eq!(HandleRisk::Critical.label(), "🔴 关键");
    assert_eq!(HandleRisk::Warning.label(), "🟡 警告");
    assert_eq!(HandleRisk::Safe.label(), "🟢 安全");
    assert_eq!(HandleRisk::Harmless.label(), "⚪ 无害");
}

#[test]
fn test_handle_risk_color() {
    use ratatui::style::Color;
    assert_eq!(HandleRisk::Critical.color(), Color::Red);
    assert_eq!(HandleRisk::Warning.color(), Color::Yellow);
    assert_eq!(HandleRisk::Safe.color(), Color::Green);
    assert_eq!(HandleRisk::Harmless.color(), Color::DarkGray);
}

#[test]
fn test_handle_risk_description() {
    assert!(!HandleRisk::Critical.description().is_empty());
    assert!(!HandleRisk::Safe.description().is_empty());
}

#[test]
fn test_get_risk_label() {
    let (label, color) = classify::get_risk_label(HandleRisk::Critical);
    assert_eq!(label, "🔴 关键");
    let _ = color;
}

#[test]
fn test_risk_weight_ordering() {
    assert!(
        classify::risk_weight(HandleRisk::Critical) > classify::risk_weight(HandleRisk::Warning)
    );
    assert!(classify::risk_weight(HandleRisk::Warning) > classify::risk_weight(HandleRisk::Safe));
    assert!(classify::risk_weight(HandleRisk::Safe) > classify::risk_weight(HandleRisk::Harmless));
}

#[test]
fn test_format_size() {
    assert_eq!(format_size(500), "500B");
    assert_eq!(format_size(1024), "1KB");
    assert_eq!(format_size(1024 * 512), "512KB");
    assert_eq!(format_size(1024 * 1024), "1MB");
    assert_eq!(format_size(1024 * 1024 * 1024), "1.0GB");
}

#[test]
fn test_flush_write_cache_no_panic() {
    let result = proc::eject::cache::flush_write_cache('Z');
    assert!(result.is_ok());
}
