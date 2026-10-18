use super::*;

    #[test]
    fn x0_is_hardwired_zero() {
        let mut h = Harness::new();
        // addi x0, x0, 123 → x0 仍为 0
        h.load(BASE, &[i_enc(OP_IMM, 0, 0b000, 0, 123)]);
        h.step();
        assert_eq!(h.cpu.reg(0), 0);
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn writeback_and_pc_advance() {
        let mut h = Harness::new();
        h.load(BASE, &[u_enc(0b0110111, 10, 0x12345), i_enc(OP_IMM, 11, 0b000, 10, -4)]);
        h.run(2);
        assert_eq!(h.cpu.reg(10), 0x1234_5000);
        assert_eq!(h.cpu.reg(11), 0x1234_4ffc);
        assert_eq!(h.cpu.cycle(), 2);
        assert_eq!(h.cpu.instret(), 2);
    }

    #[test]
    fn branch_taken_and_not_taken() {
        let mut h = Harness::new();
        // beq x0, x0, +8（taken）→ 跳过一条 addi
        h.load(BASE, &[
            b_enc(0b000, 0, 0, 8),
            i_enc(OP_IMM, 10, 0b000, 0, 99), // 被跳过
            i_enc(OP_IMM, 11, 0b000, 0, 1),  // 落点
        ]);
        h.run(1);
        assert_eq!(h.cpu.pc(), BASE + 8);
        // bne x0, x0（not taken）→ 顺序执行两步后到达落点
        h.load(BASE + 8, &[b_enc(0b001, 0, 0, 8), i_enc(OP_IMM, 12, 0b000, 0, 7)]);
        h.run(2);
        assert_eq!(h.cpu.pc(), BASE + 16);
        assert_eq!(h.cpu.reg(12), 7);
    }

    #[test]
    fn jal_jalr() {
        let mut h = Harness::new();
        // jal ra, +12
        h.load(BASE, &[j_enc(1, 12), u_enc(0b0110111, 10, 0), i_enc(OP_IMM, 0, 0b000, 0, 0)]);
        h.run(1);
        assert_eq!(h.cpu.pc(), BASE + 12);
        assert_eq!(h.cpu.reg(1), BASE + 4);
        // jalr t0 跳到寄存器地址，清 bit0
        // 先写一个目标地址到寄存器：lui t0, 0x80000; addi t0, t0, 0x123|1
        h.load(BASE + 12, &[
            u_enc(0b0110111, 5, 0x80000),
            i_enc(OP_IMM, 5, 0b000, 5, 0x123 | 1),
            i_enc(0b1100111, 1, 0b000, 5, 0), // jalr ra, 0(t0)
        ]);
        h.run(3);
        assert_eq!(h.cpu.pc(), 0x8000_0122); // bit0 被清除
        assert_eq!(h.cpu.reg(1), BASE + 24); // ra = jalr 指令地址 + 4
    }

    #[test]
    fn load_store_roundtrip() {
        let mut h = Harness::new();
        // sw t0 = 0x80000100 数据 -2；lw/lb/lbu/lh/lhu 校验符号扩展
        h.load(BASE, &[
            u_enc(0b0110111, 5, 0x80000),        // lui t0, 0x80000
            i_enc(OP_IMM, 5, 0b000, 5, 0x100),   // addi t0, t0, 0x100
            i_enc(OP_IMM, 6, 0b000, 0, -2),      // addi t1, x0, -2
            s_enc(0b010, 5, 6, 0),               // sw t1, 0(t0)
            i_enc(LOAD, 7, 0b010, 5, 0),         // lw t2, 0(t0)
            i_enc(LOAD, 8, 0b000, 5, 0),         // lb t3, 0(t0)
            i_enc(LOAD, 9, 0b100, 5, 0),         // lbu t4, 0(t0)
            i_enc(LOAD, 20, 0b001, 5, 0),        // lh t5, 0(t0)
            i_enc(LOAD, 21, 0b101, 5, 0),        // lhu t6, 0(t0)
        ]);
        h.run(9);
        assert_eq!(h.cpu.reg(7), 0xffff_fffe);
        assert_eq!(h.cpu.reg(8), 0xffff_fffe);
        assert_eq!(h.cpu.reg(9), 0xfe);
        assert_eq!(h.cpu.reg(20), 0xffff_fffe);
        assert_eq!(h.cpu.reg(21), 0xfffe);
    }

    #[test]
    fn sltiu_unsigned_compare() {
        let mut h = Harness::new();
        // sltiu a0, x0, -1 → x0(0) < 0xffffffff → 1
        h.load(BASE, &[i_enc(OP_IMM, 10, 0b011, 0, -1)]);
        h.step();
        assert_eq!(h.cpu.reg(10), 1);
    }

    #[test]
    fn r_type_wrapping_and_shift_mask() {
        let mut h = Harness::new();
        // 0 - 1 = wrapping
        h.load(BASE, &[
            r_enc(10, 0b000, 0, 0, 0b0100000), // sub a0, x0, x0 → 0
            i_enc(OP_IMM, 11, 0b000, 0, 1),    // addi a1, x0, 1
            r_enc(10, 0b000, 0, 11, 0b0100000), // sub a0, x0, a1 → 0xffffffff
            i_enc(OP_IMM, 12, 0b001, 10, 0),   // slli — shamt 0
            r_enc(12, 0b001, 10, 11, 0b0000000), // sll a2, a0, a1（移位量 1）
            r_enc(13, 0b101, 10, 11, 0b0100000), // sra a3, a0, a1
        ]);
        h.run(6);
        assert_eq!(h.cpu.reg(10), 0xffff_ffff);
        assert_eq!(h.cpu.reg(12), 0xffff_fffe);
        assert_eq!(h.cpu.reg(13), 0xffff_ffff);
    }

    #[test]
    fn trap_on_ecall_and_mret_path_exists() {
        let mut h = Harness::new();
        // 未设置 mtvec（0）：ecall → trap 到 0
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]); // ecall
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_M });
        assert_eq!(h.cpu.pc(), 0);
        assert_eq!(h.cpu.csr_state().mepc, BASE);
        assert_eq!(h.cpu.csr_state().mcause, cause::ECALL_FROM_M);
        assert_eq!(h.cpu.priv_(), Priv::Machine);
        // trap 事件已发
        assert!(h.sink.0.iter().any(|e| matches!(e, EmuEvent::Trap { cause: cause::ECALL_FROM_M, .. })));
        // cycle 前进，instret 不计陷入指令
        assert_eq!(h.cpu.cycle(), 1);
        assert_eq!(h.cpu.instret(), 0);
    }

    #[test]
    fn unimplemented_csr_read_traps_illegal() {
        let mut h = Harness::new();
        // csrr t0, 0x14d (stimecmp): 未实现，不得被 OpenSBI 探测为 SSTC。
        let raw = i_enc(0b1110011, 5, 0b010, 0, 0x14d);
        h.load(BASE, &[raw]);
        let out = h.step();
        assert!(matches!(out, StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR }));
        assert_eq!(h.cpu.csr_state().mtval, raw);
    }

    #[test]
    fn unimplemented_csr_write_traps_illegal() {
        let mut h = Harness::new();
        // csrw 0x14d, x0
        let raw = i_enc(0b1110011, 0, 0b001, 0, 0x14d);
        h.load(BASE, &[raw]);
        let out = h.step();
        assert!(matches!(out, StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR }));
        assert_eq!(h.cpu.csr_state().mtval, raw);
    }

    #[test]
    fn implemented_csr_read_still_works() {
        let mut h = Harness::new();
        // csrr t0, mhartid
        h.load(BASE, &[i_enc(0b1110011, 5, 0b010, 0, i32::from(csr::MHARTID))]);
        let out = h.step();
        assert!(matches!(out, StepOutcome::Executed { .. }));
    }

    #[test]
    fn trap_sets_mpp_to_user_for_u_mode_ecall() {
        // U-mode ecall → trap，mstatus.MPP 应写入 00（User）
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::User;
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]); // ecall
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_U });
        let mstatus = h.cpu.debug_csr_read(csr::MSTATUS).unwrap();
        assert_eq!(mstatus & csr::MSTATUS_MPP_MASK, csr::MSTATUS_MPP_U);
        // 陷入后特权级升至 M
        assert_eq!(h.cpu.priv_(), Priv::Machine);
    }

    #[test]
    fn trap_sets_mpp_to_machine_for_m_mode_ecall() {
        // M-mode ecall → trap，mstatus.MPP 应写入 11（Machine）
        let mut h = Harness::new();
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]); // ecall
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_M });
        let mstatus = h.cpu.debug_csr_read(csr::MSTATUS).unwrap();
        assert_eq!(mstatus & csr::MSTATUS_MPP_MASK, csr::MSTATUS_MPP_M);
    }

    #[test]
    fn trap_sets_mpp_to_supervisor_for_s_mode_ecall() {
        // S-mode ecall → cause 9，mstatus.MPP 应写入 01（Supervisor）
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::Supervisor;
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]); // ecall
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_S });
        let mstatus = h.cpu.debug_csr_read(csr::MSTATUS).unwrap();
        assert_eq!(mstatus & csr::MSTATUS_MPP_MASK, csr::MSTATUS_MPP_S);
        assert_eq!(h.cpu.priv_(), Priv::Machine);
    }

    #[test]
    fn trap_on_illegal_instruction() {
        let mut h = Harness::new();
        h.load(BASE, &[0xffff_ffff]); // 未定义指令
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });
    }

    #[test]
    fn trap_on_unmapped_load() {
        let mut h = Harness::new();
        // lw t0, 0(x0) → 地址 0 未映射
        h.load(BASE, &[i_enc(LOAD, 5, 0b010, 0, 0)]);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::LOAD_ACCESS_FAULT });
    }

    #[test]
    fn trap_updates_mstatus_mpp_mpie_mie() {
        // 预置 mstatus.MIE=1，trap 后 MPP=M、MPIE=旧MIE、MIE=0
        let mut h = Harness::new();
        let pre = (csr::MSTATUS_MPP_M | csr::MSTATUS_MIE) as i32;
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, pre),                    // x5 = mstatus
            i_enc(0b1110011, 0, 0b001, 5, csr::MSTATUS as i32), // csrrw x0, mstatus, x5
            i_enc(0b1110011, 0, 0b000, 0, 0),                   // ecall
        ]);
        h.run(2);
        let out = h.step();
        assert!(matches!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_M }));
        let ms = h.cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b11);
        assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 0);
    }

    #[test]
    fn mret_restores_mstatus_mie_mpie_mpp() {
        // 预置 mstatus: MPP=M, MPIE=1, MIE=0；mret 后 MIE=MPIE, MPIE=1, MPP=0
        let mut h = Harness::new();
        let pre = (csr::MSTATUS_MPP_M | csr::MSTATUS_MPIE) as i32;
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, pre),                    // x5 = mstatus
            i_enc(0b1110011, 0, 0b001, 5, csr::MSTATUS as i32), // csrrw x0, mstatus, x5
            i_enc(0b1110011, 0, 0, 0, 0x302),                   // mret
        ]);
        h.run(2);
        h.step();
        let ms = h.cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b00);
    }

    #[test]
    fn mret_enters_supervisor_when_mpp_s() {
        let mut h = Harness::new();
        // x5 = (1<<11) | MPIE(0x80) = 0x880（用移位+ori 避免立即数符号扩展）
        h.load(BASE, &[
            i_enc(OP_IMM, 5, 0b000, 0, 1),                    // addi x5, x0, 1
            i_enc(OP_IMM, 5, 0b001, 5, 11),                   // slli x5, x5, 11  -> 0x800
            i_enc(OP_IMM, 5, 0b110, 5, 0x80),                 // ori  x5, x5, 0x80 -> 0x880
            i_enc(0b1110011, 0, 0b001, 5, csr::MSTATUS as i32), // csrrw x0, mstatus, x5
            i_enc(0b1110011, 0, 0, 0, 0x302),                 // mret
        ]);
        h.run(4);
        h.step();
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
    }

    #[test]
    fn sstatus_sie_are_views_of_mstatus_mie() {
        let mut h = Harness::new();
        h.cpu.csr_write(csr::SSTATUS, csr::SSTATUS_MASK);
        assert_eq!(h.cpu.debug_csr_read(csr::SSTATUS).unwrap(), csr::SSTATUS_MASK);
        assert_eq!(h.cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::SSTATUS_MASK, csr::SSTATUS_MASK);
        h.cpu.csr_write(csr::SIE, csr::SIE_S_MASK);
        assert_eq!(h.cpu.debug_csr_read(csr::MIE).unwrap() & csr::SIE_S_MASK, csr::SIE_S_MASK);
        h.cpu.csr_write(csr::SATP, 0x8000_0000);
        assert_eq!(h.cpu.debug_csr_read(csr::SATP).unwrap(), 0x8000_0000);
    }

    #[test]
    fn sret_returns_to_supervisor_and_restores_sie() {
        let mut h = Harness::new();
        h.cpu.csr_write(csr::SSTATUS, csr::MSTATUS_SPP | csr::MSTATUS_SPIE);
        h.cpu.csr_write(csr::SEPC, BASE + 0x10);
        h.load(BASE, &[0x1020_0073]); // sret
        h.step();
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
        assert_eq!(h.cpu.pc(), BASE + 0x10);
        assert_eq!(h.cpu.debug_csr_read(csr::SSTATUS).unwrap() & csr::MSTATUS_SIE, csr::MSTATUS_SIE);
        assert_eq!(h.cpu.debug_csr_read(csr::SSTATUS).unwrap() & csr::MSTATUS_SPP, 0);
    }

    #[test]
    fn exception_delegated_to_supervisor() {
        let mut h = Harness::new();
        h.cpu.csr_write(csr::MEDELEG, 1 << cause::ECALL_FROM_U);
        h.cpu.csr_write(csr::STVEC, BASE + 0x100);
        h.cpu.priv_ = Priv::User;
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]); // ecall
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_U });
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
        assert_eq!(h.cpu.pc(), BASE + 0x100);
        assert_eq!(h.cpu.debug_csr_read(csr::SCAUSE).unwrap(), cause::ECALL_FROM_U);
        assert_eq!(h.cpu.debug_csr_read(csr::SEPC).unwrap(), BASE);
        assert_eq!(h.cpu.debug_csr_read(csr::SSTATUS).unwrap() & csr::MSTATUS_SPP, 0);
    }

    #[test]
    fn undelegated_ecall_from_u_goes_to_machine() {
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::User;
        h.load(BASE, &[i_enc(0b1110011, 0, 0b000, 0, 0)]);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ECALL_FROM_U });
        assert_eq!(h.cpu.priv_(), Priv::Machine);
    }

    #[test]
    fn pmp_csr_roundtrip() {
        let mut h = Harness::new();
        h.cpu.csr_write(csr::PMPADDR0, 0x8000_0000);
        h.cpu.csr_write(csr::PMPCFG0, 0x0F);
        h.cpu.csr_write(csr::PMPADDR0 + 15, 0xBEEF);
        assert_eq!(h.cpu.debug_csr_read(csr::PMPADDR0).unwrap(), 0x8000_0000);
        assert_eq!(h.cpu.debug_csr_read(csr::PMPCFG0).unwrap(), 0x0F);
        assert_eq!(h.cpu.debug_csr_read(csr::PMPADDR0 + 15).unwrap(), 0xBEEF);
    }

    #[test]
    fn reset_restores_state() {
        let mut h = Harness::new();
        h.load(BASE, &[i_enc(OP_IMM, 10, 0b000, 0, 5)]);
        h.run(1);
        h.cpu.reset(BASE);
        assert_eq!(h.cpu.reg(10), 0);
        assert_eq!(h.cpu.pc(), BASE);
        assert_eq!(h.cpu.cycle(), 0);
        assert_eq!(h.cpu.priv_(), Priv::Machine);
        assert_eq!(h.cpu.csr_state().mstatus, csr::MSTATUS_MPP_M);
    }

    // ---- M extension tests ----

    #[test]
    fn mret_clears_mprv_when_returning_to_s() {
        let mut h = Harness::new();
        h.cpu.csr.mstatus = csr::MSTATUS_MPRV | csr::MSTATUS_MPP_S;
        h.cpu.csr.mepc = BASE + 4;
        h.load(BASE, &[0x3020_0073]); // mret
        h.step();
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
        assert_eq!(h.cpu.csr_state().mstatus & csr::MSTATUS_MPRV, 0);
    }

    #[test]
    fn sret_clears_mprv() {
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::Supervisor;
        h.cpu.csr.mstatus = csr::MSTATUS_MPRV | csr::MSTATUS_SPP; // return S
        h.cpu.csr.sepc = BASE + 4;
        h.load(BASE, &[0x1020_0073]); // sret
        h.step();
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
        assert_eq!(h.cpu.csr_state().mstatus & csr::MSTATUS_MPRV, 0);
    }

    #[test]
    fn sip_only_allows_ssip_write() {
        let mut h = Harness::new();
        h.cpu.csr.mip = csr::MIP_STIP | csr::MIP_SEIP;
        h.cpu.csr_write(csr::SIP, csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP);
        assert_ne!(h.cpu.csr.mip & csr::MIP_SSIP, 0);
        assert_ne!(h.cpu.csr.mip & csr::MIP_STIP, 0);
        assert_ne!(h.cpu.csr.mip & csr::MIP_SEIP, 0);
        h.cpu.csr_write(csr::SIP, 0);
        assert_eq!(h.cpu.csr.mip & csr::MIP_STIP, csr::MIP_STIP);
        assert_eq!(h.cpu.csr.mip & csr::MIP_SEIP, csr::MIP_SEIP);
        assert_eq!(h.cpu.csr.mip & csr::MIP_SSIP, 0);
    }

    #[test]
    fn counter_access_respects_counteren() {
        let mut h = Harness::new();
        h.cpu.priv_ = Priv::Supervisor;
        assert!(h.cpu.check_csr_access(csr::CYCLE, false, 0).is_err());
        h.cpu.csr.mcounteren = csr::COUNTEREN_CY;
        assert!(h.cpu.check_csr_access(csr::CYCLE, false, 0).is_ok());
        h.cpu.priv_ = Priv::User;
        assert!(h.cpu.check_csr_access(csr::CYCLE, false, 0).is_err());
        h.cpu.csr.scounteren = csr::COUNTEREN_CY;
        assert!(h.cpu.check_csr_access(csr::CYCLE, false, 0).is_ok());
    }

    #[test]
    fn machine_counters_accept_low_word_writes() {
        let mut h = Harness::new();
        h.cpu.csr_write(csr::MCYCLE, 0x1234);
        h.cpu.csr_write(csr::MINSTRET, 0x5678);
        assert_eq!(h.cpu.cycle() & 0xffff_ffff, 0x1234);
        assert_eq!(h.cpu.instret() & 0xffff_ffff, 0x5678);
    }
