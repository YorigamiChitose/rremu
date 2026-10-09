//! rremu-cli：GDB 风格 REPL 调试器前端（rustyline）。只通过 machine 公开 API 交互。
//!
//! M1 命令集（PLAN §6）：`help` / `c` / `si [n]` / `x n m` / `p expr` / `info r` / `q`。
//! 断点/监视点（`b` / `w` / `info w` / `d`）在 M2 补充。

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rremu_expr::{parse as parse_expr, Expr};
use rremu_machine::{reg_name, Machine, RunResult, RunState};
use rremu_window::Window;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

/// REPL 调试器：持有 machine 实例与全局 Ctrl+C 暂停标志。
pub struct Repl {
    machine: Machine,
    pause_flag: Arc<AtomicBool>,
    /// 可选独立显示窗（CLI 模式 + fb 配置时由 bin 创建并注入）；
    /// None 表示无窗（TUI 路径、或用户已关窗、或 fb 未配置）。
    window: Option<Window>,
}

/// 单条命令的执行结果：决定 REPL 循环与进程退出码。
enum Outcome {
    Continue,
    Quit,
    ProgramExited(i32),
}

/// 有窗时 cmd_continue 的步进批量：在每批之后泵窗（dirty 上屏 / 否则 poll 关窗）。
const WINDOW_BATCH: u64 = 1024;

impl Repl {
    pub fn new(machine: Machine, pause_flag: Arc<AtomicBool>, window: Option<Window>) -> Self {
        Self { machine, pause_flag, window }
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
            }
        }
    }

    /// 批处理模式（`-c` 每条命令）。
    pub fn run_batch(&mut self, cmds: &[String]) -> i32 {
        for cmd in cmds {
            let cmd = cmd.trim();
            println!("rremu> {cmd}");
            match self.dispatch(cmd) {
                Outcome::Continue => {}
                Outcome::Quit => return 0,
                Outcome::ProgramExited(code) => return code,
            }
        }
        0
    }

    fn dispatch(&mut self, line: &str) -> Outcome {
        match parse_command(line) {
            Ok(cmd) => self.execute(cmd),
            Err(msg) => {
                eprintln!("rremu: {msg}");
                Outcome::Continue
            }
        }
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
        println!("  q             Quit");
    }

    fn cmd_continue(&mut self) -> Outcome {
        if self.is_dead() {
            eprintln!("rremu: program has already exited/trapped");
            return Outcome::Continue;
        }
        self.pause_flag.store(false, Ordering::Relaxed);

        // 有窗时自行跑 step_n 循环，每批泵窗（dirty 上屏 / 否则 poll 关窗）；
        // 无窗时走 machine.run 全速循环（仅靠 pause_flag 停机）。
        let mut window_died = false;
        let result = match &mut self.window {
            Some(win) => {
                // 初始 poll：让窗口出现并处理一次 OS 事件
                if !win.poll() {
                    window_died = true;
                    RunResult::Paused
                } else {
                    loop {
                        if self.pause_flag.load(Ordering::Relaxed) {
                            break RunResult::Paused;
                        }
                        let step_rr = self.machine.step_n(WINDOW_BATCH);
                        // 泵窗：dirty → 上屏（拷贝 frame.pixels）；否则仅 poll 检测关窗
                        let open = if let Some(frame) = self.machine.framebuffer_dirty_frame() {
                            win.update(frame.pixels)
                        } else {
                            win.poll()
                        };
                        if !open {
                            window_died = true;
                            break RunResult::Paused;
                        }
                        match step_rr {
                            RunResult::Exited { code } => break RunResult::Exited { code },
                            RunResult::HitBreakpoint { addr } => break RunResult::HitBreakpoint { addr },
                            RunResult::HitWatchpoint { id, pc, old, new } => {
                                break RunResult::HitWatchpoint { id, pc, old, new };
                            }
                            RunResult::Paused | RunResult::Trapped { .. } => {}
                        }
                    }
                }
            }
            None => self.machine.run(&self.pause_flag),
        };

        if window_died {
            // 窗已关：丢弃以免后续 c 试图再次使用已关闭的窗
            self.window = None;
            println!("Framebuffer window closed; paused. Subsequent 'c' runs without window.");
        }

        self.flush_uart();
        match result {
            RunResult::Exited { code } => {
                println!("Program exited with code {code}.");
                Outcome::ProgramExited(code)
            }
            RunResult::Trapped { cause } => {
                println!("Trap: cause=0x{cause:08x}");
                Outcome::Continue
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
            // 有窗时单步也泵窗：让用户能逐指令看 fb 变化
            if let Some(win) = self.window.as_mut() {
                let open = if let Some(frame) = self.machine.framebuffer_dirty_frame() {
                    win.update(frame.pixels)
                } else {
                    win.poll()
                };
                if !open {
                    self.window = None;
                    println!("Framebuffer window closed.");
                }
            }
            match result {
                RunResult::Exited { code } => {
                    println!("Program exited with code {code}.");
                    return Outcome::ProgramExited(code);
                }
                RunResult::Trapped { cause } => {
                    println!("Trap: cause=0x{cause:08x}");
                    return Outcome::Continue;
                }
                RunResult::HitBreakpoint { addr } => {
                    println!("Breakpoint hit at 0x{addr:08x}.");
                    return Outcome::Continue;
                }
                RunResult::HitWatchpoint { id, pc, old, new } => {
                    println!("Watchpoint {id} hit at pc 0x{pc:08x}: 0x{old:08x} -> 0x{new:08x}.");
                    return Outcome::Continue;
                }
                RunResult::Paused => {}
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

    fn flush_uart(&mut self) {
        let bytes = self.machine.drain_uart();
        if !bytes.is_empty() {
            let _ = io::stdout().flush();
            let _ = io::stdout().write_all(&bytes);
            let _ = io::stdout().flush();
        }
    }

    fn is_dead(&self) -> bool {
        matches!(self.machine.snapshot().state, RunState::Exited(_) | RunState::Trapped(_))
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
        _ => Err(format!("unknown command: {cmd} (type 'help')")),
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
}
