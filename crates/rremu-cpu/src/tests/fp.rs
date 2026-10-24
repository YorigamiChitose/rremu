use super::*;

fn fs(h: &Harness) -> u32 {
    h.cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_FS_MASK
}

fn sd(h: &Harness) -> u32 {
    h.cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_SD
}

#[test]
fn fp_csr_aliases_and_writes_mark_dirty() {
    let mut h = Harness::new();
    h.cpu.csr.mstatus = csr::MSTATUS_FS_INITIAL;

    h.cpu.csr_write(csr::FCSR, 0xe5);
    assert_eq!(h.cpu.debug_csr_read(csr::FCSR), Some(0xe5));
    assert_eq!(h.cpu.debug_csr_read(csr::FFLAGS), Some(0x05));
    assert_eq!(h.cpu.debug_csr_read(csr::FRM), Some(0x07));
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
    assert_eq!(sd(&h), csr::MSTATUS_SD);

    h.cpu.csr_write(csr::FFLAGS, 0x12);
    assert_eq!(h.cpu.debug_csr_read(csr::FCSR), Some(0xf2));

    h.cpu.csr_write(csr::FRM, 0x02);
    assert_eq!(h.cpu.debug_csr_read(csr::FCSR), Some(0x52));
    assert_eq!(h.cpu.debug_csr_read(csr::FFLAGS), Some(0x12));
    assert_eq!(h.cpu.debug_csr_read(csr::FRM), Some(0x02));

    h.cpu.csr_write(csr::FCSR, u32::MAX);
    assert_eq!(h.cpu.debug_csr_read(csr::FCSR), Some(csr::FCSR_MASK));
}

#[test]
fn fp_csr_read_does_not_mark_dirty() {
    let mut h = Harness::new();
    h.cpu.csr.mstatus = csr::MSTATUS_FS_CLEAN;
    h.load(BASE, &[i_enc(0b1110011, 1, 0b010, 0, csr::FCSR as i32)]);

    h.step();

    assert_eq!(h.cpu.reg(1), 0);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);
    assert_eq!(sd(&h), 0);
}

#[test]
fn fp_csr_access_is_illegal_when_fs_off() {
    for addr in [csr::FFLAGS, csr::FRM, csr::FCSR] {
        let mut h = Harness::new();
        let raw = i_enc(0b1110011, 1, 0b010, 0, i32::from(addr));
        h.load(BASE, &[raw]);

        let out = h.step();

        assert_eq!(
            out,
            StepOutcome::Trapped {
                cause: cause::ILLEGAL_INSTR
            }
        );
        assert_eq!(h.cpu.csr_state().mtval, raw);
        assert_eq!(fs(&h), csr::MSTATUS_FS_OFF);
    }
}

#[test]
fn sd_is_read_only_and_sstatus_views_fs() {
    let mut h = Harness::new();
    h.cpu.csr.mstatus = csr::MSTATUS_MPP_M | csr::MSTATUS_FS_DIRTY;

    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
    assert_eq!(sd(&h), csr::MSTATUS_SD);
    let sstatus = h.cpu.debug_csr_read(csr::SSTATUS).unwrap();
    assert_eq!(sstatus & csr::MSTATUS_FS_MASK, csr::MSTATUS_FS_DIRTY);
    assert_eq!(sstatus & csr::MSTATUS_SD, csr::MSTATUS_SD);

    // SD 写入被忽略，FS 可以写回 Clean。
    h.cpu
        .csr_write(csr::MSTATUS, csr::MSTATUS_SD | csr::MSTATUS_FS_CLEAN);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);
    assert_eq!(sd(&h), 0);
    assert_eq!(h.cpu.csr_state().mstatus & csr::MSTATUS_SD, 0);

    // sstatus 是 FS 的可写视图。
    h.cpu.csr_write(csr::SSTATUS, csr::MSTATUS_FS_DIRTY);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
    assert_eq!(sd(&h), csr::MSTATUS_SD);
}
