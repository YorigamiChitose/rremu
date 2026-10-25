use super::*;

    fn amo_enc(f5: u32, f3: u32, rs1: u8, rs2: u8, rd: u8) -> Word {
        (f5 << 27) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | 0b0101111
    }

    #[test]
    fn exec_lr_sc_success() {
        let mut h = Harness::new();
        let addr = BASE + 0x100;
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),          // lui t0, 0x80000
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),     // addi t0, t0, 0x100
            i_enc(OP_IMM, 6, 0b000, 0, 0x123),     // li t1, 0x123
            s_enc(0b010, 5, 6, 0),                 // sw t1, 0(t0)
            amo_enc(0b00010, 0b010, 5, 0, 7),      // lr.w t2, (t0)
            i_enc(OP_IMM, 8, 0b000, 0, 0x567),     // li s0, 0x567
            amo_enc(0b00011, 0b010, 5, 8, 9),      // sc.w s1, s0, (t0)
        ]);
        h.run(7);
        assert_eq!(h.cpu.reg(7), 0x123); // lr.w 读到旧值
        assert_eq!(h.cpu.reg(9), 0);      // sc.w 成功
        assert_eq!(h.bus.peek(addr, AccessSize::Word).unwrap(), 0x567);
    }

    #[test]
    fn exec_lr_sc_fail_after_store() {
        let mut h = Harness::new();
        let addr = BASE + 0x100;
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),
            i_enc(OP_IMM, 6, 0b000, 0, 0x123),
            s_enc(0b010, 5, 6, 0),                 // sw [addr] = 0x123
            amo_enc(0b00010, 0b010, 5, 0, 7),      // lr.w t2, (t0)
            i_enc(OP_IMM, 8, 0b000, 0, 0x789),
            s_enc(0b010, 5, 8, 0),                 // sw [addr] = 0x789（清除保留）
            amo_enc(0b00011, 0b010, 5, 8, 9),      // sc.w s1, s0, (t0)
        ]);
        h.run(8);
        assert_eq!(h.cpu.reg(7), 0x123); // lr.w 读到旧值
        assert_eq!(h.cpu.reg(9), 1);      // sc.w 失败（保留已被 store 清除）
        assert_eq!(h.bus.peek(addr, AccessSize::Word).unwrap(), 0x789); // sc.w 未写
    }

    #[test]
    fn exec_amo_arith_and_logic() {
        let mut h = Harness::new();
        let addr = BASE + 0x100;
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),     // t0 = addr
            i_enc(OP_IMM, 6, 0b000, 0, 0xFF),      // t1 = 0xFF
            s_enc(0b010, 5, 6, 0),                 // [addr] = 0xFF
            i_enc(OP_IMM, 6, 0b000, 0, 0x0F),      // t1 = 0x0F
            amo_enc(0b00100, 0b010, 5, 6, 7),      // amoxor.w t2, t1, (t0)
            amo_enc(0b01100, 0b010, 5, 6, 8),      // amoand.w s0, t1, (t0)
            amo_enc(0b01000, 0b010, 5, 6, 9),      // amoor.w s1, t1, (t0)
        ]);
        h.run(8);
        assert_eq!(h.cpu.reg(7), 0xFF); // amoxor 旧值
        assert_eq!(h.cpu.reg(8), 0xF0); // amoand 旧值（0xFF^0x0F）
        assert_eq!(h.cpu.reg(9), 0x00); // amoor 旧值（0xF0&0x0F）
        assert_eq!(h.bus.peek(addr, AccessSize::Word).unwrap(), 0x0F);
    }

    #[test]
    fn exec_amo_swap_add() {
        let mut h = Harness::new();
        let addr = BASE + 0x100;
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),     // t0 = addr
            i_enc(OP_IMM, 6, 0b000, 0, 5),         // t1 = 5
            s_enc(0b010, 5, 6, 0),                 // [addr] = 5
            i_enc(OP_IMM, 7, 0b000, 0, 3),         // t2 = 3
            amo_enc(0b00000, 0b010, 5, 7, 8),      // amoadd.w s0, t2, (t0)
            i_enc(OP_IMM, 9, 0b000, 0, 99),        // s1 = 99
            amo_enc(0b00001, 0b010, 5, 9, 10),     // amoswap.w a0, s1, (t0)
        ]);
        h.run(8);
        assert_eq!(h.cpu.reg(8), 5);  // amoadd 旧值
        assert_eq!(h.cpu.reg(10), 8); // amoswap 旧值（5+3）
        assert_eq!(h.bus.peek(addr, AccessSize::Word).unwrap(), 99);
    }

    #[test]
    fn exec_amo_minmax_signedness() {
        let mut h = Harness::new();
        let addr = BASE + 0x100;
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),     // t0 = addr
            i_enc(OP_IMM, 6, 0b000, 0, -1),        // t1 = -1 (0xFFFFFFFF)
            s_enc(0b010, 5, 6, 0),                 // [addr] = 0xFFFFFFFF
            amo_enc(0b10100, 0b010, 5, 0, 7),      // amomax.w t2, zero, (t0) → 0
            s_enc(0b010, 5, 6, 0),                 // 重设 [addr] = 0xFFFFFFFF
            i_enc(OP_IMM, 6, 0b000, 0, 5),         // t1 = 5
            amo_enc(0b11000, 0b010, 5, 6, 8),      // amominu.w s0, t1, (t0) → 5
        ]);
        h.run(8);
        assert_eq!(h.cpu.reg(7), 0xFFFF_FFFF); // amomax.w 有符号旧值
        assert_eq!(h.cpu.reg(8), 0xFFFF_FFFF); // amominu.w 无符号旧值
        assert_eq!(h.bus.peek(addr, AccessSize::Word).unwrap(), 5);
    }

    #[test]
    fn exec_fence_i_is_nop() {
        let mut h = Harness::new();
        h.load(BASE, &[0x0000_100F, i_enc(OP_IMM, 10, 0b000, 0, 42)]);
        h.run(2);
        assert_eq!(h.cpu.reg(10), 42);
        assert_eq!(h.cpu.pc(), BASE + 8);
    }

    #[test]
    fn unprivileged_counters_read_only() {
        let mut h = Harness::new();
        h.load(BASE, &[0x0000_0013, 0x0000_0013, 0x0000_0013]); // 3 nop
        h.run(3);
        assert_eq!(h.cpu.debug_csr_read(csr::CYCLE).unwrap(), 3);
        assert_eq!(h.cpu.debug_csr_read(csr::TIME).unwrap(), 3);
        assert_eq!(h.cpu.debug_csr_read(csr::INSTRET).unwrap(), 3);
        assert_eq!(h.cpu.debug_csr_read(csr::TIMEH).unwrap(), 0);
        // 只读：写入被忽略
        h.cpu.csr_write(csr::TIME, 0xDEAD);
        assert_eq!(h.cpu.debug_csr_read(csr::TIME).unwrap(), 3);
    }

    #[test]
    fn misa_advertises_extensions() {
        let h = Harness::new();
        let misa = h.cpu.debug_csr_read(csr::MISA).unwrap();
        for bit in [
            csr::MISA_I,
            csr::MISA_M,
            csr::MISA_A,
            csr::MISA_C,
            csr::MISA_F,
            csr::MISA_D,
            csr::MISA_U,
        ] {
            assert_eq!(misa & bit, bit, "missing misa bit {bit:#x}");
        }
        assert_eq!((misa >> 30) & 0b11, 0b01); // MXL=RV32
    }
    // ---- Sv32 测试辅助 ----
