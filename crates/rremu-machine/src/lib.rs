//! rremu-machine：Machine/Builder/快照/控制 —— 前端（cli/tui/window）唯一入口。

use std::collections::{BTreeSet, VecDeque};

use rremu_bus::{Bus, BusError, BusDevice, BusRouter, EventSink};
use rremu_cpu::Cpu;
use rremu_decode::{decode16, decode32, instr_len};
use rremu_expr::{EvalContext, EvalError, Expr, RegId};
use rremu_isa::csr;
use rremu_loader::Image;
#[cfg(test)]
use rremu_config::Config;
use thiserror::Error;

// 前端经 machine 间接使用（依赖红线：cli/tui 不得直接依赖 isa/bus）。
// pub use 同时将名称引入本模块，供内部代码直接使用。
pub use rremu_bus::{AccessSize, EmuEvent, MemKind};
pub use rremu_isa::{reg_name, Instruction, Priv};

pub mod dump;
mod builder;
mod debugger;
mod events;
mod runtime;
mod framebuffer;
mod snapshot;
pub mod term;

use term::Terminal;

pub use debugger::{DebugController, Watchpoint};
pub use events::RingCollector;
pub use framebuffer::FramebufferFrame;
pub use snapshot::{CsrSnapshot, MachineSnapshot, TraceEntry};
pub(crate) use framebuffer::FramebufferAccessor;

/// 运行状态（快照展示用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Paused,
    Exited(i32),
    /// 模拟遇到不可恢复错误（如 vectored mtvec 未实现），已终止。
    Faulted(String),
}

/// 运行控制结果（含断点/监视点命中）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunResult {
    Exited { code: i32 },
    Paused,
    HitBreakpoint { addr: u32 },
    HitWatchpoint { id: usize, pc: u32, old: u32, new: u32 },
    /// 模拟遇到不可恢复错误，已终止；reason 为故障描述。
    Faulted { reason: String },
}

/// 反汇编行（TUI 前端用）。
#[derive(Debug, Clone)]
pub struct DisasmLine {
    pub addr: u32,
    pub len: u8,
    pub text: String,
    pub is_current: bool,
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
    #[error("reset failed: {0}")]
    ResetFailed(String),
}

/// 机器实例：CPU + 总线 + 设备 + 事件收集。
pub struct Machine {
    cpu: Cpu,
    bus: BusRouter,
    collector: RingCollector,
    dbg: DebugController,
    fb: FramebufferAccessor,
    image: Option<Image>,
    ram_base: u32,
    uart_base: Option<u32>,
    clint_base: Option<u32>,
    entry: u32,
    state: RunState,
    /// UART 终端视图（有状态，逐字节推进）。
    terminal: Terminal,
    /// CLI 直出 UART 字节的回调；TUI/默认关闭。
    uart_sink: Option<Box<dyn FnMut(u8)>>,
    trace: VecDeque<TraceEntry>,
    /// 指令 trace 环形缓冲容量（来自配置）。
    trace_capacity: usize,
    /// 最近一次 trap 的 (mcause, mepc, mtval)；从事件环之外持久保留。
    last_trap: Option<(u32, u32, u32)>,
    /// trap 发生瞬间的指令 trace 快照（崩溃路径，不含之后的 handler）。
    trace_at_trap: Option<Vec<TraceEntry>>,
}

impl Machine {
    /// 检查所有监视点：求值表达式得到当前值，与上次值比较。
    /// 变化则返回 HitWatchpoint（pc 为引起变化的指令）并更新 last_value / hit_count。
    pub(crate) fn check_watchpoints(&mut self, pc: u32) -> Option<RunResult> {
        let entries = self.dbg.watchpoint_entries();
        for (id, expr) in entries {
            // 值语义：EXPR 求值结果就是被监视的值（如 *ADDR 解引用、x5 寄存器）。
            // 求值失败（如解引用未映射地址）则跳过，下一步重试。
            let new = match self.eval(&expr) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some((old, new)) = self.dbg.update_if_changed(id, new) {
                return Some(RunResult::HitWatchpoint { id, pc, old, new });
            }
        }
        None
    }

    /// 添加断点；返回是否为新增。
    pub fn add_breakpoint(&mut self, addr: u32) -> bool {
        self.dbg.add_breakpoint(addr)
    }

    /// 移除断点；返回是否存在。
    pub fn remove_breakpoint(&mut self, addr: u32) -> bool {
        self.dbg.remove_breakpoint(addr)
    }

    pub fn breakpoints(&self) -> &BTreeSet<u32> {
        self.dbg.breakpoints()
    }

    /// 添加监视点，返回分配的 id（从 1 开始）。
    pub fn add_watchpoint(&mut self, expr: Expr) -> usize {
        // 值语义：EXPR 求值即初值；求值失败（如地址未映射）先记 0，随步进重试
        let last_value = self.eval(&expr).unwrap_or(0);
        self.dbg.add_watchpoint(expr, last_value)
    }

    /// 按 id 移除监视点；返回是否存在。
    pub fn remove_watchpoint(&mut self, id: usize) -> bool {
        self.dbg.remove_watchpoint(id)
    }

    pub fn watchpoints(&self) -> &[Watchpoint] {
        self.dbg.watchpoints()
    }


    /// 复位：寄存器/计数器归零、RAM 清零、镜像重载。
    pub fn reset(&mut self) -> Result<(), MachineError> {
        self.cpu.reset(self.entry);
        // 复位 RAM（必须成功）
        self.reset_device(self.ram_base)?;
        // 复位其他已映射设备（UART/CLINT/FB）；未映射或不可复位则忽略
        for base in [self.uart_base, self.clint_base, self.fb.base()].into_iter().flatten() {
            let _ = self.reset_device(base);
        }
        if let Some(img) = &self.image {
            for seg in &img.segments {
                self.bus.write_bytes(seg.addr, &seg.data)?;
            }
        }
        self.collector.clear();
        self.state = RunState::Idle;
        self.terminal = Terminal::new();
        self.trace.clear();
        self.last_trap = None;
        self.trace_at_trap = None;
        self.dbg.reset();
        Ok(())
    }

    /// 复位单个设备：支持 Resetable 则复位，否则报错（调用方决定是否忽略）。
    fn reset_device(&mut self, base: u32) -> Result<(), MachineError> {
        let dev = self.bus.device_mut(base)
            .ok_or_else(|| MachineError::ResetFailed(format!("no device at {base:#x}")))?;
        if let Some(r) = dev.as_resetable() {
            r.reset();
        }
        Ok(())
    }

    /// 设置 UART 直出回调（CLI 用）；传 None 关闭。
    pub fn set_uart_sink(&mut self, sink: Option<Box<dyn FnMut(u8)>>) {
        self.uart_sink = sink;
    }

    /// 机器快照（前端唯一状态视图），包含事件环。
    pub fn snapshot(&self) -> MachineSnapshot {
        self.snapshot_inner(true)
    }

    /// 轻量快照：保留寄存器/CSR/状态，不克隆事件环。
    /// 供隐藏了内存事件面板的前端档位使用（如 TUI 的 UART 全屏）。
    pub fn snapshot_compact(&self) -> MachineSnapshot {
        self.snapshot_inner(false)
    }

    fn snapshot_inner(&self, include_events: bool) -> MachineSnapshot {
        MachineSnapshot {
            cycle: self.cpu.cycle(),
            pc: self.cpu.pc(),
            priv_: self.cpu.priv_(),
            gprs: *self.cpu.regs(),
            csrs: CsrSnapshot {
                mstatus: self.cpu.debug_csr_read(csr::MSTATUS).unwrap_or(0),
                mtvec: self.cpu.debug_csr_read(csr::MTVEC).unwrap_or(0),
                mepc: self.cpu.debug_csr_read(csr::MEPC).unwrap_or(0),
                mcause: self.cpu.debug_csr_read(csr::MCAUSE).unwrap_or(0),
                mtval: self.cpu.debug_csr_read(csr::MTVAL).unwrap_or(0),
                mie: self.cpu.debug_csr_read(csr::MIE).unwrap_or(0),
                mip: self.cpu.debug_csr_read(csr::MIP).unwrap_or(0),
                mcycle: self.cpu.cycle(),
                minstret: self.cpu.instret(),
            },
            state: self.state.clone(),
            mem_ring: if include_events {
                self.collector.mem_ring().clone()
            } else {
                VecDeque::new()
            },
        }
    }

    /// 调试 peek：无副作用读一个 Word（表达式/x 命令用）。
    /// 调试器语义：支持非对齐地址（拼字），取指路径的严格对齐不受影响。
    pub fn peek(&mut self, addr: u32) -> Result<u32, BusError> {
        self.peek_word_maybe_unaligned(addr)
    }

    /// 反汇编指定地址的指令（TUI 用，内部走无副作用 peek）。
    pub fn disasm_at(&mut self, addr: u32, is_current: bool) -> DisasmLine {
        let hw = match self.peek_half(addr) {
            Ok(h) => h,
            Err(_) => {
                return DisasmLine {
                    addr, len: 2, text: ".word ????????".into(), is_current,
                };
            }
        };
        let len = instr_len(hw);
        let instr = if len == 4 {
            match self.peek_word_maybe_unaligned(addr) {
                Ok(w) => decode32(w, addr),
                Err(_) => {
                    return DisasmLine {
                        addr, len: len as u8, text: format!(".word {:#010x}", 0),
                        is_current,
                    };
                }
            }
        } else {
            decode16(hw, addr)
        };
        let text = match instr {
            Ok(i) => i.to_string(),
            Err(_) => {
                let bits = if len == 4 {
                    self.peek_word_maybe_unaligned(addr).unwrap_or(0)
                } else {
                    u32::from(hw)
                };
                format!(".word {:#010x}", bits)
            }
        };
        DisasmLine { addr, len: len as u8, text, is_current }
    }

    /// 扫描反汇编范围（TUI 反汇编面板用）。
    /// 从 start 向前解码直到产出 `max_lines` 条或越过 end。
    pub fn disasm_range(&mut self, start: u32, end: u32, max_lines: usize, current_pc: u32) -> Vec<DisasmLine> {
        let mut lines = Vec::new();
        let mut addr = start;
        while addr <= end && lines.len() < max_lines {
            let line = self.disasm_at(addr, addr == current_pc);
            let step = line.len.max(2) as u32;
            lines.push(line);
            addr = match addr.checked_add(step) {
                Some(a) => a,
                None => break,
            };
        }
        lines
    }

    /// 无副作用读半字（小端，支持任意地址对齐）。
    fn peek_half(&mut self, addr: u32) -> Result<u16, BusError> {
        let aligned = addr & !3;
        let offset = (addr & 3) * 8;
        let word = self.bus.peek(aligned, AccessSize::Word)?;
        Ok(((word >> offset) & 0xFFFF) as u16)
    }

    /// 无副作用读字（支持非对齐地址，拼接两个对齐字）。
    fn peek_word_maybe_unaligned(&mut self, addr: u32) -> Result<u32, BusError> {
        if addr & 3 == 0 {
            return self.bus.peek(addr, AccessSize::Word);
        }
        let aligned = addr & !3;
        let offset = (addr & 3) * 8;
        let lo = self.bus.peek(aligned, AccessSize::Word)?;
        let hi = self.bus.peek(aligned + 4, AccessSize::Word)?;
        Ok((lo >> offset) | (hi << (32 - offset)))
    }

    /// 帧缓冲尺寸（无 framebuffer 返回 None）。
    pub fn framebuffer_dimensions(&self) -> Option<(u32, u32)> {
        self.fb.dimensions()
    }

    /// 帧缓冲全帧只读访问（不清 dirty）：TUI 缓存首次填充/尺寸变化时用。
    pub fn framebuffer_full_frame(&mut self) -> Option<FramebufferFrame<'_>> {
        self.fb.full_frame(&mut self.bus)
    }

    /// 帧缓冲 dirty 驱动零拷贝访问（独立窗口前端用）。
    ///
    /// - 无 framebuffer 或自上次刷新以来未写入（dirty=false）→ 返回 `None`；
    /// - dirty=true → 清 dirty 标志并返回 `Some(FramebufferFrame { width, height, pixels })`，
    ///   `pixels` 直接指向设备内部 `frame()` 缓冲，无 1.2MB clone。
    ///
    /// 生命周期约束：`pixels` 借用 `&mut self`，调用方必须先 `Window::update(pixels)`
    /// 完成上屏拷贝，再调用下一个会修改 machine 的方法。
    pub fn framebuffer_dirty_frame(&mut self) -> Option<FramebufferFrame<'_>> {
        self.fb.dirty_frame(&mut self.bus)
    }

    /// 求值调试表达式（p/x/w 命令共用；内存读走无副作用 peek）。
    pub fn eval(&mut self, expr: &Expr) -> Result<u32, EvalError> {
        let mut ctx = MachineCtx(self);
        expr.eval(&mut ctx)
    }

    /// 注入 UART 输入字节。
    pub fn uart_push_input(&mut self, byte: u8) {
        let Some(ub) = self.uart_base else { return };
        if let Some(sink) = self.bus.device_mut(ub).and_then(|d| d.as_input_sink())
        {
            sink.push_input(byte);
            self.collector.emit(EmuEvent::UartInput { byte });
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
        let e = self.trace.back()?;
        let decoded = if e.len == 2 { decode16(e.bits as u16, e.pc) } else { decode32(e.bits, e.pc) };
        decoded.ok().map(|i| (e.pc, i))
    }

    /// 记录一条已执行指令到 trace 环（满则淘汰最旧）。
    fn push_trace(&mut self, pc: u32, bits: u32, len: u8) {
        if self.trace.len() == self.trace_capacity {
            self.trace.pop_front();
        }
        self.trace.push_back(TraceEntry { pc, bits, len });
    }

    /// 最近 `trace_capacity` 条已执行指令（最旧在前），供 dump 使用。
    pub fn trace(&self) -> &VecDeque<TraceEntry> {
        &self.trace
    }

    /// 最近一次 trap 的 (mcause, mepc, mtval)（未发生过为 None）。
    pub fn last_trap(&self) -> Option<(u32, u32, u32)> {
        self.last_trap
    }

    /// trap 发生瞬间的指令 trace 快照（崩溃路径）。
    pub fn trace_at_trap(&self) -> Option<&[TraceEntry]> {
        self.trace_at_trap.as_deref()
    }

    /// 当前运行状态（轻量查询，避免为读状态做整快照）。
    pub fn state(&self) -> RunState {
        self.state.clone()
    }

    pub fn entry(&self) -> u32 {
        self.entry
    }

    pub fn clint_base(&self) -> Option<u32> {
        self.clint_base
    }

    pub fn framebuffer_base(&self) -> Option<u32> {
        self.fb.base()
    }

    /// 经地址定位总线设备（供前端直接访问设备能力，如 CLINT/FB）。
    pub fn device_mut(&mut self, addr: u32) -> Option<&mut dyn BusDevice> {
        self.bus.device_mut(addr)
    }

    /// 直接写入内存（用于在 ELF 镜像之外加载 DTB 等数据）。
    pub fn write_mem(&mut self, addr: u32, data: &[u8]) -> Result<(), MachineError> {
        self.bus.write_bytes(addr, data)?;
        Ok(())
    }

    pub fn mem_ring(&self) -> &VecDeque<EmuEvent> {
        self.collector.mem_ring()
    }

    /// UART 终端视图（借用，不走 snapshot）。
    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// 最近 UART 输入/输出事件（dump 用）。
    pub fn uart_ring(&self) -> &VecDeque<EmuEvent> {
        self.collector.uart_ring()
    }

    /// 构造结构化 dump（纯读取，无副作用）。`elf` 引用由调用方提供。
    pub fn dump(&self, elf: Option<dump::ElfRef>) -> dump::Dump {
        dump::Dump::from_machine(self, elf)
    }
}

/// EvalContext 适配：寄存器读 CPU 视图，内存读走 peek（无副作用）。
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

    /// 测试辅助：自定义 RAM 基址/大小（其余用默认布局）+ 镜像。
    pub(crate) fn test_machine(ram_base: u32, ram_size: u32, image: Image) -> Machine {
        let cfg = Config {
            layout: rremu_config::Layout { ram_base, ram_size, ..Default::default() },
            ..Config::default()
        };
        Machine::from_config(&cfg, Some(image)).unwrap()
    }

    #[test]
    fn from_config_rejects_out_of_range_segment() {
        let image = Image {
            segments: vec![rremu_loader::Segment { addr: 0x9000_0000, data: vec![0; 16] }],
            entry: 0x9000_0000,
        };
        let cfg = Config {
            layout: rremu_config::Layout { ram_base: 0x8000_0000, ram_size: 0x1000, ..Default::default() },
            ..Config::default()
        };
        let Err(err) = Machine::from_config(&cfg, Some(image)) else {
            panic!("from_config should reject out-of-range segment")
        };
        assert!(matches!(err, MachineError::SegmentOutOfRange { .. }));
    }

    #[test]
    fn from_config_injects_boot_args() {
        let base = 0x8000_0000;
        let mut cfg = Config::default();
        cfg.layout.ram_base = base;
        cfg.layout.ram_size = 0x1000;
        cfg.boot.hartid = 3;
        cfg.boot.dtb = Some(0x8000_0900);
        let machine = Machine::from_config(&cfg, Some(nop_image(base))).unwrap();
        assert_eq!(machine.cpu.reg(10), 3); // a0 = hartid
        assert_eq!(machine.cpu.reg(11), 0x8000_0900); // a1 = dtb
    }

    #[test]
    fn compact_snapshot_skips_event_ring() {
        let base = 0x8000_0000;
        let mut machine = test_machine(base, 0x1000, nop_image(base));
        machine.step_n(1);
        let full = machine.snapshot();
        let compact = machine.snapshot_compact();
        assert!(!full.mem_ring.is_empty());
        assert!(compact.mem_ring.is_empty());
        assert_eq!(full.cycle, compact.cycle);
        assert_eq!(full.pc, compact.pc);
        assert_eq!(full.gprs, compact.gprs);
        assert_eq!(full.state, compact.state);
    }

    #[test]
    fn vectored_mtvec_trap_enters_faulted_state() {
        // 程序：设置 mtvec 为 vectored 模式（bit0=1）→ ecall 触发 trap → Faulted
        let base = 0x8000_0000;
        let code = [
            0x10100293u32, // addi x5, x0, 0x101  (mtvec=0x101, vectored)
            0x30529073,     // csrrw x0, mtvec, x5
            0x00000073,     // ecall
        ];
        let mut bytes = Vec::new();
        for w in code {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let image = Image {
            segments: vec![rremu_loader::Segment { addr: base, data: bytes }],
            entry: base,
        };
        let mut machine = test_machine(base, 0x1000, image);
        let rr = machine.step_n(3);
        assert!(matches!(rr, RunResult::Faulted { reason } if reason.contains("vectored mtvec")));
        assert!(matches!(machine.state(), RunState::Faulted(_)));
    }

    #[test]
    fn expr_alias_table_matches_isa() {
        // expr 与 isa 的双别名表由本测试锁定同步（ABI 别名漂移唯一防线）
        for (i, name) in rremu_isa::ABI_NAMES.iter().enumerate() {
            assert_eq!(rremu_expr::reg_from_name(name), Some(i as u8), "alias `{name}`");
        }
        assert_eq!(rremu_expr::reg_from_name("fp"), Some(8)); // expr 侧补充别名
    }

    #[test]
    fn last_trap_recorded_on_illegal_instruction() {
        let base = 0x8000_0000;
        // 非法指令 0x0000（c.addi4spn nzuimm=0）
        let image = Image {
            segments: vec![rremu_loader::Segment { addr: base, data: 0x0000u16.to_le_bytes().to_vec() }],
            entry: base,
        };
        let mut machine = test_machine(base, 0x1000, image);
        machine.step_n(1); // trap ILLEGAL_INSTR → pc = mtvec(0)
        assert_eq!(machine.last_trap(), Some((2, base, 0)));
        // trap 时刻冻结的 trace 已生成（此例非法指令为首条，故为空）
        assert_eq!(machine.trace_at_trap(), Some(&[][..]));
    }

    #[test]
    fn trace_at_trap_captures_crash_path() {
        let base = 0x8000_0000;
        // 3 条 nop(all addi x0,x0,0) + 非法指令
        let mut bytes = Vec::new();
        for _ in 0..3 {
            bytes.extend_from_slice(&0x0000_0013u32.to_le_bytes());
        }
        bytes.extend_from_slice(&0x0000u16.to_le_bytes());
        let image = Image { segments: vec![rremu_loader::Segment { addr: base, data: bytes }], entry: base };
        let mut machine = test_machine(base, 0x1000, image);
        machine.step_n(4); // 3 nop + trap
        let frozen = machine.trace_at_trap().unwrap();
        assert_eq!(frozen.len(), 3);
        assert_eq!(frozen[0].pc, base);
        assert_eq!(frozen[2].pc, base + 8);
    }

    #[test]
    fn uart_output_feeds_terminal() {
        let base = 0x8000_0000;
        // lui t0, 0x10000; li t1,'h'; sb t1,0(t0); li t1,'i'; sb t1,0(t0)
        let code: [u32; 5] = [
            (0x10000u32 << 12) | (5 << 7) | 0b0110111,
            (0x068u32 << 20) | (6 << 7) | 0b0010011,
            0x0062_8023,
            (0x069u32 << 20) | (6 << 7) | 0b0010011,
            0x0062_8023,
        ];
        let mut bytes = Vec::new();
        for w in code {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let image = Image { segments: vec![rremu_loader::Segment { addr: base, data: bytes }], entry: base };
        let mut m = test_machine(base, 0x1000, image);
        m.step_n(5);
        let line: String = m
            .terminal()
            .lines()[0]
            .iter()
            .filter(|c| !c.wide_cont)
            .map(|c| c.ch)
            .collect();
        assert_eq!(line, "hi");
    }

    #[test]
    fn trace_records_and_caps_at_capacity() {
        let base = 0x8000_0000;
        let nop = 0x0000_0013u32; // addi x0, x0, 0
        let mut bytes = Vec::new();
        for _ in 0..40 {
            bytes.extend_from_slice(&nop.to_le_bytes());
        }
        let image = Image { segments: vec![rremu_loader::Segment { addr: base, data: bytes }], entry: base };
        let mut machine = test_machine(base, 0x1000, image);
        machine.step_n(40);
        // 容量 32：运行 40 条后只保留最后 32 条
        assert_eq!(machine.trace().len(), rremu_config::DEFAULT_TRACE_CAPACITY);
        let front = machine.trace().front().unwrap();
        assert_eq!(front.pc, base + 8 * 4); // 前 8 条被淘汰
        assert_eq!(front.bits, nop);
        assert_eq!(front.len, 4);
        // last_instruction 从 trace 派生
        assert_eq!(machine.last_instruction().unwrap().0, base + 39 * 4);
    }

    fn nop_image(entry: u32) -> Image {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0000_0013u32.to_le_bytes());
        Image { segments: vec![rremu_loader::Segment { addr: entry, data: bytes }], entry }
    }

    #[test]
    fn step_n_breakpoint_sets_paused_state() {
        let base = 0x8000_0000;
        let mut machine = test_machine(base, 0x1000, nop_image(base));
        machine.add_breakpoint(base);
        let rr = machine.step_n(1);
        assert_eq!(rr, RunResult::HitBreakpoint { addr: base });
        assert_eq!(machine.state(), RunState::Paused);
    }

    #[test]
    fn reset_clears_uart_and_clint_state() {
        let base = 0x8000_0000;
        let mut machine = test_machine(base, 0x1000, nop_image(base));

        // 注入 UART 输入 + 置 CLINT msip
        machine.uart_push_input(b'A');
        {
            let dev = machine.device_mut(0x0200_0000).unwrap();
            let mut sink = rremu_bus::NullSink;
            dev.write(0x0200_0000, AccessSize::Word, 1, &mut sink).unwrap();
        }

        machine.reset().unwrap();

        // UART FIFO 清空（RDR peek 为 0），CLINT msip 清零
        assert_eq!(machine.peek(0x1000_0000).unwrap(), 0);
        assert_eq!(machine.peek(0x0200_0000).unwrap(), 0);
    }
}
