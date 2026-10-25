//! 浮点寄存器堆访问与 NaN boxing。
//!
//! FLEN=64，但 `.s` 值必须满足高 32 位全 1 的 NaN boxing 约定。
//! 所有 boxing/unboxing 都集中在这里，执行器不得手写移位。

use rremu_fp::{Flags, Fp32, Fp64};
use rremu_isa::csr;

use super::Cpu;

/// `.s` 值的 NaN box：高 32 位必须全 1。
pub const NAN_BOX_MASK: u64 = 0xffff_ffff_0000_0000;

impl Cpu {
    /// 返回全部浮点寄存器的原始 64 位内容。
    pub fn fprs(&self) -> &[u64; 32] {
        &self.fprs
    }

    /// 返回单个浮点寄存器的原始 64 位内容。
    pub fn fpr(&self, i: u8) -> u64 {
        self.fprs[usize::from(i)]
    }

    /// 按 `.s` 语义读取：高 32 位不是全 1 时返回 canonical NaN。
    pub fn fpr_s(&self, i: u8) -> Fp32 {
        let bits = self.fprs[usize::from(i)];
        if bits & NAN_BOX_MASK == NAN_BOX_MASK {
            Fp32::from_bits(bits as u32)
        } else {
            Fp32::canonical_nan()
        }
    }

    /// 按 `.d` 语义读取完整 64 位。
    pub fn fpr_d(&self, i: u8) -> Fp64 {
        Fp64::from_bits(self.fprs[usize::from(i)])
    }

    /// 写入 `.s` 值并置 NaN box；写 FPR 后 FS=Dirty。
    pub(crate) fn write_fpr_s(&mut self, i: u8, value: Fp32) {
        self.fprs[usize::from(i)] = NAN_BOX_MASK | u64::from(value.to_bits());
        self.mark_fp_dirty();
    }

    /// 写入 `.d` 值；写 FPR 后 FS=Dirty。
    pub(crate) fn write_fpr_d(&mut self, i: u8, value: Fp64) {
        self.fprs[usize::from(i)] = value.to_bits();
        self.mark_fp_dirty();
    }

    /// 将 `flags` 的 sticky 位并入 `fcsr.fflags`；有变化时 FS=Dirty。
    pub(crate) fn raise_fp_flags(&mut self, flags: Flags) {
        let current = Flags::from_bits((self.csr.fcsr & csr::FCSR_FFLAGS_MASK) as u8);
        let merged = current | flags;
        if merged != current {
            self.csr.fcsr = (self.csr.fcsr & !csr::FCSR_FFLAGS_MASK) | u32::from(merged.bits());
            self.mark_fp_dirty();
        }
    }

    /// 返回 `fcsr`（含 fflags/frm）。
    pub fn fcsr(&self) -> u32 {
        self.csr.fcsr
    }

    /// 恢复快照中的浮点状态：FPR、fcsr 与 `mstatus.FS`。
    ///
    /// 原始 FPR 位模式原样恢复；调用方负责保证快照来自同一 FLEN。
    pub fn restore_fp_state(&mut self, fprs: &[u64; 32], fcsr: u32, fs: u32) {
        self.fprs = *fprs;
        self.csr.fcsr = fcsr & csr::FCSR_MASK;
        self.csr.mstatus = (self.csr.mstatus & !csr::MSTATUS_FS_MASK) | (fs & csr::MSTATUS_FS_MASK);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use rremu_isa::csr;

    #[test]
    fn reset_clears_fp_registers_and_csr() {
        let mut cpu = Cpu::new(0x8000_0000);
        cpu.fprs[1] = NAN_BOX_MASK | 0x3f80_0000;
        cpu.csr.fcsr = 0x1f;
        cpu.csr.mstatus |= csr::MSTATUS_FS_DIRTY;

        cpu.reset(0x8000_0000);

        assert!(cpu.fprs().iter().all(|&bits| bits == 0));
        assert_eq!(cpu.csr_state().fcsr, 0);
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_FS_MASK,
            csr::MSTATUS_FS_OFF
        );
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_SD,
            0
        );
    }

    #[test]
    fn single_precision_read_uses_low_32_bits_when_boxed() {
        let mut cpu = Cpu::new(0);
        cpu.csr.mstatus = csr::MSTATUS_FS_INITIAL;
        cpu.fprs[1] = NAN_BOX_MASK | 0x3f80_0000;

        assert_eq!(cpu.fpr(1), NAN_BOX_MASK | 0x3f80_0000);
        assert_eq!(cpu.fpr_s(1), Fp32::from_bits(0x3f80_0000));
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_FS_MASK,
            csr::MSTATUS_FS_INITIAL
        );
    }

    #[test]
    fn invalid_nan_box_reads_as_canonical_nan() {
        let mut cpu = Cpu::new(0);
        cpu.csr.mstatus = csr::MSTATUS_FS_CLEAN;
        cpu.fprs[2] = 0x0000_0000_3f80_0000;

        assert_eq!(cpu.fpr_s(2), Fp32::canonical_nan());
        assert_eq!(cpu.fpr_d(2), Fp64::from_bits(0x0000_0000_3f80_0000));
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_FS_MASK,
            csr::MSTATUS_FS_CLEAN
        );
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_SD,
            0
        );
    }

    #[test]
    fn double_precision_read_uses_full_64_bits() {
        let mut cpu = Cpu::new(0);
        cpu.csr.mstatus = csr::MSTATUS_FS_INITIAL;
        let value = Fp64::from_bits(0x3ff0_0000_0000_0000);
        cpu.fprs[3] = value.to_bits();

        assert_eq!(cpu.fpr(3), value.to_bits());
        assert_eq!(cpu.fpr_d(3), value);
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_FS_MASK,
            csr::MSTATUS_FS_INITIAL
        );
    }

    #[test]
    fn fp_snapshot_restore_round_trips() {
        let mut cpu = Cpu::new(0);
        let mut fprs = [0u64; 32];
        fprs[1] = NAN_BOX_MASK | 0x3f80_0000;
        fprs[2] = 0x4000_0000_0000_0000;

        cpu.restore_fp_state(&fprs, 0xff, csr::MSTATUS_FS_DIRTY);

        assert_eq!(cpu.fprs(), &fprs);
        assert_eq!(cpu.fcsr(), csr::FCSR_MASK);
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_FS_MASK,
            csr::MSTATUS_FS_DIRTY
        );
        assert_eq!(
            cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_SD,
            csr::MSTATUS_SD
        );
    }
}
