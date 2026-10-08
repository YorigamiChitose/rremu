//! rremu-cpu：CPU 状态与单周期执行内核。只依赖 bus trait，不感知具体设备。

use rremu_bus::{AccessSize, Bus, EmuEvent, EventSink};
use rremu_decode::{decode32, instr_len, DecodeError};
use rremu_isa::{cause, csr, Instruction, Priv};
use thiserror::Error;

/// M 状态 CSR（mcycle/minstret 由 cycle/instret 计数器承载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrState {
    pub mstatus: u32,
    pub misa: u32,
    pub mie: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mip: u32,
    pub mhartid: u32,
}

impl CsrState {
    fn new() -> Self {
        Self {
            mstatus: csr::MSTATUS_MPP_M, // 复位进入 M-mode
            misa: 0,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mip: 0,
            mhartid: 0,
        }
    }
}

/// 单步结果：正常执行一条，或陷入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Executed { instr: Instruction, len: u8 },
    Trapped { cause: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CpuError {
    #[error("unimplemented feature: {0}")]
    Unimplemented(&'static str),
}

/// 单周期 CPU：一条指令 = 一个 cycle。
#[derive(Debug, Clone)]
pub struct Cpu {
    x: [u32; 32],
    pc: u32,
    priv_: Priv,
    csr: CsrState,
    cycle: u64,
    instret: u64,
}

impl Cpu {
    pub fn new(entry: u32) -> Self {
        Self { x: [0; 32], pc: entry, priv_: Priv::Machine, csr: CsrState::new(), cycle: 0, instret: 0 }
    }

    // ---- 只读访问（machine 快照用） ----
    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn reg(&self, i: u8) -> u32 {
        self.x[usize::from(i)]
    }
    pub fn regs(&self) -> &[u32; 32] {
        &self.x
    }
    pub fn priv_(&self) -> Priv {
        self.priv_
    }
    pub fn cycle(&self) -> u64 {
        self.cycle
    }
    pub fn instret(&self) -> u64 {
        self.instret
    }
    pub fn csr_state(&self) -> &CsrState {
        &self.csr
    }

    /// CSR 视图读取（快照 + CSR 指令共用）。未知 CSR 返回 0。
    pub fn csr_read(&self, addr: u16) -> u32 {
        match addr {
            csr::MSTATUS => self.csr.mstatus,
            csr::MISA => self.csr.misa,
            csr::MIE => self.csr.mie,
            csr::MTVEC => self.csr.mtvec,
            csr::MSCRATCH => self.csr.mscratch,
            csr::MEPC => self.csr.mepc,
            csr::MCAUSE => self.csr.mcause,
            csr::MTVAL => self.csr.mtval,
            csr::MIP => self.csr.mip,
            csr::MCYCLE => self.cycle as u32,
            csr::MINSTRET => self.instret as u32,
            csr::MHARTID => self.csr.mhartid,
            _ => 0,
        }
    }

    /// CSR 写入（CSR 指令执行用）。只读 CSR（mhartid/mcycle/minstret）与
    /// 未知 CSR（satp/pmpaddr/medeleg 等）写入被忽略。
    fn csr_write(&mut self, addr: u16, val: u32) {
        match addr {
            csr::MSTATUS => self.csr.mstatus = val,
            csr::MISA => self.csr.misa = val,
            csr::MIE => self.csr.mie = val,
            csr::MTVEC => self.csr.mtvec = val,
            csr::MSCRATCH => self.csr.mscratch = val,
            csr::MEPC => self.csr.mepc = val,
            csr::MCAUSE => self.csr.mcause = val,
            csr::MTVAL => self.csr.mtval = val,
            csr::MIP => self.csr.mip = val,
            _ => {}
        }
    }

    /// 复位到入口（寄存器/CSR/计数器清零，MPP=M）。
    pub fn reset(&mut self, entry: u32) {
        self.x = [0; 32];
        self.pc = entry;
        self.priv_ = Priv::Machine;
        self.csr = CsrState::new();
        self.cycle = 0;
        self.instret = 0;
    }

    /// 取指 → 解码 → 执行 → 写回；错误路径统一走 trap。
    pub fn step<B: Bus, E: EventSink>(&mut self, bus: &mut B, sink: &mut E) -> Result<StepOutcome, CpuError> {
        bus.set_cycle(self.cycle);
        let pc = self.pc;

        // M1：无 C 扩展，pc 必须 4 字节对齐（C/2 字节对齐在 M2 开放）
        if pc & 0x3 != 0 {
            return self.trap(sink, cause::INSTR_MISALIGNED);
        }
        let word = match bus.fetch(pc, sink) {
            Ok(w) => w,
            Err(e) => {
                let c = if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
                    cause::INSTR_MISALIGNED
                } else {
                    cause::INSTR_ACCESS_FAULT
                };
                return self.trap(sink, c);
            }
        };
        // 压缩指令（低 2 位 != 11）在 M1 视为非法
        if instr_len((word & 0xffff) as u16) != 4 {
            return self.trap(sink, cause::ILLEGAL_INSTR);
        }
        let instr = match decode32(word, pc) {
            Ok(i) => i,
            Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                return self.trap(sink, cause::ILLEGAL_INSTR)
            }
        };

        match self.exec(bus, sink, pc, instr) {
            Ok(next_pc) => {
                self.pc = next_pc;
                self.cycle += 1;
                self.instret += 1;
                Ok(StepOutcome::Executed { instr, len: 4 })
            }
            Err(TrapErr { cause }) => self.trap(sink, cause),
        }
    }

    /// 执行一条已解码指令；成功返回下一 pc，失败返回 trap 原因。
    fn exec<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        pc: u32,
        instr: Instruction,
    ) -> Result<u32, TrapErr> {
        let x = |c: &Self, i: u8| c.x[usize::from(i)];
        let mut next_pc = pc.wrapping_add(4);
        macro_rules! set {
            ($rd:expr, $v:expr) => {{
                let v: u32 = $v;
                if $rd != 0 {
                    self.x[usize::from($rd)] = v;
                }
            }};
        }

        match instr {
            Instruction::Lui { rd, imm } => set!(rd, imm),
            Instruction::Auipc { rd, imm } => set!(rd, pc.wrapping_add(imm)),

            Instruction::Jal { rd, imm } => {
                set!(rd, pc.wrapping_add(4));
                next_pc = pc.wrapping_add(imm as u32);
            }
            Instruction::Jalr { rd, rs1, imm } => {
                let t = pc.wrapping_add(4);
                next_pc = x(self, rs1).wrapping_add(imm as u32) & !1;
                set!(rd, t);
            }

            Instruction::Beq { rs1, rs2, imm } => {
                if x(self, rs1) == x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bne { rs1, rs2, imm } => {
                if x(self, rs1) != x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Blt { rs1, rs2, imm } => {
                if (x(self, rs1) as i32) < (x(self, rs2) as i32) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bge { rs1, rs2, imm } => {
                if (x(self, rs1) as i32) >= (x(self, rs2) as i32) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bltu { rs1, rs2, imm } => {
                if x(self, rs1) < x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bgeu { rs1, rs2, imm } => {
                if x(self, rs1) >= x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }

            Instruction::Lb { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Byte)?;
                set!(rd, (v as u8) as i8 as i32 as u32)
            }
            Instruction::Lh { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Half)?;
                set!(rd, (v as u16) as i16 as i32 as u32)
            }
            Instruction::Lw { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word)?;
                set!(rd, v)
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Byte)?;
                set!(rd, v & 0xff)
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Half)?;
                set!(rd, v & 0xffff)
            }

            Instruction::Sb { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Byte, x(self, rs2))?
            }
            Instruction::Sh { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Half, x(self, rs2))?
            }
            Instruction::Sw { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word, x(self, rs2))?
            }

            Instruction::Addi { rd, rs1, imm } => set!(rd, x(self, rs1).wrapping_add(imm as u32)),
            Instruction::Slti { rd, rs1, imm } => set!(rd, u32::from((x(self, rs1) as i32) < imm)),
            Instruction::Sltiu { rd, rs1, imm } => set!(rd, u32::from(x(self, rs1) < imm as u32)),
            Instruction::Xori { rd, rs1, imm } => set!(rd, x(self, rs1) ^ imm as u32),
            Instruction::Ori { rd, rs1, imm } => set!(rd, x(self, rs1) | imm as u32),
            Instruction::Andi { rd, rs1, imm } => set!(rd, x(self, rs1) & imm as u32),
            Instruction::Slli { rd, rs1, shamt } => set!(rd, x(self, rs1) << shamt),
            Instruction::Srli { rd, rs1, shamt } => set!(rd, x(self, rs1) >> shamt),
            Instruction::Srai { rd, rs1, shamt } => set!(rd, ((x(self, rs1) as i32) >> shamt) as u32),

            Instruction::Add { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_add(x(self, rs2))),
            Instruction::Sub { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_sub(x(self, rs2))),
            Instruction::Sll { rd, rs1, rs2 } => set!(rd, x(self, rs1) << (x(self, rs2) & 0x1f)),
            Instruction::Slt { rd, rs1, rs2 } => set!(rd, u32::from((x(self, rs1) as i32) < (x(self, rs2) as i32))),
            Instruction::Sltu { rd, rs1, rs2 } => set!(rd, u32::from(x(self, rs1) < x(self, rs2))),
            Instruction::Xor { rd, rs1, rs2 } => set!(rd, x(self, rs1) ^ x(self, rs2)),
            Instruction::Srl { rd, rs1, rs2 } => set!(rd, x(self, rs1) >> (x(self, rs2) & 0x1f)),
            Instruction::Sra { rd, rs1, rs2 } => set!(rd, ((x(self, rs1) as i32) >> (x(self, rs2) & 0x1f)) as u32),
            Instruction::Or { rd, rs1, rs2 } => set!(rd, x(self, rs1) | x(self, rs2)),
            Instruction::And { rd, rs1, rs2 } => set!(rd, x(self, rs1) & x(self, rs2)),

            Instruction::Fence => {}
            Instruction::Ecall => {
                let c = if self.priv_ == Priv::Machine { cause::ECALL_FROM_M } else { cause::ECALL_FROM_U };
                return Err(TrapErr { cause: c });
            }
            Instruction::Ebreak => return Err(TrapErr { cause: cause::BREAKPOINT }),
            Instruction::Csrrw { rd, csr, rs1 } => {
                let old = self.csr_read(csr);
                self.csr_write(csr, x(self, rs1));
                set!(rd, old);
            }
            Instruction::Csrrs { rd, csr, rs1 } => {
                let old = self.csr_read(csr);
                let mask = x(self, rs1);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrc { rd, csr, rs1 } => {
                let old = self.csr_read(csr);
                let mask = x(self, rs1);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrwi { rd, csr, zimm } => {
                let old = self.csr_read(csr);
                self.csr_write(csr, u32::from(zimm));
                set!(rd, old);
            }
            Instruction::Csrrsi { rd, csr, zimm } => {
                let old = self.csr_read(csr);
                let mask = u32::from(zimm);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrci { rd, csr, zimm } => {
                let old = self.csr_read(csr);
                let mask = u32::from(zimm);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Mret => {
                let mpp = (self.csr.mstatus >> csr::MSTATUS_MPP_SHIFT) & 0b11;
                self.priv_ = if mpp == 0b11 { Priv::Machine } else { Priv::User };
                next_pc = self.csr.mepc;
            }
        }
        Ok(next_pc)
    }

    #[inline]
    fn load<B: Bus>(
        &mut self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        size: AccessSize,
    ) -> Result<u32, TrapErr> {
        bus.read(addr, size, sink).map_err(|e| TrapErr {
            cause: if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
                match size {
                    AccessSize::Half => cause::LOAD_MISALIGNED,
                    _ => cause::LOAD_MISALIGNED,
                }
            } else {
                cause::LOAD_ACCESS_FAULT
            },
        })
    }

    #[inline]
    fn store<B: Bus>(
        &mut self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        size: AccessSize,
        data: u32,
    ) -> Result<(), TrapErr> {
        bus.write(addr, size, data, sink).map_err(|e| TrapErr {
            cause: if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
                cause::STORE_MISALIGNED
            } else {
                cause::STORE_ACCESS_FAULT
            },
        })
    }

    /// 陷入流程（PLAN §5.4）：mepc ← pc；mcause ← cause；priv ← M；pc ← mtvec（仅 DIRECT）。
    fn trap<E: EventSink>(&mut self, sink: &mut E, cause: u32) -> Result<StepOutcome, CpuError> {
        sink.emit(EmuEvent::Trap { cause, pc: self.pc });
        self.csr.mepc = self.pc;
        self.csr.mcause = cause;
        self.priv_ = Priv::Machine;
        self.cycle += 1;
        if self.csr.mtvec & 1 == 1 {
            return Err(CpuError::Unimplemented("vectored mtvec"));
        }
        self.pc = self.csr.mtvec & !1;
        Ok(StepOutcome::Trapped { cause })
    }
}

/// 执行期内部 trap 信号（bus 错误 → cause 映射已完成）。
struct TrapErr {
    cause: u32,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;
    use core::any::Any;
    use rremu_bus::{BusDevice, BusRouter};
    use rremu_isa::Word;

    /// 测试 RAM（带 tohost 地址返回，供断言用）。
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
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
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
            let mut bus = BusRouter::new();
            bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000))).unwrap();
            Self { cpu: Cpu::new(BASE), bus, sink: Sink(Vec::new()) }
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
}
