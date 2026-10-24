//! 基础类型、特权级与 ABI 名称。

/// 32 位通用寄存器值 / 指令字。
pub type Word = u32;
/// 32 位地址。
pub type Address = u32;

/// 特权级（最简 M-mode：仅 U/M 两级；mstatus.MPP 只支持 00/11）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priv {
    User,
    Supervisor,
    Machine,
}

impl core::fmt::Display for Priv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Priv::User => write!(f, "U"),
            Priv::Supervisor => write!(f, "S"),
            Priv::Machine => write!(f, "M"),
        }
    }
}

pub const ABI_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// 浮点寄存器 ABI 名称（f0..f31）。
pub const FP_ABI_NAMES: [&str; 32] = [
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

/// 取寄存器 i 的 ABI 别名（越界时回退为 xN）。
pub fn reg_name(i: u8) -> &'static str {
    ABI_NAMES
        .get(usize::from(i))
        .copied()
        .unwrap_or_else(|| unreachable_x(i))
}

/// 取浮点寄存器 i 的 ABI 别名（越界时回退为 fN）。
pub fn freg_name(i: u8) -> &'static str {
    FP_ABI_NAMES
        .get(usize::from(i))
        .copied()
        .unwrap_or_else(|| unreachable_f(i))
}

// 辅助：寄存器编号越界（正常解码不会出现，防御性兜底）。
#[cold]
#[inline(never)]
fn unreachable_x(i: u8) -> &'static str {
    // 无法返回动态字符串的 &'static，这里以固定字符串兜底，仅在非法输入时出现
    let _ = i;
    "x?"
}

#[cold]
#[inline(never)]
fn unreachable_f(i: u8) -> &'static str {
    let _ = i;
    "f?"
}
