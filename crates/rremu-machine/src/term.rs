//! term：面向 UART 视图的最小终端状态机（ANSI/ECMA-48 子集）。
//!
//! 有状态、逐字节推进；支持 C0、常用 CSI（`J/K/H/f/A-D/G/d/s/u/h/l/m`）、
//! UTF-8（含双宽字符）与 SGR 颜色。输出为 scrollback 行 + 光标，供 TUI 渲染。
//!
//! 范围见 `PLAN-UART-TERM.md`：阶段 1 + 2 + SGR；阶段 3（滚动区域/备用屏幕等）不做。

use unicode_width::UnicodeWidthChar;

/// scrollback 行上限（超出丢弃最旧行）。
pub const SCROLLBACK_CAP: usize = 4096;
const TAB_WIDTH: usize = 8;

/// 颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// 终端默认色。
    #[default]
    Default,
    /// 256 色索引。
    Indexed(u8),
    /// 24 位真彩。
    Rgb(u8, u8, u8),
}

/// 字符样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Style {
    pub const DEFAULT: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        reverse: false,
    };
}

/// 单元格。`wide_cont` 为宽字符的次格（占位，渲染时跳过）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    pub wide_cont: bool,
}

impl Cell {
    pub const BLANK: Cell = Cell { ch: ' ', style: Style::DEFAULT, wide_cont: false };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    OscEsc,
    Charset,
}

/// 终端屏幕。
#[derive(Debug, Clone)]
pub struct Terminal {
    lines: Vec<Vec<Cell>>,
    row: usize,
    col: usize,
    style: Style,
    saved: (usize, usize),
    state: State,
    params: Vec<u32>,
    private: bool,
    // UTF-8 组装
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            lines: vec![Vec::new()],
            row: 0,
            col: 0,
            style: Style::DEFAULT,
            saved: (0, 0),
            state: State::Ground,
            params: Vec::new(),
            private: false,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
        }
    }

    /// 屏幕行（scrollback）。
    pub fn lines(&self) -> &[Vec<Cell>] {
        &self.lines
    }

    /// 光标 `(row, col)`。
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// 逐字节推进。
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.feed(b);
        }
    }

    /// 处理单个字节。
    pub fn feed(&mut self, b: u8) {
        if self.utf8_len > 0 {
            self.feed_utf8_cont(b);
            return;
        }
        match self.state {
            State::Ground => self.feed_ground(b),
            State::Escape => self.feed_escape(b),
            State::Csi => self.feed_csi(b),
            State::Osc => {
                if b == 0x07 {
                    self.state = State::Ground;
                } else if b == 0x1b {
                    self.state = State::OscEsc;
                }
            }
            State::OscEsc => {
                self.state = if b == b'\\' { State::Ground } else { State::Osc };
            }
            State::Charset => self.state = State::Ground,
        }
    }

    // ---- 解析 ----

    fn feed_ground(&mut self, b: u8) {
        match b {
            0x1b => self.state = State::Escape,
            0x07 => {} // BEL
            0x08 => self.col = self.col.saturating_sub(1),
            0x09 => self.col = (self.col / TAB_WIDTH + 1) * TAB_WIDTH,
            0x0a..=0x0c => self.line_feed(),
            0x0d => self.col = 0,
            0x00..=0x06 | 0x0e..=0x1a | 0x1c..=0x1f => {} // 其余 C0 忽略
            0x7f => {}
            0x20..=0x7e => self.put_char(b as char),
            _ => self.feed_utf8_start(b),
        }
    }

    fn feed_escape(&mut self, b: u8) {
        self.state = State::Ground;
        match b {
            b'[' => {
                self.params.clear();
                self.private = false;
                self.state = State::Csi;
            }
            b']' => self.state = State::Osc,
            b'(' | b')' => self.state = State::Charset,
            b'c' => self.reset(),
            b'7' => self.saved = (self.row, self.col),
            b'8' => {
                self.row = self.saved.0;
                self.col = self.saved.1;
                self.ensure_row(self.row);
            }
            _ => {}
        }
    }

    fn feed_csi(&mut self, b: u8) {
        match b {
            0x30..=0x3f => {
                if b == b'?' {
                    self.private = true;
                } else if b.is_ascii_digit() {
                    if self.params.is_empty() {
                        self.params.push(0);
                    }
                    let i = self.params.len() - 1;
                    self.params[i] = self.params[i].saturating_mul(10).saturating_add(u32::from(b - b'0'));
                } else {
                    // ';' ':' 等：开新参数
                    self.params.push(0);
                }
            }
            0x20..=0x2f => {} // 中间字节：忽略
            0x40..=0x7e => {
                self.csi_execute(b);
                self.state = State::Ground;
            }
            0x1b => self.state = State::Escape,
            _ => self.state = State::Ground,
        }
    }

    fn feed_utf8_start(&mut self, b: u8) {
        let need = if b < 0x80 {
            1
        } else if b >> 5 == 0b110 {
            2
        } else if b >> 4 == 0b1110 {
            3
        } else if b >> 3 == 0b11110 {
            4
        } else {
            self.put_char('\u{FFFD}');
            return;
        };
        if need == 1 {
            self.put_char(b as char);
        } else {
            self.utf8[0] = b;
            self.utf8_len = 1;
            self.utf8_need = need;
        }
    }

    fn feed_utf8_cont(&mut self, b: u8) {
        if b & 0xc0 != 0x80 {
            // 非法续字节：替换并重新按新字节解析
            self.utf8_len = 0;
            self.put_char('\u{FFFD}');
            self.feed(b);
            return;
        }
        self.utf8[self.utf8_len] = b;
        self.utf8_len += 1;
        if self.utf8_len == self.utf8_need {
            let decoded = core::str::from_utf8(&self.utf8[..self.utf8_len])
                .ok()
                .and_then(|s| s.chars().next())
                .unwrap_or('\u{FFFD}');
            self.utf8_len = 0;
            self.put_char(decoded);
        }
    }

    fn csi_execute(&mut self, final_byte: u8) {
        let p = |i: usize, def: u32| -> u32 { self.params.get(i).copied().unwrap_or(def) };
        match final_byte {
            b'A' => self.cursor_move(-1, 0, p(0, 1)),
            b'B' => self.cursor_move(1, 0, p(0, 1)),
            b'C' => self.cursor_move(0, 1, p(0, 1)),
            b'D' => self.cursor_move(0, -1, p(0, 1)),
            b'G' => self.col = (p(0, 1).max(1) - 1) as usize,
            b'd' => {
                self.row = (p(0, 1).max(1) - 1) as usize;
                self.ensure_row(self.row);
            }
            b'H' | b'f' => {
                let r = (p(0, 1).max(1) - 1) as usize;
                let c = (p(1, 1).max(1) - 1) as usize;
                self.ensure_row(r);
                self.row = r;
                self.col = c;
            }
            b'J' => self.erase_display(p(0, 0)),
            b'K' => self.erase_line(p(0, 0)),
            b'm' => self.set_style(),
            b's' => self.saved = (self.row, self.col),
            b'u' => {
                self.row = self.saved.0;
                self.col = self.saved.1;
                self.ensure_row(self.row);
            }
            b'h' | b'l' => {} // 模式：记录不渲染
            _ => {}
        }
    }

    // ---- 屏幕操作 ----

    fn reset(&mut self) {
        self.lines = vec![Vec::new()];
        self.row = 0;
        self.col = 0;
        self.style = Style::DEFAULT;
        self.saved = (0, 0);
    }

    fn ensure_row(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(Vec::new());
        }
    }

    /// 滚动：行数超上限时丢最旧行。
    fn enforce_scrollback(&mut self) {
        while self.lines.len() > SCROLLBACK_CAP {
            self.lines.remove(0);
            self.row = self.row.saturating_sub(1);
        }
    }

    fn put_char(&mut self, c: char) {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if w == 0 {
            return; // 组合/零宽字符：忽略
        }
        self.ensure_row(self.row);
        let line = &mut self.lines[self.row];
        if self.col > line.len() {
            line.resize(self.col, Cell::BLANK);
        }
        let cell = Cell { ch: c, style: self.style, wide_cont: false };
        if self.col < line.len() {
            line[self.col] = cell;
        } else {
            line.push(cell);
        }
        self.col += 1;
        if w == 2 {
            // 宽字符次格占位
            let line = &mut self.lines[self.row];
            let cont = Cell { ch: ' ', style: self.style, wide_cont: true };
            if self.col < line.len() {
                line[self.col] = cont;
            } else {
                line.push(cont);
            }
            self.col += 1;
        }
    }

    /// LF：下移一行（列不变）；末行则新建行。
    fn line_feed(&mut self) {
        if self.row + 1 >= self.lines.len() {
            self.lines.push(Vec::new());
        }
        self.row += 1;
        self.enforce_scrollback();
    }

    fn cursor_move(&mut self, dy: i32, dx: i32, n: u32) {
        let step = n as i32;
        let new_row = (self.row as i32 + dy * step).max(0) as usize;
        self.ensure_row(new_row);
        self.row = new_row.min(self.lines.len() - 1);
        self.col = ((self.col as i32 + dx * step).max(0)) as usize;
    }

    fn erase_line(&mut self, mode: u32) {
        self.ensure_row(self.row);
        let line = &mut self.lines[self.row];
        match mode {
            0 => {
                if self.col < line.len() {
                    line.truncate(self.col);
                }
            }
            1 => {
                let end = (self.col + 1).min(line.len());
                for c in &mut line[..end] {
                    *c = Cell::BLANK;
                }
            }
            2 => line.clear(),
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u32) {
        match mode {
            0 => {
                self.erase_line(0);
                self.lines.truncate(self.row + 1);
            }
            1 => {
                let n = self.row.min(self.lines.len());
                for l in &mut self.lines[..n] {
                    l.clear();
                }
                self.erase_line(1);
            }
            2 => {
                self.lines = vec![Vec::new()];
                self.row = 0;
                self.col = 0;
            }
            _ => {}
        }
    }

    // ---- SGR ----

    fn set_style(&mut self) {
        if self.params.is_empty() {
            self.style = Style::DEFAULT;
            return;
        }
        let ps = self.params.clone();
        let mut i = 0;
        while i < ps.len() {
            match ps[i] {
                0 => self.style = Style::DEFAULT,
                1 => self.style.bold = true,
                2 => self.style.dim = true,
                3 => self.style.italic = true,
                4 => self.style.underline = true,
                7 => self.style.reverse = true,
                22 => {
                    self.style.bold = false;
                    self.style.dim = false;
                }
                23 => self.style.italic = false,
                24 => self.style.underline = false,
                27 => self.style.reverse = false,
                30..=37 => self.style.fg = Color::Indexed((ps[i] - 30) as u8),
                90..=97 => self.style.fg = Color::Indexed((ps[i] - 90 + 8) as u8),
                39 => self.style.fg = Color::Default,
                40..=47 => self.style.bg = Color::Indexed((ps[i] - 40) as u8),
                100..=107 => self.style.bg = Color::Indexed((ps[i] - 100 + 8) as u8),
                49 => self.style.bg = Color::Default,
                38 => i += self.ext_color(&ps, i + 1, true),
                48 => i += self.ext_color(&ps, i + 1, false),
                _ => {}
            }
            i += 1;
        }
    }

    /// 解析 `38/48` 后的扩展颜色，返回额外消费的参数个数（不含 `38/48` 本身）。
    fn ext_color(&mut self, ps: &[u32], start: usize, fg: bool) -> usize {
        match ps.get(start) {
            Some(5) => {
                if let Some(&n) = ps.get(start + 1) {
                    let c = Color::Indexed(n as u8);
                    if fg { self.style.fg = c } else { self.style.bg = c }
                    return 2;
                }
                0
            }
            Some(2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (ps.get(start + 1), ps.get(start + 2), ps.get(start + 3))
                {
                    let c = Color::Rgb(r as u8, g as u8, b as u8);
                    if fg { self.style.fg = c } else { self.style.bg = c }
                    return 4;
                }
                0
            }
            _ => 0,
        }
    }

    /// 去掉 `wide_cont` 占位后的可见字符与样式（渲染辅助）。
    pub fn visible_cells(line: &[Cell]) -> impl Iterator<Item = &Cell> {
        line.iter().filter(|c| !c.wide_cont)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    fn text(t: &Terminal) -> Vec<String> {
        t.lines()
            .iter()
            .map(|l| l.iter().filter(|c| !c.wide_cont).map(|c| c.ch).collect())
            .collect()
    }

    #[test]
    fn basic_text_and_lf_cr() {
        let mut t = Terminal::new();
        t.feed_bytes(b"abc");
        assert_eq!(text(&t), vec!["abc"]);
        t.feed_bytes(b"\rX");
        assert_eq!(text(&t), vec!["Xbc"]);
        t.feed_bytes(b"\r\ndef");
        assert_eq!(text(&t), vec!["Xbc", "def"]);
    }

    #[test]
    fn backspace_overwrite() {
        let mut t = Terminal::new();
        t.feed_bytes(b"ab\x08 \x08");
        assert_eq!(text(&t), vec!["a "]);
    }

    #[test]
    fn tab_moves_to_next_stop() {
        let mut t = Terminal::new();
        t.feed_bytes(b"a\tb");
        assert_eq!(text(&t), vec!["a       b"]);
    }

    #[test]
    fn clear_screen() {
        let mut t = Terminal::new();
        t.feed_bytes(b"hello\r\nworld");
        t.feed_bytes(b"\x1b[2J\x1b[H");
        assert_eq!(text(&t), vec![""]);
        assert_eq!(t.cursor(), (0, 0));
        t.feed_bytes(b"X");
        assert_eq!(text(&t), vec!["X"]);
    }

    #[test]
    fn erase_line() {
        let mut t = Terminal::new();
        t.feed_bytes(b"abcdef");
        t.feed_bytes(b"\r\x1b[2K");
        assert_eq!(text(&t), vec![""]);
    }

    #[test]
    fn cursor_position_and_move() {
        let mut t = Terminal::new();
        t.feed_bytes(b"\x1b[3;5HX");
        assert_eq!(text(&t), vec!["", "", "    X"]);
        assert_eq!(t.cursor(), (2, 5));
        // 上移 2 行（列不变），再用 G 定位到列 1 写 X
        t.feed_bytes(b"\x1b[2A\x1b[1GX");
        assert_eq!(text(&t), vec!["X", "", "    X"]);
        assert_eq!(t.cursor(), (0, 1));
    }

    #[test]
    fn save_restore_cursor() {
        let mut t = Terminal::new();
        t.feed_bytes(b"ab\x1b7cd\x1b8X");
        // 保存于 (0,2)，cd 后回到 (0,2) 写 X
        assert_eq!(text(&t), vec!["abXd"]);
    }

    #[test]
    fn sgr_style_and_reset() {
        let mut t = Terminal::new();
        t.feed_bytes(b"\x1b[1;31mR\x1b[0mN");
        let line = &t.lines()[0];
        assert!(line[0].style.bold);
        assert_eq!(line[0].style.fg, Color::Indexed(1));
        assert_eq!(line[1].style, Style::DEFAULT);
    }

    #[test]
    fn sgr_256_and_truecolor() {
        let mut t = Terminal::new();
        t.feed_bytes(b"\x1b[38;5;196mX");
        assert_eq!(t.lines()[0][0].style.fg, Color::Indexed(196));
        t.feed_bytes(b"\x1b[38;2;10;20;30mY");
        assert_eq!(t.lines()[0][1].style.fg, Color::Rgb(10, 20, 30));
        t.feed_bytes(b"\x1b[48;5;21mZ");
        assert_eq!(t.lines()[0][2].style.bg, Color::Indexed(21));
    }

    #[test]
    fn utf8_and_wide_char() {
        let mut t = Terminal::new();
        t.feed_bytes("中a".as_bytes());
        // 中文宽 2：占两格（次格占位），列推进到 3
        assert_eq!(t.cursor().1, 3);
        let line = &t.lines()[0];
        assert_eq!(line[0].ch, '中');
        assert!(line[1].wide_cont);
        assert_eq!(line[2].ch, 'a');
        assert_eq!(text(&t), vec!["中a"]);
    }

    #[test]
    fn invalid_utf8_replacement() {
        let mut t = Terminal::new();
        t.feed_bytes(&[0xff]);
        assert_eq!(t.lines()[0][0].ch, '\u{FFFD}');
    }

    #[test]
    fn incomplete_escape_across_feeds() {
        let mut t = Terminal::new();
        t.feed(b'\x1b');
        t.feed(b'[');
        t.feed(b'1');
        t.feed(b'm');
        t.feed(b'X');
        assert!(t.lines()[0][0].style.bold);
    }

    #[test]
    fn osc_is_dropped() {
        let mut t = Terminal::new();
        t.feed_bytes(b"\x1b]0;title\x07abc");
        assert_eq!(text(&t), vec!["abc"]);
    }
}
