//! TuiApp：TUI 主循环 + 终端管理 + 事件分发。

use std::io::{self, stdout, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use rremu_machine::{Machine, RunResult};

use crate::render;
use crate::speed::SpeedTier;

type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TuiApp {
    machine: Machine,
    running: bool,
    speed: SpeedTier,
    input_mode: bool,
    should_quit: bool,
    exit_code: i32,
}

impl TuiApp {
    pub fn new(machine: Machine) -> Self {
        Self {
            machine,
            running: false,
            speed: SpeedTier::Steps(1),
            input_mode: false,
            should_quit: false,
            exit_code: 0,
        }
    }

    /// 启动终端 → 主循环 → 清理；返回退出码。
    pub fn run(mut self) -> io::Result<i32> {
        let mut terminal = setup_terminal()?;
        let result = self.run_loop(&mut terminal);
        let _ = teardown_terminal(&mut terminal);
        result?;
        Ok(self.exit_code)
    }

    fn run_loop(&mut self, terminal: &mut TuiTerminal) -> io::Result<()> {
        while !self.should_quit {
            let frame_start = Instant::now();

            let poll_timeout = if self.running { Duration::ZERO } else { Duration::from_millis(33) };
            while event::poll(poll_timeout)? {
                if let Event::Key(k) = event::read()? {
                    self.handle_key(k);
                }
                if self.running {
                    break;
                }
            }

            if self.should_quit {
                break;
            }

            if self.running {
                self.tick_machine();
            }

            let snap = self.machine.snapshot();
            let disasm = self.collect_disasm(snap.pc);
            terminal.draw(|f| render::draw(f, &snap, &disasm, self.speed, self.running, self.input_mode))?;

            if !self.running {
                let elapsed = frame_start.elapsed();
                if elapsed < Duration::from_millis(33) {
                    std::thread::sleep(Duration::from_millis(33) - elapsed);
                }
            }
        }
        Ok(())
    }

    fn tick_machine(&mut self) {
        let batch = self.speed.batch();
        let rr = self.machine.step_n(batch);
        match rr {
            RunResult::Exited { code } => {
                self.running = false;
                self.exit_code = code;
                self.should_quit = true;
            }
            RunResult::HitBreakpoint { .. } | RunResult::HitWatchpoint { .. } => {
                self.running = false;
            }
            RunResult::Paused | RunResult::Trapped { .. } => {}
        }
    }

    /// 反汇编：pc 前 4 后 12（最坏 4 字节/条 → 前扫 16 字节）。
    fn collect_disasm(&mut self, pc: u32) -> Vec<rremu_machine::DisasmLine> {
        let start = pc.saturating_sub(16);
        let end = pc.saturating_add(48);
        self.machine.disasm_range(start, end, 17, pc)
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.input_mode {
            self.handle_input_mode_key(key);
            return;
        }
        // Ctrl+C always quits (safety valve in raw mode)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char(' ') => {
                // Space: toggle run/pause
                self.running = !self.running;
            }
            KeyCode::Char('s') => {
                // Single step: pause + one instruction
                self.running = false;
                self.single_step();
            }
            KeyCode::Char('f') => {
                // Direct to full speed
                self.speed = SpeedTier::Full;
                self.running = true;
            }
            KeyCode::Char('i') => {
                // Enter UART input mode
                self.input_mode = true;
            }
            KeyCode::Char('r') => {
                // Reset machine
                let _ = self.machine.reset();
                self.running = false;
            }
            KeyCode::Left => {
                // Speed down
                self.speed = self.speed.prev();
            }
            KeyCode::Right => {
                // Speed up
                self.speed = self.speed.next();
            }
            _ => {}
        }
    }

    /// Execute a single instruction (for `s` key).
    fn single_step(&mut self) {
        let rr = self.machine.step_n(1);
        match rr {
            RunResult::Exited { code } => {
                self.exit_code = code;
                self.should_quit = true;
            }
            RunResult::HitBreakpoint { .. } | RunResult::HitWatchpoint { .. } => {}
            RunResult::Paused | RunResult::Trapped { .. } => {}
        }
    }

    fn handle_input_mode_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.input_mode = false,
            KeyCode::Enter => self.machine.uart_push_input(b'\n'),
            KeyCode::Char(c) => self.machine.uart_push_input(c as u8),
            _ => {}
        }
    }
}

fn setup_terminal() -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn teardown_terminal(terminal: &mut TuiTerminal) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
