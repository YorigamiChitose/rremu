//! 配置错误。

use core::fmt;

use crate::defaults::DTB_RESERVE;

/// 配置错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    ZeroRamSize,
    RamOverflow { base: u32, size: u32 },
    FbZeroDim { width: u32, height: u32 },
    FbSizeOverflow { width: u32, height: u32, bpp: u32 },
    DtbReserveTooLarge { ram_size: u32 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::ZeroRamSize => write!(f, "ram size must be non-zero"),
            ConfigError::RamOverflow { base, size } => {
                write!(f, "ram range {base:#x}+{size:#x} overflows address space")
            }
            ConfigError::FbZeroDim { width, height } => {
                write!(f, "framebuffer dimensions must be non-zero ({width}x{height})")
            }
            ConfigError::FbSizeOverflow { width, height, bpp } => {
                write!(f, "framebuffer size overflow ({width}x{height}x{bpp})")
            }
            ConfigError::DtbReserveTooLarge { ram_size } => {
                write!(f, "ram size {ram_size:#x} smaller than DTB reserve {DTB_RESERVE:#x}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

