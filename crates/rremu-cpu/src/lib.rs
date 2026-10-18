//! rremu-cpu：CPU 状态与单周期执行内核。只依赖 bus trait，不感知具体设备。

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_decode::{decode16, decode32, instr_len, DecodeError};
use rremu_isa::{cause, csr as isa_csr, Instruction, Priv};
use thiserror::Error;

mod csr;
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
        let pending = bus.pending_interrupts();
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

    /// 执行一条已解码指令；成功返回下一 pc，失败返回 trap 原因。
    fn exec<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        pc: u32,
        instr: Instruction,
        len: u8,
        raw: u32,
    ) -> Result<u32, TrapErr> {
        let x = |c: &Self, i: u8| c.x[usize::from(i)];
        let mut next_pc = pc.wrapping_add(len as u32);
        macro_rules! set {
            ($rd:expr, $v:expr) => {{
                let v: u32 = $v;
                if $rd != 0 {
                    self.x[usize::from($rd)] = v;
                }
            }};
        }

        match instr {
            Instruction::Lui { rd, imm } => set!(rd, imm),
            Instruction::Auipc { rd, imm } => set!(rd, pc.wrapping_add(imm)),

            Instruction::Jal { rd, imm } => {
                set!(rd, pc.wrapping_add(4));
                next_pc = pc.wrapping_add(imm as u32);
            }
            Instruction::Jalr { rd, rs1, imm } => {
                let t = pc.wrapping_add(4);
                next_pc = x(self, rs1).wrapping_add(imm as u32) & !1;
                set!(rd, t);
            }

            Instruction::Beq { rs1, rs2, imm } => {
                if x(self, rs1) == x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bne { rs1, rs2, imm } => {
                if x(self, rs1) != x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Blt { rs1, rs2, imm } => {
                if (x(self, rs1) as i32) < (x(self, rs2) as i32) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bge { rs1, rs2, imm } => {
                if (x(self, rs1) as i32) >= (x(self, rs2) as i32) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bltu { rs1, rs2, imm } => {
                if x(self, rs1) < x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bgeu { rs1, rs2, imm } => {
                if x(self, rs1) >= x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }

            Instruction::Lb { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Byte)?;
                set!(rd, (v as u8) as i8 as i32 as u32)
            }
            Instruction::Lh { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Half)?;
                set!(rd, (v as u16) as i16 as i32 as u32)
            }
            Instruction::Lw { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word)?;
                set!(rd, v)
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Byte)?;
                set!(rd, v & 0xff)
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Half)?;
                set!(rd, v & 0xffff)
            }

            Instruction::Sb { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Byte, x(self, rs2))?
            }
            Instruction::Sh { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Half, x(self, rs2))?
            }
            Instruction::Sw { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word, x(self, rs2))?
            }

            Instruction::Addi { rd, rs1, imm } => set!(rd, x(self, rs1).wrapping_add(imm as u32)),
            Instruction::Slti { rd, rs1, imm } => set!(rd, u32::from((x(self, rs1) as i32) < imm)),
            Instruction::Sltiu { rd, rs1, imm } => set!(rd, u32::from(x(self, rs1) < imm as u32)),
            Instruction::Xori { rd, rs1, imm } => set!(rd, x(self, rs1) ^ imm as u32),
            Instruction::Ori { rd, rs1, imm } => set!(rd, x(self, rs1) | imm as u32),
            Instruction::Andi { rd, rs1, imm } => set!(rd, x(self, rs1) & imm as u32),
            Instruction::Slli { rd, rs1, shamt } => set!(rd, x(self, rs1) << shamt),
            Instruction::Srli { rd, rs1, shamt } => set!(rd, x(self, rs1) >> shamt),
            Instruction::Srai { rd, rs1, shamt } => set!(rd, ((x(self, rs1) as i32) >> shamt) as u32),

            Instruction::Add { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_add(x(self, rs2))),
            Instruction::Sub { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_sub(x(self, rs2))),
            Instruction::Sll { rd, rs1, rs2 } => set!(rd, x(self, rs1) << (x(self, rs2) & 0x1f)),
            Instruction::Slt { rd, rs1, rs2 } => set!(rd, u32::from((x(self, rs1) as i32) < (x(self, rs2) as i32))),
            Instruction::Sltu { rd, rs1, rs2 } => set!(rd, u32::from(x(self, rs1) < x(self, rs2))),
            Instruction::Xor { rd, rs1, rs2 } => set!(rd, x(self, rs1) ^ x(self, rs2)),
            Instruction::Srl { rd, rs1, rs2 } => set!(rd, x(self, rs1) >> (x(self, rs2) & 0x1f)),
            Instruction::Sra { rd, rs1, rs2 } => set!(rd, ((x(self, rs1) as i32) >> (x(self, rs2) & 0x1f)) as u32),
            Instruction::Or { rd, rs1, rs2 } => set!(rd, x(self, rs1) | x(self, rs2)),
            Instruction::And { rd, rs1, rs2 } => set!(rd, x(self, rs1) & x(self, rs2)),

            // ---- M extension (RV32M) ----
            Instruction::Mul { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_mul(x(self, rs2))),
            Instruction::Mulh { rd, rs1, rs2 } => {
                let a = (x(self, rs1) as i32) as i64;
                let b = (x(self, rs2) as i32) as i64;
                set!(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Mulhsu { rd, rs1, rs2 } => {
                let a = (x(self, rs1) as i32) as i64;
                let b = x(self, rs2) as i64; // rs2 无符号，零扩展
                set!(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Mulhu { rd, rs1, rs2 } => {
                let a = x(self, rs1) as u64;
                let b = x(self, rs2) as u64;
                set!(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Div { rd, rs1, rs2 } => {
                let a = x(self, rs1) as i32;
                let b = x(self, rs2) as i32;
                let result = a.checked_div(b).map(|v| v as u32).unwrap_or_else(|| {
                    // 除零返回 -1；INT_MIN / -1 溢出返回 INT_MIN
                    if a == i32::MIN && b == -1 { i32::MIN as u32 } else { u32::MAX }
                });
                set!(rd, result);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let divisor = x(self, rs2);
                let result = x(self, rs1).checked_div(divisor).unwrap_or(u32::MAX);
                set!(rd, result);
            }
            Instruction::Rem { rd, rs1, rs2 } => {
                let a = x(self, rs1) as i32;
                let b = x(self, rs2) as i32;
                let result = a.checked_rem(b).map(|v| v as u32).unwrap_or_else(|| {
                    // 除零返回 rs1；INT_MIN % -1 溢出返回 0
                    if a == i32::MIN && b == -1 { 0 } else { x(self, rs1) }
                });
                set!(rd, result);
            }
            Instruction::Remu { rd, rs1, rs2 } => {
                let divisor = x(self, rs2);
                let result = x(self, rs1).checked_rem(divisor).unwrap_or_else(|| x(self, rs1));
                set!(rd, result);
            }

            // ---- A extension (RV32A) ----
            Instruction::LrW { rd, rs1 } => {
                let addr = x(self, rs1);
                let v = self.load(bus, sink, addr, AccessSize::Word)?;
                self.reservation = Some(addr);
                set!(rd, v);
            }
            Instruction::ScW { rd, rs1, rs2 } => {
                let addr = x(self, rs1);
                if self.reservation == Some(addr) {
                    self.store(bus, sink, addr, AccessSize::Word, x(self, rs2))?;
                    set!(rd, 0);
                } else {
                    set!(rd, 1);
                }
                self.reservation = None;
            }
            Instruction::AmoswapW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |_, b| b)?;
            }
            Instruction::AmoaddW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a.wrapping_add(b))?;
            }
            Instruction::AmoxorW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a ^ b)?;
            }
            Instruction::AmoandW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a & b)?;
            }
            Instruction::AmoorW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a | b)?;
            }
            Instruction::AmominW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| (a as i32).min(b as i32) as u32)?;
            }
            Instruction::AmomaxW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| (a as i32).max(b as i32) as u32)?;
            }
            Instruction::AmominuW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a.min(b))?;
            }
            Instruction::AmomaxuW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a.max(b))?;
            }

            Instruction::Sret => {
                let sstatus = self.csr.mstatus;
                let spp = (sstatus >> isa_csr::MSTATUS_SPP.trailing_zeros()) & 1;
                let spie = (sstatus >> isa_csr::MSTATUS_SPIE.trailing_zeros()) & 1;
                // SPP=0, SIE=SPIE, SPIE=1
                self.csr.mstatus &= !(isa_csr::MSTATUS_SPP | isa_csr::MSTATUS_SIE | isa_csr::MSTATUS_SPIE);
                self.csr.mstatus |= (spie << isa_csr::MSTATUS_SIE.trailing_zeros())
                    | (1 << isa_csr::MSTATUS_SPIE.trailing_zeros());
                self.priv_ = if spp == 1 { Priv::Supervisor } else { Priv::User };
                next_pc = self.csr.sepc;
            }
            Instruction::Wfi => {} // 无等待实现：NOP
            Instruction::SfenceVma => {} // 无 TLB：NOP

            Instruction::Fence => {}
            Instruction::FenceI => {}
            Instruction::Ecall => {
                let c = match self.priv_ {
                    Priv::Machine => cause::ECALL_FROM_M,
                    Priv::Supervisor => cause::ECALL_FROM_S,
                    Priv::User => cause::ECALL_FROM_U,
                };
                return Err(TrapErr { cause: c, mtval: 0 });
            }
            Instruction::Ebreak => return Err(TrapErr { cause: cause::BREAKPOINT, mtval: 0 }),
            Instruction::Csrrw { rd, csr, rs1 } => {
                self.check_csr_access(csr, true, raw)?;
                let old = self.csr_read(csr);
                self.csr_write(csr, x(self, rs1));
                set!(rd, old);
            }
            Instruction::Csrrs { rd, csr, rs1 } => {
                let mask = x(self, rs1);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrc { rd, csr, rs1 } => {
                let mask = x(self, rs1);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrwi { rd, csr, zimm } => {
                self.check_csr_access(csr, true, raw)?;
                let old = self.csr_read(csr);
                self.csr_write(csr, u32::from(zimm));
                set!(rd, old);
            }
            Instruction::Csrrsi { rd, csr, zimm } => {
                let mask = u32::from(zimm);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrci { rd, csr, zimm } => {
                let mask = u32::from(zimm);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Mret => {
                let mstatus = self.csr.mstatus;
                let mpp = (mstatus >> isa_csr::MSTATUS_MPP_SHIFT) & 0b11;
                let mpie = (mstatus >> isa_csr::MSTATUS_MPIE.trailing_zeros()) & 1;
                // MPP = 0 (U-mode), MIE = MPIE, MPIE = 1
                self.update_mstatus(0, mpie, 1);
                self.priv_ = match mpp {
                    0b11 => Priv::Machine,
                    0b01 => Priv::Supervisor,
                    _ => Priv::User,
                };
                next_pc = self.csr.mepc;
            }
            // ---- C extension (RV32C) ----
            Instruction::CAddi4spn { rd, imm } => set!(rd, x(self, 2u8).wrapping_add(imm)),
            Instruction::CLw { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word)?;
                set!(rd, v)
            }
            Instruction::CSw { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word, x(self, rs2))?
            }
            Instruction::CAddi { rd, imm } => set!(rd, x(self, rd).wrapping_add(imm as u32)),
            Instruction::CJal { imm } => {
                set!(1u8, pc.wrapping_add(2u32));
                next_pc = pc.wrapping_add(imm as u32);
            }
            Instruction::CLi { rd, imm } => set!(rd, imm as u32),
            Instruction::CAddi16sp { imm } => set!(2u8, x(self, 2u8).wrapping_add(imm as u32)),
            Instruction::CLui { rd, imm } => set!(rd, imm),
            Instruction::CSrli { rd, shamt } => set!(rd, x(self, rd) >> (shamt & 0x1f)),
            Instruction::CSrai { rd, shamt } => set!(rd, ((x(self, rd) as i32) >> (shamt & 0x1f)) as u32),
            Instruction::CAndi { rd, imm } => set!(rd, x(self, rd) & imm as u32),
            Instruction::CSub { rd, rs2 } => set!(rd, x(self, rd).wrapping_sub(x(self, rs2))),
            Instruction::CXor { rd, rs2 } => set!(rd, x(self, rd) ^ x(self, rs2)),
            Instruction::COr { rd, rs2 } => set!(rd, x(self, rd) | x(self, rs2)),
            Instruction::CAnd { rd, rs2 } => set!(rd, x(self, rd) & x(self, rs2)),
            Instruction::CJ { imm } => next_pc = pc.wrapping_add(imm as u32),
            Instruction::CBeqz { rs1, imm } => {
                if x(self, rs1) == 0 {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CBnez { rs1, imm } => {
                if x(self, rs1) != 0 {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CSlli { rd, shamt } => set!(rd, x(self, rd) << (shamt & 0x1f)),
            Instruction::CLwsp { rd, imm } => {
                let v = self.load(bus, sink, x(self, 2u8).wrapping_add(imm as u32), AccessSize::Word)?;
                set!(rd, v)
            }
            Instruction::CJr { rs1 } => next_pc = x(self, rs1) & !1,
            Instruction::CMv { rd, rs2 } => set!(rd, x(self, rs2)),
            Instruction::CEbreak => return Err(TrapErr { cause: cause::BREAKPOINT, mtval: 0 }),
            Instruction::CJalr { rs1 } => {
                set!(1u8, pc.wrapping_add(2u32));
                next_pc = x(self, rs1) & !1;
            }
            Instruction::CAdd { rd, rs2 } => set!(rd, x(self, rd).wrapping_add(x(self, rs2))),
            Instruction::CSwsp { rs2, imm } => {
                self.store(bus, sink, x(self, 2u8).wrapping_add(imm as u32), AccessSize::Word, x(self, rs2))?
            }
        }
        Ok(next_pc)
    }

    #[inline]
    fn load<B: Bus>(
        &mut self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        size: AccessSize,
    ) -> Result<u32, TrapErr> {
        let pa = self.translate(bus, sink, addr, AccessType::Load)?;
        if !self.pmp_check(pa, AccessType::Load) {
            return Err(TrapErr { cause: cause::LOAD_ACCESS_FAULT, mtval: addr });
        }
        bus.data_read(pa, size, sink).map_err(|e| TrapErr {
            cause: Self::cause_of(&e, cause::LOAD_MISALIGNED, cause::LOAD_ACCESS_FAULT),
            mtval: addr,
        })
    }

    #[inline]
    fn store<B: Bus>(
        &mut self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        size: AccessSize,
        data: u32,
    ) -> Result<(), TrapErr> {
        self.reservation = None; // 任何 store 都使 lr.w 保留失效
        let pa = self.translate(bus, sink, addr, AccessType::Store)?;
        if !self.pmp_check(pa, AccessType::Store) {
            return Err(TrapErr { cause: cause::STORE_ACCESS_FAULT, mtval: addr });
        }
        bus.data_write(pa, size, data, sink).map_err(|e| TrapErr {
            cause: Self::cause_of(&e, cause::STORE_MISALIGNED, cause::STORE_ACCESS_FAULT),
            mtval: addr,
        })
    }

    /// AMO 原子读-改-写：读旧值 → 计算新值 → 写回 → rd = 旧值。
    /// 单核单线程下无需总线锁，直接 load + store 即可保持原子性。
    fn amo<B: Bus, E: EventSink, F: Fn(u32, u32) -> u32>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        rd: u8,
        rs1: u8,
        rs2: u8,
        f: F,
    ) -> Result<(), TrapErr> {
        let addr = self.x[usize::from(rs1)];
        let val = self.x[usize::from(rs2)];
        let old = self.load(bus, sink, addr, AccessSize::Word)?;
        let new = f(old, val);
        self.store(bus, sink, addr, AccessSize::Word, new)?;
        if rd != 0 {
            self.x[usize::from(rd)] = old;
        }
        Ok(())
    }

}

#[cfg(test)]
mod tests;
