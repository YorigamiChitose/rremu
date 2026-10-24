//! 指令执行分派与访存/AMO 实现。

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_isa::Instruction;

use super::{Cpu, TrapErr};

mod alu;
mod compressed;
mod csr;
mod fp;
mod system;

mod memory;

impl Cpu {
    /// 执行一条已解码指令；成功返回下一 pc，失败返回 trap 原因。
    pub(crate) fn exec<B: Bus, E: EventSink>(
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

        if let Some(next_pc) = self.exec_alu(pc, len, instr) {
            return Ok(next_pc);
        }
        if let Some(next_pc) = self.exec_compressed(bus, sink, pc, len, instr)? {
            return Ok(next_pc);
        }
        if let Some(next_pc) = self.exec_fp(bus, sink, pc, len, instr, raw)? {
            return Ok(next_pc);
        }

        match instr {
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

            Instruction::Sret => next_pc = self.exec_sret(),
            Instruction::Wfi => {} // 无等待实现：NOP
            Instruction::SfenceVma => {} // 无 TLB：NOP

            Instruction::Fence => {}
            Instruction::FenceI => {}
            Instruction::Ecall => return Err(self.ecall_trap()),
            Instruction::Ebreak => return Err(Self::breakpoint_trap()),
            Instruction::Csrrw { rd, csr, rs1 } => self.exec_csrrw(rd, csr, rs1, raw)?,
            Instruction::Csrrs { rd, csr, rs1 } => self.exec_csrrs(rd, csr, rs1, raw)?,
            Instruction::Csrrc { rd, csr, rs1 } => self.exec_csrrc(rd, csr, rs1, raw)?,
            Instruction::Csrrwi { rd, csr, zimm } => self.exec_csrrwi(rd, csr, zimm, raw)?,
            Instruction::Csrrsi { rd, csr, zimm } => self.exec_csrrsi(rd, csr, zimm, raw)?,
            Instruction::Csrrci { rd, csr, zimm } => self.exec_csrrci(rd, csr, zimm, raw)?,
            Instruction::Mret => next_pc = self.exec_mret(),
            _ => {}
        }
        Ok(next_pc)
    }


}
