use std::error::Error as StdError;

use thiserror::Error;

/// 进程工具统一错误类型。
///
/// 各变体使用 struct form 保留底层错误链（`#[source]`），通过 `ProcError::sysinfo`
/// 等辅助构造函数调用，避免调用站点手写 struct 字面量。
#[derive(Error, Debug)]
pub enum ProcError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Sysinfo error: {message}")]
    Sysinfo {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("Port scan error: {message}")]
    PortScan {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("USB detection error: {message}")]
    UsbDetect {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("Monitor error: {message}")]
    Monitor {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("Docker error: {message}")]
    Docker {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("Not found: {message}")]
    NotFound {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("Permission denied: {message}")]
    PermissionDenied {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },

    #[error("SMART error: {message}")]
    Smart {
        message: String,
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },
}

impl ProcError {
    pub fn sysinfo(msg: impl Into<String>) -> Self {
        Self::Sysinfo {
            message: msg.into(),
            source: None,
        }
    }

    pub fn sysinfo_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Sysinfo {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn port_scan(msg: impl Into<String>) -> Self {
        Self::PortScan {
            message: msg.into(),
            source: None,
        }
    }

    pub fn port_scan_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::PortScan {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn usb_detect(msg: impl Into<String>) -> Self {
        Self::UsbDetect {
            message: msg.into(),
            source: None,
        }
    }

    pub fn usb_detect_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::UsbDetect {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn monitor(msg: impl Into<String>) -> Self {
        Self::Monitor {
            message: msg.into(),
            source: None,
        }
    }

    pub fn monitor_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Monitor {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn docker(msg: impl Into<String>) -> Self {
        Self::Docker {
            message: msg.into(),
            source: None,
        }
    }

    pub fn docker_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Docker {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound {
            message: msg.into(),
            source: None,
        }
    }

    pub fn not_found_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::NotFound {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self::PermissionDenied {
            message: msg.into(),
            source: None,
        }
    }

    pub fn permission_denied_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::PermissionDenied {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn smart(msg: impl Into<String>) -> Self {
        Self::Smart {
            message: msg.into(),
            source: None,
        }
    }

    pub fn smart_with(
        msg: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Smart {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn smart_msg(msg: impl Into<String>) -> Self {
        Self::Smart {
            message: msg.into(),
            source: None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ProcError>;
