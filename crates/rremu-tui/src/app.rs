//! TuiApp：TUI 主循环 + 终端管理 + 事件分发。

use std::io::{self, stdout, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use rremu_machine::{FramebufferFrame, Machine, RunResult};

use crate::render;
use crate::speed::SpeedTier;

type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

/// 统一帧预算：约 30fps。
const FRAME_BUDGET: Duration = Duration::from_millis(33);

pub struct TuiApp {
    machine: Machine,
    running: bool,
    exited: bool,
    speed: SpeedTier,
    input_mode: bool,
    fb_view: bool,
    should_quit: bool,
    exit_code: i32,
    /// 模拟故障原因（不可恢复错误时设置）。
    fault_reason: Option<String>,
    /// 帧缓冲缓存：仅当 dirty 时从设备拷入，render 读缓存。
    fb_cache: Vec<u32>,
    fb_dims: (u32, u32),
}

impl TuiApp {
    pub fn new(machine: Machine) -> Self {
        Self {
            machine,
            running: false,
            exited: false,
            speed: SpeedTier::Steps(1),
            input_mode: false,
            fb_view: false,
            should_quit: false,
            exit_code: 0,
            fault_reason: None,
            fb_cache: Vec::new(),
            fb_dims: (0, 0),
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

            // 非运行时：poll 到帧预算（30fps 节奏，按键即时响应）；
            // 运行时：poll(ZERO) 取走已排队按键，首个按键后即回到执行。
            let poll_timeout =
                if self.running { Duration::ZERO } else { FRAME_BUDGET.saturating_sub(frame_start.elapsed()) };
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
                self.tick_machine(frame_start);
                // 帧节奏：Steps 档每帧恰好一批，提前完成则等满预算；
                // Full 档在预算内循环，通常已用满。
                let elapsed = frame_start.elapsed();
                if elapsed < FRAME_BUDGET {
                    std::thread::sleep(FRAME_BUDGET - elapsed);
                }
            }

            self.sync_fb_cache();
            let snap = self.machine.snapshot();
            let disasm = self.collect_disasm(snap.pc);
            let fb = if self.fb_view {
                Some(FramebufferFrame {
                    width: self.fb_dims.0,
                    height: self.fb_dims.1,
                    pixels: &self.fb_cache,
                })
            } else {
                None
            };
            let data = render::RenderData {
                snap: &snap,
                disasm: &disasm,
                fb: fb.as_ref(),
                speed: self.speed,
                running: self.running,
                input_mode: self.input_mode,
                fb_view: self.fb_view,
            };
            terminal.draw(|f| render::draw(f, &data))?;
        }
        Ok(())
    }

    /// 推进机器：Steps 档每帧恰好一批；Full 档在帧预算内连续批量执行。
    fn tick_machine(&mut self, frame_start: Instant) {
        match self.speed {
            SpeedTier::Steps(n) => {
                let rr = self.machine.step_n(n);
                self.apply_run_result(rr);
            }
            SpeedTier::Full => {
                while self.running && frame_start.elapsed() < FRAME_BUDGET {
                    let rr = self.machine.step_n(SpeedTier::Full.batch());
                    self.apply_run_result(rr);
                }
            }
        }
    }

    /// 统一处理执行结果（tick_machine 与 single_step 共用）。
    fn apply_run_result(&mut self, rr: RunResult) {
        match rr {
            RunResult::Exited { code } => {
                self.running = false;
                self.exited = true;
                self.exit_code = code;
                // 不设 should_quit：程序结束后 TUI 保持打开，让用户查看
                // 最终寄存器/内存状态，按 q 退出。
            }
            RunResult::Faulted { reason } => {
                self.running = false;
                self.fault_reason = Some(reason);
            }
            RunResult::HitBreakpoint { .. } | RunResult::HitWatchpoint { .. } => {
                self.running = false;
            }
            RunResult::Paused => {}
        }
    }

    /// 同步帧缓冲缓存：首次开启/尺寸变化时全量拉取，否则仅在 dirty 时拷贝。
    fn sync_fb_cache(&mut self) {
        if !self.fb_view {
            return;
        }
        let Some((width, height)) = self.machine.framebuffer_dimensions() else {
            return;
        };
        if self.fb_dims != (width, height) {
            // 全量拉取（不清 dirty），保证开启即有画面。
            if let Some(frame) = self.machine.framebuffer_full_frame() {
                self.fb_dims = (frame.width, frame.height);
                self.fb_cache = frame.pixels.to_vec();
            }
            return;
        }
        if let Some(frame) = self.machine.framebuffer_dirty_frame() {
            self.fb_cache.copy_from_slice(frame.pixels);
        }
    }

    /// 反汇编：pc 前 4 后 12（最坏 4 字节/条 → 前扫 16 字节）。
    fn collect_disasm(&mut self, pc: u32) -> Vec<rremu_machine::DisasmLine> {
        let start = pc.saturating_sub(16);
        let end = pc.saturating_add(48);
        self.machine.disasm_range(start, end, 17, pc)
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits (safety valve in raw mode) — 优先于输入模式。
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.input_mode {
            self.handle_input_mode_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char(' ') if !self.exited && self.fault_reason.is_none() => {
                // Space: toggle run/pause (程序已退出或故障则忽略)
                self.running = !self.running;
            }
            KeyCode::Char('s') if !self.exited && self.fault_reason.is_none() => {
                // Single step: pause + one instruction
                self.running = false;
                self.single_step();
            }
            KeyCode::Char('f') if !self.exited && self.fault_reason.is_none() => {
                // Direct to full speed
                self.speed = SpeedTier::Full;
                self.running = true;
            }
            KeyCode::Char('i') if !self.exited && self.fault_reason.is_none() => {
                // Enter UART input mode
                self.input_mode = true;
            }
            KeyCode::Char('r') => {
                // Reset machine → 清除退出状态，可重新运行
                if self.machine.reset().is_ok() {
                    self.exited = false;
                    self.running = false;
                    self.exit_code = 0;
                    self.fault_reason = None;
                    self.input_mode = false;
                    // 缓存失效：下一帧按重置后的设备全量重填
                    self.fb_dims = (0, 0);
                    self.fb_cache.clear();
                }
            }
            KeyCode::Char('b') => {
                // Toggle framebuffer view
                self.fb_view = !self.fb_view;
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
        self.apply_run_result(rr);
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
