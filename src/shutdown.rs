use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// Registers the global Ctrl+C / SIGINT handler. Safe to call multiple times;
/// only the first call installs the handler.
pub fn init() {
    FLAG.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_for_handler = flag.clone();
        // Best-effort registration. ctrlc may fail on some platforms; we
        // still return the flag so callers can poll it (it just won't flip).
        let _ = ctrlc::set_handler(move || {
            flag_for_handler.store(true, Ordering::SeqCst);
        });
        flag
    });
}

/// Returns true if Ctrl+C / SIGINT has been received since process start.
/// Always false if [`init`] was never called.
pub fn requested() -> bool {
    FLAG.get()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
}
