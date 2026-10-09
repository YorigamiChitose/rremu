//! rremu-machine：Machine/Builder/快照/控制 —— 前端（cli/tui/window）唯一入口。

use std::collections::{BTreeSet, VecDeque};
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

/// 运行控制结果（含断点/监视点命中）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Exited { code: i32 },
    Trapped { cause: u32 },
    Paused,
    HitBreakpoint { addr: u32 },
    HitWatchpoint { id: usize, pc: u32, old: u32, new: u32 },
}

/// 监视点：跟踪某表达式的值变化（值语义，EXPR 与 p 命令同模型）。
#[derive(Debug, Clone)]
pub struct Watchpoint {
    pub id: usize,
    pub expr: Expr,
    pub last_value: u32,
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
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            next_wp_id: 1,
            bp_suppress: None,
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
    breakpoints: BTreeSet<u32>,
    watchpoints: Vec<Watchpoint>,
    next_wp_id: usize,
    /// 刚命中过的断点地址：下一次 step_once 对该地址放行一次，
    /// 使 `c`/`si` 能越过当前断点（经典调试器语义），后续命中照常。
    bp_suppress: Option<u32>,
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
    /// 仅 Halt、断点/监视点命中或 pause 置位时返回。
    pub fn run(&mut self, pause: &AtomicBool) -> RunResult {
        self.state = RunState::Running;
        loop {
            if pause.load(Ordering::Relaxed) {
                self.state = RunState::Paused;
                return RunResult::Paused;
            }
            match self.step_once() {
                Some(RunResult::Exited { code }) => {
                    self.state = RunState::Exited(code);
                    return RunResult::Exited { code };
                }
                Some(rr @ (RunResult::HitBreakpoint { .. } | RunResult::HitWatchpoint { .. })) => {
                    self.state = RunState::Paused;
                    return rr;
                }
                Some(RunResult::Trapped { .. }) | Some(RunResult::Paused) | None => {}
            }
        }
    }

    /// 执行一条指令；需要停止时返回 Some(RunResult)。
    /// 断点在取指前检查（命中则不执行该指令）；监视点在执行后检查。
    /// 注意：陷入（ecall/ebreak/misaligned）是非致命的 —— CPU 已将 pc 重定向
    /// 至 mtvec，陷入向量代码将在下一轮 step_once 中执行。state 保持 Running，
    /// 以便前端可继续 c/si；RunResult::Trapped 仅供 si 单步时报告。
    fn step_once(&mut self) -> Option<RunResult> {
        self.bus.set_cycle(self.cpu.cycle());
        let pc_before = self.cpu.pc();

        // 断点：命中则不执行，直接返回；刚命中过的地址放行一次（越过断点）
        if self.breakpoints.contains(&pc_before) && self.bp_suppress != Some(pc_before) {
            self.bp_suppress = Some(pc_before);
            return Some(RunResult::HitBreakpoint { addr: pc_before });
        }
        self.bp_suppress = None;

        match self.cpu.step(&mut self.bus, &mut self.collector) {
            Ok(StepOutcome::Executed { instr, .. }) => {
                self.last_instr = Some((pc_before, instr));
                self.sync_uart_in_len();
                if let Some(code) = self.collector.take_halt() {
                    self.state = RunState::Exited(code);
                    return Some(RunResult::Exited { code });
                }
                // 监视点：检查每个被跟踪表达式的值是否变化
                self.check_watchpoints(pc_before)
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

    /// 检查所有监视点：求值表达式得到当前值，与上次值比较。
    /// 变化则返回 HitWatchpoint（pc 为引起变化的指令）并更新 last_value。
    fn check_watchpoints(&mut self, pc: u32) -> Option<RunResult> {
        for i in 0..self.watchpoints.len() {
            // 值语义：EXPR 求值结果就是被监视的值（如 *ADDR 解引用、x5 寄存器）。
            // 求值失败（如解引用未映射地址）则跳过，下一步重试。
            let new = match self.eval(&self.watchpoints[i].expr.clone()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if new != self.watchpoints[i].last_value {
                let (id, old) = (self.watchpoints[i].id, self.watchpoints[i].last_value);
                self.watchpoints[i].last_value = new;
                return Some(RunResult::HitWatchpoint { id, pc, old, new });
            }
        }
        None
    }

    /// 添加断点；返回是否为新增。
    pub fn add_breakpoint(&mut self, addr: u32) -> bool {
        self.breakpoints.insert(addr)
    }

    /// 移除断点；返回是否存在。
    pub fn remove_breakpoint(&mut self, addr: u32) -> bool {
        self.breakpoints.remove(&addr)
    }

    pub fn breakpoints(&self) -> &BTreeSet<u32> {
        &self.breakpoints
    }

    /// 添加监视点，返回分配的 id（从 1 开始）。
    pub fn add_watchpoint(&mut self, expr: Expr) -> usize {
        let id = self.next_wp_id;
        self.next_wp_id += 1;
        // 值语义：EXPR 求值即初值；求值失败（如地址未映射）先记 0，随步进重试
        let last_value = self.eval(&expr).unwrap_or(0);
        self.watchpoints.push(Watchpoint { id, expr, last_value });
        id
    }

    /// 按 id 移除监视点；返回是否存在。
    pub fn remove_watchpoint(&mut self, id: usize) -> bool {
        let before = self.watchpoints.len();
        self.watchpoints.retain(|w| w.id != id);
        self.watchpoints.len() != before
    }

    pub fn watchpoints(&self) -> &[Watchpoint] {
        &self.watchpoints
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
        self.bp_suppress = None;
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
    fn breakpoint_stops_run_before_instruction() {
        let mut m = hello_machine();
        // hello.S 第 3 条指令在 0x80000008（auipc）
        m.add_breakpoint(0x8000_0008);
        let pause = AtomicBool::new(false);
        let rr = m.run(&pause);
        assert_eq!(rr, RunResult::HitBreakpoint { addr: 0x8000_0008 });
        // 断点命中时该指令尚未执行，pc 仍为断点地址
        assert_eq!(m.snapshot().pc, 0x8000_0008);
        // 移除断点后可继续跑完
        m.remove_breakpoint(0x8000_0008);
        assert_eq!(m.run(&AtomicBool::new(false)), RunResult::Exited { code: 0 });
    }

    #[test]
    fn continue_past_breakpoint_then_hits_next_time() {
        let mut m = hello_machine();
        // hello.S 第 3 条指令在 0x80000008（auipc）；hello 只经过它一次
        m.add_breakpoint(0x8000_0008);
        let pause = AtomicBool::new(false);
        assert_eq!(m.run(&pause), RunResult::HitBreakpoint { addr: 0x8000_0008 });
        // 不移除断点，直接继续：应放行一次并跑完（而非原地重复命中）
        assert_eq!(m.run(&pause), RunResult::Exited { code: 0 });
    }

    #[test]
    fn watchpoint_value_semantics_reports_pc_and_old_new() {
        let mut m = hello_machine();
        // 值语义：EXPR 求值即被监视的值。监视 x2（sp）：
        // 第 1 条 auipc sp 改变它；第 2 条是 mv sp,sp（lo=0 优化），
        // 值不变 → 不误报 —— 这正是值语义的意义
        let expr = rremu_expr::parse("x2").unwrap();
        m.add_watchpoint(expr);
        let pause = AtomicBool::new(false);
        match m.run(&pause) {
            RunResult::HitWatchpoint { pc, old, new, .. } => {
                assert_eq!(pc, 0x8000_0000); // auipc sp 所在指令
                assert_eq!(old, 0);
                assert_ne!(new, 0); // sp 已获得栈顶高 20 位
            }
            rr => panic!("expected HitWatchpoint, got {rr:?}"),
        }
        // mv sp,sp 未改变值；x2 此后稳定 → 一路跑完
        assert_eq!(m.run(&pause), RunResult::Exited { code: 0 });
    }

    #[test]
    fn watchpoint_deref_expr_watches_memory_value() {
        let mut m = hello_machine();
        // 值语义下 *ADDR 直接监视该地址的字；hello 运行中无人写 RAM，
        // 初始 0 → 无变化 → 直接跑完退出（不命中）
        let expr = rremu_expr::parse("*0x80000100").unwrap();
        m.add_watchpoint(expr);
        let pause = AtomicBool::new(false);
        assert_eq!(m.run(&pause), RunResult::Exited { code: 0 });
    }

    #[test]
    fn breakpoint_rehits_on_second_pass() {
        let mut m = hello_machine();
        // 0x80000000 是入口第一条指令：run 停在它，继续放行执行；
        // reset 后重跑应再次命中（suppress 已被第一步清除/复位）
        m.add_breakpoint(0x8000_0000);
        let pause = AtomicBool::new(false);
        assert_eq!(m.run(&pause), RunResult::HitBreakpoint { addr: 0x8000_0000 });
        m.step_n(1); // 放行一条
        m.reset().unwrap();
        assert_eq!(m.run(&pause), RunResult::HitBreakpoint { addr: 0x8000_0000 });
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

    #[test]
    fn exit_code_nonzero_via_tohost() {
        // riscv-tests htif 约定：tohost 写值 bit0==1 表示退出，
        // exit code = (value >> 1)。hello.elf 写 1 → exit 0；
        // 这里手动经总线写 (42 << 1) | 1 = 85，期望 exit code 42。
        let mut m = hello_machine();
        let tohost = m.tohost_addr().expect("hello.elf has tohost");
        let value: u32 = (42 << 1) | 1;
        // 直接经 Bus::write 写 tohost：私有字段本模块内可访问；
        // bus 与 collector 分离借用，避免对 self 整体借用。
        {
            let bus = &mut m.bus;
            let collector = &mut m.collector;
            bus.write(tohost, AccessSize::Word, value, collector).unwrap();
        }
        let pause = AtomicBool::new(false);
        let rr = m.run(&pause);
        assert_eq!(rr, RunResult::Exited { code: 42 });
        assert_eq!(m.snapshot().state, RunState::Exited(42));
        // tohost 内存值已更新为 85
        assert_eq!(m.peek(tohost).unwrap(), value);
    }

    #[test]
    fn multiple_breakpoints_hit_in_order() {
        // hello.S：0x80000000 第一条指令；0x80000008 第三条指令（auipc）。
        // 两个断点应按地址顺序依次命中：第一次停在 0x80000000；
        // 继续放行后越过 0x80000000、执行若干指令，停在 0x80000008。
        let mut m = hello_machine();
        m.add_breakpoint(0x8000_0000);
        m.add_breakpoint(0x8000_0008);
        let pause = AtomicBool::new(false);
        let rr1 = m.run(&pause);
        assert_eq!(rr1, RunResult::HitBreakpoint { addr: 0x8000_0000 });
        assert_eq!(m.snapshot().pc, 0x8000_0000);
        let rr2 = m.run(&pause);
        assert_eq!(rr2, RunResult::HitBreakpoint { addr: 0x8000_0008 });
        assert_eq!(m.snapshot().pc, 0x8000_0008);
    }

    #[test]
    fn snapshot_contains_all_fields() {
        // 跑到退出后校验快照所有字段：cycle/pc 推进、状态为 Exited、
        // gprs 为 [u32;32]、uart_out 含 "hello"。
        const BASE: u32 = 0x8000_0000;
        let mut m = hello_machine();
        let pause = AtomicBool::new(false);
        assert_eq!(m.run(&pause), RunResult::Exited { code: 0 });
        let snap = m.snapshot();
        assert!(snap.cycle > 0, "cycle should advance: {}", snap.cycle);
        assert!(snap.pc > BASE, "pc should advance past BASE: {:#x}", snap.pc);
        assert!(
            matches!(snap.state, RunState::Exited(_) | RunState::Running),
            "state should be Exited or Running: {:?}",
            snap.state,
        );
        assert_eq!(snap.gprs.len(), 32);
        let out = String::from_utf8_lossy(&snap.uart_out);
        assert!(out.contains("hello"), "uart_out should contain 'hello': {out}");
    }
}
