#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use rremu_bus::{AccessSize, BusDevice, BusRouter, EmuEvent, EventSink, NullSink};
use rremu_decode::encode_c;
use rremu_isa::{cause, csr, Instruction, Priv, Word};

    /// 测试 RAM。
    struct MockRam {
        base: u32,
        mem: Vec<u8>,
    }

    impl MockRam {
        fn new(base: u32, size: usize) -> Self {
            Self { base, mem: vec![0; size] }
        }
    }

    impl BusDevice for MockRam {
        fn read(&mut self, addr: u32, size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
            let o = (addr - self.base) as usize;
            let mut v = 0u32;
            for i in 0..size.bytes() {
                v |= u32::from(self.mem[o + i as usize]) << (8 * i);
            }
            Ok(v)
        }
        fn write(&mut self, addr: u32, size: AccessSize, data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
            let o = (addr - self.base) as usize;
            for i in 0..size.bytes() {
                self.mem[o + i as usize] = (data >> (8 * i)) as u8;
            }
            Ok(())
        }
        fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
            let mut sink = rremu_bus::NullSink;
            BusDevice::read(self, addr, size, &mut sink)
        }
        fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
            let o = (addr - self.base) as usize;
            self.mem[o..o + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn device_name(&self) -> &str {
            "ram"
        }
    }

    use rremu_bus::BusError;

    /// 简易事件收集器。
    struct Sink(Vec<EmuEvent>);
    impl EventSink for Sink {
        fn emit(&mut self, ev: EmuEvent) {
            self.0.push(ev);
        }
    }

    const BASE: u32 = 0x8000_0000;

    struct Harness {
        cpu: Cpu,
        bus: BusRouter,
        sink: Sink,
    }

    impl Harness {
        fn new() -> Self {
            Self::new_with_ram(0x1000)
        }

        fn new_with_ram(size: u32) -> Self {
            let mut bus = BusRouter::new();
            bus.map(BASE..BASE + size, Box::new(MockRam::new(BASE, size as usize))).unwrap();
            let mut cpu = Cpu::new(BASE);
            // 测试默认给 S/U 全地址 R/W/X 权限；PMP 测试会显式清空。
            cpu.csr.pmpaddr[0] = 0x1fff_ffff;
            cpu.csr.pmpcfg[0] = 0x1f;
            Self { cpu, bus, sink: Sink(Vec::new()) }
        }
        /// 把指令/数据写入内存后复位 CPU。
        fn load(&mut self, addr: u32, words: &[Word]) {
            let mut bytes = Vec::new();
            for w in words {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
            self.bus.write_bytes(addr, &bytes).unwrap();
        }
        fn step(&mut self) -> StepOutcome {
            self.cpu.step(&mut self.bus, &mut self.sink).unwrap()
        }
        fn run(&mut self, n: usize) -> Vec<StepOutcome> {
            (0..n).map(|_| self.step()).collect()
        }
    }

    // 编码辅助（与 decode 测试一致）
    fn i_enc(op: u32, rd: u8, f3: u32, rs1: u8, imm: i32) -> Word {
        ((imm as u32 & 0xfff) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | op
    }
    fn r_enc(rd: u8, f3: u32, rs1: u8, rs2: u8, f7: u32) -> Word {
        (f7 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | 0b0110011
    }
    fn b_enc(f3: u32, rs1: u8, rs2: u8, off: i32) -> Word {
        let o = off as u32;
        let b12 = (o >> 12) & 1;
        let b11 = (o >> 11) & 1;
        let b10_5 = (o >> 5) & 0x3f;
        let b4_1 = (o >> 1) & 0xf;
        (b12 << 31) | (b10_5 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (b4_1 << 8) | (b11 << 7) | 0b1100011
    }
    fn u_enc(op: u32, rd: u8, imm20: u32) -> Word {
        (imm20 << 12) | (u32::from(rd) << 7) | op
    }
    fn j_enc(rd: u8, off: i32) -> Word {
        let o = off as u32;
        let b20 = (o >> 20) & 1;
        let b19_12 = (o >> 12) & 0xff;
        let b11 = (o >> 11) & 1;
        let b10_1 = (o >> 1) & 0x3ff;
        (b20 << 31) | (b10_1 << 21) | (b11 << 20) | (b19_12 << 12) | (u32::from(rd) << 7) | 0b1101111
    }
    fn s_enc(f3: u32, rs1: u8, rs2: u8, imm: i32) -> Word {
        let i = imm as u32;
        (((i >> 5) & 0x7f) << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | ((i & 0x1f) << 7) | 0b0100011
    }

    const OP_IMM: u32 = 0b0010011;
    const OP_LUI: u32 = 0b0110111;
    const LOAD: u32 = 0b0000011;

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
        let mstatus = h.cpu.csr_read(csr::MSTATUS);
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
        let mstatus = h.cpu.csr_read(csr::MSTATUS);
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
        let mstatus = h.cpu.csr_read(csr::MSTATUS);
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
        assert_eq!(h.cpu.csr_read(csr::SSTATUS), csr::SSTATUS_MASK);
        assert_eq!(h.cpu.csr_read(csr::MSTATUS) & csr::SSTATUS_MASK, csr::SSTATUS_MASK);
        h.cpu.csr_write(csr::SIE, csr::SIE_S_MASK);
        assert_eq!(h.cpu.csr_read(csr::MIE) & csr::SIE_S_MASK, csr::SIE_S_MASK);
        h.cpu.csr_write(csr::SATP, 0x8000_0000);
        assert_eq!(h.cpu.csr_read(csr::SATP), 0x8000_0000);
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
        assert_eq!(h.cpu.csr_read(csr::SSTATUS) & csr::MSTATUS_SIE, csr::MSTATUS_SIE);
        assert_eq!(h.cpu.csr_read(csr::SSTATUS) & csr::MSTATUS_SPP, 0);
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
        assert_eq!(h.cpu.csr_read(csr::SCAUSE), cause::ECALL_FROM_U);
        assert_eq!(h.cpu.csr_read(csr::SEPC), BASE);
        assert_eq!(h.cpu.csr_read(csr::SSTATUS) & csr::MSTATUS_SPP, 0);
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
        assert_eq!(h.cpu.csr_read(csr::PMPADDR0), 0x8000_0000);
        assert_eq!(h.cpu.csr_read(csr::PMPCFG0), 0x0F);
        assert_eq!(h.cpu.csr_read(csr::PMPADDR0 + 15), 0xBEEF);
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
        let misa = h.cpu.csr_read(csr::MISA);
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

    /// 测试 IRQ 设备：返回可配置的中断 cause 列表。
    struct MockIrq {
        causes: Vec<u32>,
    }

    impl MockIrq {
        fn new(cause: u32) -> Self {
            Self { causes: vec![cause] }
        }
        fn multi(causes: Vec<u32>) -> Self {
            Self { causes }
        }
    }

    impl BusDevice for MockIrq {
        fn read(&mut self, _addr: u32, _size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
            Ok(0)
        }
        fn write(&mut self, _addr: u32, _size: AccessSize, _data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
            Ok(())
        }
        fn peek_read(&mut self, _addr: u32, _size: AccessSize) -> Result<u32, BusError> {
            Ok(0)
        }
        fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
            Ok(())
        }
        fn device_name(&self) -> &str {
            "mock-irq"
        }
        fn check_interrupt(&self) -> Vec<u32> {
            self.causes.clone()
        }
    }

    #[test]
    fn interrupt_taken_when_mie_and_mtie_enabled() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;

        let mut sink = Sink(Vec::new());
        let outcome = cpu.step(&mut bus, &mut sink).unwrap();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_TI });
        assert_eq!(cpu.csr_state().mepc, BASE); // mepc = 旧 pc（未执行的指令）
        assert_eq!(cpu.pc(), BASE + 0x800); // pc = mtvec
        // trap 后 MIE=0（防嵌套），MPIE=旧MIE=1，MPP=M
        let ms = cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 0);
        assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
        assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b11);
    }

    #[test]
    fn interrupt_not_taken_when_mstatus_mie_disabled() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MPP_M; // MIE=0
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;
        // 放一条 NOP
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // addi x0, x0, 0 (NOP)
        let outcome = h.step();
        // 中断未取，NOP 正常执行
        assert!(matches!(outcome, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn interrupt_not_taken_when_mie_mtie_disabled() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M; // MIE=1
        cpu.csr.mie = 0; // MTIE=0
        cpu.csr.mtvec = BASE + 0x800;
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // NOP
        let outcome = h.step();
        assert!(matches!(outcome, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn interrupt_mret_restores_mie_and_resumes() {
        // 中断 → trap → handler 第一条 mret → 恢复 MIE → pc = mepc
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let handler = BASE + 0x800;
        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = handler;

        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        // 主程序放 NOP；handler 放 mret
        h.load(BASE, &[0x0000_0013]); // NOP at BASE
        h.load(handler, &[0x3020_0073]); // mret at handler

        // 第 1 步：中断触发，trap 到 handler
        let out1 = h.step();
        assert_eq!(out1, StepOutcome::Trapped { cause: cause::M_TI });
        assert_eq!(h.cpu.pc(), handler);
        assert_eq!(h.cpu.csr_state().mepc, BASE);

        // 第 2 步：执行 mret → pc = mepc = BASE，MIE 恢复为 1
        let out2 = h.step();
        assert!(matches!(out2, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE);
        let ms = h.cpu.csr_state().mstatus;
        assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 1);

        // 第 3 步：中断再次 pending → 再次 trap（MockIrq 恒 pending）
        let out3 = h.step();
        assert_eq!(out3, StepOutcome::Trapped { cause: cause::M_TI });
    }

    #[test]
    fn msi_serviced_when_mti_pending_but_unenabled() {
        // MSI 使能 + MTI 未使能 + 两者同时 pending → 必须进入 MSI handler。
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        // 单个设备同时 pending MSI + MTI
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::multi(vec![cause::M_SI, cause::M_TI]))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MSIE; // 仅 MSIE，MTIE=0
        cpu.csr.mtvec = BASE + 0x800;

        let mut sink = Sink(Vec::new());
        let outcome = cpu.step(&mut bus, &mut sink).unwrap();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_SI });
        assert_eq!(cpu.csr_state().mcause, cause::M_SI);
    }

    #[test]
    fn mei_highest_priority_over_msi_and_mti() {
        // spec 序 MEI > MSI > MTI：三者同时 pending 且全部使能 → 取 MEI。
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010,
            Box::new(MockIrq::multi(vec![cause::M_EI, cause::M_SI, cause::M_TI]))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MEIE | csr::MIE_MSIE | csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;

        let mut sink = Sink(Vec::new());
        let outcome = cpu.step(&mut bus, &mut sink).unwrap();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_EI });
    }

    #[test]
    fn msi_priority_over_mti_when_both_enabled() {
        // spec 序 MSI > MTI：MSI 与 MTI 同时 pending 且均使能 → 取 MSI。
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010,
            Box::new(MockIrq::multi(vec![cause::M_SI, cause::M_TI]))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MSIE | csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;

        let mut sink = Sink(Vec::new());
        let outcome = cpu.step(&mut bus, &mut sink).unwrap();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_SI });
    }

    // ---- audit 修复回归测试 ----

    #[test]
    fn trap_sets_mtval_for_load_fault() {
        // lw t0, 0x234(x0) → 地址 0x234 未映射 → LOAD_ACCESS_FAULT，mtval=0x234
        let mut h = Harness::new();
        h.load(BASE, &[i_enc(LOAD, 5, 0b010, 0, 0x234)]);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::LOAD_ACCESS_FAULT });
        assert_eq!(h.cpu.csr_read(csr::MTVAL), 0x234);
    }

    #[test]
    fn trap_sets_mtval_for_illegal_instr() {
        let mut h = Harness::new();
        h.load(BASE, &[0xffff_ffff]);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::ILLEGAL_INSTR });
        assert_eq!(h.cpu.csr_read(csr::MTVAL), 0xffff_ffff);
    }

    #[test]
    fn exit_ecall_counts_cycle_and_instret() {
        // lui a7, 0xCAFE0 ; addi a0, x0, 42 ; ecall → 三条指令都计入 cycle/instret
        let mut h = Harness::new();
        h.load(BASE, &[
            u_enc(0b0110111, 17, 0xCAFE0),
            i_enc(OP_IMM, 10, 0b000, 0, 0x2a),
            i_enc(0b1110011, 0, 0b000, 0, 0),
        ]);
        h.step();
        h.step();
        let out = h.step();
        assert_eq!(out, StepOutcome::Exited { code: 42 });
        assert_eq!(h.cpu.cycle(), 3);
        assert_eq!(h.cpu.instret(), 3);
    }

    #[test]
    fn fetch_misaligned_has_priority_over_interrupt() {
        // 奇数 pc + MTI pending + 使能 → 取指 misaligned 优先于中断
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();

        let mut cpu = Cpu::new(BASE + 1); // 奇数 pc
        cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
        cpu.csr.mie = csr::MIE_MTIE;
        cpu.csr.mtvec = BASE + 0x800;

        let mut sink = Sink(Vec::new());
        let outcome = cpu.step(&mut bus, &mut sink).unwrap();
        assert_eq!(outcome, StepOutcome::Trapped { cause: cause::INSTR_MISALIGNED });
    }

    #[test]
    fn mip_reflects_device_pending() {
        // MSI pending（但 MIE=0 不触发）→ step 后 mip.MSIP 应被投影为 1
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_SI))).unwrap();

        let mut cpu = Cpu::new(BASE);
        cpu.csr.mstatus = csr::MSTATUS_MPP_M; // MIE=0
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // NOP
        h.step();
        assert_eq!(h.cpu.csr_read(csr::MIP) & csr::MIP_MSIP, csr::MIP_MSIP);
    }

    // ---- A extension tests ----

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
        assert_eq!(h.cpu.csr_read(csr::CYCLE), 3);
        assert_eq!(h.cpu.csr_read(csr::TIME), 3);
        assert_eq!(h.cpu.csr_read(csr::INSTRET), 3);
        assert_eq!(h.cpu.csr_read(csr::TIMEH), 0);
        // 只读：写入被忽略
        h.cpu.csr_write(csr::TIME, 0xDEAD);
        assert_eq!(h.cpu.csr_read(csr::TIME), 3);
    }

    #[test]
    fn misa_advertises_extensions() {
        let h = Harness::new();
        let misa = h.cpu.csr_read(csr::MISA);
        for bit in [csr::MISA_I, csr::MISA_M, csr::MISA_A, csr::MISA_C, csr::MISA_U] {
            assert_eq!(misa & bit, bit, "missing misa bit {bit:#x}");
        }
        assert_eq!((misa >> 30) & 0b11, 0b01); // MXL=RV32
    }
    // ---- Sv32 测试辅助 ----
    const MMU_VA: u32 = 0x4000_0000;
    const ROOT_PA: u32 = BASE + 0x1000;
    const L1_PA: u32 = BASE + 0x2000;
    const DATA_PA: u32 = BASE + 0x3000;

    fn write_u32(h: &mut Harness, addr: u32, val: u32) {
        h.bus.write_bytes(addr, &val.to_le_bytes()).unwrap();
    }

    fn read_u32(h: &mut Harness, addr: u32) -> u32 {
        h.bus.peek(addr, AccessSize::Word).unwrap()
    }

    fn pte_ppn(pa: u32) -> u32 {
        (pa >> 12) & csr::SATP_PPN_MASK
    }

    fn enable_sv32(h: &mut Harness, root: u32) {
        h.cpu.priv_ = Priv::Supervisor;
        h.cpu.csr_write(csr::SATP, csr::SATP_MODE_SV32 | pte_ppn(root));
    }

    /// 建立 4KiB 映射，返回第二级 PTE 地址。
    fn map_4k(h: &mut Harness, va: u32, pa: u32, flags: u32) -> u32 {
        let root_pte = ROOT_PA + ((va >> 22) & 0x3ff) * 4;
        write_u32(h, root_pte, PTE_V | (pte_ppn(L1_PA) << 10));
        let leaf = L1_PA + ((va >> 12) & 0x3ff) * 4;
        write_u32(h, leaf, PTE_V | flags | (pte_ppn(pa) << 10));
        leaf
    }

    #[test]
    fn sv32_fetch_translates_and_sets_accessed() {
        let mut h = Harness::new_with_ram(0x10000);
        let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
        enable_sv32(&mut h, ROOT_PA);
        h.load(DATA_PA, &[0x0000_0013]); // nop
        h.cpu.pc = MMU_VA;
        let out = h.step();
        assert!(matches!(out, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), MMU_VA + 4);
        assert_ne!(read_u32(&mut h, leaf) & PTE_A, 0);
    }

    #[test]
    fn sv32_invalid_pte_raises_instruction_page_fault() {
        let mut h = Harness::new_with_ram(0x10000);
        let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
        write_u32(&mut h, leaf, 0);
        enable_sv32(&mut h, ROOT_PA);
        h.cpu.pc = MMU_VA;
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::INSTR_PAGE_FAULT });
        assert_eq!(h.cpu.csr_state().mtval, MMU_VA);
    }

    #[test]
    fn sv32_load_sets_accessed_and_store_requires_write() {
        let mut h = Harness::new_with_ram(0x10000);
        let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R);
        enable_sv32(&mut h, ROOT_PA);
        h.load(DATA_PA, &[0x1234_5678]);
        let v = h.cpu.load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word).unwrap();
        assert_eq!(v, 0x1234_5678);
        assert_ne!(read_u32(&mut h, leaf) & PTE_A, 0);
        assert_eq!(read_u32(&mut h, leaf) & PTE_D, 0);

        let err = h.cpu.store(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word, 1).err().unwrap();
        assert_eq!(err.cause, cause::STORE_PAGE_FAULT);
        assert_eq!(err.mtval, MMU_VA);
    }

    #[test]
    fn sv32_sum_controls_supervisor_access_to_user_page() {
        let mut h = Harness::new_with_ram(0x10000);
        map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_U);
        enable_sv32(&mut h, ROOT_PA);
        h.load(DATA_PA, &[0x55]);
        let err = h.cpu.load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word).err().unwrap();
        assert_eq!(err.cause, cause::LOAD_PAGE_FAULT);

        h.cpu.csr_write(csr::SSTATUS, csr::MSTATUS_SUM);
        let v = h.cpu.load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word).unwrap();
        assert_eq!(v, 0x55);
    }

    #[test]
    fn sv32_superpage_maps_4mib() {
        let mut h = Harness::new_with_ram(0x10000);
        let root_pte = ROOT_PA + ((MMU_VA >> 22) & 0x3ff) * 4;
        write_u32(&mut h, root_pte, PTE_V | PTE_R | PTE_W | PTE_X | (pte_ppn(BASE) << 10));
        enable_sv32(&mut h, ROOT_PA);
        h.load(BASE, &[0x0000_0013]);
        h.cpu.pc = MMU_VA;
        let out = h.step();
        assert!(matches!(out, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), MMU_VA + 4);
    }

    #[test]
    fn sv32_32bit_instruction_crossing_page_uses_two_translations() {
        let mut h = Harness::new_with_ram(0x10000);
        let va = MMU_VA + 0xffe;
        map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
        map_4k(&mut h, MMU_VA + 0x1000, BASE + 0x4000, PTE_R | PTE_W | PTE_X);
        // addi x1, x0, 1 = 0x00100093：低半字在页尾，高半字在下一页。
        h.bus.write_bytes(DATA_PA + 0xffe, &0x0093u16.to_le_bytes()).unwrap();
        h.bus.write_bytes(BASE + 0x4000, &0x0010u16.to_le_bytes()).unwrap();
        enable_sv32(&mut h, ROOT_PA);
        h.cpu.pc = va;
        let out = h.step();
        assert!(matches!(out, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.reg(1), 1);
        assert_eq!(h.cpu.pc(), MMU_VA + 0x1002);
    }

    #[test]
    fn sv32_compressed_instruction_at_page_end_does_not_fetch_next_page() {
        let mut h = Harness::new_with_ram(0x10000);
        let va = MMU_VA + 0xffe;
        map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
        // c.nop = 0x0001，下一页未映射也不应被读取。
        h.bus.write_bytes(DATA_PA + 0xffe, &0x0001u16.to_le_bytes()).unwrap();
        enable_sv32(&mut h, ROOT_PA);
        h.cpu.pc = va;
        let out = h.step();
        assert!(matches!(out, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), MMU_VA + 0x1000);
    }

    #[test]
    fn supervisor_software_interrupt_delegated_to_s() {
        let mut h = Harness::new();
        h.load(BASE, &[0x0000_0013]); // nop
        h.cpu.priv_ = Priv::Supervisor;
        h.cpu.csr_write(csr::MSTATUS, csr::MSTATUS_SIE);
        h.cpu.csr_write(csr::MIE, csr::SIE_SSIE);
        h.cpu.csr_write(csr::MIDELEG, 1 << 1);
        h.cpu.csr_write(csr::MIP, csr::MIP_SSIP);
        h.cpu.csr_write(csr::STVEC, BASE + 0x100);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::S_SI });
        assert_eq!(h.cpu.pc(), BASE + 0x100);
        assert_eq!(h.cpu.csr_read(csr::SCAUSE), cause::S_SI);
        assert_eq!(h.cpu.csr_read(csr::STVAL), 0);
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
        assert_eq!(h.cpu.csr_read(csr::SSTATUS) & csr::MSTATUS_SPP, csr::MSTATUS_SPP);
    }

    #[test]
    fn delegated_s_interrupt_is_not_taken_in_m() {
        let mut h = Harness::new();
        h.load(BASE, &[0x0000_0013]); // nop
        h.cpu.csr_write(csr::MSTATUS, csr::MSTATUS_MIE);
        h.cpu.csr_write(csr::MIE, csr::SIE_SSIE);
        h.cpu.csr_write(csr::MIDELEG, 1 << 1);
        h.cpu.csr_write(csr::MIP, csr::MIP_SSIP);
        let out = h.step();
        assert!(matches!(out, StepOutcome::Executed { .. }));
        assert_eq!(h.cpu.pc(), BASE + 4);
    }

    #[test]
    fn delegated_s_interrupt_taken_in_u_without_sie() {
        let mut h = Harness::new();
        h.load(BASE, &[0x0000_0013]); // nop
        h.cpu.priv_ = Priv::User;
        h.cpu.csr_write(csr::MIE, csr::SIE_SSIE);
        h.cpu.csr_write(csr::MIDELEG, 1 << 1);
        h.cpu.csr_write(csr::MIP, csr::MIP_SSIP);
        h.cpu.csr_write(csr::STVEC, BASE + 0x100);
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::S_SI });
        assert_eq!(h.cpu.priv_(), Priv::Supervisor);
        assert_eq!(h.cpu.csr_read(csr::SSTATUS) & csr::MSTATUS_SPP, 0);
    }

    #[test]
    fn m_interrupt_taken_in_s_without_mie() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::M_TI))).unwrap();
        let mut cpu = Cpu::new(BASE);
        cpu.csr.pmpaddr[0] = 0x1fff_ffff;
        cpu.csr.pmpcfg[0] = 0x1f;
        cpu.priv_ = Priv::Supervisor;
        cpu.csr_write(csr::MIE, csr::MIE_MTIE);
        cpu.csr_write(csr::MTVEC, BASE + 0x100);
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // nop
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::M_TI });
        assert_eq!(h.cpu.priv_(), Priv::Machine);
        assert_eq!(h.cpu.csr_read(csr::MSTATUS) & csr::MSTATUS_MPP_MASK, csr::MSTATUS_MPP_S);
    }

    #[test]
    fn plic_external_irq_injected_as_s_ei() {
        let mut bus = BusRouter::new();
        bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
        bus.map(0x0200_0000..0x0200_0010, Box::new(MockIrq::new(cause::S_EI))).unwrap();
        let mut cpu = Cpu::new(BASE);
        cpu.csr.pmpaddr[0] = 0x1fff_ffff;
        cpu.csr.pmpcfg[0] = 0x1f;
        cpu.priv_ = Priv::Supervisor;
        cpu.csr_write(csr::MSTATUS, csr::MSTATUS_SIE);
        cpu.csr_write(csr::MIE, csr::SIE_SEIE);
        cpu.csr_write(csr::MIDELEG, 1 << 9);
        cpu.csr_write(csr::STVEC, BASE + 0x100);
        let mut h = Harness { cpu, bus, sink: Sink(Vec::new()) };
        h.load(BASE, &[0x0000_0013]); // nop
        let out = h.step();
        assert_eq!(out, StepOutcome::Trapped { cause: cause::S_EI });
        assert_eq!(h.cpu.csr_read(csr::SIP) & csr::SIE_SEIE, csr::SIE_SEIE);
    }

    // ---- PMP tests ----

    fn clear_pmp(h: &mut Harness) {
        h.cpu.csr.pmpcfg = [0; 4];
        h.cpu.csr.pmpaddr = [0; 16];
    }

    #[test]
    fn pmp_without_entries_denies_s_but_allows_m() {
        let mut h = Harness::new();
        clear_pmp(&mut h);
        h.load(BASE, &[0x0000_0013]); // nop
        h.cpu.priv_ = Priv::Supervisor;
        assert_eq!(h.step(), StepOutcome::Trapped { cause: cause::INSTR_ACCESS_FAULT });

        let mut h = Harness::new();
        clear_pmp(&mut h);
        h.load(BASE, &[0x0000_0013]);
        assert!(matches!(h.step(), StepOutcome::Executed { .. }));
    }

    #[test]
    fn pmp_napot_region_and_outside() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        // NAPOT 4KiB，覆盖 BASE..BASE+0x1000。
        h.cpu.csr.pmpaddr[0] = (BASE >> 2) | ((1 << 9) - 1);
        h.cpu.csr.pmpcfg[0] = 0x1f; // R/W/X + NAPOT
        h.cpu.priv_ = Priv::Supervisor;
        assert!(h.cpu.pmp_check(BASE, AccessType::Fetch));
        assert!(h.cpu.pmp_check(BASE + 0xfff, AccessType::Load));
        assert!(!h.cpu.pmp_check(BASE + 0x1000, AccessType::Load));
    }

    #[test]
    fn pmp_tor_uses_previous_address() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        let start = BASE + 0x800;
        let end = BASE + 0x1000;
        h.cpu.csr.pmpaddr[0] = start >> 2;
        h.cpu.csr.pmpaddr[1] = end >> 2;
        h.cpu.csr.pmpcfg[0] = 0x0f00; // entry1: TOR + R/W/X；entry0=OFF
        h.cpu.priv_ = Priv::Supervisor;
        assert!(!h.cpu.pmp_check(start - 1, AccessType::Load));
        assert!(h.cpu.pmp_check(start, AccessType::Load));
        assert!(h.cpu.pmp_check(end - 1, AccessType::Load));
        assert!(!h.cpu.pmp_check(end, AccessType::Load));
    }

    #[test]
    fn pmp_na4_matches_single_word() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        let addr = BASE + 0x100;
        h.cpu.csr.pmpaddr[0] = addr >> 2;
        h.cpu.csr.pmpcfg[0] = 0x17; // NA4 + R/W/X
        h.cpu.priv_ = Priv::Supervisor;
        assert!(h.cpu.pmp_check(addr, AccessType::Load));
        assert!(h.cpu.pmp_check(addr + 3, AccessType::Load));
        assert!(!h.cpu.pmp_check(addr + 4, AccessType::Load));
    }

    #[test]
    fn pmp_locked_entry_applies_to_m() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        h.cpu.csr.pmpaddr[0] = (BASE >> 2) | ((1 << 9) - 1);
        h.cpu.csr.pmpcfg[0] = 0x99; // NAPOT + R + L（无 W/X）
        h.cpu.priv_ = Priv::Machine;
        assert!(h.cpu.pmp_check(BASE, AccessType::Load));
        assert!(!h.cpu.pmp_check(BASE, AccessType::Store));
        assert!(!h.cpu.pmp_check(BASE, AccessType::Fetch));
        // 有锁定项且无匹配 → M 也拒绝
        assert!(!h.cpu.pmp_check(BASE + 0x1000, AccessType::Load));
    }
