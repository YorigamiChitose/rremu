//! TuiApp：TUI 主循环 + 终端管理 + 事件分发。

use std::io::{self, stdout, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use rremu_machine::{Machine, RunResult};

use crate::render;

type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TuiApp {
    machine: Machine,
    running: bool,
    should_quit: bool,
    exit_code: i32,
}

impl TuiApp {
    pub fn new(machine: Machine) -> Self {
        Self { machine, running: false, should_quit: false, exit_code: 0 }
    }

    /// 启动终端 → 主循环 → 清理；返回退出码。
    pub fn run(mut self) -> io::Result<i32> {
        let mut terminal = setup_terminal()?;
        let result = self.run_loop(&mut terminal);
        // 无论主循环是否出错，必须恢复终端
        let _ = teardown_terminal(&mut terminal);
        result?;
        Ok(self.exit_code)
    }

    fn run_loop(&mut self, terminal: &mut TuiTerminal) -> io::Result<()> {
        while !self.should_quit {
            let frame_start = Instant::now();

            // 运行态非阻塞轮询（零超时）；暂停态短超时（33ms ≈ 30fps）
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
            terminal.draw(|f| render::draw(f, &snap))?;

            // 暂停时节流到 ~30fps；运行态全速（由 poll 超时与 batch 自然限流）
            if !self.running {
                let elapsed = frame_start.elapsed();
                if elapsed < Duration::from_millis(33) {
                    std::thread::sleep(Duration::from_millis(33) - elapsed);
                }
            }
        }
        Ok(())
    }

    /// 批量 tick 机器（M4.3 接入速度档位；M4.1 仅预留 1 步/帧）。
    fn tick_machine(&mut self) {
        let rr = self.machine.step_n(1);
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

    fn handle_key(&mut self, key: KeyEvent) {
        if let KeyCode::Char('q') = key.code {
            self.should_quit = true;
        }
    }
}

/// 进入 raw 模式 + 备用屏幕，创建 ratatui 终端。
fn setup_terminal() -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

/// 恢复终端：退出备用屏幕 + 关闭 raw 模式。
fn teardown_terminal(terminal: &mut TuiTerminal) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
