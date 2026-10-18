use super::*;

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
    fn exec_c_ebreak_traps_normally() {
        // c.ebreak 标准陷阱语义：mcause=BREAKPOINT，pc=mtvec（未设时 0）。
        let mut h = Harness::new();
        c_load(&mut h, BASE, &[
            Instruction::CAddi { rd: 10, imm: 1 }, // a0 = 1
            Instruction::CEbreak,
        ]);
        h.step(); // c.addi a0, 1
        let outcome = h.step(); // c.ebreak → Trapped { cause: BREAKPOINT }
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::BREAKPOINT });
    }

    #[test]
    fn exec_ebreak_traps_normally_32bit() {
        // 32 位 ebreak 同样走标准陷阱路径（确认未被宿主协议拦截）。
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 10, 0b000, 0, 0x2a), // addi a0, zero, 42
            0x00100073,                         // ebreak
        ]);
        h.step();
        let outcome = h.step();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::BREAKPOINT });
    }

    #[test]
    fn exec_ecall_magic_exits_with_a0() {
        // ecall + a7=EXIT_MAGIC → 宿主退出，退出码取 a0(x10)。
        let mut h = Harness::new();
        h.load(BASE, &[
            u_enc(0b0110111, 17, 0xCAFE0),                 // lui a7, 0xCAFE0  → a7 = EXIT_MAGIC
            i_enc(OP_IMM, 10, 0b000, 0, 0x2a),             // addi a0, zero, 42
            i_enc(0b1110011, 0, 0b000, 0, 0),               // ecall
        ]);
        h.step(); // lui
        h.step(); // addi
        let outcome = h.step(); // ecall → Exited { code: 42 }
        assert_eq!(outcome, StepOutcome::Exited { code: 42 });
    }

    #[test]
    fn exec_ecall_without_magic_traps_normally() {
        // ecall 但 a7≠EXIT_MAGIC → 走标准 ecall-from-M 陷阱路径。
        let mut h = Harness::new();
        h.load(BASE, &[
            i_enc(OP_IMM, 17, 0b000, 0, 1),                // a7 = 1（非魔术数）
            i_enc(0b1110011, 0, 0b000, 0, 0),               // ecall
        ]);
        h.step(); // addi
        let outcome = h.step(); // ecall → Trapped { cause: ECALL_FROM_M }
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::ECALL_FROM_M });
    }

    #[test]
    fn exec_cross_boundary_32bit_instr() {
        // 32 位指令放在 2 字节对齐但非 4 字节对齐的地址，测试跨边界取指（IFU 拆分）
        let mut h = Harness::new();
        let nop = encode_c(&Instruction::CAddi { rd: 0, imm: 0 }); // c.nop
        let addi = i_enc(OP_IMM, 1, 0b000, 0, 42); // = 0x02a00093
        let mut bytes = nop.to_le_bytes().to_vec();
        bytes.extend_from_slice(&addi.to_le_bytes());
        h.bus.write_bytes(BASE, &bytes).unwrap();
        h.run(2);
        assert_eq!(h.cpu.reg(1), 42);
        assert_eq!(h.cpu.pc(), BASE + 6);

        // 取指事件：第 1 条 c.nop 对齐取一条 IF W；第 2 条 addi 跨 XLEN，
        // IFU 发两条 IF H（@BASE+2 低半字 0x0093，@BASE+4 高半字 0x02a0）。addi 无访存。
        let mem: Vec<_> = h
            .sink
            .0
            .iter()
            .filter_map(|e| match e {
                EmuEvent::MemAccess { kind, size, addr, value, .. } => Some((*kind, *size, *addr, *value)),
                _ => None,
            })
            .collect();
        assert_eq!(
            mem,
            vec![
                (rremu_bus::MemKind::Inst, AccessSize::Word, BASE, (addi & 0xffff) << 16 | u32::from(nop)),
                (rremu_bus::MemKind::Inst, AccessSize::Half, BASE + 2, addi & 0xffff),
                (rremu_bus::MemKind::Inst, AccessSize::Half, BASE + 4, addi >> 16),
            ]
        );
    }

    // ---- interrupt tests ----
