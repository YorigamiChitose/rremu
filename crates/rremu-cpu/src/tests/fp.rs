use super::*;
use rremu_fp::{Fp32, Fp64};

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

fn fp_load_enc(f3: u32, rd: u8, rs1: u8, imm: i32) -> Word {
    i_enc(0b0000111, rd, f3, rs1, imm)
}

fn fp_store_enc(f3: u32, rs1: u8, rs2: u8, imm: i32) -> Word {
    let i = imm as u32;
    (((i >> 5) & 0x7f) << 25)
        | (u32::from(rs2) << 20)
        | (u32::from(rs1) << 15)
        | (f3 << 12)
        | ((i & 0x1f) << 7)
        | 0b0100111
}

fn fp_r_enc(rd: u8, f3: u32, rs1: u8, rs2: u8, f7: u32) -> Word {
    (f7 << 25)
        | (u32::from(rs2) << 20)
        | (u32::from(rs1) << 15)
        | (f3 << 12)
        | (u32::from(rd) << 7)
        | 0b1010011
}

fn fp_fma_enc(op: u32, rd: u8, rs1: u8, rs2: u8, rs3: u8, fmt: u32, rm: u32) -> Word {
    (u32::from(rs3) << 27)
        | (fmt << 25)
        | (u32::from(rs2) << 20)
        | (u32::from(rs1) << 15)
        | (rm << 12)
        | (u32::from(rd) << 7)
        | op
}

fn set_fs(h: &mut Harness, fs: u32) {
    h.cpu.csr.mstatus = (h.cpu.csr.mstatus & !csr::MSTATUS_FS_MASK) | fs;
}

fn fflags(h: &Harness) -> u32 {
    h.cpu.debug_csr_read(csr::FFLAGS).unwrap()
}

#[test]
fn fp_load_store_and_compressed_track_fs() {
    let mut h = Harness::new();
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[2] = BASE + 0x100;
    h.load(BASE + 0x100, &[0x3f80_0000]);
    h.load(BASE, &[fp_load_enc(0b010, 1, 2, 0)]);

    assert_eq!(
        h.step(),
        StepOutcome::Executed {
            instr: Instruction::Flw {
                rd: 1,
                rs1: 2,
                imm: 0
            },
            len: 4,
            bits: fp_load_enc(0b010, 1, 2, 0),
        }
    );
    assert_eq!(h.cpu.fpr_s(1).to_bits(), 0x3f80_0000);
    assert_eq!(h.cpu.fprs()[1], 0xffff_ffff_3f80_0000);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[3] = BASE + 0x104;
    h.load(BASE + 4, &[fp_store_enc(0b010, 3, 1, 0)]);
    h.step();
    assert_eq!(
        h.bus.peek(BASE + 0x104, AccessSize::Word).unwrap(),
        0x3f80_0000
    );
    // fsw 只读取 FPR，不应把 Clean 变 Dirty。
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);

    // C.FLW / C.FSW 复用相同路径。
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    let c_flw = encode_c(&Instruction::CFlw {
        rd: 8,
        rs1: 9,
        imm: 4,
    });
    h.bus.write_bytes(BASE + 8, &c_flw.to_le_bytes()).unwrap();
    h.cpu.x[9] = BASE + 0x100;
    h.load(BASE + 0x110, &[0x4040_0000]);
    h.cpu.x[9] = BASE + 0x10c; // c.flw f8, 4(x9) -> BASE+0x110
    h.step();
    assert_eq!(h.cpu.fpr_s(8).to_bits(), 0x4040_0000);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    let c_fsw = encode_c(&Instruction::CFsw {
        rs1: 9,
        rs2: 8,
        imm: 4,
    });
    h.bus.write_bytes(BASE + 10, &c_fsw.to_le_bytes()).unwrap();
    h.cpu.pc = BASE + 10;
    h.step();
    assert_eq!(
        h.bus.peek(BASE + 0x110, AccessSize::Word).unwrap(),
        0x4040_0000
    );

    // C.FLD / C.FSD 走 64 位读写路径。
    h.load(BASE + 0x140, &[0x0123_4567, 0x89ab_cdef]);
    let c_fld = encode_c(&Instruction::CFld {
        rd: 10,
        rs1: 9,
        imm: 8,
    });
    h.bus.write_bytes(BASE + 12, &c_fld.to_le_bytes()).unwrap();
    h.cpu.pc = BASE + 12;
    h.cpu.x[9] = BASE + 0x138; // +8 -> BASE+0x140
    h.step();
    assert_eq!(h.cpu.fpr_d(10).to_bits(), 0x89ab_cdef_0123_4567);

    let c_fsd = encode_c(&Instruction::CFsd {
        rs1: 9,
        rs2: 10,
        imm: 8,
    });
    h.bus.write_bytes(BASE + 14, &c_fsd.to_le_bytes()).unwrap();
    h.cpu.pc = BASE + 14;
    h.step();
    assert_eq!(
        h.bus.peek(BASE + 0x140, AccessSize::Word).unwrap(),
        0x0123_4567
    );
    assert_eq!(
        h.bus.peek(BASE + 0x144, AccessSize::Word).unwrap(),
        0x89ab_cdef
    );
}

#[test]
fn fp_double_load_store_and_alignment() {
    let mut h = Harness::new();
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[2] = BASE + 0x140;
    h.load(BASE + 0x140, &[0x0123_4567, 0x89ab_cdef]);
    h.load(BASE, &[fp_load_enc(0b011, 2, 2, 0)]);

    h.step();
    assert_eq!(h.cpu.fpr_d(2).to_bits(), 0x89ab_cdef_0123_4567);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[3] = BASE + 0x150;
    h.load(BASE + 4, &[fp_store_enc(0b011, 3, 2, 0)]);
    h.step();
    assert_eq!(
        h.bus.peek(BASE + 0x150, AccessSize::Word).unwrap(),
        0x0123_4567
    );
    assert_eq!(
        h.bus.peek(BASE + 0x154, AccessSize::Word).unwrap(),
        0x89ab_cdef
    );
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);

    // 8 字节访问必须自然对齐。
    let mut h = Harness::new();
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[2] = BASE + 0x142;
    h.load(BASE, &[fp_load_enc(0b011, 2, 2, 0)]);
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::LOAD_MISALIGNED
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, BASE + 0x142);

    let mut h = Harness::new();
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[2] = BASE + 0x146;
    h.load(BASE, &[fp_store_enc(0b011, 2, 1, 0)]);
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::STORE_MISALIGNED
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, BASE + 0x146);
}

#[test]
fn fp_move_nan_box_and_sign_injection() {
    let mut h = Harness::new();
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.x[5] = 0x7fc0_1234;
    h.load(BASE, &[fp_r_enc(6, 0b000, 5, 0, 0b1111000)]);
    h.step();
    assert_eq!(h.cpu.fprs()[6], 0xffff_ffff_7fc0_1234);
    assert_eq!(h.cpu.fpr_s(6).to_bits(), 0x7fc0_1234);

    // 非法 NaN box 的高半不是全 1：读取时必须规范化为 canonical NaN。
    h.cpu.fprs[7] = 0x0000_0000_7fc0_1234;
    assert_eq!(h.cpu.fpr_s(7).to_bits(), 0x7fc0_0000);

    // fsgnj.s 保留 payload，只替换符号位。
    h.load(BASE + 4, &[fp_r_enc(8, 0b000, 6, 6, 0b0010000)]);
    h.step();
    assert_eq!(h.cpu.fpr_s(8).to_bits(), 0x7fc0_1234);
    assert_eq!(h.cpu.fprs()[8], 0xffff_ffff_7fc0_1234);

    // fsgnjn.s 反转 rs2 的符号位。
    h.load(BASE + 8, &[fp_r_enc(9, 0b001, 7, 7, 0b0010000)]);
    h.step();
    assert_eq!(h.cpu.fpr_s(9).to_bits(), 0xffc0_0000);

    // fsgnjx.s 对两个符号位做 XOR。
    h.cpu.fprs[10] = 0xffff_ffff_8000_0001;
    h.load(BASE + 12, &[fp_r_enc(11, 0b010, 6, 10, 0b0010000)]);
    h.step();
    assert_eq!(h.cpu.fpr_s(11).to_bits(), 0xffc0_1234);

    // fmv.x.w 只取低 32 位。
    h.load(BASE + 16, &[fp_r_enc(12, 0b000, 6, 0, 0b1110000)]);
    h.step();
    assert_eq!(h.cpu.reg(12), 0x7fc0_1234);

    // .d 的符号注入保留完整 64 位 payload。
    h.cpu
        .write_fpr_d(13, Fp64::from_bits(0x7ff8_1234_5678_9abc));
    h.cpu
        .write_fpr_d(14, Fp64::from_bits(0x8000_0000_0000_0000));
    h.load(BASE + 20, &[fp_r_enc(15, 0b000, 13, 14, 0b0010001)]);
    h.step();
    assert_eq!(h.cpu.fpr_d(15).to_bits(), 0xfff8_1234_5678_9abc);
}

#[test]
fn fp_compare_flags_and_classify() {
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3f80_0000)); // +1
    h.cpu.write_fpr_s(2, Fp32::from_bits(0x3f80_0000)); // +1
    h.cpu.write_fpr_s(3, Fp32::from_bits(0x4000_0000)); // +2
    h.cpu.write_fpr_s(4, Fp32::from_bits(0x7fc0_0000)); // qNaN
    h.cpu.write_fpr_s(5, Fp32::from_bits(0x7f80_0001)); // sNaN
    h.cpu.write_fpr_s(6, Fp32::from_bits(0x8000_0000)); // -0
    h.cpu.write_fpr_s(7, Fp32::from_bits(0x0000_0000)); // +0
    h.cpu.write_fpr_s(8, Fp32::from_bits(0x8000_0001)); // -min subnormal
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.cpu.csr.fcsr = 0;

    h.load(
        BASE,
        &[
            fp_r_enc(20, 0b010, 1, 2, 0b1010000), // feq.s
            fp_r_enc(21, 0b001, 1, 3, 0b1010000), // flt.s
            fp_r_enc(22, 0b000, 3, 1, 0b1010000), // fle.s
            fp_r_enc(23, 0b010, 6, 7, 0b1010000), // feq.s -0,+0
            fp_r_enc(24, 0b010, 4, 4, 0b1010000), // feq.s qNaN,qNaN
            fp_r_enc(25, 0b001, 5, 1, 0b1010000), // flt.s sNaN,+1
            fp_r_enc(26, 0b001, 8, 1, 0b1010000), // flt.s -min,+1
            fp_r_enc(27, 0b001, 8, 0, 0b1110000), // fclass.s
        ],
    );
    h.run(5);
    assert_eq!(h.cpu.reg(20), 1);
    assert_eq!(h.cpu.reg(21), 1);
    assert_eq!(h.cpu.reg(22), 0);
    assert_eq!(h.cpu.reg(23), 1);
    assert_eq!(h.cpu.reg(24), 0);
    assert_eq!(fflags(&h), 0);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);

    h.step(); // sNaN 比较
    assert_eq!(h.cpu.reg(25), 0);
    assert_eq!(fflags(&h), 0x10);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    h.step();
    assert_eq!(h.cpu.reg(26), 1);
    h.step();
    assert_eq!(h.cpu.reg(27), 1 << 2); // negative subnormal
}

#[test]
fn fp_arithmetic_s_updates_result_flags_and_fs() {
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3f80_0000)); // 1.0
    h.cpu.write_fpr_s(2, Fp32::from_bits(0x4000_0000)); // 2.0
    h.cpu.write_fpr_s(3, Fp32::from_bits(0x4040_0000)); // 3.0
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 2, 0b0000000), // fadd.s 1+2
            fp_r_enc(11, 0, 1, 2, 0b0000100), // fsub.s 1-2
            fp_r_enc(12, 0, 1, 2, 0b0001000), // fmul.s 1*2
            fp_r_enc(13, 0, 1, 3, 0b0001100), // fdiv.s 1/3
            fp_r_enc(14, 0, 2, 0, 0b0101100), // fsqrt.s sqrt(2)
        ],
    );

    h.run(4);
    assert_eq!(h.cpu.fpr_s(10).to_bits(), 0x4040_0000);
    assert_eq!(h.cpu.fpr_s(11).to_bits(), 0xbf80_0000);
    assert_eq!(h.cpu.fpr_s(12).to_bits(), 0x4000_0000);
    assert_eq!(h.cpu.fpr_s(13).to_bits(), 0x3eaa_aaab);
    assert_eq!(fflags(&h), 0x01);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    h.step();
    assert_eq!(h.cpu.fpr_s(14).to_bits(), 0x3fb5_04f3);
    assert_eq!(fflags(&h), 0x01);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
}

#[test]
fn fp_arithmetic_d_updates_result_flags_and_fs() {
    let mut h = Harness::new();
    h.cpu.write_fpr_d(1, Fp64::from_bits(0x3ff0_0000_0000_0000)); // 1.0
    h.cpu.write_fpr_d(2, Fp64::from_bits(0x4000_0000_0000_0000)); // 2.0
    h.cpu.write_fpr_d(3, Fp64::from_bits(0x4008_0000_0000_0000)); // 3.0
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 2, 0b0000001), // fadd.d 1+2
            fp_r_enc(11, 0, 1, 2, 0b0000101), // fsub.d 1-2
            fp_r_enc(12, 0, 1, 2, 0b0001001), // fmul.d 1*2
            fp_r_enc(13, 0, 1, 3, 0b0001101), // fdiv.d 1/3
            fp_r_enc(14, 0, 2, 0, 0b0101101), // fsqrt.d sqrt(2)
        ],
    );

    h.run(4);
    assert_eq!(h.cpu.fpr_d(10).to_bits(), 0x4008_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(11).to_bits(), 0xbff0_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(12).to_bits(), 0x4000_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(13).to_bits(), 0x3fd5_5555_5555_5555);
    assert_eq!(fflags(&h), 0x01);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    h.step();
    assert_eq!(h.cpu.fpr_d(14).to_bits(), 0x3ff6_a09e_667f_3bcd);
    assert_eq!(fflags(&h), 0x01);
}

#[test]
fn fp_dynamic_rounding_mode_and_reserved_modes() {
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3f80_0000)); // 1.0
    h.cpu.write_fpr_s(2, Fp32::from_bits(0x4040_0000)); // 3.0
    h.cpu.csr.fcsr = 1 << csr::FCSR_FRM_SHIFT; // rtz
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(BASE, &[fp_r_enc(10, 7, 1, 2, 0b0001100)]); // fdiv.s rtz

    h.step();
    assert_eq!(h.cpu.fpr_s(10).to_bits(), 0x3eaa_aaaa);
    assert_eq!(fflags(&h), 0x01);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    // 动态 frm 为保留值时，指令本身是合法编码，但执行时 illegal。
    let raw = fp_r_enc(11, 7, 1, 2, 0b0001100);
    h.cpu.csr.fcsr = 5 << csr::FCSR_FRM_SHIFT;
    h.cpu.write_fpr_s(11, Fp32::from_bits(0x3f80_0000));
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(BASE + 4, &[raw]);
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::ILLEGAL_INSTR
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, raw);
    assert_eq!(h.cpu.fpr_s(11).to_bits(), 0x3f80_0000);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);

    // 静态 rm=5/6 在解码阶段就被拒绝。
    for rm in [5, 6] {
        let raw = fp_r_enc(12, rm, 1, 2, 0b0000000);
        let mut h = Harness::new();
        h.load(BASE, &[raw]);
        assert_eq!(
            h.step(),
            StepOutcome::Trapped {
                cause: cause::ILLEGAL_INSTR
            }
        );
        assert_eq!(h.cpu.csr_state().mtval, raw);
    }
}

#[test]
fn fp_minmax_s_zero_nan_and_flags() {
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3f80_0000)); // 1.0
    h.cpu.write_fpr_s(2, Fp32::from_bits(0x4000_0000)); // 2.0
    h.cpu.write_fpr_s(3, Fp32::from_bits(0x8000_0000)); // -0
    h.cpu.write_fpr_s(4, Fp32::from_bits(0x0000_0000)); // +0
    h.cpu.write_fpr_s(5, Fp32::from_bits(0x7fc0_0000)); // qNaN
    h.cpu.write_fpr_s(6, Fp32::from_bits(0x7f80_0001)); // sNaN
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 2, 0b0010100), // fmin.s 1,2
            fp_r_enc(11, 1, 1, 2, 0b0010100), // fmax.s 1,2
            fp_r_enc(12, 0, 3, 4, 0b0010100), // fmin.s -0,+0
            fp_r_enc(13, 1, 3, 4, 0b0010100), // fmax.s -0,+0
            fp_r_enc(14, 0, 5, 1, 0b0010100), // fmin.s qNaN,1
            fp_r_enc(15, 0, 5, 5, 0b0010100), // fmin.s qNaN,qNaN
            fp_r_enc(16, 1, 6, 1, 0b0010100), // fmax.s sNaN,1
        ],
    );

    h.run(6);
    assert_eq!(h.cpu.fpr_s(10).to_bits(), 0x3f80_0000);
    assert_eq!(h.cpu.fpr_s(11).to_bits(), 0x4000_0000);
    assert_eq!(h.cpu.fpr_s(12).to_bits(), 0x8000_0000);
    assert_eq!(h.cpu.fpr_s(13).to_bits(), 0x0000_0000);
    assert_eq!(h.cpu.fpr_s(14).to_bits(), 0x3f80_0000);
    assert_eq!(h.cpu.fpr_s(15).to_bits(), 0x7fc0_0000);
    assert_eq!(fflags(&h), 0);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    h.step();
    assert_eq!(h.cpu.fpr_s(16).to_bits(), 0x3f80_0000);
    assert_eq!(fflags(&h), 0x10);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
}

#[test]
fn fp_minmax_d_zero_nan_and_flags() {
    let mut h = Harness::new();
    h.cpu.write_fpr_d(1, Fp64::from_bits(0x3ff0_0000_0000_0000)); // 1.0
    h.cpu.write_fpr_d(2, Fp64::from_bits(0x4000_0000_0000_0000)); // 2.0
    h.cpu.write_fpr_d(3, Fp64::from_bits(0x8000_0000_0000_0000)); // -0
    h.cpu.write_fpr_d(4, Fp64::from_bits(0x0000_0000_0000_0000)); // +0
    h.cpu.write_fpr_d(5, Fp64::from_bits(0x7ff8_0000_0000_0000)); // qNaN
    h.cpu.write_fpr_d(6, Fp64::from_bits(0x7ff0_0000_0000_0001)); // sNaN
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 2, 0b0010101), // fmin.d 1,2
            fp_r_enc(11, 1, 1, 2, 0b0010101), // fmax.d 1,2
            fp_r_enc(12, 0, 3, 4, 0b0010101), // fmin.d -0,+0
            fp_r_enc(13, 1, 3, 4, 0b0010101), // fmax.d -0,+0
            fp_r_enc(14, 0, 5, 1, 0b0010101), // fmin.d qNaN,1
            fp_r_enc(15, 1, 6, 1, 0b0010101), // fmax.d sNaN,1
        ],
    );

    h.run(5);
    assert_eq!(h.cpu.fpr_d(10).to_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(11).to_bits(), 0x4000_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(12).to_bits(), 0x8000_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(13).to_bits(), 0x0000_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(14).to_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(fflags(&h), 0);

    h.step();
    assert_eq!(h.cpu.fpr_d(15).to_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(fflags(&h), 0x10);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
}

#[test]
fn fp_fma_s_operations_and_single_rounding() {
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3f80_0000)); // 1.0
    h.cpu.write_fpr_s(2, Fp32::from_bits(0x4000_0000)); // 2.0
    h.cpu.write_fpr_s(3, Fp32::from_bits(0x4040_0000)); // 3.0
    h.cpu.write_fpr_s(4, Fp32::from_bits(0x3f80_0001)); // 1+ulp
    h.cpu.write_fpr_s(5, Fp32::from_bits(0xbf80_0002)); // -(1+2ulp)
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_fma_enc(0b1000011, 10, 1, 2, 3, 0, 0), // fmadd.s 1*2+3
            fp_fma_enc(0b1000111, 11, 1, 2, 3, 0, 0), // fmsub.s 1*2-3
            fp_fma_enc(0b1001011, 12, 1, 2, 3, 0, 0), // fnmsub.s -(1*2)+3
            fp_fma_enc(0b1001111, 13, 1, 2, 3, 0, 0), // fnmadd.s -(1*2)-3
            fp_fma_enc(0b1000011, 14, 4, 4, 5, 0, 0), // fused exact 2^-46
        ],
    );

    h.run(5);
    assert_eq!(h.cpu.fpr_s(10).to_bits(), 0x40a0_0000);
    assert_eq!(h.cpu.fpr_s(11).to_bits(), 0xbf80_0000);
    assert_eq!(h.cpu.fpr_s(12).to_bits(), 0x3f80_0000);
    assert_eq!(h.cpu.fpr_s(13).to_bits(), 0xc0a0_0000);
    assert_eq!(h.cpu.fpr_s(14).to_bits(), 0x2880_0000);
    assert_eq!(fflags(&h), 0);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
}

#[test]
fn fp_fma_d_operations_and_rounding_modes() {
    let mut h = Harness::new();
    h.cpu.write_fpr_d(1, Fp64::from_bits(0x3ff0_0000_0000_0000)); // 1.0
    h.cpu.write_fpr_d(2, Fp64::from_bits(0x4000_0000_0000_0000)); // 2.0
    h.cpu.write_fpr_d(3, Fp64::from_bits(0x4008_0000_0000_0000)); // 3.0
    h.cpu.write_fpr_d(4, Fp64::from_bits(0x3ca0_0000_0000_0000)); // 2^-53
    h.cpu.csr.fcsr = 1 << csr::FCSR_FRM_SHIFT; // rtz
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_fma_enc(0b1000011, 10, 1, 2, 3, 1, 0), // fmadd.d 1*2+3
            fp_fma_enc(0b1000111, 11, 1, 2, 3, 1, 0), // fmsub.d 1*2-3
            fp_fma_enc(0b1001011, 12, 1, 2, 3, 1, 0), // fnmsub.d -(1*2)+3
            fp_fma_enc(0b1001111, 13, 1, 2, 3, 1, 0), // fnmadd.d -(1*2)-3
            fp_fma_enc(0b1000011, 14, 1, 1, 4, 1, 7), // fmadd.d dyn rtz tie
        ],
    );

    h.run(5);
    assert_eq!(h.cpu.fpr_d(10).to_bits(), 0x4014_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(11).to_bits(), 0xbff0_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(12).to_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(13).to_bits(), 0xc014_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(14).to_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(fflags(&h), 0x01);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    // 动态 frm 为保留值时 FMA 在执行阶段 illegal。
    let raw = fp_fma_enc(0b1000011, 15, 1, 2, 3, 1, 7);
    h.cpu.csr.fcsr = 5 << csr::FCSR_FRM_SHIFT;
    h.cpu
        .write_fpr_d(15, Fp64::from_bits(0x3ff0_0000_0000_0000));
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(BASE + 20, &[raw]);
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::ILLEGAL_INSTR
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, raw);
    assert_eq!(h.cpu.fpr_d(15).to_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);

    // 静态 rm=5/6 在解码阶段被拒绝。
    for rm in [5, 6] {
        let raw = fp_fma_enc(0b1000011, 16, 1, 2, 3, 1, rm);
        let mut h = Harness::new();
        h.load(BASE, &[raw]);
        assert_eq!(
            h.step(),
            StepOutcome::Trapped {
                cause: cause::ILLEGAL_INSTR
            }
        );
        assert_eq!(h.cpu.csr_state().mtval, raw);
    }
}

#[test]
fn fp_convert_float_to_int_and_flags() {
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3fc0_0000)); // 1.5
    h.cpu.write_fpr_s(2, Fp32::from_bits(0x4020_0000)); // 2.5
    h.cpu.write_fpr_s(3, Fp32::from_bits(0x4f00_0000)); // 2^31
    h.cpu.write_fpr_s(4, Fp32::from_bits(0xbf00_0000)); // -0.5
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 0, 0b1100000), // fcvt.w.s rne
            fp_r_enc(11, 1, 2, 1, 0b1100000), // fcvt.wu.s rtz
            fp_r_enc(12, 1, 3, 0, 0b1100000), // fcvt.w.s rtz overflow
            fp_r_enc(13, 0, 4, 1, 0b1100000), // fcvt.wu.s rne negative fraction
            fp_r_enc(14, 2, 4, 0, 0b1100000), // fcvt.w.s rdn negative fraction
        ],
    );

    h.run(5);
    assert_eq!(h.cpu.reg(10), 2);
    assert_eq!(h.cpu.reg(11), 2);
    assert_eq!(h.cpu.reg(12), 0x7fff_ffff);
    assert_eq!(h.cpu.reg(13), 0);
    assert_eq!(h.cpu.reg(14), 0xffff_ffff);
    assert_eq!(fflags(&h), 0x11); // INVALID | INEXACT
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    // 精确且无 flags 的 f->x 转换不应写 FP 状态。
    let mut h = Harness::new();
    h.cpu.write_fpr_s(1, Fp32::from_bits(0x3f80_0000)); // 1.0
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(BASE, &[fp_r_enc(10, 0, 1, 0, 0b1100000)]);
    h.step();
    assert_eq!(h.cpu.reg(10), 1);
    assert_eq!(fflags(&h), 0);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);
}

#[test]
fn fp_convert_double_to_int_and_flags() {
    let mut h = Harness::new();
    h.cpu.write_fpr_d(1, Fp64::from_bits(0x3ff8_0000_0000_0000)); // 1.5
    h.cpu.write_fpr_d(2, Fp64::from_bits(0x4004_0000_0000_0000)); // 2.5
    h.cpu.write_fpr_d(3, Fp64::from_bits(0x41e0_0000_0000_0000)); // 2^31
    h.cpu.write_fpr_d(4, Fp64::from_bits(0xbfe0_0000_0000_0000)); // -0.5
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 0, 0b1100001), // fcvt.w.d rne
            fp_r_enc(11, 1, 2, 1, 0b1100001), // fcvt.wu.d rtz
            fp_r_enc(12, 1, 3, 0, 0b1100001), // fcvt.w.d rtz overflow
            fp_r_enc(13, 0, 4, 1, 0b1100001), // fcvt.wu.d rne negative fraction
            fp_r_enc(14, 2, 4, 0, 0b1100001), // fcvt.w.d rdn negative fraction
        ],
    );

    h.run(5);
    assert_eq!(h.cpu.reg(10), 2);
    assert_eq!(h.cpu.reg(11), 2);
    assert_eq!(h.cpu.reg(12), 0x7fff_ffff);
    assert_eq!(h.cpu.reg(13), 0);
    assert_eq!(h.cpu.reg(14), 0xffff_ffff);
    assert_eq!(fflags(&h), 0x11);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);
}

#[test]
fn fp_convert_int_to_float_and_cross_precision() {
    let mut h = Harness::new();
    h.cpu.x[1] = 0x0100_0001; // 2^24 + 1
    h.cpu.x[2] = 0xffff_ffff; // u32::MAX
    h.cpu.x[3] = 0x8000_0000; // i32::MIN
    h.cpu.csr.fcsr = 0;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(
        BASE,
        &[
            fp_r_enc(10, 0, 1, 0, 0b1101000),  // fcvt.s.w rne
            fp_r_enc(11, 0, 2, 1, 0b1101000),  // fcvt.s.wu rne
            fp_r_enc(12, 0, 3, 0, 0b1101001),  // fcvt.d.w rne
            fp_r_enc(13, 0, 2, 1, 0b1101001),  // fcvt.d.wu rne
            fp_r_enc(14, 0, 12, 1, 0b0100000), // fcvt.s.d rne
            fp_r_enc(15, 0, 14, 0, 0b0100001), // fcvt.d.s
        ],
    );

    h.run(4);
    assert_eq!(h.cpu.fpr_s(10).to_bits(), 0x4b80_0000);
    assert_eq!(h.cpu.fpr_s(11).to_bits(), 0x4f80_0000);
    assert_eq!(h.cpu.fpr_d(12).to_bits(), 0xc1e0_0000_0000_0000);
    assert_eq!(h.cpu.fpr_d(13).to_bits(), 0x41ef_ffff_ffe0_0000);
    assert_eq!(fflags(&h), 0x01);
    assert_eq!(fs(&h), csr::MSTATUS_FS_DIRTY);

    h.step(); // fcvt.s.d -2^31
    assert_eq!(h.cpu.fpr_s(14).to_bits(), 0xcf00_0000);
    h.step(); // fcvt.d.s back
    assert_eq!(h.cpu.fpr_d(15).to_bits(), 0xc1e0_0000_0000_0000);
    assert_eq!(fflags(&h), 0x01);
}

#[test]
fn fp_convert_dynamic_rounding_and_reserved_modes() {
    let mut h = Harness::new();
    h.cpu.x[1] = 0x0100_0001; // 2^24 + 1
    h.cpu.csr.fcsr = 1 << csr::FCSR_FRM_SHIFT; // rtz
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(BASE, &[fp_r_enc(10, 7, 1, 0, 0b1101000)]); // fcvt.s.w dyn
    h.step();
    assert_eq!(h.cpu.fpr_s(10).to_bits(), 0x4b80_0000);
    assert_eq!(fflags(&h), 0x01);

    let raw = fp_r_enc(11, 7, 1, 0, 0b1100000); // fcvt.w.s dyn
    h.cpu.csr.fcsr = 5 << csr::FCSR_FRM_SHIFT;
    h.cpu.x[11] = 0x1234_5678;
    set_fs(&mut h, csr::MSTATUS_FS_CLEAN);
    h.load(BASE + 4, &[raw]);
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::ILLEGAL_INSTR
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, raw);
    assert_eq!(h.cpu.reg(11), 0x1234_5678);
    assert_eq!(fs(&h), csr::MSTATUS_FS_CLEAN);

    for rm in [5, 6] {
        let raw = fp_r_enc(12, rm, 1, 0, 0b1100000);
        let mut h = Harness::new();
        h.load(BASE, &[raw]);
        assert_eq!(
            h.step(),
            StepOutcome::Trapped {
                cause: cause::ILLEGAL_INSTR
            }
        );
        assert_eq!(h.cpu.csr_state().mtval, raw);
    }
}

#[test]
fn fp_instructions_are_illegal_when_fs_off() {
    let instructions = [
        fp_load_enc(0b010, 1, 0, 0),
        fp_store_enc(0b010, 0, 1, 0),
        fp_load_enc(0b011, 1, 0, 0),
        fp_store_enc(0b011, 0, 1, 0),
        fp_r_enc(1, 0b000, 2, 0, 0b1111000),
        fp_r_enc(1, 0b000, 2, 3, 0b0010000),
        fp_r_enc(1, 0b010, 2, 3, 0b1010000),
        fp_r_enc(1, 0b001, 2, 0, 0b1110000),
        fp_r_enc(1, 0b000, 2, 3, 0b0000000),
        fp_r_enc(1, 0b000, 2, 3, 0b0001100),
        fp_r_enc(1, 0b000, 2, 0, 0b0101100),
        fp_r_enc(1, 0b000, 2, 3, 0b0010100),
        fp_r_enc(1, 0b001, 2, 3, 0b0010101),
        fp_fma_enc(0b1000011, 1, 2, 3, 4, 0, 0),
        fp_r_enc(1, 0, 2, 0, 0b1100000),
        fp_r_enc(1, 0, 2, 1, 0b1100001),
        fp_r_enc(1, 0, 2, 0, 0b1101000),
        fp_r_enc(1, 0, 2, 1, 0b1101001),
        fp_r_enc(1, 0, 2, 1, 0b0100000),
        fp_r_enc(1, 0, 2, 0, 0b0100001),
    ];
    for raw in instructions {
        let mut h = Harness::new();
        h.load(BASE, &[raw]);

        assert_eq!(
            h.step(),
            StepOutcome::Trapped {
                cause: cause::ILLEGAL_INSTR
            }
        );
        assert_eq!(h.cpu.csr_state().mtval, raw);
        assert_eq!(fs(&h), csr::MSTATUS_FS_OFF);
    }

    let half = encode_c(&Instruction::CFlw {
        rd: 8,
        rs1: 8,
        imm: 0,
    });
    let mut h = Harness::new();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&half.to_le_bytes());
    bytes.extend_from_slice(&[0, 0]);
    h.bus.write_bytes(BASE, &bytes).unwrap();
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::ILLEGAL_INSTR
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, u32::from(half));
}
