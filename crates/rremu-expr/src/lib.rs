//! rremu-expr：调试器表达式 —— 词法 / 递归下降解析 / AST 求值 + EvalContext trait。
//! 零外部依赖的叶子 crate（thiserror 除外）。
//!
//! 文法（C 优先级，低→高）：
//!
//! ```text
//! expr    := or
//! or      := and  ("||" and)*
//! and     := bitor ("&&" bitor)*
//! bitor   := bitxor ("|" bitxor)*
//! bitxor  := bitand ("^" bitand)*
//! bitand  := eq  ("&" eq)*
//! eq      := rel  (("=="|"!=") rel)*
//! rel     := shift (("<"|">"|"<="|">=") shift)*
//! shift   := add  (("<<"|">>") add)*
//! add     := mul  (("+"|"-") mul)*
//! mul     := unary (("*"|"/"|"%") unary)*
//! unary   := ("-"|"~"|"!") unary | primary
//! primary := NUMBER | REG | "pc" | "*" unary | "(" expr ")"
//! NUMBER  := "0x" hex+ | "0b" bin+ | dec+
//! REG     := "x" 0..31 | ABI 别名
//! ```

use thiserror::Error;

/// 寄存器编号（x0..x31）。
pub type RegId = u8;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num(u32),
    Reg(RegId),
    Pc,
    Deref(Box<Expr>),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Logic(LogicOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-`：wrapping 取负
    Neg,
    /// `~`：按位非
    BitNot,
    /// `!`：逻辑非，结果 0/1
    LogNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    /// 逻辑右移（对齐 RV SRL）
    Shr,
    /// 有符号比较（i32）
    Lt,
    Gt,
    Le,
    Ge,
    /// 按 u32
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    /// `&&`：短路，结果 0/1
    And,
    /// `||`：短路，结果 0/1
    Or,
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("parse error at byte {pos}: {msg}")]
pub struct ParseError {
    pub pos: usize,
    pub msg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvalError {
    #[error("division by zero")]
    DivideByZero,
    #[error("peek 0x{addr:08x} failed: {msg}")]
    Peek { addr: u32, msg: String },
}

// ---------------------------------------------------------------------------
// 求值上下文
// ---------------------------------------------------------------------------

/// 求值环境：由 machine 层实现（寄存器视图 + 无副作用总线 peek）。
pub trait EvalContext {
    fn reg(&self, id: RegId) -> u32;
    fn pc(&self) -> u32;
    /// 无副作用读一个 XLEN 字。
    fn peek_word(&mut self, addr: u32) -> Result<u32, EvalError>;
}

impl Expr {
    pub fn eval(&self, ctx: &mut dyn EvalContext) -> Result<u32, EvalError> {
        match self {
            Expr::Num(v) => Ok(*v),
            Expr::Reg(0) => Ok(0), // x0 恒 0
            Expr::Reg(id) => Ok(ctx.reg(*id)),
            Expr::Pc => Ok(ctx.pc()),
            Expr::Deref(e) => {
                let addr = e.eval(ctx)?;
                ctx.peek_word(addr)
            }
            Expr::Un(op, e) => {
                let v = e.eval(ctx)?;
                Ok(match op {
                    UnOp::Neg => v.wrapping_neg(),
                    UnOp::BitNot => !v,
                    UnOp::LogNot => u32::from(v == 0),
                })
            }
            Expr::Logic(op, l, r) => {
                let lv = l.eval(ctx)?;
                let hit = match op {
                    LogicOp::And => lv != 0 && r.eval(ctx)? != 0,
                    LogicOp::Or => lv != 0 || r.eval(ctx)? != 0,
                };
                Ok(u32::from(hit))
            }
            Expr::Bin(op, l, r) => {
                let a = l.eval(ctx)?;
                let b = r.eval(ctx)?;
                eval_bin(*op, a, b)
            }
        }
    }
}

fn eval_bin(op: BinOp, a: u32, b: u32) -> Result<u32, EvalError> {
    Ok(match op {
        BinOp::Add => a.wrapping_add(b),
        BinOp::Sub => a.wrapping_sub(b),
        BinOp::Mul => a.wrapping_mul(b),
        BinOp::Div => {
            let (x, y) = (a as i32, b as i32);
            if y == 0 {
                return Err(EvalError::DivideByZero);
            }
            // INT_MIN / -1 溢出：按 RISC-V DIV 硬件惯例返回原值
            if x == i32::MIN && y == -1 { a } else { (x / y) as u32 }
        }
        BinOp::Rem => {
            let (x, y) = (a as i32, b as i32);
            if y == 0 {
                return Err(EvalError::DivideByZero);
            }
            // INT_MIN % -1 → 0（RISC-V REM 语义）
            if x == i32::MIN && y == -1 { 0 } else { (x % y) as u32 }
        }
        // wrapping_shl/shr 对 u32 自动取移位量低 5 位；u32 >> 即逻辑右移
        BinOp::Shl => a.wrapping_shl(b),
        BinOp::Shr => a.wrapping_shr(b),
        BinOp::Lt => u32::from((a as i32) < (b as i32)),
        BinOp::Gt => u32::from((a as i32) > (b as i32)),
        BinOp::Le => u32::from((a as i32) <= (b as i32)),
        BinOp::Ge => u32::from((a as i32) >= (b as i32)),
        BinOp::Eq => u32::from(a == b),
        BinOp::Ne => u32::from(a != b),
        BinOp::BitAnd => a & b,
        BinOp::BitXor => a ^ b,
        BinOp::BitOr => a | b,
    })
}

// ---------------------------------------------------------------------------
// 寄存器名解析
// ---------------------------------------------------------------------------

/// ABI 别名 → 编号。与 rremu-isa::ABI_NAMES 一致（另含 fp 别名）；
/// 双表各自独立以保持叶子 crate 零耦合，由 machine 层一致性测试锁定同步。
const ALIASES: [(&str, RegId); 33] = [
    ("zero", 0),
    ("ra", 1),
    ("sp", 2),
    ("gp", 3),
    ("tp", 4),
    ("t0", 5),
    ("t1", 6),
    ("t2", 7),
    ("s0", 8),
    ("fp", 8),
    ("s1", 9),
    ("a0", 10),
    ("a1", 11),
    ("a2", 12),
    ("a3", 13),
    ("a4", 14),
    ("a5", 15),
    ("a6", 16),
    ("a7", 17),
    ("s2", 18),
    ("s3", 19),
    ("s4", 20),
    ("s5", 21),
    ("s6", 22),
    ("s7", 23),
    ("s8", 24),
    ("s9", 25),
    ("s10", 26),
    ("s11", 27),
    ("t3", 28),
    ("t4", 29),
    ("t5", 30),
    ("t6", 31),
];

/// 寄存器名 → 编号：`x0`..`x31`、ABI 别名；非寄存器名返回 None。
pub fn reg_from_name(name: &str) -> Option<RegId> {
    if let Some(rest) = name.strip_prefix('x') {
        if !rest.is_empty()
            && rest.bytes().all(|b| b.is_ascii_digit())
            && let Ok(n) = rest.parse::<u8>()
            && n <= 31
        {
            return Some(n);
        }
        return None;
    }
    ALIASES.iter().find(|(n, _)| *n == name).map(|(_, i)| *i)
}

// ---------------------------------------------------------------------------
// 词法
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokKind {
    Num(u32),
    Ident(String),
    LParen,
    RParen,
    Star,
    Slash,
    Percent,
    Plus,
    Minus,
    Tilde,
    Bang,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Caret,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Shl,
    Shr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tok {
    kind: TokKind,
    pos: usize,
}

fn lex(src: &str) -> Result<Vec<Tok>, ParseError> {
    let bytes = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let pos = i;
        // 数字字面量：吞掉整段字母数字，统一在此解析（错误信息更明确）
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let text = &src[start..i];
            let (radix, raw_digits) = if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                (16u32, rest)
            } else if let Some(rest) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
                (2, rest)
            } else {
                (10, text)
            };
            // 允许视觉分隔下划线：解析前剥离
            let digits: String = raw_digits.chars().filter(|&c| c != '_').collect();
            let v = u32::from_str_radix(&digits, radix)
                .map_err(|_| ParseError { pos, msg: format!("invalid number literal `{text}`") })?;
            toks.push(Tok { kind: TokKind::Num(v), pos });
            continue;
        }
        // 标识符（寄存器名 / pc）
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            toks.push(Tok { kind: TokKind::Ident(src[start..i].to_owned()), pos });
            continue;
        }
        // 非 ASCII 字符直接报错（避免切到 UTF-8 中间字节）
        if !bytes[i].is_ascii() {
            return Err(ParseError { pos: i, msg: "unexpected non-ASCII character".to_owned() });
        }
        // 双字运算符优先
        let two = if i + 1 < bytes.len() && bytes[i + 1].is_ascii() { Some(&src[i..i + 2]) } else { None };
        let kind = match two {
            Some("||") => { i += 2; TokKind::PipePipe }
            Some("&&") => { i += 2; TokKind::AmpAmp }
            Some("==") => { i += 2; TokKind::Eq }
            Some("!=") => { i += 2; TokKind::Ne }
            Some("<=") => { i += 2; TokKind::Le }
            Some(">=") => { i += 2; TokKind::Ge }
            Some("<<") => { i += 2; TokKind::Shl }
            Some(">>") => { i += 2; TokKind::Shr }
            _ => {
                i += 1;
                match c {
                    '(' => TokKind::LParen,
                    ')' => TokKind::RParen,
                    '*' => TokKind::Star,
                    '/' => TokKind::Slash,
                    '%' => TokKind::Percent,
                    '+' => TokKind::Plus,
                    '-' => TokKind::Minus,
                    '~' => TokKind::Tilde,
                    '!' => TokKind::Bang,
                    '&' => TokKind::Amp,
                    '|' => TokKind::Pipe,
                    '^' => TokKind::Caret,
                    '<' => TokKind::Lt,
                    '>' => TokKind::Gt,
                    _ => return Err(ParseError { pos, msg: format!("unexpected character `{c}`") }),
                }
            }
        };
        toks.push(Tok { kind, pos });
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// 解析（递归下降，逐层对应文法）
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<Tok>,
    i: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn eat(&mut self, pred: impl Fn(&TokKind) -> bool) -> bool {
        if self.peek().is_some_and(|t| pred(&t.kind)) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn unexpected_end(&self) -> ParseError {
        ParseError { pos: self.end, msg: "unexpected end of input".to_owned() }
    }

    fn unexpected(&self, t: &Tok) -> ParseError {
        ParseError { pos: t.pos, msg: format!("unexpected token {:?}", t.kind) }
    }

    fn binop_chain(&mut self, next: fn(&mut Self) -> Result<Expr, ParseError>, ops: &[(TokKind, BinOp)]) -> Result<Expr, ParseError> {
        let mut lhs = next(self)?;
        'outer: while let Some(t) = self.peek() {
            for (kind, op) in ops {
                if t.kind == *kind {
                    self.i += 1;
                    let rhs = next(self)?;
                    lhs = Expr::Bin(*op, Box::new(lhs), Box::new(rhs));
                    continue 'outer;
                }
            }
            break;
        }
        Ok(lhs)
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while self.eat(|k| matches!(k, TokKind::PipePipe)) {
            let rhs = self.parse_and()?;
            lhs = Expr::Logic(LogicOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_bitor()?;
        while self.eat(|k| matches!(k, TokKind::AmpAmp)) {
            let rhs = self.parse_bitor()?;
            lhs = Expr::Logic(LogicOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_bitor(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(Self::parse_bitxor, &[(TokKind::Pipe, BinOp::BitOr)])
    }

    fn parse_bitxor(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(Self::parse_bitand, &[(TokKind::Caret, BinOp::BitXor)])
    }

    fn parse_bitand(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(Self::parse_eq, &[(TokKind::Amp, BinOp::BitAnd)])
    }

    fn parse_eq(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(Self::parse_rel, &[(TokKind::Eq, BinOp::Eq), (TokKind::Ne, BinOp::Ne)])
    }

    fn parse_rel(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(
            Self::parse_shift,
            &[(TokKind::Lt, BinOp::Lt), (TokKind::Gt, BinOp::Gt), (TokKind::Le, BinOp::Le), (TokKind::Ge, BinOp::Ge)],
        )
    }

    fn parse_shift(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(Self::parse_add, &[(TokKind::Shl, BinOp::Shl), (TokKind::Shr, BinOp::Shr)])
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(Self::parse_mul, &[(TokKind::Plus, BinOp::Add), (TokKind::Minus, BinOp::Sub)])
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        self.binop_chain(
            Self::parse_unary,
            &[(TokKind::Star, BinOp::Mul), (TokKind::Slash, BinOp::Div), (TokKind::Percent, BinOp::Rem)],
        )
    }

    /// unary := ("-"|"~"|"!") unary | primary；`*`（primary 层）折叠到此处等价且支持 `**p`。
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let Some(t) = self.peek() else { return Err(self.unexpected_end()) };
        let op = match t.kind {
            TokKind::Minus => Some(UnOp::Neg),
            TokKind::Tilde => Some(UnOp::BitNot),
            TokKind::Bang => Some(UnOp::LogNot),
            TokKind::Star => {
                self.i += 1;
                return Ok(Expr::Deref(Box::new(self.parse_unary()?)));
            }
            _ => None,
        };
        if let Some(op) = op {
            self.i += 1;
            let e = self.parse_unary()?;
            return Ok(Expr::Un(op, Box::new(e)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let Some(t) = self.toks.get(self.i).cloned() else { return Err(self.unexpected_end()) };
        self.i += 1;
        match t.kind {
            TokKind::Num(v) => Ok(Expr::Num(v)),
            TokKind::Ident(name) => resolve_ident(&name, t.pos),
            TokKind::LParen => {
                let e = self.parse_or()?;
                if !self.eat(|k| matches!(k, TokKind::RParen)) {
                    return Err(self.peek().map_or_else(|| self.unexpected_end(), |t| self.unexpected(t)));
                }
                Ok(e)
            }
            _ => Err(self.unexpected(&t)),
        }
    }
}

fn resolve_ident(name: &str, pos: usize) -> Result<Expr, ParseError> {
    if name == "pc" {
        return Ok(Expr::Pc);
    }
    reg_from_name(name)
        .map(Expr::Reg)
        .ok_or_else(|| ParseError { pos, msg: format!("unknown register `{name}`") })
}

/// 解析表达式；错误带字节位置。
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, i: 0, end: src.len() };
    let e = p.parse_or()?;
    if let Some(t) = p.peek() {
        return Err(p.unexpected(t));
    }
    Ok(e)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    struct Ctx {
        regs: [u32; 32],
        pc: u32,
        mem: Vec<(u32, u32)>,
    }

    impl Ctx {
        fn new() -> Self {
            Self { regs: [0; 32], pc: 0x8000_0000, mem: Vec::new() }
        }
        fn with_reg(mut self, i: u8, v: u32) -> Self {
            self.regs[usize::from(i)] = v;
            self
        }
        fn with_mem(mut self, addr: u32, v: u32) -> Self {
            self.mem.push((addr, v));
            self
        }
    }

    impl EvalContext for Ctx {
        fn reg(&self, id: RegId) -> u32 {
            self.regs[usize::from(id)]
        }
        fn pc(&self) -> u32 {
            self.pc
        }
        fn peek_word(&mut self, addr: u32) -> Result<u32, EvalError> {
            self.mem
                .iter()
                .find(|(a, _)| *a == addr)
                .map(|(_, v)| *v)
                .ok_or_else(|| EvalError::Peek { addr, msg: "unmapped".to_owned() })
        }
    }

    fn eval(src: &str, ctx: &mut Ctx) -> u32 {
        parse(src).unwrap().eval(ctx).unwrap()
    }

    // ---- 优先级 ----

    #[test]
    fn precedence_mul_over_add() {
        assert_eq!(eval("1+2*3", &mut Ctx::new()), 7);
        assert_eq!(eval("10-4/2", &mut Ctx::new()), 8);
    }

    #[test]
    fn precedence_add_over_shift() {
        // C 语义：+ 高于 <<
        assert_eq!(eval("1<<2+3", &mut Ctx::new()), 32);
    }

    #[test]
    fn parens_override() {
        assert_eq!(eval("(1+2)*3", &mut Ctx::new()), 9);
        assert_eq!(eval("1<<(2+3)", &mut Ctx::new()), 32);
    }

    #[test]
    fn precedence_eq_over_bitand() {
        // == 高于 &：3 & (1 == 1) = 1
        assert_eq!(eval("3&1==1", &mut Ctx::new()), 1);
        // < 高于 ==：(1 < 2) == 1
        assert_eq!(eval("1<2==1", &mut Ctx::new()), 1);
    }

    #[test]
    fn precedence_xor_over_or() {
        // ^ 高于 |：2 | (4 ^ 1) = 7
        assert_eq!(eval("2|4^1", &mut Ctx::new()), 7);
    }

    #[test]
    fn precedence_and_over_or_logic() {
        // || 最低：2 || (0 && 0) = 1
        assert_eq!(eval("2||0&&0", &mut Ctx::new()), 1);
    }

    // ---- 逻辑运算与短路 ----

    #[test]
    fn logic_results_are_0_or_1() {
        assert_eq!(eval("1&&0", &mut Ctx::new()), 0);
        assert_eq!(eval("3&&2", &mut Ctx::new()), 1);
        assert_eq!(eval("0||0", &mut Ctx::new()), 0);
        assert_eq!(eval("0||7", &mut Ctx::new()), 1);
    }

    #[test]
    fn short_circuit_skips_rhs() {
        // RHS 含除零，短路后不应求值
        assert_eq!(eval("1||1/0", &mut Ctx::new()), 1);
        assert_eq!(eval("0&&1/0", &mut Ctx::new()), 0);
    }

    // ---- 单目 ----

    #[test]
    fn unary_ops() {
        assert_eq!(eval("!5", &mut Ctx::new()), 0);
        assert_eq!(eval("!0", &mut Ctx::new()), 1);
        assert_eq!(eval("~0", &mut Ctx::new()), 0xffff_ffff);
        assert_eq!(eval("~~5", &mut Ctx::new()), 5);
        assert_eq!(eval("-5", &mut Ctx::new()), 0xffff_fffb);
        assert_eq!(eval("--5", &mut Ctx::new()), 5);
    }

    // ---- 算术语义 ----

    #[test]
    fn wrapping_add_sub() {
        assert_eq!(eval("0xffffffff+1", &mut Ctx::new()), 0);
        assert_eq!(eval("0-1", &mut Ctx::new()), 0xffff_ffff);
    }

    #[test]
    fn shift_low5_and_logical_shr() {
        assert_eq!(eval("1<<33", &mut Ctx::new()), 2); // 移位量取低 5 位
        assert_eq!(eval("1<<31", &mut Ctx::new()), 0x8000_0000);
        assert_eq!(eval("0x80000000>>31", &mut Ctx::new()), 1); // 逻辑右移
        assert_eq!(eval("0xffffffff>>28", &mut Ctx::new()), 0xf);
    }

    #[test]
    fn rel_is_signed() {
        // 0-1 = 0xffffffff，有符号比较 -1 < 1
        assert_eq!(eval("0-1<1", &mut Ctx::new()), 1);
        // 反向：-1 > -2
        assert_eq!(eval("0-1>0-2", &mut Ctx::new()), 1);
    }

    #[test]
    fn eq_is_unsigned() {
        assert_eq!(eval("0xffffffff==4294967295", &mut Ctx::new()), 1);
        assert_eq!(eval("0xffffffff!=4294967295", &mut Ctx::new()), 0);
    }

    #[test]
    fn div_rem_signed_truncation() {
        // -7 / 2 = -3（向零截断）
        assert_eq!(eval("(0-7)/2", &mut Ctx::new()), 0xffff_fffd);
        // -7 % 2 = -1
        assert_eq!(eval("(0-7)%2", &mut Ctx::new()), 0xffff_ffff);
        assert_eq!(eval("7%3", &mut Ctx::new()), 1);
    }

    #[test]
    fn div_by_zero_errors() {
        assert_eq!(parse("1/0").unwrap().eval(&mut Ctx::new()), Err(EvalError::DivideByZero));
        assert_eq!(parse("5%0").unwrap().eval(&mut Ctx::new()), Err(EvalError::DivideByZero));
    }

    #[test]
    fn int_min_div_minus_one_follows_hardware() {
        // INT_MIN / -1 → 原值；INT_MIN % -1 → 0
        let expr = "(0-2147483648)/(0-1)";
        assert_eq!(eval(expr, &mut Ctx::new()), 0x8000_0000);
        let expr = "(0-2147483648)%(0-1)";
        assert_eq!(eval(expr, &mut Ctx::new()), 0);
    }

    // ---- 寄存器与 pc ----

    #[test]
    fn x0_is_constant_zero() {
        assert_eq!(eval("x0", &mut Ctx::new()), 0);
        assert_eq!(eval("x0+1", &mut Ctx::new()), 1);
    }

    #[test]
    fn aliases_and_xn_agree() {
        let mut ctx = Ctx::new().with_reg(2, 0x100).with_reg(10, 7).with_reg(8, 0x40);
        assert_eq!(eval("sp", &mut ctx), 0x100);
        assert_eq!(eval("x2", &mut ctx), 0x100);
        assert_eq!(eval("a0", &mut ctx), 7);
        assert_eq!(eval("x10", &mut ctx), 7);
        assert_eq!(eval("fp", &mut ctx), 0x40);
        assert_eq!(eval("s0", &mut ctx), 0x40);
        assert_eq!(eval("sp+a0", &mut ctx), 0x107);
    }

    #[test]
    fn pc_reads_context() {
        assert_eq!(eval("pc", &mut Ctx::new()), 0x8000_0000);
        assert_eq!(eval("pc==0x80000000", &mut Ctx::new()), 1);
    }

    // ---- 解引用 ----

    #[test]
    fn deref_reads_context_memory() {
        let mut ctx = Ctx::new().with_mem(0x100, 0x200).with_mem(0x200, 0xdead_beef);
        assert_eq!(eval("*0x100", &mut ctx), 0x200);
        assert_eq!(eval("**0x100", &mut ctx), 0xdead_beef);
        assert_eq!(eval("*(0x100+0x100)", &mut ctx), 0xdead_beef);
    }

    #[test]
    fn deref_unmapped_is_error() {
        let mut ctx = Ctx::new();
        let e = parse("*0x1234").unwrap().eval(&mut ctx).unwrap_err();
        assert_eq!(e, EvalError::Peek { addr: 0x1234, msg: "unmapped".to_owned() });
    }

    // ---- 解析错误与位置 ----

    #[test]
    fn error_positions() {
        assert_eq!(parse("1+").unwrap_err().pos, 2); // 末尾缺操作数
        assert_eq!(parse("@").unwrap_err().pos, 0); // 非法字符
        assert_eq!(parse("1 2").unwrap_err().pos, 2); // 多余 token
        assert_eq!(parse("0x").unwrap_err().pos, 0); // 数字残缺
    }

    #[test]
    fn unknown_register_reports_name_and_pos() {
        let e = parse("xyz").unwrap_err();
        assert_eq!(e.pos, 0);
        assert!(e.msg.contains("unknown register"), "msg = {}", e.msg);
        // x32 越界同样拒绝
        assert!(parse("x32").is_err());
    }

    #[test]
    fn unbalanced_paren_reports_end() {
        let e = parse("(1+2").unwrap_err();
        assert_eq!(e.pos, 4);
    }

    #[test]
    fn hex_bin_and_decimal_literals() {
        assert_eq!(eval("0xff", &mut Ctx::new()), 255);
        assert_eq!(eval("0b1010", &mut Ctx::new()), 10);
        assert_eq!(eval("42", &mut Ctx::new()), 42);
        assert_eq!(eval("0X10", &mut Ctx::new()), 16);
    }

    #[test]
    fn underscores_are_visual_separators() {
        assert_eq!(eval("1_0", &mut Ctx::new()), 10);
        assert_eq!(eval("0x1_0", &mut Ctx::new()), 16);
        assert_eq!(eval("0b1_0", &mut Ctx::new()), 2);
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(eval("  1 + 2 * 3 ", &mut Ctx::new()), 7);
    }
}
