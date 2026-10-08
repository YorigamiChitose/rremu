//! rremu-machine：Machine/Builder/快照/控制 —— 前端（cli/tui/window）唯一入口。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

use rremu_bus::{AccessSize, Bus, BusError, BusRouter, EmuEvent, EventSink};
use rremu_cpu::{Cpu, StepOutcome};
use rremu_devices::{Ram, Uart};
use rremu_expr::{EvalContext, EvalError, Expr, RegId};
use rremu_isa::{csr, Address, Word};
use rremu_loader::Image;
use thiserror::Error;

// 前端经 machine 间接使用（依赖红线：cli/tui 不得直接依赖 isa）。
// pub use 同时将名称引入本模块，供内部代码直接使用。
pub use rremu_isa::{reg_name, Instruction, Priv};

/// 运行状态（快照展示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Paused,
    Exited(i32),
    Trapped(u32),
}

/// 运行控制结果（断点/监视点变体在 M2 扩展）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Exited { code: i32 },
    Trapped { cause: u32 },
    Paused,
}

#[derive(Debug, Error)]
pub enum MachineError {
    #[error("segment {addr:#x}+{len} out of RAM range {ram_base:#x}..{ram_end:#x}")]
    SegmentOutOfRange { addr: u32, len: usize, ram_base: u32, ram_end: u32 },
    #[error("no entry point (neither override nor ELF entry)")]
    NoEntry,
    #[error(transparent)]
    Bus(#[from] BusError),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// 事件环形收集器：machine 持有，前端经快照读取。
/// mem 环默认 16 条 MemAccess，uart 环默认 1024 字节。
pub struct RingCollector {
    mem: VecDeque<EmuEvent>,
    uart: VecDeque<EmuEvent>,
    halt: Option<i32>,
    mem_cap: usize,
    uart_cap: usize,
}

impl RingCollector {
    pub fn new() -> Self {
        Self { mem: VecDeque::new(), uart: VecDeque::new(), halt: None, mem_cap: 16, uart_cap: 1024 }
    }

    pub fn set_capacity(&mut self, mem: usize, uart: usize) {
        self.mem_cap = mem;
        self.uart_cap = uart;
        self.mem.truncate(mem);
        self.uart.truncate(uart);
    }

    pub fn mem_ring(&self) -> &VecDeque<EmuEvent> {
        &self.mem
    }

    pub fn uart_ring(&self) -> &VecDeque<EmuEvent> {
        &self.uart
    }

    pub fn take_halt(&mut self) -> Option<i32> {
        self.halt.take()
    }

    /// 取走全部 UART 输出字节（CLI 打印用）。
    pub fn drain_uart(&mut self) -> Vec<u8> {
        self.uart
            .drain(..)
            .filter_map(|ev| match ev {
                EmuEvent::UartOutput { byte } => Some(byte),
                _ => None,
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.mem.clear();
        self.uart.clear();
        self.halt = None;
    }
}

impl Default for RingCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for RingCollector {
    fn emit(&mut self, ev: EmuEvent) {
        match ev {
            EmuEvent::MemAccess { .. } => {
                if self.mem.len() == self.mem_cap {
                    self.mem.pop_front();
                }
                self.mem.push_back(ev);
            }
            EmuEvent::UartOutput { .. } | EmuEvent::UartInput { .. } => {
                if self.uart.len() == self.uart_cap {
                    self.uart.pop_front();
                }
                self.uart.push_back(ev);
            }
            EmuEvent::Halt { code } => self.halt = Some(code),
            EmuEvent::Trap { .. } => {}
        }
    }
}

/// CSR 快照（TUI/CLI 显示所需的最简 M-mode 集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsrSnapshot {
    pub mstatus: u32,
    pub mtvec: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mie: u32,
    pub mip: u32,
    pub mcycle: u64,
    pub minstret: u64,
}

/// 机器快照：前端的唯一状态视图。
#[derive(Debug, Clone)]
pub struct MachineSnapshot {
    pub cycle: u64,
    pub pc: Address,
    pub priv_: Priv,
    pub gprs: [Word; 32],
    pub csrs: CsrSnapshot,
    pub state: RunState,
    pub mem_ring: VecDeque<EmuEvent>,
    pub uart_out: Vec<u8>,
    pub uart_in_len: usize,
}

#[derive(Debug, Default)]
pub struct MachineBuilder {
    ram_base: u32,
    ram_size: u32,
    uart_base: Option<u32>,
    entry: Option<u32>,
    image: Option<Image>,
}

impl MachineBuilder {
    pub fn new() -> Self {
        Self { ram_base: 0x8000_0000, ram_size: 16 * 1024 * 1024, ..Default::default() }
    }

    pub fn ram(mut self, base: u32, size: u32) -> Self {
        self.ram_base = base;
        self.ram_size = size;
        self
    }

    pub fn uart(mut self, base: u32) -> Self {
        self.uart_base = Some(base);
        self
    }

    pub fn entry(mut self, addr: u32) -> Self {
        self.entry = Some(addr);
        self
    }

    pub fn load(mut self, image: Image) -> Self {
        self.image = Some(image);
        self
    }

    pub fn build(self) -> Result<Machine, MachineError> {
        if self.ram_size == 0 {
            return Err(MachineError::InvalidConfig("ram size must be non-zero".into()));
        }
        let ram_end = self
            .ram_base
            .checked_add(self.ram_size)
            .ok_or_else(|| MachineError::InvalidConfig("ram range overflows address space".into()))?;

        let mut bus = BusRouter::new();
        bus.map(self.ram_base..ram_end, Box::new(Ram::new(self.ram_base, self.ram_size)))?;
        if let Some(ub) = self.uart_base {
            bus.map(ub..ub + 8, Box::new(Uart::new(ub)))?;
        }

        let image = self.image;
        if let Some(img) = &image {
            for seg in &img.segments {
                let end = seg.addr.checked_add(seg.data.len() as u32).ok_or(
                    MachineError::SegmentOutOfRange {
                        addr: seg.addr,
                        len: seg.data.len(),
                        ram_base: self.ram_base,
                        ram_end,
                    },
                )?;
                if seg.addr < self.ram_base || end > ram_end {
                    return Err(MachineError::SegmentOutOfRange {
                        addr: seg.addr,
                        len: seg.data.len(),
                        ram_base: self.ram_base,
                        ram_end,
                    });
                }
            }
            for seg in &img.segments {
                bus.write_bytes(seg.addr, &seg.data)?;
            }
            // tohost 监视：写入值 bit0==1 → 进程退出（riscv-tests 约定）
            bus.set_watch(img.tohost);
        }

        let entry =
            self.entry.or_else(|| image.as_ref().map(|i| i.entry)).ok_or(MachineError::NoEntry)?;

        Ok(Machine {
            cpu: Cpu::new(entry),
            bus,
            collector: RingCollector::new(),
            image,
            ram_base: self.ram_base,
            uart_base: self.uart_base,
            entry,
            state: RunState::Idle,
            last_instr: None,
            uart_in_len: 0,
        })
    }
}

/// 机器实例：CPU + 总线 + 设备 + 事件收集。
pub struct Machine {
    cpu: Cpu,
    bus: BusRouter,
    collector: RingCollector,
    image: Option<Image>,
    ram_base: u32,
    uart_base: Option<u32>,
    entry: u32,
    state: RunState,
    last_instr: Option<(u32, Instruction)>,
    /// UART 输入 FIFO 待读字节数（push 时 +1；每步后经 LSR peek 校正）
    uart_in_len: usize,
}

impl Machine {
    /// 单步执行 n 条；中途退出（Halt/Trap）即返回。
    pub fn step_n(&mut self, n: u64) -> RunResult {
        self.state = RunState::Running;
        for _ in 0..n {
            if let Some(rr) = self.step_once() {
                return rr;
            }
        }
        self.state = RunState::Paused;
        RunResult::Paused
    }

    /// 全速运行；pause 置位后停止（Ctrl+C 由前端控制）。
    /// 非致命陷入（ecall/ebreak 等）已被 CPU 重定向至 mtvec，run 不停止；
    /// 仅 Halt（tohost）或 pause 置位时返回。
    pub fn run(&mut self, pause: &AtomicBool) -> RunResult {
        self.state = RunState::Running;
        loop {
            if pause.load(Ordering::Relaxed) {
                self.state = RunState::Paused;
                return RunResult::Paused;
            }
            match self.step_once() {
                // 仅 Halt 终止 run；Trapped 已重定向 pc=mtvec，继续执行陷入向量
                Some(RunResult::Exited { code }) => {
                    self.state = RunState::Exited(code);
                    return RunResult::Exited { code };
                }
                Some(RunResult::Trapped { .. }) | Some(RunResult::Paused) | None => {}
            }
        }
    }

    /// 执行一条指令；需要停止时返回 Some(RunResult)。
    /// 注意：陷入（ecall/ebreak/misaligned）是非致命的 —— CPU 已将 pc 重定向
    /// 至 mtvec，陷入向量代码将在下一轮 step_once 中执行。state 保持 Running，
    /// 以便前端可继续 c/si；RunResult::Trapped 仅供 si 单步时报告。
    fn step_once(&mut self) -> Option<RunResult> {
        self.bus.set_cycle(self.cpu.cycle());
        let pc_before = self.cpu.pc();
        match self.cpu.step(&mut self.bus, &mut self.collector) {
            Ok(StepOutcome::Executed { instr, .. }) => {
                self.last_instr = Some((pc_before, instr));
                self.sync_uart_in_len();
                if let Some(code) = self.collector.take_halt() {
                    self.state = RunState::Exited(code);
                    return Some(RunResult::Exited { code });
                }
                None
            }
            Ok(StepOutcome::Trapped { cause }) => Some(RunResult::Trapped { cause }),
            Err(e) => {
                // 唯一错误路径：vectored mtvec 未实现；按陷入处理
                let cause = self.cpu.csr_state().mcause;
                tracing::error!("cpu error: {e}");
                Some(RunResult::Trapped { cause })
            }
        }
    }

    /// 经无副作用 LSR 读校正输入 FIFO 计数（0 或 1）。
    fn sync_uart_in_len(&mut self) {
        if let Some(ub) = self.uart_base {
            if let Ok(lsr) = self.bus.peek(ub + 5, AccessSize::Byte) {
                self.uart_in_len = usize::from(lsr & 0x01 != 0);
            }
        }
    }

    /// 复位：寄存器/计数器归零、RAM 清零、镜像重载。
    pub fn reset(&mut self) -> Result<(), MachineError> {
        self.cpu.reset(self.entry);
        if let Some(dev) = self.bus.device_mut(self.ram_base) {
            if let Some(ram) = dev.as_any_mut().downcast_mut::<Ram>() {
                ram.reset();
            }
        }
        if let Some(img) = &self.image {
            for seg in &img.segments {
                self.bus.write_bytes(seg.addr, &seg.data)?;
            }
        }
        self.collector.clear();
        self.state = RunState::Idle;
        self.last_instr = None;
        self.uart_in_len = 0;
        Ok(())
    }

    /// 机器快照（前端唯一状态视图）。
    pub fn snapshot(&self) -> MachineSnapshot {
        MachineSnapshot {
            cycle: self.cpu.cycle(),
            pc: self.cpu.pc(),
            priv_: self.cpu.priv_(),
            gprs: *self.cpu.regs(),
            csrs: CsrSnapshot {
                mstatus: self.cpu.csr_read(csr::MSTATUS),
                mtvec: self.cpu.csr_read(csr::MTVEC),
                mepc: self.cpu.csr_read(csr::MEPC),
                mcause: self.cpu.csr_read(csr::MCAUSE),
                mie: self.cpu.csr_read(csr::MIE),
                mip: self.cpu.csr_read(csr::MIP),
                mcycle: self.cpu.cycle(),
                minstret: self.cpu.instret(),
            },
            state: self.state,
            mem_ring: self.collector.mem_ring().clone(),
            uart_out: self
                .collector
                .uart_ring()
                .iter()
                .filter_map(|e| match e {
                    EmuEvent::UartOutput { byte } => Some(*byte),
                    _ => None,
                })
                .collect(),
            uart_in_len: self.uart_in_len,
        }
    }

    /// 调试 peek：无副作用读一个 Word（表达式/x 命令用，D12）。
    pub fn peek(&mut self, addr: u32) -> Result<u32, BusError> {
        self.bus.peek(addr, AccessSize::Word)
    }

    /// 求值调试表达式（p/x/w 命令共用；内存读走无副作用 peek，D12）。
    pub fn eval(&mut self, expr: &Expr) -> Result<u32, EvalError> {
        let mut ctx = MachineCtx(self);
        expr.eval(&mut ctx)
    }

    /// 注入 UART 输入字节。
    pub fn uart_push_input(&mut self, byte: u8) {
        let Some(ub) = self.uart_base else { return };
        if let Some(dev) = self.bus.device_mut(ub) {
            if let Some(uart) = dev.as_any_mut().downcast_mut::<Uart>() {
                uart.push_input(byte);
                self.uart_in_len += 1;
                self.collector.emit(EmuEvent::UartInput { byte });
            }
        }
    }

    /// 设置事件环容量（mem 条数 / uart 字节数）。
    pub fn set_event_ring_capacity(&mut self, mem: usize, uart: usize) {
        self.collector.set_capacity(mem, uart);
    }

    /// 取走全部 UART 输出字节（CLI 打印）。
    pub fn drain_uart(&mut self) -> Vec<u8> {
        self.collector.drain_uart()
    }

    /// 最近一条已执行指令（si 回显用）：pc 与指令。
    pub fn last_instruction(&self) -> Option<(u32, Instruction)> {
        self.last_instr
    }

    pub fn tohost_addr(&self) -> Option<u32> {
        self.image.as_ref().and_then(|i| i.tohost)
    }

    pub fn entry(&self) -> u32 {
        self.entry
    }

    pub fn mem_ring(&self) -> &VecDeque<EmuEvent> {
        self.collector.mem_ring()
    }
}

/// EvalContext 适配：寄存器读 CPU 视图，内存读走 peek（无副作用，D12）。
struct MachineCtx<'a>(&'a mut Machine);

impl EvalContext for MachineCtx<'_> {
    fn reg(&self, id: RegId) -> u32 {
        self.0.cpu.reg(id)
    }
    fn pc(&self) -> u32 {
        self.0.cpu.pc()
    }
    fn peek_word(&mut self, addr: u32) -> Result<u32, EvalError> {
        self.0
            .peek(addr)
            .map_err(|e| EvalError::Peek { addr, msg: e.to_string() })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    const HELLO_ELF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data/hello.elf");

    fn hello_machine() -> Machine {
        let bytes = std::fs::read(HELLO_ELF)
            .unwrap_or_else(|e| panic!("缺少 {HELLO_ELF}（先运行 tools/build-firmware.sh）：{e}"));
        let image = rremu_loader::load_elf(&bytes).unwrap();
        MachineBuilder::new().ram(0x8000_0000, 16 * 1024 * 1024).uart(0x1000_0000).load(image).build().unwrap()
    }

    #[test]
    fn hello_run_exits_zero_with_uart_output() {
        let mut m = hello_machine();
        let pause = AtomicBool::new(false);
        let rr = m.run(&pause);
        assert_eq!(rr, RunResult::Exited { code: 0 });
        assert_eq!(m.snapshot().state, RunState::Exited(0));
        // 事件流包含 "hello world\n"
        let out = m.drain_uart();
        assert_eq!(String::from_utf8_lossy(&out), "hello world\n");
        // mem ring 有访问记录
        assert!(!m.mem_ring().is_empty());
    }

    #[test]
    fn hello_step_n_pauses_and_tracks_last_instr() {
        let mut m = hello_machine();
        let rr = m.step_n(3);
        assert_eq!(rr, RunResult::Paused);
        assert_eq!(m.snapshot().state, RunState::Paused);
        // hello.S 的 `la` 伪指令展开为 auipc+addi：3 步后最后执行的是 0x80000008 的 auipc
        let (pc, instr) = m.last_instruction().expect("last instruction recorded");
        assert_eq!(pc, 0x8000_0008);
        assert_eq!(instr.to_string(), "auipc t0, 0x0");
        // 继续跑完
        let pause = AtomicBool::new(false);
        assert_eq!(m.run(&pause), RunResult::Exited { code: 0 });
    }

    #[test]
    fn reset_restores_initial_state() {
        let mut m = hello_machine();
        let pause = AtomicBool::new(false);
        m.run(&pause);
        assert_eq!(m.snapshot().state, RunState::Exited(0));
        m.reset().unwrap();
        let snap = m.snapshot();
        assert_eq!(snap.state, RunState::Idle);
        assert_eq!(snap.pc, 0x8000_0000);
        assert_eq!(snap.cycle, 0);
        assert!(m.mem_ring().is_empty());
        // 复位后可再次运行到退出
        assert_eq!(m.run(&AtomicBool::new(false)), RunResult::Exited { code: 0 });
    }

    #[test]
    fn peek_reads_ram_without_side_effects() {
        let mut m = hello_machine();
        // tohost 在 0x80000080，初始为 0
        let tohost = m.tohost_addr().unwrap();
        assert_eq!(m.peek(tohost).unwrap(), 0);
        m.step_n(1);
        // peek 不影响执行流
        assert_eq!(m.run(&AtomicBool::new(false)), RunResult::Exited { code: 0 });
        assert_eq!(m.peek(tohost).unwrap(), 1);
    }

    #[test]
    fn builder_rejects_out_of_range_segment() {
        let image = Image {
            segments: vec![rremu_loader::Segment { addr: 0x9000_0000, data: vec![0; 16] }],
            entry: 0x9000_0000,
            tohost: None,
        };
        let Err(err) = MachineBuilder::new().ram(0x8000_0000, 0x1000).load(image).build() else {
            panic!("builder should reject out-of-range segment")
        };
        assert!(matches!(err, MachineError::SegmentOutOfRange { .. }));
    }

    #[test]
    fn uart_input_reaches_fifo() {
        let mut m = hello_machine();
        m.uart_push_input(b'Z');
        assert_eq!(m.snapshot().uart_in_len, 1);
        // peek RDR 不消耗（D12 一致性）
        let rdr = 0x1000_0000;
        assert_eq!(m.peek(rdr).unwrap(), b'Z' as u32);
        assert_eq!(m.snapshot().uart_in_len, 1);
    }

    #[test]
    fn expr_alias_table_matches_isa() {
        // PLAN §5.7：expr 与 isa 的双别名表由本测试锁定同步
        for (i, name) in rremu_isa::ABI_NAMES.iter().enumerate() {
            assert_eq!(rremu_expr::reg_from_name(name), Some(i as u8), "alias `{name}`");
        }
        assert_eq!(rremu_expr::reg_from_name("fp"), Some(8)); // expr 侧补充别名
    }

    #[test]
    fn eval_reads_regs_pc_and_memory() {
        let mut m = hello_machine();
        assert_eq!(m.eval(&rremu_expr::parse("pc").unwrap()).unwrap(), 0x8000_0000);
        assert_eq!(m.eval(&rremu_expr::parse("x0+1").unwrap()).unwrap(), 1);
        // 跑前 tohost == 0，跑后 == 1；eval 的 peek 不影响运行（D12）
        let tohost = m.tohost_addr().unwrap();
        let expr = rremu_expr::parse(&format!("*{tohost:#x}")).unwrap();
        assert_eq!(m.eval(&expr).unwrap(), 0);
        let pause = AtomicBool::new(false);
        assert_eq!(m.run(&pause), RunResult::Exited { code: 0 });
        assert_eq!(m.eval(&expr).unwrap(), 1);
    }
}
