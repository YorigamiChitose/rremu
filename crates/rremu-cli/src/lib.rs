//! rremu-cli：GDB 风格 REPL 调试器前端（rustyline）。只通过 machine 公开 API 交互。
//!
//! 命令集：`help` / `c` / `si [n]` / `x n m` / `p expr` / `info r` / `q`，
//! 以及断点/监视点（`b` / `w` / `info w` / `d`），
//! 以及 UART 输入（`uart`：进入 raw 模式逐键发给 UART RX，Esc 退出）。

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rremu_config::Config;
use rremu_expr::{parse as parse_expr, Expr};
use rremu_machine::{reg_name, EmuEvent, Machine, RunResult, RunState};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

/// 帧缓冲显示后端（具体实现由 bin 提供，如 minifb 窗口）。
///
/// 库本身不依赖任何窗口/图形系统；pixels 长度契约 = 帧宽 × 帧高。
pub trait FbDisplay {
    /// 上传新像素帧；返回后端是否仍然打开。
    fn update(&mut self, pixels: &[u32]) -> bool;
    /// 仅处理后端事件（无新帧时）；返回是否打开。
    fn poll(&mut self) -> bool;
}

/// 窗口工厂（懒启动，bin 注入）：fb 首次产生写入时调用以创建窗口；
/// 返回 None 表示当前环境无法开窗（如无 X11）。
pub type WindowFactory = Box<dyn FnMut() -> Option<Box<dyn FbDisplay>>>;

/// REPL 调试器：持有 machine 实例与全局 Ctrl+C 暂停标志。
pub struct Repl {
    machine: Machine,
    /// 统一配置（含 dump/elf 路径等）。
    cfg: Config,
    pause_flag: Arc<AtomicBool>,
    /// 活动显示后端；None 表示尚未懒启动、用户已关窗或无窗可用。
    display: Option<Box<dyn FbDisplay>>,
    /// 窗口工厂：尚无窗口且 fb 产生写入时调用；开窗失败或用户关窗后丢弃。
    window_factory: Option<WindowFactory>,
    /// UART 输出已在 machine 事件路径直出，暂停时不再重复 flush。
    uart_direct: bool,
}

/// 单条命令的执行结果：决定 REPL 循环与进程退出码。
enum Outcome {
    Continue,
    Quit,
    ProgramExited(i32),
    /// 模拟遇到不可恢复错误。
    Faulted(String),
}

/// 有窗时 cmd_continue 的步进批量：在每批之后泵窗（dirty 上屏 / 否则 poll 关窗）。
const WINDOW_BATCH: u64 = 1024;

impl Repl {
    pub fn new(
        mut machine: Machine,
        cfg: Config,
        pause_flag: Arc<AtomicBool>,
        window_factory: Option<WindowFactory>,
    ) -> Self {
        // CLI 模式 UART 直出：字节随事件流实时写 stdout，便于 `rremu -c c > log`。
        machine.set_uart_sink(Some(Box::new(|byte| {
            let mut out = io::stdout();
            let _ = out.write_all(&[byte]);
            if byte == b'\n' {
                let _ = out.flush();
            }
        })));
        Self { machine, cfg, pause_flag, display: None, window_factory, uart_direct: true }
    }

    /// 将当前机器状态序列化为 JSON 并写入 `path`；失败仅告警。
    fn write_dump(&self, path: &str) {
        let elf = self.cfg.paths.elf.as_deref().map(rremu_machine::dump::ElfRef::from_path);
        match self.machine.dump(elf).to_json() {
            Ok(json) => match std::fs::write(path, json) {
                Ok(()) => println!("Dump written to {path}."),
                Err(e) => eprintln!("rremu: dump write failed: {e}"),
            },
            Err(e) => eprintln!("rremu: dump serialize failed: {e}"),
        }
    }

    /// 交互式 REPL（无 `-c` 时）。
    pub fn run_interactive(&mut self) -> i32 {
        let mut rl = match DefaultEditor::new() {
            Ok(rl) => rl,
            Err(e) => {
                eprintln!("rremu: failed to initialize readline: {e}");
                return 1;
            }
        };
        loop {
            let line = match rl.readline("rremu> ") {
                Ok(l) => l,
                Err(ReadlineError::Interrupted) => continue, // 提示符态 Ctrl+C：清行重来
                Err(ReadlineError::Eof) => break 0,           // Ctrl+D：等同 q
                Err(e) => {
                    eprintln!("rremu: readline error: {e}");
                    break 1;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let _ = rl.add_history_entry(trimmed);
            match self.dispatch(trimmed) {
                Outcome::Continue => {}
                Outcome::Quit => break 0,
                // 交互模式下程序退出后仍保留 REPL 供检查，用户 q 退出。
                Outcome::ProgramExited(_) => {}
                Outcome::Faulted(reason) => {
                    eprintln!("faulted: {reason}");
                }
            }
        }
    }

    /// 批处理模式（`-c` 每条命令）。
    pub fn run_batch(&mut self, cmds: &[String]) -> i32 {
        // 程序退出/故障后继续执行剩余命令（如退出后 `dump`），最终返回退出码。
        let mut exit_code = 0;
        for cmd in cmds {
            let cmd = cmd.trim();
            println!("rremu> {cmd}");
            match self.dispatch(cmd) {
                Outcome::Continue => {}
                Outcome::Quit => break,
                Outcome::ProgramExited(code) => exit_code = code,
                Outcome::Faulted(reason) => {
                    eprintln!("faulted: {reason}");
                    exit_code = 1;
                }
            }
        }
        exit_code
    }

    fn dispatch(&mut self, line: &str) -> Outcome {
        let outcome = match parse_command(line) {
            Ok(cmd) => self.execute(cmd),
            Err(msg) => {
                eprintln!("rremu: {msg}");
                Outcome::Continue
            }
        };
        // 程序终止（退出/故障）时按配置自动写 dump
        if let Some(path) = self.cfg.paths.dump_on_exit.clone()
            && matches!(outcome, Outcome::ProgramExited(_) | Outcome::Faulted(_))
        {
            self.write_dump(&path);
        }
        outcome
    }

    fn execute(&mut self, cmd: Command) -> Outcome {
        match cmd {
            Command::Help => {
                self.print_help();
                Outcome::Continue
            }
            Command::Continue => self.cmd_continue(),
            Command::Step(n) => self.cmd_step(n),
            Command::Examine { count, addr } => self.cmd_examine(count, &addr),
            Command::Print(expr) => self.cmd_print(&expr),
            Command::InfoRegs => {
                self.cmd_info_regs();
                Outcome::Continue
            }
            Command::Breakpoint(expr) => self.cmd_breakpoint(&expr),
            Command::Watchpoint(expr) => self.cmd_watchpoint(&expr),
            Command::Delete(n) => self.cmd_delete(n),
            Command::InfoWatchpoints => {
                self.cmd_info_watchpoints();
                Outcome::Continue
            }
            Command::Uart => self.cmd_uart_input(),
            Command::Dump(path) => {
                self.write_dump(&path);
                Outcome::Continue
            }
            Command::Quit => Outcome::Quit,
        }
    }

    fn print_help(&self) {
        println!("Available commands:");
        println!("  help          Show this help");
        println!("  c             Continue execution (stops on exit/trap/breakpoint/Ctrl+C)");
        println!("  si [N]        Step N instructions (default 1)");
        println!("  x N ADDR      Examine N words at ADDR (peek, no side effects)");
        println!("  p EXPR        Evaluate and print expression");
        println!("  b EXPR        Set breakpoint at address EXPR");
        println!("  w EXPR        Watch value of EXPR, stops on change (*ADDR for memory, x5 for reg)");
        println!("  d N           Delete watchpoint N, or breakpoint N (index from 'info w')");
        println!("  info r        Show registers and CSRs");
        println!("  info w        List breakpoints and watchpoints");
        println!("  uart          Enter UART input mode (raw key→byte→UART RX, Esc to exit)");
        println!("  dump FILE     Write a structured machine-state dump (JSON) to FILE");
        println!("  q             Quit");
    }

    fn cmd_continue(&mut self) -> Outcome {
        if self.is_dead() {
            eprintln!("rremu: program has already exited/trapped");
            return Outcome::Continue;
        }
        self.pause_flag.store(false, Ordering::Relaxed);

        // 有显示后端（或窗口可能懒启动）时自行跑 step_n 循环：每批先
        // ensure_window（fb 首次写入时拉起窗口），再泵后端（dirty 上屏 /
        // 否则 poll 关窗）；确定无窗可能时走 machine.run 全速循环（仅靠
        // pause_flag 停机）。
        let mut display_closed = false;
        let result = if self.display.is_some() || self.window_factory.is_some() {
            loop {
                if self.pause_flag.load(Ordering::Relaxed) {
                    break RunResult::Paused;
                }
                let step_rr = self.machine.step_n(WINDOW_BATCH);
                self.ensure_window();
                if !self.pump_display() {
                    display_closed = true;
                    break RunResult::Paused;
                }
                match step_rr {
                    RunResult::Exited { code } => break RunResult::Exited { code },
                    RunResult::Faulted { reason } => break RunResult::Faulted { reason },
                    RunResult::HitBreakpoint { addr } => break RunResult::HitBreakpoint { addr },
                    RunResult::HitWatchpoint { id, pc, old, new } => {
                        break RunResult::HitWatchpoint { id, pc, old, new };
                    }
                    RunResult::Paused => {}
                }
            }
        } else {
            self.machine.run(&self.pause_flag)
        };

        if display_closed {
            println!("Framebuffer window closed; paused. Subsequent 'c' runs without window.");
        }

        let paused = matches!(result, RunResult::Paused);
        self.flush_uart();
        let outcome = self.report_result(result);
        // 挂起（Ctrl+C/关窗/步数上限）也写 dump，便于排查死循环
        if paused && let Some(path) = self.cfg.paths.dump_on_exit.clone() {
            self.write_dump(&path);
        }
        outcome
    }

    fn cmd_step(&mut self, n: u64) -> Outcome {
        if self.is_dead() {
            eprintln!("rremu: program has already exited/trapped");
            return Outcome::Continue;
        }
        for _ in 0..n {
            let result = self.machine.step_n(1);
            if let Some((pc, instr)) = self.machine.last_instruction() {
                println!("0x{pc:08x}:  {instr}");
            }
            self.flush_uart();
            // 单步也泵：懒启动窗口（首次写 fb 时弹出）+ 逐指令看 fb 变化
            self.ensure_window();
            if self.display.is_some() && !self.pump_display() {
                println!("Framebuffer window closed.");
            }
            // 陷入非致命：CPU 已跳转 mtvec，Trap 是本步最后一个事件；
            // si 在陷入处停止并报告
            if let Some(EmuEvent::Trap { cause, pc }) = self.machine.mem_ring().back() {
                println!("Trap: cause=0x{cause:08x} at 0x{pc:08x}");
                return Outcome::Continue;
            }
            // Paused（Ctrl+C）：保持原行为，静默继续剩余步数；
            // 其余结果统一报告并结束本批。
            match result {
                RunResult::Paused => {}
                rr => return self.report_result(rr),
            }
        }
        Outcome::Continue
    }

    fn cmd_examine(&mut self, count: u64, addr: &Expr) -> Outcome {
        let base = match self.machine.eval(addr) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("rremu: {e}");
                return Outcome::Continue;
            }
        };
        let mut idx = 0u64;
        while idx < count {
            let line_start = base.wrapping_add((idx as u32).wrapping_mul(4));
            print!("0x{line_start:08x}:");
            for j in 0..4u64 {
                let cur = idx + j;
                if cur >= count {
                    break;
                }
                let a = base.wrapping_add((cur as u32).wrapping_mul(4));
                match self.machine.peek(a) {
                    Ok(word) => print!("  0x{word:08x}"),
                    Err(_) => print!("  ????????"),
                }
            }
            println!();
            idx += 4;
        }
        Outcome::Continue
    }

    fn cmd_print(&mut self, expr: &Expr) -> Outcome {
        match self.machine.eval(expr) {
            Ok(v) => println!("= {v} (0x{v:x})"),
            Err(e) => eprintln!("rremu: {e}"),
        }
        Outcome::Continue
    }

    fn cmd_info_regs(&self) {
        let snap = self.machine.snapshot();
        println!("pc  = 0x{:08x}  priv = {}", snap.pc, snap.priv_);
        for i in 0..32u8 {
            let name = reg_name(i);
            let val = snap.gprs[usize::from(i)];
            print!("x{i:<2}({name:>4})=0x{val:08x}  ");
            if i % 4 == 3 {
                println!();
            }
        }
        let c = &snap.csrs;
        println!("mstatus   = 0x{:08x}", c.mstatus);
        println!("mtvec     = 0x{:08x}", c.mtvec);
        println!("mepc      = 0x{:08x}", c.mepc);
        println!("mcause    = 0x{:08x}", c.mcause);
        println!("mie       = 0x{:08x}", c.mie);
        println!("mip       = 0x{:08x}", c.mip);
        println!("mcycle    = {}", c.mcycle);
        println!("minstret  = {}", c.minstret);
    }

    fn cmd_breakpoint(&mut self, expr: &Expr) -> Outcome {
        let addr = match self.machine.eval(expr) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("rremu: {e}");
                return Outcome::Continue;
            }
        };
        if self.machine.add_breakpoint(addr) {
            println!("Breakpoint set at 0x{addr:08x}.");
        } else {
            println!("Breakpoint already exists at 0x{addr:08x}.");
        }
        Outcome::Continue
    }

    fn cmd_watchpoint(&mut self, expr: &Expr) -> Outcome {
        let id = self.machine.add_watchpoint(expr.clone());
        println!("Watchpoint {id} set.");
        Outcome::Continue
    }

    fn cmd_delete(&mut self, n: usize) -> Outcome {
        if self.machine.remove_watchpoint(n) {
            println!("Watchpoint {n} deleted.");
        } else if let Some(addr) = self.machine.breakpoints().iter().nth(n).copied() {
            self.machine.remove_breakpoint(addr);
            println!("Breakpoint at 0x{addr:08x} deleted.");
        } else {
            eprintln!("rremu: no watchpoint or breakpoint {n}");
        }
        Outcome::Continue
    }

    fn cmd_info_watchpoints(&self) {
        let bps = self.machine.breakpoints();
        let wps = self.machine.watchpoints();
        if bps.is_empty() && wps.is_empty() {
            println!("No breakpoints or watchpoints.");
            return;
        }
        for (i, addr) in bps.iter().enumerate() {
            println!("breakpoint {i}: 0x{addr:08x}");
        }
        for wp in wps {
            let desc = match &wp.expr {
                Expr::Deref(inner) => match **inner {
                    Expr::Num(v) => format!("*0x{v:08x}"),
                    ref e => format!("*{e:?}"),
                },
                Expr::Num(v) => format!("0x{v:08x}"),
                e => format!("{e:?}"),
            };
            println!("watchpoint {}: expr={} value=0x{:08x} hits={}", wp.id, desc, wp.last_value, wp.hit_count);
        }
    }

    /// 进入 UART 输入模式：切到 raw mode，逐键→字节→`uart_push_input`；
    /// Esc 退出。期间不步进机器（字节入 FIFO，用户后续 `c` 时由程序 drain）。
    /// 非 TTY（管道/批处理）拒绝执行。
    fn cmd_uart_input(&mut self) -> Outcome {
        use std::io::IsTerminal;

        if !std::io::stdin().is_terminal() {
            eprintln!("rremu: uart: stdin is not a terminal");
            return Outcome::Continue;
        }

        // banner 先于 raw mode：raw 下 \n 不回列首，写起来要 \r\n，麻烦且无必要。
        println!("[input mode — Esc 退出，按键发给 UART]");

        if let Err(e) = crossterm::terminal::enable_raw_mode() {
            eprintln!("rremu: failed to enable raw mode: {e}");
            return Outcome::Continue;
        }

        loop {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(k)) => {
                    if matches!(k.code, crossterm::event::KeyCode::Esc) {
                        break;
                    }
                    if let Some(b) = key_to_byte(k) {
                        self.machine.uart_push_input(b);
                    }
                }
                Ok(_) => {} // mouse/resize/paste 暂忽略；paste 处理 TODO
                Err(e) => {
                    eprintln!("\r\nrremu: event read error: {e}");
                    break;
                }
            }
        }

        let _ = crossterm::terminal::disable_raw_mode();
        println!();
        Outcome::Continue
    }

    fn flush_uart(&mut self) {
        if self.uart_direct {
            // 已实时写出；确保未换行的尾部字节也到达 stdout。
            let _ = io::stdout().flush();
            let _ = self.machine.drain_uart();
            return;
        }
        let bytes = self.machine.drain_uart();
        if !bytes.is_empty() {
            let _ = io::stdout().flush();
            let _ = io::stdout().write_all(&bytes);
            let _ = io::stdout().flush();
        }
    }

    /// 懒启动窗口：尚无窗口且工厂可用时，若 fb 已产生写入（dirty）则创建
    /// 窗口并推送当前帧。工厂返回 None（如无 X11）→ 丢弃工厂，之后不再尝试。
    fn ensure_window(&mut self) {
        if self.display.is_some() {
            return;
        }
        if self.machine.framebuffer_dirty_frame().is_none() {
            return;
        }
        let Some(factory) = self.window_factory.as_mut() else { return };
        match factory() {
            Some(mut window) => {
                // 推送触发创建的这一帧（全帧读取，不清 dirty）
                let open = self
                    .machine
                    .framebuffer_full_frame()
                    .is_none_or(|f| window.update(f.pixels));
                if open {
                    self.display = Some(window);
                } else {
                    self.window_factory = None;
                }
            }
            None => self.window_factory = None,
        }
    }

    /// 泵显示后端：dirty → 上屏；否则仅 poll。返回后端是否打开；
    /// 关窗时置 None（后续命令按无后端运行，工厂一并丢弃）。
    fn pump_display(&mut self) -> bool {
        let Some(display) = self.display.as_mut() else {
            return true;
        };
        let open = if let Some(frame) = self.machine.framebuffer_dirty_frame() {
            display.update(frame.pixels)
        } else {
            display.poll()
        };
        if !open {
            self.display = None;
            self.window_factory = None; // 用户已关窗：之后不再重开
        }
        open
    }

    /// 统一打印 RunResult 并映射为命令 Outcome。
    fn report_result(&self, rr: RunResult) -> Outcome {
        match rr {
            RunResult::Exited { code } => {
                println!("Program exited with code {code}.");
                Outcome::ProgramExited(code)
            }
            RunResult::Faulted { reason } => {
                eprintln!("Emulation faulted: {reason}.");
                Outcome::Faulted(reason)
            }
            RunResult::HitBreakpoint { addr } => {
                println!("Breakpoint hit at 0x{addr:08x}.");
                Outcome::Continue
            }
            RunResult::HitWatchpoint { id, pc, old, new } => {
                println!("Watchpoint {id} hit at pc 0x{pc:08x}: 0x{old:08x} -> 0x{new:08x}.");
                Outcome::Continue
            }
            RunResult::Paused => {
                println!("Paused.");
                Outcome::Continue
            }
        }
    }

    fn is_dead(&self) -> bool {
        matches!(self.machine.state(), RunState::Exited(_) | RunState::Faulted(_))
    }
}

enum Command {
    Help,
    Continue,
    Step(u64),
    Examine { count: u64, addr: Expr },
    Print(Expr),
    InfoRegs,
    Breakpoint(Expr),
    Watchpoint(Expr),
    Delete(usize),
    InfoWatchpoints,
    /// 进入 UART 输入模式：raw mode 下逐键→字节→UART RX，Esc 退出。
    Uart,
    /// 写出结构化机器状态 dump 到指定文件。
    Dump(String),
    Quit,
}

fn parse_command(line: &str) -> Result<Command, String> {
    let line = line.trim();
    let (cmd, rest) = match line.split_once(char::is_whitespace) {
        Some((c, r)) => (c, r.trim()),
        None => (line, ""),
    };
    match cmd {
        "help" | "h" | "?" => Ok(Command::Help),
        "c" | "continue" => Ok(Command::Continue),
        "si" | "stepi" => {
            if rest.is_empty() {
                Ok(Command::Step(1))
            } else {
                let n = rest
                    .parse::<u64>()
                    .map_err(|_| format!("invalid step count: {rest}"))?;
                Ok(Command::Step(n))
            }
        }
        "x" => {
            let (n_str, m_str) = rest
                .split_once(char::is_whitespace)
                .ok_or("usage: x N ADDR")?;
            let count = n_str
                .parse::<u64>()
                .map_err(|_| format!("invalid count: {n_str}"))?;
            let addr = parse_expr(m_str).map_err(|e| format!("parse error: {e}"))?;
            Ok(Command::Examine { count, addr })
        }
        "p" => {
            if rest.is_empty() {
                return Err("usage: p EXPR".into());
            }
            let expr = parse_expr(rest).map_err(|e| format!("parse error: {e}"))?;
            Ok(Command::Print(expr))
        }
        "info" | "i" => match rest {
            "r" | "reg" | "registers" => Ok(Command::InfoRegs),
            "w" | "wp" | "watchpoints" => Ok(Command::InfoWatchpoints),
            "" => Err("usage: info r|w".into()),
            _ => Err(format!("unknown info subcommand: {rest} (try 'info r' or 'info w')")),
        },
        "b" | "break" => {
            if rest.is_empty() {
                return Err("usage: b EXPR".into());
            }
            let expr = parse_expr(rest).map_err(|e| format!("parse error: {e}"))?;
            Ok(Command::Breakpoint(expr))
        }
        "w" | "watch" => {
            if rest.is_empty() {
                return Err("usage: w EXPR".into());
            }
            let expr = parse_expr(rest).map_err(|e| format!("parse error: {e}"))?;
            Ok(Command::Watchpoint(expr))
        }
        "d" | "delete" => {
            let n = rest
                .parse::<usize>()
                .map_err(|_| "usage: d N (watchpoint id)".to_string())?;
            Ok(Command::Delete(n))
        }
        "q" | "quit" | "exit" => Ok(Command::Quit),
        "uart" => Ok(Command::Uart),
        "dump" => {
            if rest.is_empty() {
                return Err("usage: dump FILE".into());
            }
            Ok(Command::Dump(rest.to_owned()))
        }
        _ => Err(format!("unknown command: {cmd} (type 'help')")),
    }
}

/// crossterm KeyEvent → UART 字节映射（CLI 与 TUI 各持一份，刻意不抽公共 crate）。
///
/// 规则：
/// - 可打印 ASCII 0x20..=0x7E（无 Alt）→ 该字节
/// - Ctrl+A..Z（无 Alt，大小写不敏感）→ 0x01..=0x1A
/// - Enter → 0x0A、Backspace → 0x7F、Tab → 0x09
/// - Esc → None（由调用方当退出信号）
/// - 其余（F1-F12、方向键、Alt 修饰、非 ASCII）→ None 忽略
fn key_to_byte(key: crossterm::event::KeyEvent) -> Option<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    if key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter => Some(b'\n'),
        KeyCode::Tab => Some(b'\t'),
        KeyCode::Backspace => Some(0x7F),
        KeyCode::Char(c) if ctrl => {
            let lc = c.to_ascii_lowercase();
            if lc.is_ascii_lowercase() {
                Some((lc as u8) - b'a' + 1)
            } else {
                None
            }
        }
        KeyCode::Char(c) => {
            if c.is_ascii() {
                let b = c as u8;
                if (0x20..=0x7E).contains(&b) {
                    return Some(b);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert!(matches!(parse_command("help").unwrap(), Command::Help));
        assert!(matches!(parse_command("c").unwrap(), Command::Continue));
        assert!(matches!(parse_command("si").unwrap(), Command::Step(1)));
        assert!(matches!(parse_command("si 5").unwrap(), Command::Step(5)));
        assert!(matches!(parse_command("q").unwrap(), Command::Quit));
        assert!(matches!(parse_command("info r").unwrap(), Command::InfoRegs));
    }

    #[test]
    fn parse_examine_and_print() {
        let (count, _addr) = match parse_command("x 4 0x80000000").unwrap() {
            Command::Examine { count, addr } => (count, addr),
            _ => panic!("expected Examine"),
        };
        assert_eq!(count, 4);
        assert!(matches!(parse_command("p pc").unwrap(), Command::Print(_)));
        assert!(matches!(parse_command("p x1 + x2").unwrap(), Command::Print(_)));
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(parse_command("frobnicate").is_err());
        assert!(parse_command("info zzz").is_err());
        assert!(parse_command("x").is_err());
        assert!(parse_command("p").is_err());
    }

    #[test]
    fn parse_uart_command() {
        assert!(matches!(parse_command("uart").unwrap(), Command::Uart));
    }

    #[test]
    fn parse_dump_command() {
        match parse_command("dump /tmp/out.json").unwrap() {
            Command::Dump(p) => assert_eq!(p, "/tmp/out.json"),
            _ => panic!("expected Dump"),
        }
        assert!(parse_command("dump").is_err());
    }

    #[test]
    fn key_to_byte_printable_ascii() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let nm = KeyModifiers::NONE;
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('a'), nm)), Some(b'a'));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)), Some(b'A'));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char(' '), nm)), Some(b' '));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('~'), nm)), Some(b'~'));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('/'), nm)), Some(b'/'));
    }

    #[test]
    fn key_to_byte_control_letters() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(0x03));
        assert_eq!(
            key_to_byte(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
            Some(0x03)
        );
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)), Some(0x01));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)), Some(0x1A));
        // Ctrl+1 等非字母不做映射
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL)), None);
    }

    #[test]
    fn key_to_byte_special_keys() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let nm = KeyModifiers::NONE;
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Enter, nm)), Some(b'\n'));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Tab, nm)), Some(b'\t'));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Backspace, nm)), Some(0x7F));
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Esc, nm)), None);
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::F(1), nm)), None);
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Left, nm)), None);
    }

    #[test]
    fn key_to_byte_alt_suppresses() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)), None);
        assert_eq!(
            key_to_byte(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT | KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn key_to_byte_non_ascii_dropped() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        // CJK 等 Unicode 字符暂不支持
        assert_eq!(key_to_byte(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE)), None);
    }
}
