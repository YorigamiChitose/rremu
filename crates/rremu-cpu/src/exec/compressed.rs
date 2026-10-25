//! RVC compressed instruction execution helpers.

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_isa::Instruction;

use super::{Cpu, TrapErr};

impl Cpu {
    pub(crate) fn exec_compressed<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        pc: u32,
        len: u8,
        instr: Instruction,
    ) -> Result<Option<u32>, TrapErr> {
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
            // ---- C extension (RV32C) ----
            Instruction::CAddi4spn { rd, imm } => set!(rd, x(self, 2u8).wrapping_add(imm)),
            Instruction::CLw { rd, rs1, imm } => {
                let v = self.load(
                    bus,
                    sink,
                    x(self, rs1).wrapping_add(imm as u32),
                    AccessSize::Word,
                )?;
                set!(rd, v)
            }
            Instruction::CSw { rs1, rs2, imm } => self.store(
                bus,
                sink,
                x(self, rs1).wrapping_add(imm as u32),
                AccessSize::Word,
                x(self, rs2),
            )?,
            Instruction::CAddi { rd, imm } => set!(rd, x(self, rd).wrapping_add(imm as u32)),
            Instruction::CJal { imm } => {
                set!(1u8, pc.wrapping_add(2u32));
                next_pc = pc.wrapping_add(imm as u32);
            }
            Instruction::CLi { rd, imm } => set!(rd, imm as u32),
            Instruction::CAddi16sp { imm } => set!(2u8, x(self, 2u8).wrapping_add(imm as u32)),
            Instruction::CLui { rd, imm } => set!(rd, imm),
            Instruction::CSrli { rd, shamt } => set!(rd, x(self, rd) >> (shamt & 0x1f)),
            Instruction::CSrai { rd, shamt } => {
                set!(rd, ((x(self, rd) as i32) >> (shamt & 0x1f)) as u32)
            }
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
                let v = self.load(
                    bus,
                    sink,
                    x(self, 2u8).wrapping_add(imm as u32),
                    AccessSize::Word,
                )?;
                set!(rd, v)
            }
            Instruction::CJr { rs1 } => next_pc = x(self, rs1) & !1,
            Instruction::CMv { rd, rs2 } => set!(rd, x(self, rs2)),
            Instruction::CEbreak => return Err(Self::breakpoint_trap()),
            Instruction::CJalr { rs1 } => {
                set!(1u8, pc.wrapping_add(2u32));
                next_pc = x(self, rs1) & !1;
            }
            Instruction::CAdd { rd, rs2 } => set!(rd, x(self, rd).wrapping_add(x(self, rs2))),
            Instruction::CSwsp { rs2, imm } => self.store(
                bus,
                sink,
                x(self, 2u8).wrapping_add(imm as u32),
                AccessSize::Word,
                x(self, rs2),
            )?,
            _ => return Ok(None),
        }
        Ok(Some(next_pc))
    }
}
