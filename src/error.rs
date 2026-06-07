use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProcError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Sysinfo error: {0}")]
    Sysinfo(String),

    #[error("Port scan error: {0}")]
    PortScan(String),

    #[error("USB detection error: {0}")]
    UsbDetect(String),

    #[error("Monitor error: {0}")]
    Monitor(String),

    #[error("Docker error: {0}")]
    Docker(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

pub type Result<T> = std::result::Result<T, ProcError>;
