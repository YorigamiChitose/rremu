//! rremu-tui：ratatui 多面板 TUI 前端。
//!
//! 依赖红线：仅通过 `rremu_machine` 公开 API 访问状态，不直接接触总线/寄存器。
//! 线程模型：单线程协作式 —— 事件轮询 + 机器批量 tick + 30fps 渲染。

mod app;
mod render;
mod speed;

pub use speed::SpeedTier;

use rremu_machine::Machine;

/// 启动 TUI 主循环；返回退出码。
pub fn run(machine: Machine) -> i32 {
    match app::TuiApp::new(machine).run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rremu-tui: {e}");
            1
        }
    }
}
