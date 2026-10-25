//! 总线基础类型：访问宽度、内存访问类别与总线错误。

use core::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSize {
    Byte,
    Half,
    Word,
}

impl AccessSize {
    /// 字节数。
    #[inline]
    pub fn bytes(self) -> u32 {
        match self {
            AccessSize::Byte => 1,
            AccessSize::Half => 2,
            AccessSize::Word => 4,
        }
    }
    /// 对齐检查所需掩码。
    #[inline]
    pub fn align_mask(self) -> u32 {
        self.bytes() - 1
    }
}

impl fmt::Display for AccessSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessSize::Byte => write!(f, "b"),
            AccessSize::Half => write!(f, "h"),
            AccessSize::Word => write!(f, "w"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BusError {
    #[error("unmapped address {addr:#x}")]
    Unmapped { addr: u32 },
    #[error("misaligned {size} access at {addr:#x}")]
    Misaligned { addr: u32, size: AccessSize },
    #[error("device error at {addr:#x}: {msg}")]
    Device { addr: u32, msg: String },
    /// 注册地址区间与既有设备重叠（builder 阶段）。
    #[error("device range {start:#x}..{end:#x} overlaps existing mapping")]
    Overlap { start: u32, end: u32 },
}

/// 设备侧事件类型（存储访问的类别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemKind {
    /// 指令取（instruction fetch）。
    Inst,
    /// 数据读（load）。
    DataRead,
    /// 数据写（store）。
    DataWrite,
}

impl fmt::Display for MemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemKind::Inst => write!(f, "IF"),
            MemKind::DataRead => write!(f, "DR"),
            MemKind::DataWrite => write!(f, "DW"),
        }
    }
}
