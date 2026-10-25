//! 机器快照与 trace 条目。

use std::collections::VecDeque;

use rremu_bus::EmuEvent;
use rremu_isa::{Address, Priv, Word};

use crate::RunState;

/// 指令 trace 条目（dump 用）：pc + 原始编码 + 长度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceEntry {
    pub pc: u32,
    pub bits: u32,
    pub len: u8,
}

/// CSR 快照（TUI/CLI 显示所需的 M-mode + FP 集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsrSnapshot {
    pub mstatus: u32,
    pub mtvec: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mie: u32,
    pub mip: u32,
    /// `fcsr`：fflags 与 frm 的合并视图。
    pub fcsr: u32,
    pub mcycle: u64,
    pub minstret: u64,
}

/// 机器快照：前端的唯一状态视图。
#[derive(Debug, Clone)]
pub struct MachineSnapshot {
    pub cycle: u64,
    pub pc: Address,
    pub priv_: Priv,
    pub gprs: [Word; 32],
    /// 原始 64 位浮点寄存器（FLEN=64）。
    pub fprs: [u64; 32],
    pub csrs: CsrSnapshot,
    pub state: RunState,
    pub mem_ring: VecDeque<EmuEvent>,
}
