//! rremu-cpu：CPU 状态与单周期执行内核。只依赖 bus trait，不感知具体设备。

use rremu_bus::{AccessSize, Bus, EmuEvent, EventSink};
use rremu_decode::{decode16, decode32, instr_len, DecodeError};
use rremu_isa::{cause, csr, Instruction, Priv};
use thiserror::Error;

mod mmu;
mod pmp;

pub(crate) use mmu::AccessType;
#[cfg(test)]
pub(crate) use mmu::{PTE_A, PTE_D, PTE_R, PTE_U, PTE_V, PTE_W, PTE_X};

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
    pub medeleg: u32,
    pub mideleg: u32,
    pub stvec: u32,
    pub sscratch: u32,
    pub sepc: u32,
    pub scause: u32,
    pub stval: u32,
    pub satp: u32,
    pub pmpcfg: [u32; 4],
    pub pmpaddr: [u32; 16],
}

impl CsrState {
    fn new() -> Self {
        Self {
            mstatus: csr::MSTATUS_MPP_M, // 复位进入 M-mode
            // RV32 + I/M/A/C/S/U
            misa: csr::MISA_MXL_32
                | csr::MISA_I
                | csr::MISA_M
                | csr::MISA_A
                | csr::MISA_C
                | csr::MISA_S
                | csr::MISA_U,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mip: 0,
            mhartid: 0,
            medeleg: 0,
            mideleg: 0,
            stvec: 0,
            sscratch: 0,
            sepc: 0,
            scause: 0,
            stval: 0,
            satp: 0,
            pmpcfg: [0; 4],
            pmpaddr: [0; 16],
        }
    }
}

/// 单步结果：正常执行一条、陷入，或 ecall+a7=EXIT_MAGIC 触发宿主退出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// `bits` 为该指令的原始编码（len==2 时低 16 位，len==4 时全 32 位），供 trace/dump。
    Executed { instr: Instruction, len: u8, bits: u32 },
    Trapped { cause: u32 },
    /// 执行 ecall 且 a7==EXIT_MAGIC：以 a0（x10）为退出码终止模拟（宿主陷阱协议）。
    Exited { code: i32 },
}

/// 宿主陷阱协议魔术数：M-mode ecall 且 `a7 == EXIT_MAGIC` 时模拟器以 `a0` 为退出码退出。
/// 选 0xCAFE_0000 是 rremu 专用值，避开 Linux syscall ABI（a7=0..~300）与 AM yield（a7=-1）。
pub const EXIT_MAGIC: u32 = 0xCAFE_0000;

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
    /// lr.w 保留地址（A 扩展）；None 表示无有效保留。
    reservation: Option<u32>,
}

impl Cpu {
    pub fn new(entry: u32) -> Self {
        Self { x: [0; 32], pc: entry, priv_: Priv::Machine, csr: CsrState::new(), cycle: 0, instret: 0, reservation: None }
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
    /// `sstatus`/`sie`/`sip` 是 `mstatus`/`mie`/`mip` 的 S 位掩码视图。
    pub fn csr_read(&self, addr: u16) -> u32 {
        match addr {
            csr::MSTATUS => self.csr.mstatus,
            csr::MSTATUSH | csr::SSTATUSH => 0,
            csr::MISA => self.csr.misa,
            csr::MIE => self.csr.mie,
            csr::MTVEC => self.csr.mtvec,
            csr::MSCRATCH => self.csr.mscratch,
            csr::MEPC => self.csr.mepc,
            csr::MCAUSE => self.csr.mcause,
            csr::MTVAL => self.csr.mtval,
            csr::MIP => self.csr.mip,
            csr::MEDELEG => self.csr.medeleg,
            csr::MIDELEG => self.csr.mideleg,
            csr::SSTATUS => self.csr.mstatus & csr::SSTATUS_MASK,
            csr::SIE => self.csr.mie & csr::SIE_S_MASK,
            csr::SIP => self.csr.mip & csr::SIE_S_MASK,
            csr::STVEC => self.csr.stvec,
            csr::SSCRATCH => self.csr.sscratch,
            csr::SEPC => self.csr.sepc,
            csr::SCAUSE => self.csr.scause,
            csr::STVAL => self.csr.stval,
            csr::SATP => self.csr.satp,
            csr::MCYCLE | csr::CYCLE | csr::TIME => self.cycle as u32,
            csr::MINSTRET | csr::INSTRET => self.instret as u32,
            csr::TIMEH => (self.cycle >> 32) as u32,
            csr::MHARTID => self.csr.mhartid,
            a if (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&a) => {
                self.csr.pmpcfg[(a - csr::PMPCFG0) as usize]
            }
            a if (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&a) => {
                self.csr.pmpaddr[(a - csr::PMPADDR0) as usize]
            }
            _ => 0,
        }
    }

    /// CSR 写入（CSR 指令执行用）。只读/未知 CSR 写入忽略。
    fn csr_write(&mut self, addr: u16, val: u32) {
        match addr {
            csr::MSTATUS => self.csr.mstatus = val & csr::MSTATUS_WRITE_MASK,
            // mstatush/sstatush 的高位当前无状态，忽略写入。
            csr::MSTATUSH | csr::SSTATUSH => {}
            // misa 为只读/WARL：忽略写入。
            csr::MIE => self.csr.mie = val,
            csr::MTVEC => self.csr.mtvec = val,
            csr::MSCRATCH => self.csr.mscratch = val,
            csr::MEPC => self.csr.mepc = val,
            csr::MCAUSE => self.csr.mcause = val,
            csr::MTVAL => self.csr.mtval = val,
            csr::MIP => self.csr.mip = val,
            csr::MEDELEG => self.csr.medeleg = val,
            csr::MIDELEG => self.csr.mideleg = val,
            csr::SSTATUS => {
                self.csr.mstatus =
                    (self.csr.mstatus & !csr::SSTATUS_MASK) | (val & csr::SSTATUS_MASK)
            }
            csr::SIE => self.csr.mie = (self.csr.mie & !csr::SIE_S_MASK) | (val & csr::SIE_S_MASK),
            csr::SIP => self.csr.mip = (self.csr.mip & !csr::SIE_S_MASK) | (val & csr::SIE_S_MASK),
            csr::STVEC => self.csr.stvec = val,
            csr::SSCRATCH => self.csr.sscratch = val,
            csr::SEPC => self.csr.sepc = val,
            csr::SCAUSE => self.csr.scause = val,
            csr::STVAL => self.csr.stval = val,
            csr::SATP => self.csr.satp = val,
            a if (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&a) => {
                self.csr.pmpcfg[(a - csr::PMPCFG0) as usize] = val;
            }
            a if (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&a) => {
                self.csr.pmpaddr[(a - csr::PMPADDR0) as usize] = val;
            }
            _ => {}
        }
    }

    /// 当前特权级对应的 CSR 地址 bit[9:8] 数值（U=0, S=1, M=3）。
    fn priv_csr_level(&self) -> u32 {
        match self.priv_ {
            Priv::User => 0,
            Priv::Supervisor => 1,
            Priv::Machine => 3,
        }
    }

    /// 当前实现存在的 CSR。未列出的 CSR 读/写必须触发 illegal instruction，
    /// 不能返回 0：OpenSBI 等固件会通过“读取是否成功”探测可选扩展。
    fn csr_implemented(addr: u16) -> bool {
        matches!(
            addr,
            csr::MSTATUS
                | csr::MSTATUSH
                | csr::MISA
                | csr::MIE
                | csr::MTVEC
                | csr::MSCRATCH
                | csr::MEPC
                | csr::MCAUSE
                | csr::MTVAL
                | csr::MIP
                | csr::MEDELEG
                | csr::MIDELEG
                | csr::SSTATUS
                | csr::SSTATUSH
                | csr::SIE
                | csr::SIP
                | csr::STVEC
                | csr::SSCRATCH
                | csr::SEPC
                | csr::SCAUSE
                | csr::STVAL
                | csr::SATP
                | csr::MCYCLE
                | csr::MINSTRET
                | csr::CYCLE
                | csr::TIME
                | csr::INSTRET
                | csr::TIMEH
                | csr::MVENDORID
                | csr::MARCHID
                | csr::MIMPID
                | csr::MHARTID
        ) || (csr::PMPCFG0..csr::PMPCFG0 + 4).contains(&addr)
            || (csr::PMPADDR0..csr::PMPADDR0 + 16).contains(&addr)
    }

    /// CSR 访问权限检查：CSR 不存在、特权级不足或写入只读 CSR 时触发
    /// illegal instruction。`write` 由具体指令决定（CSRRS/CSRRC 的 rs1=x0
    /// 不构成写）。
    fn check_csr_access(&self, addr: u16, write: bool, raw: u32) -> Result<(), TrapErr> {
        if !Self::csr_implemented(addr) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        let required = u32::from((addr >> 8) & 0x3);
        let read_only = (addr >> 10) & 0x3 == 0x3;
        if required > self.priv_csr_level() || (write && read_only) {
            return Err(TrapErr { cause: cause::ILLEGAL_INSTR, mtval: raw });
        }
        Ok(())
    }

    /// 总线错误 → 异常 cause：对齐违例返 `misalign`，其余（未映射/设备）返 `fault`。
    fn cause_of(e: &rremu_bus::BusError, misalign: u32, fault: u32) -> u32 {
        if matches!(e, rremu_bus::BusError::Misaligned { .. }) {
            misalign
        } else {
            fault
        }
    }

    /// 更新 mstatus 三域：先清 MPP/MPIE/MIE，再写 MPP、MIE、MPIE。
    fn update_mstatus(&mut self, mpp: u32, mie: u32, mpie: u32) {
        let mut mstatus =
            self.csr.mstatus & !(csr::MSTATUS_MPP_MASK | csr::MSTATUS_MPIE | csr::MSTATUS_MIE);
        mstatus |= (mpp << csr::MSTATUS_MPP_SHIFT)
            | (mie << csr::MSTATUS_MIE.trailing_zeros())
            | (mpie << csr::MSTATUS_MPIE.trailing_zeros());
        self.csr.mstatus = mstatus;
    }

    /// 复位到入口（寄存器/CSR/计数器清零，MPP=M）。
    pub fn reset(&mut self, entry: u32) {
        self.x = [0; 32];
        self.pc = entry;
        self.priv_ = Priv::Machine;
        self.csr = CsrState::new();
        self.cycle = 0;
        self.instret = 0;
        self.reservation = None;
    }

    /// 注入启动参数到 `a0`(x10)/`a1`(x11)（qemu-virt 约定：hartid / dtb）。
    pub fn set_args(&mut self, a0: u32, a1: u32) {
        self.x[10] = a0;
        self.x[11] = a1;
    }

    /// 取指 → 解码 → 执行 → 写回；错误路径统一走 trap。
    ///
    /// 陷阱优先级（取指异常优先于中断）：先取指（含对齐检查），取指成功后再检查
    /// 中断（三层 AND 门控 —— mstatus.MIE & mie 对应位 & 硬件 pending）。
    /// 设备侧 `check_interrupt()` 返回全部 pending cause（不裁剪优先级）；
    /// CPU 按 spec 序 MEI>MSI>MTI 遍历，跳过未使能者，取第一个已使能的中断。
    /// 中断命中时 mepc = 当前 pc（未执行的下一条指令），mret 后重新执行。
    pub fn step<B: Bus, E: EventSink>(&mut self, bus: &mut B, sink: &mut E) -> Result<StepOutcome, CpuError> {
        bus.set_cycle(self.cycle);
        let pc = self.pc;

        // 取指前先做对齐检查：取指 misaligned 优先于中断
        if pc & 0x1 != 0 {
            return self.trap(sink, cause::INSTR_MISALIGNED, pc);
        }

        // 取指单元(IFU)：取指 access fault 优先于中断
        let word = match self.fetch_word(bus, pc, sink) {
            Ok(w) => w,
            Err(c) => return self.trap(sink, c, pc),
        };

        // 设备 pending 同步到 mip（软件读 mip 可见），并在取指成功后检查中断
        let pending = bus.pending_interrupts();
        self.sync_mip(&pending);
        if let Some(cause) = self.pending_interrupt() {
            return self.trap(sink, cause, 0);
        }

        let lo = word as u16;

        let (instr, len) = if instr_len(lo) == 2 {
            // 16 位压缩指令（mtval 为 16 位编码本身）
            match decode16(lo, pc) {
                Ok(i) => (i, 2u8),
                Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                    return self.trap(sink, cause::ILLEGAL_INSTR, u32::from(lo))
                }
            }
        } else {
            // 32 位指令：word 已包含完整 4 字节，直接解码
            match decode32(word, pc) {
                Ok(i) => (i, 4u8),
                Err(DecodeError::Illegal { .. } | DecodeError::Unimplemented { .. }) => {
                    return self.trap(sink, cause::ILLEGAL_INSTR, word)
                }
            }
        };

        match self.exec(bus, sink, pc, instr, len, if len == 2 { word & 0xffff } else { word }) {
            Ok(next_pc) => {
                self.pc = next_pc;
                self.cycle += 1;
                self.instret += 1;
                let bits = if len == 2 { word & 0xffff } else { word };
                Ok(StepOutcome::Executed { instr, len, bits })
            }
            Err(TrapErr { cause, mtval }) => {
                // ecall+a7=EXIT_MAGIC：宿主陷阱协议——以 a0(x10) 为退出码终止模拟。
                // 退出指令也计入一个 cycle 与 instret。
                if cause == cause::ECALL_FROM_M && self.x[17] == EXIT_MAGIC {
                    let code = self.x[10] as i32;
                    self.cycle += 1;
                    self.instret += 1;
                    return Ok(StepOutcome::Exited { code });
                }
                self.trap(sink, cause, mtval)
            }
        }
    }

    /// 取指单元(IFU)：取回 `pc` 处的指令字，高位在需要时补入。
    ///
    /// 跨页策略：4 字节对齐地址的 Word 读不会跨 4KiB 页；`pc & 3 == 2` 时
    /// 先读低半字判断长度，只有 32 位指令才读取 `pc+2`。这样页尾的 16 位
    /// 指令不会误触发下一页的 page fault。
    ///
    /// 调用前 `pc` 已保证偶数；返回 Err 为取指异常 cause。
    fn fetch_word<B: Bus, E: EventSink>(&self, bus: &mut B, pc: u32, sink: &mut E) -> Result<u32, u32> {
        // 取指总线错误 → 异常 cause：对齐违例 INSTR_MISALIGNED，其余 INSTR_ACCESS_FAULT
        fn map(e: rremu_bus::BusError) -> u32 {
            Cpu::cause_of(&e, cause::INSTR_MISALIGNED, cause::INSTR_ACCESS_FAULT)
        }
        let pa = match self.translate(bus, sink, pc, AccessType::Fetch) {
            Ok(pa) => pa,
            Err(t) => return Err(t.cause),
        };
        if pc & 3 == 0 {
            if !self.pmp_check(pa, AccessType::Fetch) {
                return Err(cause::INSTR_ACCESS_FAULT);
            }
            return bus.inst_read(pa, AccessSize::Word, sink).map_err(map);
        }

        // pc & 3 == 2（仅 RVC 允许 2 字节对齐）：先读低半字。
        if !self.pmp_check(pa, AccessType::Fetch) {
            return Err(cause::INSTR_ACCESS_FAULT);
        }
        let lo = bus.inst_read(pa, AccessSize::Half, sink).map_err(map)?;
        if instr_len(lo as u16) == 2 {
            return Ok(lo);
        }
        // 32 位指令：仅 pc 位于页尾 0xffe 时需要翻译下一页。
        let hi_pa = if pc & 0xfff == 0xffe {
            match self.translate(bus, sink, pc.wrapping_add(2), AccessType::Fetch) {
                Ok(pa) => pa,
                Err(t) => return Err(t.cause),
            }
        } else {
            pa.wrapping_add(2)
        };
        if !self.pmp_check(hi_pa, AccessType::Fetch) {
            return Err(cause::INSTR_ACCESS_FAULT);
        }
        let hi = bus.inst_read(hi_pa, AccessSize::Half, sink).map_err(map)?;
        Ok(lo | (hi << 16))
    }

    /// 执行一条已解码指令；成功返回下一 pc，失败返回 trap 原因。
    fn exec<B: Bus, E: EventSink>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        pc: u32,
        instr: Instruction,
        len: u8,
        raw: u32,
    ) -> Result<u32, TrapErr> {
        let x = |c: &Self, i: u8| c.x[usize::from(i)];
        let mut next_pc = pc.wrapping_add(len as u32);
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

            // ---- M extension (RV32M) ----
            Instruction::Mul { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_mul(x(self, rs2))),
            Instruction::Mulh { rd, rs1, rs2 } => {
                let a = (x(self, rs1) as i32) as i64;
                let b = (x(self, rs2) as i32) as i64;
                set!(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Mulhsu { rd, rs1, rs2 } => {
                let a = (x(self, rs1) as i32) as i64;
                let b = x(self, rs2) as i64; // rs2 无符号，零扩展
                set!(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Mulhu { rd, rs1, rs2 } => {
                let a = x(self, rs1) as u64;
                let b = x(self, rs2) as u64;
                set!(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Div { rd, rs1, rs2 } => {
                let a = x(self, rs1) as i32;
                let b = x(self, rs2) as i32;
                let result = a.checked_div(b).map(|v| v as u32).unwrap_or_else(|| {
                    // 除零返回 -1；INT_MIN / -1 溢出返回 INT_MIN
                    if a == i32::MIN && b == -1 { i32::MIN as u32 } else { u32::MAX }
                });
                set!(rd, result);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let divisor = x(self, rs2);
                let result = x(self, rs1).checked_div(divisor).unwrap_or(u32::MAX);
                set!(rd, result);
            }
            Instruction::Rem { rd, rs1, rs2 } => {
                let a = x(self, rs1) as i32;
                let b = x(self, rs2) as i32;
                let result = a.checked_rem(b).map(|v| v as u32).unwrap_or_else(|| {
                    // 除零返回 rs1；INT_MIN % -1 溢出返回 0
                    if a == i32::MIN && b == -1 { 0 } else { x(self, rs1) }
                });
                set!(rd, result);
            }
            Instruction::Remu { rd, rs1, rs2 } => {
                let divisor = x(self, rs2);
                let result = x(self, rs1).checked_rem(divisor).unwrap_or_else(|| x(self, rs1));
                set!(rd, result);
            }

            // ---- A extension (RV32A) ----
            Instruction::LrW { rd, rs1 } => {
                let addr = x(self, rs1);
                let v = self.load(bus, sink, addr, AccessSize::Word)?;
                self.reservation = Some(addr);
                set!(rd, v);
            }
            Instruction::ScW { rd, rs1, rs2 } => {
                let addr = x(self, rs1);
                if self.reservation == Some(addr) {
                    self.store(bus, sink, addr, AccessSize::Word, x(self, rs2))?;
                    set!(rd, 0);
                } else {
                    set!(rd, 1);
                }
                self.reservation = None;
            }
            Instruction::AmoswapW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |_, b| b)?;
            }
            Instruction::AmoaddW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a.wrapping_add(b))?;
            }
            Instruction::AmoxorW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a ^ b)?;
            }
            Instruction::AmoandW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a & b)?;
            }
            Instruction::AmoorW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a | b)?;
            }
            Instruction::AmominW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| (a as i32).min(b as i32) as u32)?;
            }
            Instruction::AmomaxW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| (a as i32).max(b as i32) as u32)?;
            }
            Instruction::AmominuW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a.min(b))?;
            }
            Instruction::AmomaxuW { rd, rs1, rs2 } => {
                self.amo(bus, sink, rd, rs1, rs2, |a, b| a.max(b))?;
            }

            Instruction::Sret => {
                let sstatus = self.csr.mstatus;
                let spp = (sstatus >> csr::MSTATUS_SPP.trailing_zeros()) & 1;
                let spie = (sstatus >> csr::MSTATUS_SPIE.trailing_zeros()) & 1;
                // SPP=0, SIE=SPIE, SPIE=1
                self.csr.mstatus &= !(csr::MSTATUS_SPP | csr::MSTATUS_SIE | csr::MSTATUS_SPIE);
                self.csr.mstatus |= (spie << csr::MSTATUS_SIE.trailing_zeros())
                    | (1 << csr::MSTATUS_SPIE.trailing_zeros());
                self.priv_ = if spp == 1 { Priv::Supervisor } else { Priv::User };
                next_pc = self.csr.sepc;
            }
            Instruction::Wfi => {} // 无等待实现：NOP
            Instruction::SfenceVma => {} // 无 TLB：NOP

            Instruction::Fence => {}
            Instruction::FenceI => {}
            Instruction::Ecall => {
                let c = match self.priv_ {
                    Priv::Machine => cause::ECALL_FROM_M,
                    Priv::Supervisor => cause::ECALL_FROM_S,
                    Priv::User => cause::ECALL_FROM_U,
                };
                return Err(TrapErr { cause: c, mtval: 0 });
            }
            Instruction::Ebreak => return Err(TrapErr { cause: cause::BREAKPOINT, mtval: 0 }),
            Instruction::Csrrw { rd, csr, rs1 } => {
                self.check_csr_access(csr, true, raw)?;
                let old = self.csr_read(csr);
                self.csr_write(csr, x(self, rs1));
                set!(rd, old);
            }
            Instruction::Csrrs { rd, csr, rs1 } => {
                let mask = x(self, rs1);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrc { rd, csr, rs1 } => {
                let mask = x(self, rs1);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrwi { rd, csr, zimm } => {
                self.check_csr_access(csr, true, raw)?;
                let old = self.csr_read(csr);
                self.csr_write(csr, u32::from(zimm));
                set!(rd, old);
            }
            Instruction::Csrrsi { rd, csr, zimm } => {
                let mask = u32::from(zimm);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old | mask);
                }
                set!(rd, old);
            }
            Instruction::Csrrci { rd, csr, zimm } => {
                let mask = u32::from(zimm);
                self.check_csr_access(csr, mask != 0, raw)?;
                let old = self.csr_read(csr);
                if mask != 0 {
                    self.csr_write(csr, old & !mask);
                }
                set!(rd, old);
            }
            Instruction::Mret => {
                let mstatus = self.csr.mstatus;
                let mpp = (mstatus >> csr::MSTATUS_MPP_SHIFT) & 0b11;
                let mpie = (mstatus >> csr::MSTATUS_MPIE.trailing_zeros()) & 1;
                // MPP = 0 (U-mode), MIE = MPIE, MPIE = 1
                self.update_mstatus(0, mpie, 1);
                self.priv_ = match mpp {
                    0b11 => Priv::Machine,
                    0b01 => Priv::Supervisor,
                    _ => Priv::User,
                };
                next_pc = self.csr.mepc;
            }
            // ---- C extension (RV32C) ----
            Instruction::CAddi4spn { rd, imm } => set!(rd, x(self, 2u8).wrapping_add(imm)),
            Instruction::CLw { rd, rs1, imm } => {
                let v = self.load(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word)?;
                set!(rd, v)
            }
            Instruction::CSw { rs1, rs2, imm } => {
                self.store(bus, sink, x(self, rs1).wrapping_add(imm as u32), AccessSize::Word, x(self, rs2))?
            }
            Instruction::CAddi { rd, imm } => set!(rd, x(self, rd).wrapping_add(imm as u32)),
            Instruction::CJal { imm } => {
                set!(1u8, pc.wrapping_add(2u32));
                next_pc = pc.wrapping_add(imm as u32);
            }
            Instruction::CLi { rd, imm } => set!(rd, imm as u32),
            Instruction::CAddi16sp { imm } => set!(2u8, x(self, 2u8).wrapping_add(imm as u32)),
            Instruction::CLui { rd, imm } => set!(rd, imm),
            Instruction::CSrli { rd, shamt } => set!(rd, x(self, rd) >> (shamt & 0x1f)),
            Instruction::CSrai { rd, shamt } => set!(rd, ((x(self, rd) as i32) >> (shamt & 0x1f)) as u32),
            Instruction::CAndi { rd, imm } => set!(rd, x(self, rd) & imm as u32),
            Instruction::CSub { rd, rs2 } => set!(rd, x(self, rd).wrapping_sub(x(self, rs2))),
            Instruction::CXor { rd, rs2 } => set!(rd, x(self, rd) ^ x(self, rs2)),
            Instruction::COr { rd, rs2 } => set!(rd, x(self, rd) | x(self, rs2)),
            Instruction::CAnd { rd, rs2 } => set!(rd, x(self, rd) & x(self, rs2)),
            Instruction::CJ { imm } => next_pc = pc.wrapping_add(imm as u32),
            Instruction::CBeqz { rs1, imm } => {
                if x(self, rs1) == 0 {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CBnez { rs1, imm } => {
                if x(self, rs1) != 0 {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CSlli { rd, shamt } => set!(rd, x(self, rd) << (shamt & 0x1f)),
            Instruction::CLwsp { rd, imm } => {
                let v = self.load(bus, sink, x(self, 2u8).wrapping_add(imm as u32), AccessSize::Word)?;
                set!(rd, v)
            }
            Instruction::CJr { rs1 } => next_pc = x(self, rs1) & !1,
            Instruction::CMv { rd, rs2 } => set!(rd, x(self, rs2)),
            Instruction::CEbreak => return Err(TrapErr { cause: cause::BREAKPOINT, mtval: 0 }),
            Instruction::CJalr { rs1 } => {
                set!(1u8, pc.wrapping_add(2u32));
                next_pc = x(self, rs1) & !1;
            }
            Instruction::CAdd { rd, rs2 } => set!(rd, x(self, rd).wrapping_add(x(self, rs2))),
            Instruction::CSwsp { rs2, imm } => {
                self.store(bus, sink, x(self, 2u8).wrapping_add(imm as u32), AccessSize::Word, x(self, rs2))?
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
        let pa = self.translate(bus, sink, addr, AccessType::Load)?;
        if !self.pmp_check(pa, AccessType::Load) {
            return Err(TrapErr { cause: cause::LOAD_ACCESS_FAULT, mtval: addr });
        }
        bus.data_read(pa, size, sink).map_err(|e| TrapErr {
            cause: Self::cause_of(&e, cause::LOAD_MISALIGNED, cause::LOAD_ACCESS_FAULT),
            mtval: addr,
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
        self.reservation = None; // 任何 store 都使 lr.w 保留失效
        let pa = self.translate(bus, sink, addr, AccessType::Store)?;
        if !self.pmp_check(pa, AccessType::Store) {
            return Err(TrapErr { cause: cause::STORE_ACCESS_FAULT, mtval: addr });
        }
        bus.data_write(pa, size, data, sink).map_err(|e| TrapErr {
            cause: Self::cause_of(&e, cause::STORE_MISALIGNED, cause::STORE_ACCESS_FAULT),
            mtval: addr,
        })
    }

    /// AMO 原子读-改-写：读旧值 → 计算新值 → 写回 → rd = 旧值。
    /// 单核单线程下无需总线锁，直接 load + store 即可保持原子性。
    fn amo<B: Bus, E: EventSink, F: Fn(u32, u32) -> u32>(
        &mut self,
        bus: &mut B,
        sink: &mut E,
        rd: u8,
        rs1: u8,
        rs2: u8,
        f: F,
    ) -> Result<(), TrapErr> {
        let addr = self.x[usize::from(rs1)];
        let val = self.x[usize::from(rs2)];
        let old = self.load(bus, sink, addr, AccessSize::Word)?;
        let new = f(old, val);
        self.store(bus, sink, addr, AccessSize::Word, new)?;
        if rd != 0 {
            self.x[usize::from(rd)] = old;
        }
        Ok(())
    }

    /// 陷入流程：mepc ← pc；mcause ← cause；mtval ← mtval；priv ← M；pc ← mtvec（仅 DIRECT）。
    fn trap<E: EventSink>(&mut self, sink: &mut E, cause: u32, mtval: u32) -> Result<StepOutcome, CpuError> {
        sink.emit(EmuEvent::Trap { cause, pc: self.pc });

        // 陷阱委派：非 M 特权下，已委派（medeleg/mideleg）的异常/中断交 S-mode
        let is_interrupt = cause & 0x8000_0000 != 0;
        let code = cause & 0x7fff_ffff;
        let delegated = self.priv_ != Priv::Machine
            && if is_interrupt {
                (self.csr.mideleg >> code) & 1 == 1
            } else {
                (self.csr.medeleg >> code) & 1 == 1
            };
        if delegated {
            return self.trap_supervisor(cause, mtval);
        }

        self.csr.mepc = self.pc;
        self.csr.mcause = cause;
        self.csr.mtval = mtval;

        // 更新 mstatus: MPP = 当前特权级, MPIE = 旧 MIE, MIE = 0
        let mpp = match self.priv_ {
            Priv::Machine => 0b11,
            Priv::Supervisor => 0b01,
            Priv::User => 0b00,
        };
        let old_mie = (self.csr.mstatus >> csr::MSTATUS_MIE.trailing_zeros()) & 1;
        self.update_mstatus(mpp, 0, old_mie);

        self.priv_ = Priv::Machine;
        self.cycle += 1;
        if self.csr.mtvec & 1 == 1 {
            return Err(CpuError::Unimplemented("vectored mtvec"));
        }
        self.pc = self.csr.mtvec & !1;
        Ok(StepOutcome::Trapped { cause })
    }

    /// 陷入到 S-mode：sepc/scause/stval ← 当前；sstatus.SPP/SPIE/SIE；pc ← stvec（仅 DIRECT）。
    fn trap_supervisor(&mut self, cause: u32, mtval: u32) -> Result<StepOutcome, CpuError> {
        self.csr.sepc = self.pc;
        self.csr.scause = cause;
        self.csr.stval = mtval;
        let spp = u32::from(self.priv_ == Priv::Supervisor);
        let old_sie = (self.csr.mstatus >> csr::MSTATUS_SIE.trailing_zeros()) & 1;
        self.csr.mstatus &= !(csr::MSTATUS_SPP | csr::MSTATUS_SPIE | csr::MSTATUS_SIE);
        self.csr.mstatus |= (spp << csr::MSTATUS_SPP.trailing_zeros())
            | (old_sie << csr::MSTATUS_SPIE.trailing_zeros());
        self.priv_ = Priv::Supervisor;
        self.cycle += 1;
        if self.csr.stvec & 1 == 1 {
            return Err(CpuError::Unimplemented("vectored stvec"));
        }
        self.pc = self.csr.stvec & !1;
        Ok(StepOutcome::Trapped { cause })
    }

    /// 将设备 pending 中断投影到 mip 的设备位。
    /// SSIP/STIP 由软件写入（如 OpenSBI 注入 S timer），不被设备覆盖。
    fn sync_mip(&mut self, pending: &[u32]) {
        let mut dev = 0u32;
        for &c in pending {
            dev |= match c {
                cause::M_SI => csr::MIP_MSIP,
                cause::M_TI => csr::MIP_MTIP,
                cause::M_EI => csr::MIP_MEIP,
                cause::S_EI => csr::MIP_SEIP,
                _ => 0,
            };
        }
        let mask = csr::MIP_MSIP | csr::MIP_MTIP | csr::MIP_MEIP | csr::MIP_SEIP;
        self.csr.mip = (self.csr.mip & !mask) | dev;
    }

    /// 按规范优先级选择当前可接收的中断。
    ///
    /// 目标为 M 的中断：当前 `< M` 时全局使能；当前为 M 时需 MIE。
    /// 目标为 S 的中断：当前 `< S` 时全局使能；当前为 S 时需 SIE；
    /// 当前为 M 时不接收已委派给 S 的中断。
    fn pending_interrupt(&self) -> Option<u32> {
        const PRIORITY: [u32; 6] = [
            cause::M_EI,
            cause::M_SI,
            cause::M_TI,
            cause::S_EI,
            cause::S_SI,
            cause::S_TI,
        ];
        for &cause in &PRIORITY {
            let code = cause & 0x7fff_ffff;
            if (self.csr.mip >> code) & 1 == 0 || (self.csr.mie >> code) & 1 == 0 {
                continue;
            }
            let delegated = (self.csr.mideleg >> code) & 1 == 1;
            let enabled = if delegated {
                match self.priv_ {
                    Priv::Machine => false,
                    Priv::Supervisor => (self.csr.mstatus & csr::MSTATUS_SIE) != 0,
                    Priv::User => true,
                }
            } else {
                match self.priv_ {
                    Priv::Machine => (self.csr.mstatus & csr::MSTATUS_MIE) != 0,
                    _ => true,
                }
            };
            if enabled {
                return Some(cause);
            }
        }
        None
    }
}

/// 执行期内部 trap 信号（bus 错误 → cause 映射已完成）。
#[derive(Debug)]
struct TrapErr {
    cause: u32,
    mtval: u32,
}

#[cfg(test)]
mod tests;
