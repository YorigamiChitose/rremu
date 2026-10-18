use super::*;

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
    fn csr_privilege_and_read_only_checks() {
        // S-mode 访问 M-CSR → illegal，mtval 为指令编码
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::Supervisor;
        let instr = i_enc(0b1110011, 1, 0b010, 0, csr::MSTATUS as i32); // csrrs x1, mstatus, x0
        h.load(BASE, &[instr]);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });
        assert_eq!(h.cpu.csr_state().mtval, instr as u32);

        // U-mode 访问 S-CSR → illegal
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::User;
        h.load(BASE, &[i_enc(0b1110011, 1, 0b010, 0, csr::SSTATUS as i32)]);
        assert_eq!(h.step(), StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });

        // M-mode 写只读 CSR（mhartid）→ illegal
        let mut h = Harness::new();
        h.load(BASE, &[i_enc(0b1110011, 0, 0b001, 0, csr::MHARTID as i32)]);
        assert_eq!(h.step(), StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });

        // 只读 CSR 的 CSRRS rs1=x0 不算写，允许读取
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(0b1110011, 1, 0b010, 0, csr::CYCLE as i32),
            i_enc(0b1110011, 2, 0b110, 1, csr::CYCLE as i32), // csrrsi x2, cycle, 1
        ]);
        assert!(matches!(h.step(), StepOutcome::Executed { .. }));
        assert_eq!(h.step(), StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });

        // S-mode 访问 S-CSR 正常
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::Supervisor;
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, 0x5A),
            i_enc(0b1110011, 0, 0b001, 5, csr::SSCRATCH as i32),
            i_enc(0b1110011, 1, 0b010, 0, csr::SSCRATCH as i32),
        ]);
        h.run(3);
        assert_eq!(h.cpu.reg(1), 0x5A);
    }

    #[test]
    fn misa_is_read_only_and_mstatus_write_is_masked() {
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, -1), // x5 = 0xffffffff
            i_enc(0b1110011, 0, 0b001, 5, csr::MISA as i32),
            i_enc(0b1110011, 0, 0b001, 5, csr::MSTATUS as i32),
            i_enc(0b1110011, 1, 0b010, 0, csr::MISA as i32),
            i_enc(0b1110011, 2, 0b010, 0, csr::MSTATUS as i32),
        ]);
        h.run(5);
        let misa = h.cpu.debug_csr_read(csr::MISA).unwrap();
        assert_eq!(misa & csr::MISA_I, csr::MISA_I);
        assert_eq!(misa & csr::MISA_S, csr::MISA_S);
        // mstatus 只接受已实现的可写位
        assert_eq!(h.cpu.reg(2) & !csr::MSTATUS_WRITE_MASK, 0);
        assert_eq!(h.cpu.reg(2) & csr::MSTATUS_WRITE_MASK, csr::MSTATUS_WRITE_MASK);
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
