//! 渲染：六面板布局。
//!
//! 面板：顶栏 / 左栏寄存器 / 右上反汇编 / 右下存储访问 / 底部串口 / 状态栏。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use rremu_machine::term::{Cell as TermCell, Color as TermColor, Style as TermStyle, Terminal};
use rremu_machine::{
    reg_name, AccessSize, DisasmLine, EmuEvent, FramebufferFrame, MachineSnapshot, MemKind,
    RunState,
};

use crate::speed::SpeedTier;

/// 渲染所需的全部数据（避免 draw() 参数过多）。
pub struct RenderData<'a> {
    pub snap: &'a MachineSnapshot,
    pub disasm: &'a [DisasmLine],
    pub fb: Option<&'a FramebufferFrame<'a>>,
    pub speed: SpeedTier,
    pub running: bool,
    pub input_mode: bool,
    pub fb_view: bool,
    /// UART 终端（有状态屏幕）。
    pub terminal: &'a Terminal,
    /// 面板信息档位（0=全开；1=关指令；2=关指令+读写；3=全关）。
    pub panel_level: u8,
}

pub fn draw(f: &mut Frame, data: &RenderData) {
    let area = f.area();

    // min 档（全部面板关闭）：UART 顺便全屏
    if data.panel_level >= 3 {
        let main = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // top bar
                Constraint::Min(0),    // uart（全屏）
                Constraint::Length(1), // status bar
            ])
            .split(area);
        draw_top_bar(f, main[0], data.snap, data.speed, data.running);
        draw_uart(f, main[1], data.terminal, data.input_mode);
        draw_status_bar(f, main[2], data.input_mode, data.panel_level);
        return;
    }

    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top bar
            Constraint::Min(0),    // middle
            Constraint::Length(6), // uart
            Constraint::Length(1), // status bar
        ])
        .split(area);

    draw_top_bar(f, main[0], data.snap, data.speed, data.running);
    let panels = Panels::from_level(data.panel_level);
    draw_middle(f, main[1], data.snap, data.disasm, data.fb, data.fb_view, panels);
    draw_uart(f, main[2], data.terminal, data.input_mode);
    draw_status_bar(f, main[3], data.input_mode, data.panel_level);
}

/// 各面板显示/刷新开关。
#[derive(Clone, Copy)]
struct Panels {
    disasm: bool,
    events: bool,
    registers: bool,
}

impl Panels {
    /// 档位 → 开关：0 全开，1 关指令，2 再关读写，3 全关。
    fn from_level(level: u8) -> Self {
        Self { disasm: level < 1, events: level < 2, registers: level < 3 }
    }
}

fn draw_top_bar(f: &mut Frame, area: Rect, snap: &MachineSnapshot, speed: SpeedTier, running: bool) {
    let state_str = if running {
        "running"
    } else {
        match &snap.state {
            RunState::Idle => "idle",
            RunState::Running => "running",
            RunState::Paused => "paused",
            RunState::Exited(c) => return draw_bar(f, area, vec![
                span(format!(" rremu │ cycle={} │ speed={} │ EXITED({})", snap.cycle, speed.label(), c), Color::Yellow),
            ]),
            RunState::Faulted(reason) => return draw_bar(f, area, vec![
                span(format!(" rremu │ cycle={} │ speed={} │ FAULTED: {}", snap.cycle, speed.label(), reason), Color::Red),
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
    fb: Option<&FramebufferFrame<'_>>,
    fb_view: bool,
    panels: Panels,
) {
    if panels.registers {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(38), Constraint::Min(0)])
            .split(area);
        draw_registers(f, cols[0], snap);
        draw_right_panels(f, cols[1], snap, disasm, fb, fb_view, panels);
    } else {
        draw_right_panels(f, area, snap, disasm, fb, fb_view, panels);
    }
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
        rremu_machine::Priv::Supervisor => "S",
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
    fb: Option<&FramebufferFrame<'_>>,
    fb_view: bool,
    panels: Panels,
) {
    // 底部面板：fb 视图优先；否则按 events 开关显示内存事件
    let bottom_on = if fb_view { fb.is_some() } else { panels.events };

    match (panels.disasm, bottom_on) {
        (true, true) => {
            let halves = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            draw_disasm(f, halves[0], disasm);
            if fb_view {
                if let Some(fb_data) = fb {
                    draw_framebuffer(f, halves[1], fb_data);
                }
            } else {
                draw_mem_ring(f, halves[1], snap);
            }
        }
        (true, false) => draw_disasm(f, area, disasm),
        (false, true) => {
            if fb_view {
                if let Some(fb_data) = fb {
                    draw_framebuffer(f, area, fb_data);
                }
            } else {
                draw_mem_ring(f, area, snap);
            }
        }
        (false, false) => {}
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
                // value 位宽与访问类型严格同步、保留前导 0：
                // B=2 位 hex（0xXX，宽4）/ H=4 位（0xXXXX，宽6）/ W=8 位（0xXXXXXXXX，宽10），
                // 再 {:>10} 用前导空格补齐，让所有行数值右边缘对齐到 W 宽度。
                let val_natural = match size {
                    AccessSize::Byte => format!("{:#04x}", value),
                    AccessSize::Half => format!("{:#06x}", value),
                    AccessSize::Word => format!("{:#010x}", value),
                };
                let val_padded = format!("{val_natural:>10}");
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

fn draw_uart(f: &mut Frame, area: Rect, term: &Terminal, input_mode: bool) {
    let title = if input_mode { " UART [INPUT MODE - Esc to exit] " } else { " UART " };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);

    // 只渲染末尾可见行（scrollback 可能很长）
    let lines = term.lines();
    let start = lines.len().saturating_sub(inner.height as usize);
    let (crow, ccol) = term.cursor();
    let rat_lines: Vec<Line> = lines[start..]
        .iter()
        .enumerate()
        .map(|(i, l)| cell_line(l, if start + i == crow { Some(ccol) } else { None }))
        .collect();

    let para = Paragraph::new(rat_lines).block(block);
    f.render_widget(para, area);
}

/// 一行单元格 → ratatui `Line`：跳过宽字符次格、合并同一样式段；
/// `cursor` 指定的列以反显标出。
fn cell_line(cells: &[TermCell], cursor: Option<usize>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut prev: Option<TermStyle> = None;
    for (idx, cell) in cells.iter().enumerate() {
        if cell.wide_cont {
            continue;
        }
        let mut st = cell.style;
        if cursor == Some(idx) {
            st.reverse = !st.reverse;
        }
        if prev == Some(st) {
            run.push(cell.ch);
        } else {
            if let Some(p) = prev.replace(st) {
                spans.push(Span::styled(core::mem::take(&mut run), to_style(p)));
            }
            run.push(cell.ch);
        }
    }
    if let Some(p) = prev {
        spans.push(Span::styled(run, to_style(p)));
    }
    Line::from(spans)
}

fn to_color(c: TermColor) -> Color {
    match c {
        TermColor::Default => Color::Reset,
        TermColor::Indexed(n) => Color::Indexed(n),
        TermColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn to_style(s: TermStyle) -> Style {
    let mut m = Modifier::empty();
    if s.bold {
        m |= Modifier::BOLD;
    }
    if s.dim {
        m |= Modifier::DIM;
    }
    if s.italic {
        m |= Modifier::ITALIC;
    }
    if s.underline {
        m |= Modifier::UNDERLINED;
    }
    if s.reverse {
        m |= Modifier::REVERSED;
    }
    Style::default().fg(to_color(s.fg)).bg(to_color(s.bg)).add_modifier(m)
}

fn draw_status_bar(f: &mut Frame, area: Rect, input_mode: bool, panel_level: u8) {
    let level_label = match panel_level {
        0 => "full",
        1 => "no-insn",
        2 => "no-insn,io",
        _ => "uart-full",
    };
    let hint = if input_mode {
        " Esc: exit input mode | type to send to UART ".to_owned()
    } else {
        format!(
            " Space:run/pause  s:step  ←→:speed  f:fastest  i:input  p:panels[{level_label}]  r:reset  b:fb  q:quit "
        )
    };
    draw_bar(f, area, vec![Span::styled(hint, Style::default().fg(Color::Black).bg(Color::White))]);
}

/// 帧缓冲内嵌渲染：下采样到面板尺寸，每格背景色 = 对应像素。
fn draw_framebuffer(f: &mut Frame, area: Rect, fb: &FramebufferFrame) {
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


