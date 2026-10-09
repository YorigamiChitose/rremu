//! rremu-cpu：CPU 状态与单周期执行内核。只依赖 bus trait，不感知具体设备。

use rremu_bus::{AccessSize, Bus, EmuEvent, EventSink};
use rremu_decode::{decode16, decode32, instr_len, DecodeError};
use rremu_isa::{cause, csr, Instruction, Priv};
use thiserror::Error;

/// M 状态 CSR（mcycle/minstret 由 cycle/instret 计数器承载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrState {
    pub mstatus: u32,
    pub misa: u32,
    pub mie: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mip: u32,
    pub mhartid: u32,
}

impl CsrState {
    fn new() -> Self {
        Self {
            mstatus: csr::MSTATUS_MPP_M, // 复位进入 M-mode
            misa: 0,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mip: 0,
            mhartid: 0,
        }
    }
}

/// 单步结果：正常执行一条，或陷入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Executed { instr: Instruction, len: u8 },
    Trapped { cause: u32 },
}

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
}

impl Cpu {
    pub fn new(entry: u32) -> Self {
        Self { x: [0; 32], pc: entry, priv_: Priv::Machine, csr: CsrState::new(), cycle: 0, instret: 0 }
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

    /// CSR 视图读取（快照 + CSR 指令共用）。未知 CSR 返回 0。
    pub fn csr_read(&self, addr: u16) -> u32 {
        match addr {
            csr::MSTATUS => self.csr.mstatus,
            csr::MISA => self.csr.misa,
            csr::MIE => self.csr.mie,
            csr::MTVEC => self.csr.mtvec,
            csr::MSCRATCH => self.csr.mscratch,
            csr::MEPC => self.csr.mepc,
            csr::MCAUSE => self.csr.mcause,
            csr::MTVAL => self.csr.mtval,
            csr::MIP => self.csr.mip,
            csr::MCYCLE => self.cycle as u32,
            csr::MINSTRET => self.instret as u32,
            csr::MHARTID => self.csr.mhartid,
            _ => 0,
        }
    }

    /// CSR 写入（CSR 指令执行用）。只读 CSR（mhartid/mcycle/minstret）与
    /// 未知 CSR（satp/pmpaddr/medeleg 等）写入被忽略。
    fn csr_write(&mut self, addr: u16, val: u32) {
        match addr {
            csr::MSTATUS => self.csr.mstatus = val,
            csr::MISA => self.csr.misa = val,
            csr::MIE => self.csr.mie = val,
            csr::MTVEC => self.csr.mtvec = val,
            csr::MSCRATCH => self.csr.mscratch = val,
            csr::MEPC => self.csr.mepc = val,
            csr::MCAUSE => self.csr.mcause = val,
            csr::MTVAL => self.csr.mtval = val,
            csr::MIP => self.csr.mip = val,
            _ => {}
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
    }

    /// 取指 → 解码 → 执行 → 写回；错误路径统一走 trap。
    ///
    /// 中断检查在取指**之前**：三层 AND 门控 ——
    /// mstatus.MIE（全局使能）& mie 对应位（单独使能）& bus.pending_interrupt()（硬件 pending）。
    /// 中断命中时 mepc = 当前 pc（未执行的下一条指令），mret 后重新执行。
    pub fn step<B: Bus, E: EventSink>(&mut self, bus: &mut B, sink: &mut E) -> Result<StepOutcome, CpuError> {
        bus.set_cycle(self.cycle);
        let pc = self.pc;

        // 中断检查（取指前）
        if (self.csr.mstatus & csr::MSTATUS_MIE) != 0 {
            if let Some(irq_cause) = bus.pending_interrupt() {
                let int_id = irq_cause & 0x7fff_ffff;
                if (self.csr.mie >> int_id) & 1 == 1 {
                    return self.trap(sink, irq_cause);
                }
            }
        }

        // pc 必须 2 字节对齐（C 扩展允许 2 字节对齐）。inst_read 内部也会查，
        // 这里先查可让 trap 路径与本文件历史一致；inst_read 拿到的奇数地址
        // 会返 Misaligned{size:Half}，CPU 侧一并映射到 INSTR_MISALIGNED。
        if pc & 0x1 != 0 {
            return self.trap(sink, cause::INSTR_MISALIGNED);
        }

        // 取指：一次 inst_read 拿全 4 字节（4 字节对齐时单次对齐 Word 读；
        // 2 字节对齐时 bus 内部读两个对齐字拼合并发一条 IF W 合成事件）。
        let word = match bus.inst_read(pc, sink) {
            Ok(w) => w,
            Err(e) => {
                let c = if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
                    cause::INSTR_MISALIGNED
                } else {
                    cause::INSTR_ACCESS_FAULT
                };
                return self.trap(sink, c);
            }
        };
        let lo = word as u16;

        let (instr, len) = if instr_len(lo) == 2 {
            // 16 位压缩指令
            match decode16(lo, pc) {
                Ok(i) => (i, 2u8),
                Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                    return self.trap(sink, cause::ILLEGAL_INSTR)
                }
            }
        } else {
            // 32 位指令：word 已包含完整 4 字节，直接解码
            match decode32(word, pc) {
                Ok(i) => (i, 4u8),
                Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                    return self.trap(sink, cause::ILLEGAL_INSTR)
                }
            }
        };

        match self.exec(bus, sink, pc, instr, len) {
            Ok(next_pc) => {
                self.pc = next_pc;
                self.cycle += 1;
                self.instret += 1;
                Ok(StepOutcome::Executed { instr, len })
            }
            Err(TrapErr { cause }) => self.trap(sink, cause),
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

            Instruction::Fence => {}
            Instruction::Ecall => {
                let c = if self.priv_ == Priv::Machine { cause::ECALL_FROM_M } else { cause::ECALL_FROM_U };
                return Err(TrapErr { cause: c });
            }
            Instruction::Ebreak => return Err(TrapErr { cause: cause::BREAKPOINT }),
            Instruction::Csrrw { rd, csr, rs1 } => {
                let old = self.csr_read(csr);
                self.csr_write(csr, x(self, rs1));
                set!(rd, old);
            }
            Instruction::Csrrs { rd, csr, rs1 } => {
                let old = self.csr_read(csr);
                let mask = x(self, rs1);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrc { rd, csr, rs1 } => {
                let old = self.csr_read(csr);
                let mask = x(self, rs1);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrwi { rd, csr, zimm } => {
                let old = self.csr_read(csr);
                self.csr_write(csr, u32::from(zimm));
                set!(rd, old);
            }
            Instruction::Csrrsi { rd, csr, zimm } => {
                let old = self.csr_read(csr);
                let mask = u32::from(zimm);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrci { rd, csr, zimm } => {
                let old = self.csr_read(csr);
                let mask = u32::from(zimm);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Mret => {
                let mstatus = self.csr.mstatus;
                let mpp = (mstatus >> csr::MSTATUS_MPP_SHIFT) & 0b11;
                let mpie = (mstatus >> csr::MSTATUS_MPIE.trailing_zeros()) & 1;
                // MIE = MPIE, MPIE = 1, MPP = 0 (U-mode)
                let mut new_mstatus =
                    mstatus & !(csr::MSTATUS_MPP_MASK | csr::MSTATUS_MPIE | csr::MSTATUS_MIE);
                new_mstatus |= (mpie << csr::MSTATUS_MIE.trailing_zeros()) | csr::MSTATUS_MPIE;
                self.csr.mstatus = new_mstatus;
                self.priv_ = if mpp == 0b11 { Priv::Machine } else { Priv::User };
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
            Instruction::CEbreak => return Err(TrapErr { cause: cause::BREAKPOINT }),
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
        bus.data_read(addr, size, sink).map_err(|e| TrapErr {
            cause: if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
                match size {
                    AccessSize::Half => cause::LOAD_MISALIGNED,
                    _ => cause::LOAD_MISALIGNED,
                }
            } else {
                cause::LOAD_ACCESS_FAULT
            },
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
        bus.data_write(addr, size, data, sink).map_err(|e| TrapErr {
            cause: if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
                cause::STORE_MISALIGNED
            } else {
                cause::STORE_ACCESS_FAULT
            },
        })
    }

    /// 陷入流程（PLAN §5.4）：mepc ← pc；mcause ← cause；priv ← M；pc ← mtvec（仅 DIRECT）。
    fn trap<E: EventSink>(&mut self, sink: &mut E, cause: u32) -> Result<StepOutcome, CpuError> {
        sink.emit(EmuEvent::Trap { cause, pc: self.pc });
        self.csr.mepc = self.pc;
        self.csr.mcause = cause;

        // 更新 mstatus: MPP = 当前特权级, MPIE = 旧 MIE, MIE = 0
        let mpp = match self.priv_ {
            Priv::Machine => 0b11,
            Priv::User => 0b00,
        };
        let old_mie = (self.csr.mstatus >> csr::MSTATUS_MIE.trailing_zeros()) & 1;
        let mut mstatus =
            self.csr.mstatus & !(csr::MSTATUS_MPP_MASK | csr::MSTATUS_MPIE | csr::MSTATUS_MIE);
        mstatus |= (mpp << csr::MSTATUS_MPP_SHIFT)
            | (old_mie << csr::MSTATUS_MPIE.trailing_zeros());
        self.csr.mstatus = mstatus;

        self.priv_ = Priv::Machine;
        self.cycle += 1;
        if self.csr.mtvec & 1 == 1 {
            return Err(CpuError::Unimplemented("vectored mtvec"));
        }
        self.pc = self.csr.mtvec & !1;
        Ok(StepOutcome::Trapped { cause })
    }
}

/// 执行期内部 trap 信号（bus 错误 → cause 映射已完成）。
struct TrapErr {
    cause: u32,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;
    use core::any::Any;
    use rremu_bus::{BusDevice, BusRouter};
    use rremu_decode::encode_c;
    use rremu_isa::Word;

    /// 测试 RAM（带 tohost 地址返回，供断言用）。
    struct MockRam {
        base: u32,
        mem: Vec<u8>,
    }

    impl MockRam {
        fn new(base: u32, size: usize) -> Self {
            Self { base, mem: vec![0; size] }
        }
    }

    impl BusDevice for MockRam {
        fn read(&mut self, addr: u32, size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
            let o = (addr - self.base) as usize;
            let mut v = 0u32;
            for i in 0..size.bytes() {
                v |= u32::from(self.mem[o + i as usize]) << (8 * i);
            }
            Ok(v)
        }
        fn write(&mut self, addr: u32, size: AccessSize, data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
            let o = (addr - self.base) as usize;
            for i in 0..size.bytes() {
                self.mem[o + i as usize] = (data >> (8 * i)) as u8;
            }
            Ok(())
        }
        fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
            let mut sink = rremu_bus::NullSink;
            BusDevice::read(self, addr, size, &mut sink)
        }
        fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
            let o = (addr - self.base) as usize;
            self.mem[o..o + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn device_name(&self) -> &str {
            "ram"
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    use rremu_bus::BusError;

    /// 简易事件收集器。
    struct Sink(Vec<EmuEvent>);
    impl EventSink for Sink {
        fn emit(&mut self, ev: EmuEvent) {
            self.0.push(ev);
        }
    }

    const BASE: u32 = 0x8000_0000;

    struct Harness {
        cpu: Cpu,
        bus: BusRouter,
        sink: Sink,
    }

    impl Harness {
        fn new() -> Self {
            let mut bus = BusRouter::new();
            bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
            Self { cpu: Cpu::new(BASE), bus, sink: Sink(Vec::new()) }
        }
        /// 把指令/数据写入内存后复位 CPU。
        fn load(&mut self, addr: u32, words: &[Word]) {
            let mut bytes = Vec::new();
            for w in words {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
            self.bus.write_bytes(addr, &bytes).unwrap();
        }
        fn step(&mut self) -> StepOutcome {
            self.cpu.step(&mut self.bus, &mut self.sink).unwrap()
        }
        fn run(&mut self, n: usize) -> Vec<StepOutcome> {
            (0..n).map(|_| self.step()).collect()
        }
    }

    // 编码辅助（与 decode 测试一致）
    fn i_enc(op: u32, rd: u8, f3: u32, rs1: u8, imm: i32) -> Word {
        ((imm as u32 & 0xfff) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | op
    }
    fn r_enc(rd: u8, f3: u32, rs1: u8, rs2: u8, f7: u32) -> Word {
        (f7 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | 0b0110011
    }
    fn b_enc(f3: u32, rs1: u8, rs2: u8, off: i32) -> Word {
        let o = off as u32;
        let b12 = (o >> 12) & 1;
        let b11 = (o >> 11) & 1;
        let b10_5 = (o >> 5) & 0x3f;
        let b4_1 = (o >> 1) & 0xf;
        (b12 << 31) | (b10_5 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (b4_1 << 8) | (b11 << 7) | 0b1100011
    }
    fn u_enc(op: u32, rd: u8, imm20: u32) -> Word {
        (imm20 << 12) | (u32::from(rd) << 7) | op
    }
    fn j_enc(rd: u8, off: i32) -> Word {
        let o = off as u32;
        let b20 = (o >> 20) & 1;
        let b19_12 = (o >> 12) & 0xff;
        let b11 = (o >> 11) & 1;
        let b10_1 = (o >> 1) & 0x3ff;
        (b20 << 31) | (b10_1 << 21) | (b11 << 20) | (b19_12 << 12) | (u32::from(rd) << 7) | 0b1101111
    }
    fn s_enc(f3: u32, rs1: u8, rs2: u8, imm: i32) -> Word {
        let i = imm as u32;
        (((i >> 5) & 0x7f) << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | ((i & 0x1f) << 7) | 0b0100011
    }

    const OP_IMM: u32 = 0b0010011;
    const OP_LUI: u32 = 0b0110111;
    const LOAD: u32 = 0b0000011;

    #[test]
    fn x0_is_hardwired_zero() {
        let mut h = Harness::new();
        // addi x0, x0, 123 → x0 仍为 0
        h.load(BASE, &[i_enc(OP_IMM, 0, 0b000, 0, 123)]);
        h.step();
        assert_eq!(h.cpu.reg(0), 0);
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn writeback_and_pc_advance() {
        let mut h = Harness::new();
        h.load(BASE, &[u_enc(0b0110111, 10, 0x12345), i_enc(OP_IMM, 11, 0b000, 10, -4)]);
        h.run(2);
        assert_eq!(h.cpu.reg(10), 0x1234_5000);
        assert_eq!(h.cpu.reg(11), 0x1234_4ffc);
        assert_eq!(h.cpu.cycle(), 2);
        assert_eq!(h.cpu.instret(), 2);
    }

    #[test]
    fn branch_taken_and_not_taken() {
        let mut h = Harness::new();
        // beq x0, x0, +8（taken）→ 跳过一条 addi
        h.load(BASE, &[
            b_enc(0b000, 0, 0, 8),
            i_enc(OP_IMM, 10, 0b000, 0, 99), // 被跳过
            i_enc(OP_IMM, 11, 0b000, 0, 1),  // 落点
        ]);
        h.run(1);
        assert_eq!(h.cpu.pc(), BASE + 8);
        // bne x0, x0（not taken）→ 顺序执行两步后到达落点
        h.load(BASE + 8, &[b_enc(0b001, 0, 0, 8), i_enc(OP_IMM, 12, 0b000, 0, 7)]);
        h.run(2);
        assert_eq!(h.cpu.pc(), BASE + 16);
        assert_eq!(h.cpu.reg(12), 7);
    }

    #[test]
    fn jal_jalr() {
        let mut h = Harness::new();
        // jal ra, +12
        h.load(BASE, &[j_enc(1, 12), u_enc(0b0110111, 10, 0), i_enc(OP_IMM, 0, 0b000, 0, 0)]);
        h.run(1);
        assert_eq!(h.cpu.pc(), BASE + 12);
        assert_eq!(h.cpu.reg(1), BASE + 4);
        // jalr t0 跳到寄存器地址，清 bit0
        // 先写一个目标地址到寄存器：lui t0, 0x80000; addi t0, t0, 0x123|1
        h.load(BASE + 12, &[
            u_enc(0b0110111, 5, 0x80000),
            i_enc(OP_IMM, 5, 0b000, 5, 0x123 | 1),
            i_enc(0b1100111, 1, 0b000, 5, 0), // jalr ra, 0(t0)
        ]);
        h.run(3);
        assert_eq!(h.cpu.pc(), 0x8000_0122); // bit0 被清除
        assert_eq!(h.cpu.reg(1), BASE + 24); // ra = jalr 指令地址 + 4
    }

    #[test]
    fn load_store_roundtrip() {
        let mut h = Harness::new();
        // sw t0 = 0x80000100 数据 -2；lw/lb/lbu/lh/lhu 校验符号扩展
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),        // lui t0, 0x80000
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),   // addi t0, t0, 0x100
            i_enc(OP_IMM, 6, 0b000, 0, -2),      // addi t1, x0, -2
            s_enc(0b010, 5, 6, 0),               // sw t1, 0(t0)
            i_enc(LOAD, 7, 0b010, 5, 0),         // lw t2, 0(t0)
            i_enc(LOAD, 8, 0b000, 5, 0),         // lb t3, 0(t0)
            i_enc(LOAD, 9, 0b100, 5, 0),         // lbu t4, 0(t0)
            i_enc(LOAD, 20, 0b001, 5, 0),        // lh t5, 0(t0)
            i_enc(LOAD, 21, 0b101, 5, 0),        // lhu t6, 0(t0)
        ]);
        h.run(9);
        assert_eq!(h.cpu.reg(7), 0xffff_fffe);
        assert_eq!(h.cpu.reg(8), 0xffff_fffe);
        assert_eq!(h.cpu.reg(9), 0xfe);
        assert_eq!(h.cpu.reg(20), 0xffff_fffe);
        assert_eq!(h.cpu.reg(21), 0xfffe);
    }

    #[test]
    fn sltiu_unsigned_compare() {
        let mut h = Harness::new();
        // sltiu a0, x0, -1 → x0(0) < 0xffffffff → 1
        h.load(BASE, &[i_enc(OP_IMM, 10, 0b011, 0, -1)]);
        h.step();
        assert_eq!(h.cpu.reg(10), 1);
    }

    #[test]
    fn r_type_wrapping_and_shift_mask() {
        let mut h = Harness::new();
        // 0 - 1 = wrapping
        h.load(BASE, &[
            r_enc(10, 0b000, 0, 0, 0b0100000), // sub a0, x0, x0 → 0
            i_enc(OP_IMM, 11, 0b000, 0, 1),    // addi a1, x0, 1
            r_enc(10, 0b000, 0, 11, 0b0100000), // sub a0, x0, a1 → 0xffffffff
            i_enc(OP_IMM, 12, 0b001, 10, 0),   // slli — shamt 0
            r_enc(12, 0b001, 10, 11, 0b0000000), // sll a2, a0, a1（移位量 1）
            r_enc(13, 0b101, 10, 11, 0b0100000), // sra a3, a0, a1
        ]);
        h.run(6);
        assert_eq!(h.cpu.reg(10), 0xffff_ffff);
        assert_eq!(h.cpu.reg(12), 0xffff_fffe);
        assert_eq!(h.cpu.reg(13), 0xffff_ffff);
    }

    #[test]
    fn trap_on_ecall_and_mret_path_exists() {
        let mut h = Harness::new();
        // 未设置 mtvec（0）：ecall → trap 到 0
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]); // ecall
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_M });
        assert_eq!(h.cpu.pc(), 0);
        assert_eq!(h.cpu.csr_state().mepc, BASE);
        assert_eq!(h.cpu.csr_state().mcause, cause::ECALL_FROM_M);
        assert_eq!(h.cpu.priv_(), Priv::Machine);
        // trap 事件已发
        assert!(h.sink.0.iter().any(|e| matches!(e, EmuEvent::Trap { cause: cause::ECALL_FROM_M, .. })));
        // cycle 前进，instret 不计陷入指令
        assert_eq!(h.cpu.cycle(), 1);
        assert_eq!(h.cpu.instret(), 0);
    }

    #[test]
    fn trap_on_illegal_instruction() {
        let mut h = Harness::new();
        h.load(BASE, &[0xffff_ffff]); // 未定义指令
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });
    }

    #[test]
    fn trap_on_unmapped_load() {
        let mut h = Harness::new();
        // lw t0, 0(x0) → 地址 0 未映射
        h.load(BASE, &[i_enc(LOAD, 5, 0b010, 0, 0)]);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::LOAD_ACCESS_FAULT });
    }

    #[test]
    fn trap_updates_mstatus_mpp_mpie_mie() {
        // 预置 mstatus.MIE=1，trap 后 MPP=M、MPIE=旧MIE、MIE=0
        let mut h = Harness::new();
        let pre = (csr::MSTATUS_MPP_M | csr::MSTATUS_MIE) as i32;
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, pre),                    // x5 = mstatus
            i_enc(0b1110011, 0, 0b001, 5, csr::MSTATUS as i32), // csrrw x0, mstatus, x5
            i_enc(0b1110011, 0, 0b000, 0, 0),                   // ecall
        ]);
        h.run(2);
        let out = h.step();
        assert!(matches!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_M }));
        let ms = h.cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b11);
        assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 0);
    }

    #[test]
    fn mret_restores_mstatus_mie_mpie_mpp() {
        // 预置 mstatus: MPP=M, MPIE=1, MIE=0；mret 后 MIE=MPIE, MPIE=1, MPP=0
        let mut h = Harness::new();
        let pre = (csr::MSTATUS_MPP_M | csr::MSTATUS_MPIE) as i32;
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, pre),                    // x5 = mstatus
            i_enc(0b1110011, 0, 0b001, 5, csr::MSTATUS as i32), // csrrw x0, mstatus, x5
            i_enc(0b1110011, 0, 0, 0, 0x302),                   // mret
        ]);
        h.run(2);
        h.step();
        let ms = h.cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b00);
    }

    #[test]
    fn reset_restores_state() {
        let mut h = Harness::new();
        h.load(BASE, &[i_enc(OP_IMM, 10, 0b000, 0, 5)]);
        h.run(1);
        h.cpu.reset(BASE);
        assert_eq!(h.cpu.reg(10), 0);
        assert_eq!(h.cpu.pc(), BASE);
        assert_eq!(h.cpu.cycle(), 0);
        assert_eq!(h.cpu.priv_(), Priv::Machine);
        assert_eq!(h.cpu.csr_state().mstatus, csr::MSTATUS_MPP_M);
    }

    // ---- M extension tests ----

    #[test]
    fn exec_mul_and_high() {
        let mut h = Harness::new();
        // li x1, -5; li x2, 3; mul x3,x1,x2; mulh x4,x1,x2
        h.load(BASE, &[
            i_enc(OP_IMM, 1, 0b000, 0, -5),
            i_enc(OP_IMM, 2, 0b000, 0, 3),
            r_enc(3, 0b000, 1, 2, 0b0000001), // mul
            r_enc(4, 0b001, 1, 2, 0b0000001), // mulh
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(3), (-15i32) as u32);
        assert_eq!(h.cpu.reg(4) as i32, -1); // (-5*3=-15) high32 = -1
    }

    #[test]
    fn exec_mulhu_and_mulhsu() {
        let mut h = Harness::new();
        // x1 = 0x8000_0000 (as signed: -2147483648), x2 = 2
        h.load(BASE, &[
            u_enc(0b0110111, 1, 0x80000), // lui x1, 0x80000 → 0x8000_0000
            i_enc(OP_IMM, 2, 0b000, 0, 2),
            r_enc(3, 0b011, 1, 2, 0b0000001), // mulhu: high(0x8000_0000 * 2) = 1
            r_enc(4, 0b010, 1, 2, 0b0000001), // mulhsu: high(signed(-2^31) * 2) = -1
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(3), 1);
        assert_eq!(h.cpu.reg(4) as i32, -1);
    }

    #[test]
    fn exec_div_basic() {
        let mut h = Harness::new();
        // x1 = 13, x2 = 5; div x3,x1,x2; rem x4,x1,x2
        h.load(BASE, &[
            i_enc(OP_IMM, 1, 0b000, 0, 13),
            i_enc(OP_IMM, 2, 0b000, 0, 5),
            r_enc(3, 0b100, 1, 2, 0b0000001), // div
            r_enc(4, 0b110, 1, 2, 0b0000001), // rem
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(3), 2);
        assert_eq!(h.cpu.reg(4), 3);
    }

    #[test]
    fn exec_div_by_zero_and_overflow() {
        let mut h = Harness::new();
        // x1 = INT_MIN, x2 = -1
        h.load(BASE, &[
            u_enc(0b0110111, 1, 0x80000), // x1 = 0x8000_0000 = INT_MIN
            i_enc(OP_IMM, 2, 0b000, 0, -1),
            r_enc(3, 0b100, 1, 2, 0b0000001), // div → overflow → INT_MIN
            r_enc(4, 0b110, 1, 2, 0b0000001), // rem → overflow → 0
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(3), 0x8000_0000u32);
        assert_eq!(h.cpu.reg(4), 0);
    }

    #[test]
    fn exec_divu_remu_by_zero() {
        let mut h = Harness::new();
        // x1 = 42, x2 = 0
        h.load(BASE, &[
            i_enc(OP_IMM, 1, 0b000, 0, 42),
            i_enc(OP_IMM, 2, 0b000, 0, 0),
            r_enc(3, 0b101, 1, 2, 0b0000001), // divu by zero → 0xffff_ffff
            r_enc(4, 0b111, 1, 2, 0b0000001), // remu by zero → rs1 = 42
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(3), 0xffff_ffffu32);
        assert_eq!(h.cpu.reg(4), 42);
    }

    #[test]
    fn exec_csrrs_readonly_and_set() {
        // csrrs with rs1=x0 (read-only, no write) and csrrs with non-zero mask
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, 0xAB),                          // li x5, 0xAB
            i_enc(0b1110011, 0, 0b001, 5, csr::MSCRATCH as i32),       // csrrw x0, mscratch, x5 → mscratch=0xAB
            i_enc(0b1110011, 1, 0b010, 0, csr::MSCRATCH as i32),       // csrrs x1, mscratch, x0 → x1=0xAB, mscratch unchanged
            i_enc(OP_IMM, 3, 0b000, 0, 0x100),                        // li x3, 0x100
            i_enc(0b1110011, 2, 0b010, 3, csr::MSCRATCH as i32),       // csrrs x2, mscratch, x3 → mscratch=0x1AB, x2=0xAB
        ]);
        h.run(5);
        assert_eq!(h.cpu.reg(1), 0xAB);
        assert_eq!(h.cpu.reg(2), 0xAB);
        assert_eq!(h.cpu.csr_state().mscratch, 0x1AB);
    }

    #[test]
    fn exec_csrrc_clears_bits() {
        // csrrc clears bits: mscratch=0xFF, mask=0x0F → mscratch=0xF0
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, 0xFF),                          // li x5, 0xFF
            i_enc(0b1110011, 0, 0b001, 5, csr::MSCRATCH as i32),       // csrrw x0, mscratch, x5 → mscratch=0xFF
            i_enc(OP_IMM, 2, 0b000, 0, 0x0F),                         // li x2, 0x0F
            i_enc(0b1110011, 1, 0b011, 2, csr::MSCRATCH as i32),       // csrrc x1, mscratch, x2 → x1=0xFF, mscratch=0xF0
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(1), 0xFF);
        assert_eq!(h.cpu.csr_state().mscratch, 0xF0);
    }

    #[test]
    fn exec_csrrwi_csrrsi_csrrci() {
        // Immediate variants of CSR instructions
        let mut h = Harness::new();
        h.load(BASE, &[
            // csrrwi x1, mscratch, 5 → mscratch=5, x1=old(0)
            i_enc(0b1110011, 1, 0b101, 5, csr::MSCRATCH as i32),
            // set mscratch=0xF0 via csrrw (zimm only 5 bits, so use register)
            i_enc(OP_IMM, 5, 0b000, 0, 0xF0),                          // li x5, 0xF0
            i_enc(0b1110011, 0, 0b001, 5, csr::MSCRATCH as i32),       // csrrw x0, mscratch, x5 → mscratch=0xF0
            // csrrsi x2, mscratch, 0x0F → mscratch=0xF0|0x0F=0xFF, x2=0xF0
            i_enc(0b1110011, 2, 0b110, 0x0F, csr::MSCRATCH as i32),
            // csrrci x3, mscratch, 0x0F → mscratch=0xFF & !0x0F=0xF0, x3=0xFF
            i_enc(0b1110011, 3, 0b111, 0x0F, csr::MSCRATCH as i32),
        ]);
        h.run(5);
        assert_eq!(h.cpu.reg(1), 0);    // old mscratch was 0
        assert_eq!(h.cpu.reg(2), 0xF0); // mscratch was 0xF0 before csrrsi
        assert_eq!(h.cpu.reg(3), 0xFF); // mscratch was 0xFF before csrrci
        assert_eq!(h.cpu.csr_state().mscratch, 0xF0); // after csrrci
    }

    #[test]
    fn exec_signed_div_by_zero() {
        // Signed division by zero: div returns -1, rem returns dividend
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 1, 0b000, 0, 7),                          // li x1, 7
            i_enc(OP_IMM, 2, 0b000, 0, 0),                          // li x2, 0
            r_enc(3, 0b100, 1, 2, 0b0000001), // div x3, x1, x2 → 0xFFFFFFFF (-1)
            r_enc(4, 0b110, 1, 2, 0b0000001), // rem x4, x1, x2 → 7
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(3), 0xFFFF_FFFF);
        assert_eq!(h.cpu.reg(4), 7);
    }

    #[test]
    fn exec_c_lui() {
        // c.lui: NZimm is 6-bit sign-extended, imm = NZimm << 12
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CLui { rd: 8, imm: 0x10000 },      // NZimm=0x10, x8=0x10000
            Instruction::CLui { rd: 9, imm: 0xFFFF_F000 },   // NZimm=-1, x9=0xFFFFF000
        ]);
        h.run(2);
        assert_eq!(h.cpu.reg(8), 0x10000);
        assert_eq!(h.cpu.reg(9), 0xFFFF_F000);
    }

    #[test]
    fn exec_c_jalr() {
        // c.jalr x8 → ra = pc+2, pc = x8 (clear bit0)
        let mut h = Harness::new();
        h.load(BASE, &[
            u_enc(OP_LUI, 8, 0x80000),               // lui x8, 0x80000 → x8 = BASE
            i_enc(OP_IMM, 8, 0b000, 8, 0x100),       // addi x8, x8, 0x100 → x8 = BASE+0x100
        ]);
        h.run(2);
        c_load(&mut h, BASE + 8, &[
            Instruction::CJalr { rs1: 8 },
        ]);
        c_load(&mut h, BASE + 0x100, &[
            Instruction::CAddi { rd: 7, imm: 1 },
        ]);
        h.step(); // c.jalr x8 → ra = BASE+0xA, pc = BASE+0x100
        assert_eq!(h.cpu.reg(1), BASE + 0xA); // ra = pc of c.jalr + 2
        assert_eq!(h.cpu.pc(), BASE + 0x100);
        h.step(); // c.addi x7, 1
        assert_eq!(h.cpu.reg(7), 1);
    }

    // ---- C extension tests ----

    fn c_load(h: &mut Harness, addr: u32, instrs: &[Instruction]) {
        let mut bytes = Vec::new();
        for i in instrs {
            bytes.extend_from_slice(&encode_c(i).to_le_bytes());
        }
        h.bus.write_bytes(addr, &bytes).unwrap();
    }

    #[test]
    fn exec_c_addi_li_mv_add() {
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CAddi { rd: 1, imm: 5 },
            Instruction::CLi { rd: 2, imm: -1 },
            Instruction::CMv { rd: 3, rs2: 1 },
            Instruction::CAdd { rd: 3, rs2: 2 },
        ]);
        h.run(4);
        assert_eq!(h.cpu.reg(1), 5);
        assert_eq!(h.cpu.reg(2) as i32, -1);
        assert_eq!(h.cpu.reg(3), 4); // 5 + (-1)
        assert_eq!(h.cpu.pc(), BASE + 8);
    }

    #[test]
    fn exec_c_sub_xor_or_and() {
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CAddi { rd: 8, imm: 12 },
            Instruction::CAddi { rd: 9, imm: 10 },
            Instruction::CSub { rd: 8, rs2: 9 },
            Instruction::CXor { rd: 8, rs2: 9 },
            Instruction::COr { rd: 8, rs2: 9 },
            Instruction::CAnd { rd: 8, rs2: 9 },
        ]);
        h.run(2);
        h.step(); assert_eq!(h.cpu.reg(8), 2);   // 12 - 10
        h.step(); assert_eq!(h.cpu.reg(8), 8);   // 2 ^ 10
        h.step(); assert_eq!(h.cpu.reg(8), 10);  // 8 | 10
        h.step(); assert_eq!(h.cpu.reg(8), 10);  // 10 & 10
    }

    #[test]
    fn exec_c_slli_srli_srai_andi() {
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CLi { rd: 8, imm: -1 },
            Instruction::CSlli { rd: 8, shamt: 4 },
            Instruction::CSrli { rd: 8, shamt: 4 },
            Instruction::CSrai { rd: 8, shamt: 4 },
            Instruction::CAndi { rd: 8, imm: -16 },
        ]);
        h.step(); assert_eq!(h.cpu.reg(8), 0xffff_ffffu32);
        h.step(); assert_eq!(h.cpu.reg(8), 0xffff_fff0u32);   // << 4
        h.step(); assert_eq!(h.cpu.reg(8), 0x0fff_ffffu32);   // >> 4 (logical)
        h.step(); assert_eq!(h.cpu.reg(8), 0x00ff_ffffu32);   // >> 4 (arithmetic, sign bit 0)
        h.step(); assert_eq!(h.cpu.reg(8), 0x00ff_fff0u32);   // & 0xffff_fff0
    }

    #[test]
    fn exec_c_jump_and_link() {
        // c.jal +4 跳到第三条指令，x1 保存返回地址 BASE+2
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CJal { imm: 4 },
            Instruction::CAddi { rd: 1, imm: 99 }, // 跳过
            Instruction::CAddi { rd: 2, imm: 1 },
        ]);
        h.run(2); // c.jal → c.addi x2,1
        assert_eq!(h.cpu.reg(1), BASE + 2); // 返回地址
        assert_eq!(h.cpu.reg(2), 1);
    }

    #[test]
    fn exec_c_j_beqz_bnez() {
        // c.j +4 跳过一条; c.beqz x8, +4 (x8=0 则跳); c.bnez x9, +4 (x9!=0 则跳)
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CAddi { rd: 8, imm: 0 },   // x8 = 0
            Instruction::CAddi { rd: 9, imm: 5 },   // x9 = 5
            Instruction::CJ { imm: 4 },             // 跳到 BASE+10
            Instruction::CAddi { rd: 10, imm: 99 }, // 跳过
            Instruction::CBeqz { rs1: 8, imm: 4 },  // x8==0 → 跳
            Instruction::CAddi { rd: 11, imm: 99 }, // 跳过
            Instruction::CBnez { rs1: 9, imm: 4 },  // x9!=0 → 跳
            Instruction::CAddi { rd: 12, imm: 99 }, // 跳过
            Instruction::CAddi { rd: 13, imm: 1 },  // 到达
        ]);
        h.run(6); // x8=0, x9=5, c.j, c.beqz, c.bnez, c.addi x13,1
        assert_eq!(h.cpu.reg(10), 0);
        assert_eq!(h.cpu.reg(11), 0);
        assert_eq!(h.cpu.reg(12), 0);
        assert_eq!(h.cpu.reg(13), 1);
    }

    #[test]
    fn exec_c_lw_sw_sp_stack() {
        // 设置 sp，然后 c.addi16sp / c.swsp / c.lwsp 往返
        let mut h = Harness::new();
        h.load(BASE, &[
            u_enc(OP_LUI, 2, 0x80000),                 // sp = 0x8000_0000
            i_enc(OP_IMM, 2, 0b000, 2, 0x200),         // sp += 0x200 → 0x8000_0200
            i_enc(OP_IMM, 1, 0b000, 0, 0x123),         // x1 = 0x123
        ]);
        h.run(3);
        c_load(&mut h, BASE + 12, &[
            Instruction::CAddi16sp { imm: -16 },        // sp = 0x8000_01f0
            Instruction::CSwsp { rs2: 1, imm: 4 },      // mem[0x8000_01f4] = x1
            Instruction::CLwsp { rd: 3, imm: 4 },       // x3 = mem[0x8000_01f4]
        ]);
        h.run(3);
        assert_eq!(h.cpu.reg(2), BASE + 0x200 - 16);
        assert_eq!(h.cpu.reg(3), 0x123);
    }

    #[test]
    fn exec_c_lw_sw() {
        // c.lw / c.sw 使用压缩寄存器 x8..x15
        let mut h = Harness::new();
        h.load(BASE, &[
            u_enc(OP_LUI, 8, 0x80000),                 // x8 = 0x8000_0000 (基址)
            i_enc(OP_IMM, 8, 0b000, 8, 0x100),         // x8 += 0x100
            i_enc(OP_IMM, 9, 0b000, 0, 0x7ab),         // x9 = 0x7ab
        ]);
        h.run(3);
        c_load(&mut h, BASE + 12, &[
            Instruction::CSw { rs1: 8, rs2: 9, imm: 4 },  // mem[BASE+0x104] = 0x7ab
            Instruction::CLw { rd: 10, rs1: 8, imm: 4 },  // x10 = mem[BASE+0x104]
        ]);
        h.run(2);
        assert_eq!(h.cpu.reg(10), 0x7ab);
    }

    #[test]
    fn exec_c_jr_mv_jalr() {
        // c.mv x5, x6; c.jr x5 (跳转到 x5 的值)
        let mut h = Harness::new();
        // x6 = BASE + 0x100 (用 lui + addi，因为超出 12 位立即数)
        h.load(BASE, &[
            u_enc(OP_LUI, 6, 0x80000),               // lui x6, 0x80000 → x6 = 0x8000_0000
            i_enc(OP_IMM, 6, 0b000, 6, 0x100),       // addi x6, x6, 0x100 → x6 = BASE+0x100
        ]);
        h.run(2);
        c_load(&mut h, BASE + 8, &[
            Instruction::CMv { rd: 5, rs2: 6 },   // x5 = x6
            Instruction::CJr { rs1: 5 },          // pc = x5
            Instruction::CAddi { rd: 7, imm: 99 },// 跳过
        ]);
        // 在 BASE+0x100 放一条 c.addi
        c_load(&mut h, BASE + 0x100, &[
            Instruction::CAddi { rd: 7, imm: 1 },
        ]);
        h.run(3); // c.mv, c.jr, c.addi x7,1
        assert_eq!(h.cpu.reg(7), 1);
        assert_eq!(h.cpu.pc(), BASE + 0x100 + 2);
    }

    #[test]
    fn exec_c_addi4spn() {
        // c.addi4spn x8, x2, 16 → x8 = sp + 16
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 2, 0b000, 0, 0x700), // sp = 0x700
        ]);
        h.step();
        c_load(&mut h, BASE + 4, &[
            Instruction::CAddi4spn { rd: 8, imm: 16 },
        ]);
        h.step();
        assert_eq!(h.cpu.reg(8), 0x700 + 16);
    }

    #[test]
    fn exec_c_ebreak() {
        // c.ebreak 触发断点 trap，验证 step 返回且不崩溃
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CAddi { rd: 1, imm: 1 },
            Instruction::CEbreak,
        ]);
        h.step(); // c.addi x1, 1
        let outcome = h.step(); // c.ebreak → trap
        assert!(matches!(outcome, StepOutcome::Trapped { .. }));
    }

    #[test]
    fn exec_cross_boundary_32bit_instr() {
        // 32 位指令放在 2 字节对齐但非 4 字节对齐的地址，测试跨边界取指
        let mut h = Harness::new();
        let nop = encode_c(&Instruction::CAddi { rd: 0, imm: 0 }); // c.nop
        let addi = i_enc(OP_IMM, 1, 0b000, 0, 42);
        let mut bytes = nop.to_le_bytes().to_vec();
        bytes.extend_from_slice(&addi.to_le_bytes());
        h.bus.write_bytes(BASE, &bytes).unwrap();
        h.run(2);
        assert_eq!(h.cpu.reg(1), 42);
        assert_eq!(h.cpu.pc(), BASE + 6);
    }

    // ---- interrupt tests (M3.3) ----

    /// 测试 IRQ 设备：返回可配置的中断 cause。
    struct MockIrq {
        cause: Option<u32>,
    }

    impl MockIrq {
        fn new(cause: u32) -> Self {
            Self { cause: Some(cause) }
        }
    }

    impl BusDevice for MockIrq {
        fn read(&mut self, _addr: u32, _size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
            Ok(0)
        }
        fn write(&mut self, _addr: u32, _size: AccessSize, _data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
            Ok(())
        }
        fn peek_read(&mut self, _addr: u32, _size: AccessSize) -> Result<u32, BusError> {
            Ok(0)
        }
        fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
            Ok(())
        }
        fn device_name(&self) -> &str {
            "mock-irq"
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn check_interrupt(&self) -> Option<u32> {
            self.cause
        }
    }

    #[test]
    fn interrupt_taken_when_mie_and_mtie_enabled() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;

        let mut sink = Sink(Vec::new());
        let outcome = cpu.step(&mut bus, &mut sink).unwrap();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_TI });
        assert_eq!(cpu.csr_state().mepc, BASE); // mepc = 旧 pc（未执行的指令）
        assert_eq!(cpu.pc(), BASE + 0x800); // pc = mtvec
        // trap 后 MIE=0（防嵌套），MPIE=旧MIE=1，MPP=M
        let ms = cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 0);
        assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b11);
    }

    #[test]
    fn interrupt_not_taken_when_mstatus_mie_disabled() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MPP_M; // MIE=0
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;
        // 放一条 NOP
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // addi x0, x0, 0 (NOP)
        let outcome = h.step();
        // 中断未取，NOP 正常执行
        assert!(matches!(outcome, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn interrupt_not_taken_when_mie_mtie_disabled() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M; // MIE=1
        cpu.csr.mie = 0; // MTIE=0
        cpu.csr.mtvec = BASE + 0x800;
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // NOP
        let outcome = h.step();
        assert!(matches!(outcome, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn interrupt_mret_restores_mie_and_resumes() {
        // 中断 → trap → handler 第一条 mret → 恢复 MIE → pc = mepc
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let handler = BASE + 0x800;
        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = handler;

        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        // 主程序放 NOP；handler 放 mret
        h.load(BASE, &[0x0000_0013]); // NOP at BASE
        h.load(handler, &[0x3020_0073]); // mret at handler

        // 第 1 步：中断触发，trap 到 handler
        let out1 = h.step();
        assert_eq!(out1, StepOutcome::Trapped { cause: cause::M_TI });
        assert_eq!(h.cpu.pc(), handler);
        assert_eq!(h.cpu.csr_state().mepc, BASE);

        // 第 2 步：执行 mret → pc = mepc = BASE，MIE 恢复为 1
        let out2 = h.step();
        assert!(matches!(out2, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE);
        let ms = h.cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 1);

        // 第 3 步：中断再次 pending → 再次 trap（MockIrq 恒 pending）
        let out3 = h.step();
        assert_eq!(out3, StepOutcome::Trapped { cause: cause::M_TI });
    }
}
