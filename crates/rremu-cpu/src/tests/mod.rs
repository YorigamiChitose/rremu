#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use rremu_bus::{AccessSize, BusDevice, BusRouter, EmuEvent, EventSink};
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

mod basic;
mod exec;
mod compressed;
mod interrupt;
mod amo;
mod mmu;
mod s_interrupt;
mod pmp;
