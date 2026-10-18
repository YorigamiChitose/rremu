//! rremu-cpu：CPU 状态与单周期执行内核。只依赖 bus trait，不感知具体设备。

use rremu_bus::{Bus, EventSink};
use rremu_decode::{decode16, decode32, instr_len, DecodeError};
use rremu_isa::{cause, Instruction, Priv};
use thiserror::Error;

mod csr;
mod exec;
mod fetch;
mod interrupt;
mod trap;
mod mmu;
mod pmp;

pub use csr::CsrState;
pub(crate) use trap::TrapErr;
pub(crate) use mmu::AccessType;
#[cfg(test)]
pub(crate) use mmu::{PTE_A, PTE_D, PTE_R, PTE_U, PTE_V, PTE_W, PTE_X};

/// 单步结果：正常执行一条、陷入，或 ecall+a7=EXIT_MAGIC 触发宿主退出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// `bits` 为该指令的原始编码（len==2 时低 16 位，len==4 时全 32 位），供 trace/dump。
    Executed { instr: Instruction, len: u8, bits: u32 },
    Trapped { cause: u32 },
    /// 执行 ecall 且 a7==EXIT_MAGIC：以 a0（x10）为退出码终止模拟（宿主陷阱协议）。
    Exited { code: i32 },
}

/// 宿主陷阱协议魔术数：M-mode ecall 且 `a7 == EXIT_MAGIC` 时模拟器以 `a0` 为退出码退出。
/// 选 0xCAFE_0000 是 rremu 专用值，避开 Linux syscall ABI（a7=0..~300）与 AM yield（a7=-1）。
pub const EXIT_MAGIC: u32 = 0xCAFE_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CpuError {
    #[error("unimplemented feature: {0}")]
    Unimplemented(&'static str),
}

/// 单周期 CPU：一条指令 = 一个 cycle。
#[derive(Debug, Clone)]
pub struct Cpu {
    x: [u32; 32],
    pc: u32,
    priv_: Priv,
    csr: CsrState,
    cycle: u64,
    instret: u64,
    /// lr.w 保留地址（A 扩展）；None 表示无有效保留。
    reservation: Option<u32>,
}

impl Cpu {
    pub fn new(entry: u32) -> Self {
        Self { x: [0; 32], pc: entry, priv_: Priv::Machine, csr: CsrState::new(), cycle: 0, instret: 0, reservation: None }
    }

    // ---- 只读访问（machine 快照用） ----
    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn reg(&self, i: u8) -> u32 {
        self.x[usize::from(i)]
    }
    pub fn regs(&self) -> &[u32; 32] {
        &self.x
    }
    pub fn priv_(&self) -> Priv {
        self.priv_
    }
    pub fn cycle(&self) -> u64 {
        self.cycle
    }
    pub fn instret(&self) -> u64 {
        self.instret
    }
    pub fn csr_state(&self) -> &CsrState {
        &self.csr
    }

    /// 总线错误 → 异常 cause：对齐违例返 `misalign`，其余（未映射/设备）返 `fault`。
    pub(crate) fn cause_of(e: &rremu_bus::BusError, misalign: u32, fault: u32) -> u32 {
        if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
            misalign
        } else {
            fault
        }
    }

    /// 复位到入口（寄存器/CSR/计数器清零，MPP=M）。
    pub fn reset(&mut self, entry: u32) {
        self.x = [0; 32];
        self.pc = entry;
        self.priv_ = Priv::Machine;
        self.csr = CsrState::new();
        self.cycle = 0;
        self.instret = 0;
        self.reservation = None;
    }

    /// 注入启动参数到 `a0`(x10)/`a1`(x11)（qemu-virt 约定：hartid / dtb）。
    pub fn set_args(&mut self, a0: u32, a1: u32) {
        self.x[10] = a0;
        self.x[11] = a1;
    }

    /// 取指 → 解码 → 执行 → 写回；错误路径统一走 trap。
    ///
    /// 陷阱优先级（取指异常优先于中断）：先取指（含对齐检查），取指成功后再检查
    /// 中断（三层 AND 门控 —— mstatus.MIE & mie 对应位 & 硬件 pending）。
    /// 设备侧 `check_interrupt()` 返回全部 pending cause（不裁剪优先级）；
    /// CPU 按 spec 序 MEI>MSI>MTI 遍历，跳过未使能者，取第一个已使能的中断。
    /// 中断命中时 mepc = 当前 pc（未执行的下一条指令），mret 后重新执行。
    pub fn step<B: Bus, E: EventSink>(&mut self, bus: &mut B, sink: &mut E) -> Result<StepOutcome, CpuError> {
        bus.set_cycle(self.cycle);
        let pc = self.pc;

        // 取指前先做对齐检查：取指 misaligned 优先于中断
        if pc & 0x1 != 0 {
            return self.trap(sink, cause::INSTR_MISALIGNED, pc);
        }

        // 取指单元(IFU)：取指 access fault 优先于中断
        let word = match self.fetch_word(bus, pc, sink) {
            Ok(w) => w,
            Err(c) => return self.trap(sink, c, pc),
        };

        // 设备 pending 同步到 mip（软件读 mip 可见），并在取指成功后检查中断
        let pending = bus.service_interrupts();
        self.sync_mip(&pending);
        if let Some(cause) = self.pending_interrupt() {
            return self.trap(sink, cause, 0);
        }

        let lo = word as u16;

        let (instr, len) = if instr_len(lo) == 2 {
            // 16 位压缩指令（mtval 为 16 位编码本身）
            match decode16(lo, pc) {
                Ok(i) => (i, 2u8),
                Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                    return self.trap(sink, cause::ILLEGAL_INSTR, u32::from(lo))
                }
            }
        } else {
            // 32 位指令：word 已包含完整 4 字节，直接解码
            match decode32(word, pc) {
                Ok(i) => (i, 4u8),
                Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                    return self.trap(sink, cause::ILLEGAL_INSTR, word)
                }
            }
        };

        match self.exec(bus, sink, pc, instr, len, if len == 2 { word & 0xffff } else { word }) {
            Ok(next_pc) => {
                self.pc = next_pc;
                self.cycle += 1;
                self.instret += 1;
                let bits = if len == 2 { word & 0xffff } else { word };
                Ok(StepOutcome::Executed { instr, len, bits })
            }
            Err(TrapErr { cause, mtval }) => {
                // ecall+a7=EXIT_MAGIC：宿主陷阱协议——以 a0(x10) 为退出码终止模拟。
                // 退出指令也计入一个 cycle 与 instret。
                if cause == cause::ECALL_FROM_M && self.x[17] == EXIT_MAGIC {
                    let code = self.x[10] as i32;
                    self.cycle += 1;
                    self.instret += 1;
                    return Ok(StepOutcome::Exited { code });
                }
                self.trap(sink, cause, mtval)
            }
        }
    }


}

#[cfg(test)]
mod tests;
