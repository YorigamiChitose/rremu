//! 渲染：M4.1 占位布局；M4.2 替换为六面板。
//!
//! 当前只渲染顶栏（cycle/pc/状态）与提示，验证快照管线与无闪烁渲染。

use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use rremu_machine::{MachineSnapshot, RunState};

pub fn draw(f: &mut Frame, snap: &MachineSnapshot) {
    let area = f.area();
    let state_label = match snap.state {
        RunState::Idle => "idle",
        RunState::Running => "running",
        RunState::Paused => "paused",
        RunState::Exited(_) => "exited",
        RunState::Trapped(_) => "trapped",
    };

    let header = Line::styled(
        format!(
            " rremu  │  pc={:#010x}  │  cycle={}  │  state={}  │  q: quit",
            snap.pc, snap.cycle, state_label,
        ),
        Style::default().fg(Color::Cyan),
    );

    let body = Line::raw("");
    let hint = Line::raw(" [M4.1 skeleton — full panels in M4.2]");

    let para = Paragraph::new(vec![header, body, hint])
        .block(Block::default().borders(Borders::ALL).title(" rremu TUI "))
        .alignment(Alignment::Left);

    f.render_widget(para, area);
}
