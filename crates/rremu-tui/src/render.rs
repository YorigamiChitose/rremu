//! 渲染：六面板布局（§5.10）。
//!
//! 面板：顶栏 / 左栏寄存器 / 右上反汇编 / 右下存储访问 / 底部串口 / 状态栏。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use rremu_machine::{
    reg_name, AccessSize, DisasmLine, EmuEvent, FramebufferSnapshot, MachineSnapshot, MemKind,
    RunState,
};

use crate::speed::SpeedTier;

/// 渲染所需的全部数据（避免 draw() 参数过多）。
pub struct RenderData<'a> {
    pub snap: &'a MachineSnapshot,
    pub disasm: &'a [DisasmLine],
    pub fb: Option<&'a FramebufferSnapshot>,
    pub speed: SpeedTier,
    pub running: bool,
    pub input_mode: bool,
    pub fb_view: bool,
}

pub fn draw(f: &mut Frame, data: &RenderData) {
    let area = f.area();

    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // top bar
            Constraint::Min(0),     // middle
            Constraint::Length(6), // uart
            Constraint::Length(1),  // status bar
        ])
        .split(area);

    draw_top_bar(f, main[0], data.snap, data.speed, data.running);
    draw_middle(f, main[1], data.snap, data.disasm, data.fb, data.fb_view);
    draw_uart(f, main[2], data.snap, data.input_mode);
    draw_status_bar(f, main[3], data.input_mode);
}

fn draw_top_bar(f: &mut Frame, area: Rect, snap: &MachineSnapshot, speed: SpeedTier, running: bool) {
    let state_str = if running {
        "running"
    } else {
        match snap.state {
            RunState::Idle => "idle",
            RunState::Running => "running",
            RunState::Paused => "paused",
            RunState::Exited(c) => return draw_bar(f, area, vec![
                span(format!(" rremu │ cycle={} │ speed={} │ EXITED({})", snap.cycle, speed.label(), c), Color::Yellow),
            ]),
            RunState::Trapped(c) => return draw_bar(f, area, vec![
                span(format!(" rremu │ cycle={} │ speed={} │ TRAPPED(cause={})", snap.cycle, speed.label(), c), Color::Red),
            ]),
        }
    };
    draw_bar(f, area, vec![
        span(format!(" rremu │ cycle={} │ speed={} │ {} │ pc={:#010x}", snap.cycle, speed.label(), state_str, snap.pc), Color::Cyan),
    ]);
}

fn draw_middle(
    f: &mut Frame,
    area: Rect,
    snap: &MachineSnapshot,
    disasm: &[DisasmLine],
    fb: Option<&FramebufferSnapshot>,
    fb_view: bool,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(38), Constraint::Min(0)])
        .split(area);

    draw_registers(f, cols[0], snap);
    draw_right_panels(f, cols[1], snap, disasm, fb, fb_view);
}

fn draw_registers(f: &mut Frame, area: Rect, snap: &MachineSnapshot) {
    let mut lines = Vec::new();
    for i in 0..32u8 {
        let name = reg_name(i);
        let val = snap.gprs[i as usize];
        lines.push(Line::from(vec![
            Span::styled(format!("x{i:<2}"), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(format!("{name:>4}"), Style::default().fg(Color::Yellow)),
            Span::raw(" = "),
            Span::styled(format!("{val:#010x}"), Style::default().fg(Color::Green)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" pc ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{:#010x}", snap.pc), Style::default().fg(Color::Green)),
    ]));
    let priv_label = match snap.priv_ {
        rremu_machine::Priv::Machine => "M",
        rremu_machine::Priv::User => "U",
    };
    lines.push(Line::from(vec![
        Span::styled("priv", Style::default().fg(Color::DarkGray)),
        Span::raw(" = "),
        Span::styled(priv_label, Style::default().fg(Color::Yellow)),
    ]));
    lines.push(Line::raw(""));

    let csr_lines = [
        ("mstatus", snap.csrs.mstatus),
        ("mtvec", snap.csrs.mtvec),
        ("mepc", snap.csrs.mepc),
        ("mcause", snap.csrs.mcause),
        ("mie", snap.csrs.mie),
        ("mip", snap.csrs.mip),
    ];
    for (name, val) in csr_lines {
        lines.push(Line::from(vec![
            Span::styled(format!("{name:>8}", ), Style::default().fg(Color::DarkGray)),
            Span::raw(" = "),
            Span::styled(format!("{val:#010x}"), Style::default().fg(Color::Cyan)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("  mcycle", Style::default().fg(Color::DarkGray)),
        Span::raw(" = "),
        Span::styled(snap.csrs.mcycle.to_string(), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("minstret", Style::default().fg(Color::DarkGray)),
        Span::raw(" = "),
        Span::styled(snap.csrs.minstret.to_string(), Style::default().fg(Color::Cyan)),
    ]));

    let para = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Registers "));
    f.render_widget(para, area);
}

fn draw_right_panels(
    f: &mut Frame,
    area: Rect,
    snap: &MachineSnapshot,
    disasm: &[DisasmLine],
    fb: Option<&FramebufferSnapshot>,
    fb_view: bool,
) {
    let halves = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_disasm(f, halves[0], disasm);
    match (fb_view, fb) {
        (true, Some(fb_data)) => draw_framebuffer(f, halves[1], fb_data),
        _ => draw_mem_ring(f, halves[1], snap),
    }
}

fn draw_disasm(f: &mut Frame, area: Rect, disasm: &[DisasmLine]) {
    let items: Vec<ListItem> = disasm
        .iter()
        .map(|line| {
            let prefix = if line.is_current { "▶ " } else { "  " };
            let text = format!("{}{:#010x}  {}", prefix, line.addr, line.text);
            let style = if line.is_current {
                Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::styled(text, style))
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Disassembly "));
    f.render_widget(list, area);
}

fn draw_mem_ring(f: &mut Frame, area: Rect, snap: &MachineSnapshot) {
    let items: Vec<ListItem> = snap
        .mem_ring
        .iter()
        .rev()
        .map(|ev| match ev {
            EmuEvent::MemAccess { cycle, kind, size, addr, value, device } => {
                let kind_str = match kind {
                    MemKind::Inst => "IF",
                    MemKind::DataRead => "DR",
                    MemKind::DataWrite => "DW",
                };
                let size_str = match size {
                    AccessSize::Byte => "B",
                    AccessSize::Half => "H",
                    AccessSize::Word => "W",
                };
                // value 按实际宽度显示（{:#x} 无前导 0），再 {:>10} 右对齐到 10 列
                let val_str = format!("{:#x}", value);
                let val_padded = format!("{val_str:>10}");
                ListItem::new(Line::from(vec![
                    Span::styled(format!("#{cycle:<6}"), Style::default().fg(Color::DarkGray)),
                    Span::raw(" "),
                    Span::styled(format!("{kind_str} {size_str}"), Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled(format!("{addr:#010x}"), Style::default().fg(Color::Cyan)),
                    Span::raw(" "),
                    Span::styled(val_padded, Style::default().fg(Color::Green)),
                    Span::raw(" "),
                    Span::styled(device.as_str(), Style::default().fg(Color::Magenta)),
                ]))
            }
            _ => ListItem::new(Line::raw("")),
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Memory Access "));
    f.render_widget(list, area);
}

fn draw_uart(f: &mut Frame, area: Rect, snap: &MachineSnapshot, input_mode: bool) {
    let text = String::from_utf8_lossy(&snap.uart_out).into_owned();
    let title = if input_mode { " UART [INPUT MODE - Esc to exit] " } else { " UART " };
    let para = Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(para, area);
}

fn draw_status_bar(f: &mut Frame, area: Rect, input_mode: bool) {
    let hint = if input_mode {
        " Esc: exit input mode | type to send to UART "
    } else {
        " Space:run/pause  s:step  ←→:speed  f:full  i:input  r:reset  b:fb  q:quit "
    };
    draw_bar(f, area, vec![Span::styled(hint, Style::default().fg(Color::Black).bg(Color::White))]);
}

/// 帧缓冲内嵌渲染：下采样到面板尺寸，每格背景色 = 对应像素。
fn draw_framebuffer(f: &mut Frame, area: Rect, fb: &FramebufferSnapshot) {
    let block = Block::default().borders(Borders::ALL).title(" Framebuffer ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if fb.width == 0 || fb.height == 0 || fb.pixels.is_empty() || inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(inner.height as usize);
    for y in 0..inner.height {
        let mut spans: Vec<Span> = Vec::with_capacity(inner.width as usize);
        for x in 0..inner.width {
            let px = (u32::from(x) * fb.width) / u32::from(inner.width);
            let py = (u32::from(y) * fb.height) / u32::from(inner.height);
            let idx = (py * fb.width + px) as usize;
            let pixel = fb.pixels.get(idx).copied().unwrap_or(0);
            let r = ((pixel >> 16) & 0xff) as u8;
            let g = ((pixel >> 8) & 0xff) as u8;
            let b = (pixel & 0xff) as u8;
            spans.push(Span::styled(" ", Style::default().bg(Color::Rgb(r, g, b))));
        }
        lines.push(Line::from(spans));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

/// 渲染单行状态栏。
fn draw_bar(f: &mut Frame, area: Rect, spans: Vec<Span>) {
    let line = Line::from(spans);
    let para = Paragraph::new(line).alignment(Alignment::Left);
    f.render_widget(para, area);
}

fn span(text: String, color: Color) -> Span<'static> {
    Span::styled(text, Style::default().fg(color))
}
