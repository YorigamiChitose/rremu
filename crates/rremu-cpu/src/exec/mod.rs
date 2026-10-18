//! 指令执行分派与访存/AMO 实现。

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_isa::{cause, csr as isa_csr, Instruction, Priv};

use super::{Cpu, TrapErr};

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
                let old = self.architectural_csr_read(csr)?;
                self.csr_write(csr, x(self, rs1));
                set!(rd, old);
            }
            Instruction::Csrrs { rd, csr, rs1 } => {
                let mask = x(self, rs1);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.architectural_csr_read(csr)?;
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrc { rd, csr, rs1 } => {
                let mask = x(self, rs1);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.architectural_csr_read(csr)?;
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrwi { rd, csr, zimm } => {
                self.check_csr_access(csr, true, raw)?;
                let old = self.architectural_csr_read(csr)?;
                self.csr_write(csr, u32::from(zimm));
                set!(rd, old);
            }
            Instruction::Csrrsi { rd, csr, zimm } => {
                let mask = u32::from(zimm);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.architectural_csr_read(csr)?;
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrci { rd, csr, zimm } => {
                let mask = u32::from(zimm);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.architectural_csr_read(csr)?;
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


}
