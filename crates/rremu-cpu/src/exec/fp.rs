//! F/D 扩展执行：load/store、move、符号注入、比较、分类与算术。

use core::cmp::Ordering;

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_fp::{
    Flags, Fp32, Fp64, RoundingMode, add32, add64, div32, div64, max32, max64, min32, min64,
    mul32, mul64, sqrt32, sqrt64, sub32, sub64,
};
use rremu_isa::{Instruction, cause, csr};

use super::{Cpu, TrapErr};

impl Cpu {
    /// 执行 F/D 指令。非 FP 指令返回 `Ok(None)`。
    pub(crate) fn exec_fp<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        pc: u32,
        len: u8,
        instr: Instruction,
        raw: u32,
    ) -> Result<Option<u32>, TrapErr> {
        if !is_fp_instruction(instr) {
            return Ok(None);
        }
        if !self.fp_enabled() {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }

        let next_pc = pc.wrapping_add(len as u32);
        let x = |c: &Self, i: u8| c.x[usize::from(i)];

        match instr {
            Instruction::Flw { rd, rs1, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.load(bus, sink, addr, AccessSize::Word)?;
                self.write_fpr_s(rd, Fp32::from_bits(bits));
            }
            Instruction::Fsw { rs1, rs2, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.fpr_s(rs2).to_bits();
                self.store(bus, sink, addr, AccessSize::Word, bits)?;
            }
            Instruction::Fld { rd, rs1, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.load_u64(bus, sink, addr)?;
                self.write_fpr_d(rd, Fp64::from_bits(bits));
            }
            Instruction::Fsd { rs1, rs2, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.fpr_d(rs2).to_bits();
                self.store_u64(bus, sink, addr, bits)?;
            }
            Instruction::FmvWX { rd, rs1 } => {
                self.write_fpr_s(rd, Fp32::from_bits(x(self, rs1)));
            }
            Instruction::FmvXW { rd, rs1 } => {
                let bits = self.fpr_s(rs1).to_bits();
                if rd != 0 {
                    self.x[usize::from(rd)] = bits;
                }
            }
            Instruction::FsgnjS { rd, rs1, rs2 } => {
                let a = self.fpr_s(rs1);
                let b = self.fpr_s(rs2);
                self.write_fpr_s(rd, a.with_sign(b.sign()));
            }
            Instruction::FsgnjnS { rd, rs1, rs2 } => {
                let a = self.fpr_s(rs1);
                let b = self.fpr_s(rs2);
                self.write_fpr_s(rd, a.with_sign(!b.sign()));
            }
            Instruction::FsgnjxS { rd, rs1, rs2 } => {
                let a = self.fpr_s(rs1);
                let b = self.fpr_s(rs2);
                self.write_fpr_s(rd, a.with_sign(a.sign() ^ b.sign()));
            }
            Instruction::FsgnjD { rd, rs1, rs2 } => {
                let a = self.fpr_d(rs1);
                let b = self.fpr_d(rs2);
                self.write_fpr_d(rd, a.with_sign(b.sign()));
            }
            Instruction::FsgnjnD { rd, rs1, rs2 } => {
                let a = self.fpr_d(rs1);
                let b = self.fpr_d(rs2);
                self.write_fpr_d(rd, a.with_sign(!b.sign()));
            }
            Instruction::FsgnjxD { rd, rs1, rs2 } => {
                let a = self.fpr_d(rs1);
                let b = self.fpr_d(rs2);
                self.write_fpr_d(rd, a.with_sign(a.sign() ^ b.sign()));
            }
            Instruction::FeqS { rd, rs1, rs2 } => {
                let a = self.fpr_s(rs1);
                let b = self.fpr_s(rs2);
                self.compare_s(rd, a, b, |o| o == Ordering::Equal);
            }
            Instruction::FltS { rd, rs1, rs2 } => {
                let a = self.fpr_s(rs1);
                let b = self.fpr_s(rs2);
                self.compare_s(rd, a, b, |o| o == Ordering::Less);
            }
            Instruction::FleS { rd, rs1, rs2 } => {
                let a = self.fpr_s(rs1);
                let b = self.fpr_s(rs2);
                self.compare_s(rd, a, b, |o| o != Ordering::Greater);
            }
            Instruction::FeqD { rd, rs1, rs2 } => {
                let a = self.fpr_d(rs1);
                let b = self.fpr_d(rs2);
                self.compare_d(rd, a, b, |o| o == Ordering::Equal);
            }
            Instruction::FltD { rd, rs1, rs2 } => {
                let a = self.fpr_d(rs1);
                let b = self.fpr_d(rs2);
                self.compare_d(rd, a, b, |o| o == Ordering::Less);
            }
            Instruction::FleD { rd, rs1, rs2 } => {
                let a = self.fpr_d(rs1);
                let b = self.fpr_d(rs2);
                self.compare_d(rd, a, b, |o| o != Ordering::Greater);
            }
            Instruction::FclassS { rd, rs1 } => {
                let class = self.fpr_s(rs1).classify();
                if rd != 0 {
                    self.x[usize::from(rd)] = u32::from(class.mask());
                }
            }
            Instruction::FclassD { rd, rs1 } => {
                let class = self.fpr_d(rs1).classify();
                if rd != 0 {
                    self.x[usize::from(rd)] = u32::from(class.mask());
                }
            }
            Instruction::FaddS { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = add32(self.fpr_s(rs1), self.fpr_s(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FsubS { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = sub32(self.fpr_s(rs1), self.fpr_s(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FmulS { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = mul32(self.fpr_s(rs1), self.fpr_s(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FdivS { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = div32(self.fpr_s(rs1), self.fpr_s(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FsqrtS { rd, rs1, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = sqrt32(self.fpr_s(rs1), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FaddD { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = add64(self.fpr_d(rs1), self.fpr_d(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::FsubD { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = sub64(self.fpr_d(rs1), self.fpr_d(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::FmulD { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = mul64(self.fpr_d(rs1), self.fpr_d(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::FdivD { rd, rs1, rs2, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = div64(self.fpr_d(rs1), self.fpr_d(rs2), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::FsqrtD { rd, rs1, rm } => {
                let mode = self.fp_rounding_mode(rm, raw)?;
                let (value, flags) = sqrt64(self.fpr_d(rs1), mode);
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::FminS { rd, rs1, rs2 } => {
                let (value, flags) = min32(self.fpr_s(rs1), self.fpr_s(rs2));
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FmaxS { rd, rs1, rs2 } => {
                let (value, flags) = max32(self.fpr_s(rs1), self.fpr_s(rs2));
                self.raise_fp_flags(flags);
                self.write_fpr_s(rd, value);
            }
            Instruction::FminD { rd, rs1, rs2 } => {
                let (value, flags) = min64(self.fpr_d(rs1), self.fpr_d(rs2));
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::FmaxD { rd, rs1, rs2 } => {
                let (value, flags) = max64(self.fpr_d(rs1), self.fpr_d(rs2));
                self.raise_fp_flags(flags);
                self.write_fpr_d(rd, value);
            }
            Instruction::CFlw { rd, rs1, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.load(bus, sink, addr, AccessSize::Word)?;
                self.write_fpr_s(rd, Fp32::from_bits(bits));
            }
            Instruction::CFsw { rs1, rs2, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.fpr_s(rs2).to_bits();
                self.store(bus, sink, addr, AccessSize::Word, bits)?;
            }
            Instruction::CFld { rd, rs1, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.load_u64(bus, sink, addr)?;
                self.write_fpr_d(rd, Fp64::from_bits(bits));
            }
            Instruction::CFsd { rs1, rs2, imm } => {
                let addr = x(self, rs1).wrapping_add(imm as u32);
                let bits = self.fpr_d(rs2).to_bits();
                self.store_u64(bus, sink, addr, bits)?;
            }
            Instruction::CFlwsp { rd, imm } => {
                let addr = x(self, 2).wrapping_add(imm as u32);
                let bits = self.load(bus, sink, addr, AccessSize::Word)?;
                self.write_fpr_s(rd, Fp32::from_bits(bits));
            }
            Instruction::CFswsp { rs2, imm } => {
                let addr = x(self, 2).wrapping_add(imm as u32);
                let bits = self.fpr_s(rs2).to_bits();
                self.store(bus, sink, addr, AccessSize::Word, bits)?;
            }
            Instruction::CFldsp { rd, imm } => {
                let addr = x(self, 2).wrapping_add(imm as u32);
                let bits = self.load_u64(bus, sink, addr)?;
                self.write_fpr_d(rd, Fp64::from_bits(bits));
            }
            Instruction::CFsdsp { rs2, imm } => {
                let addr = x(self, 2).wrapping_add(imm as u32);
                let bits = self.fpr_d(rs2).to_bits();
                self.store_u64(bus, sink, addr, bits)?;
            }
            _ => unreachable_fp_instruction(),
        }

        Ok(Some(next_pc))
    }

    fn fp_rounding_mode(&self, rm: u8, raw: u32) -> Result<RoundingMode, TrapErr> {
        let rm = if rm == 7 {
            ((self.csr.fcsr & csr::FCSR_FRM_MASK) >> csr::FCSR_FRM_SHIFT) as u8
        } else {
            rm
        };
        RoundingMode::from_bits(rm).ok_or(TrapErr {
            cause: cause::ILLEGAL_INSTR,
            mtval: raw,
        })
    }

    fn load_u64<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        addr: u32,
    ) -> Result<u64, TrapErr> {
        if addr & 0x7 != 0 {
            return Err(TrapErr { cause: cause::LOAD_MISALIGNED, mtval: addr });
        }
        let lo = self.load(bus, sink, addr, AccessSize::Word)?;
        let hi = self.load(bus, sink, addr.wrapping_add(4), AccessSize::Word)?;
        Ok(u64::from(lo) | (u64::from(hi) << 32))
    }

    fn store_u64<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        addr: u32,
        value: u64,
    ) -> Result<(), TrapErr> {
        if addr & 0x7 != 0 {
            return Err(TrapErr { cause: cause::STORE_MISALIGNED, mtval: addr });
        }
        self.store(bus, sink, addr, AccessSize::Word, value as u32)?;
        self.store(bus, sink, addr.wrapping_add(4), AccessSize::Word, (value >> 32) as u32)
    }

    fn compare_s<F: FnOnce(Ordering) -> bool>(
        &mut self,
        rd: u8,
        a: Fp32,
        b: Fp32,
        pred: F,
    ) {
        let result = if a.is_signaling_nan() || b.is_signaling_nan() {
            self.raise_fp_flags(Flags::INVALID);
            0
        } else {
            match a.partial_cmp(b) {
                Some(o) if pred(o) => 1,
                _ => 0,
            }
        };
        if rd != 0 {
            self.x[usize::from(rd)] = result;
        }
    }

    fn compare_d<F: FnOnce(Ordering) -> bool>(
        &mut self,
        rd: u8,
        a: Fp64,
        b: Fp64,
        pred: F,
    ) {
        let result = if a.is_signaling_nan() || b.is_signaling_nan() {
            self.raise_fp_flags(Flags::INVALID);
            0
        } else {
            match a.partial_cmp(b) {
                Some(o) if pred(o) => 1,
                _ => 0,
            }
        };
        if rd != 0 {
            self.x[usize::from(rd)] = result;
        }
    }
}

fn is_fp_instruction(instr: Instruction) -> bool {
    matches!(
        instr,
        Instruction::Flw { .. }
            | Instruction::Fsw { .. }
            | Instruction::Fld { .. }
            | Instruction::Fsd { .. }
            | Instruction::FmvWX { .. }
            | Instruction::FmvXW { .. }
            | Instruction::FsgnjS { .. }
            | Instruction::FsgnjnS { .. }
            | Instruction::FsgnjxS { .. }
            | Instruction::FsgnjD { .. }
            | Instruction::FsgnjnD { .. }
            | Instruction::FsgnjxD { .. }
            | Instruction::FeqS { .. }
            | Instruction::FltS { .. }
            | Instruction::FleS { .. }
            | Instruction::FeqD { .. }
            | Instruction::FltD { .. }
            | Instruction::FleD { .. }
            | Instruction::FclassS { .. }
            | Instruction::FclassD { .. }
            | Instruction::FaddS { .. }
            | Instruction::FsubS { .. }
            | Instruction::FmulS { .. }
            | Instruction::FdivS { .. }
            | Instruction::FsqrtS { .. }
            | Instruction::FaddD { .. }
            | Instruction::FsubD { .. }
            | Instruction::FmulD { .. }
            | Instruction::FdivD { .. }
            | Instruction::FsqrtD { .. }
            | Instruction::FminS { .. }
            | Instruction::FmaxS { .. }
            | Instruction::FminD { .. }
            | Instruction::FmaxD { .. }
            | Instruction::CFlw { .. }
            | Instruction::CFsw { .. }
            | Instruction::CFld { .. }
            | Instruction::CFsd { .. }
            | Instruction::CFlwsp { .. }
            | Instruction::CFswsp { .. }
            | Instruction::CFldsp { .. }
            | Instruction::CFsdsp { .. }
    )
}

fn unreachable_fp_instruction() -> ! {
    panic!("exec_fp called with non-FP instruction")
}
