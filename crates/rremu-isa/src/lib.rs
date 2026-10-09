//! rremu-isa：ISA 层 —— 基础类型、指令枚举、CSR 常量、特权级定义。
//! 零外部依赖、零 IO 的叶子 crate。

/// 32 位通用寄存器值 / 指令字。
pub type Word = u32;
/// 32 位地址。
pub type Address = u32;

/// 特权级（最简 M-mode：仅 U/M 两级；mstatus.MPP 只支持 00/11）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priv {
    User,
    Machine,
}

impl core::fmt::Display for Priv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Priv::User => write!(f, "U"),
            Priv::Machine => write!(f, "M"),
        }
    }
}

/// RV32I 指令枚举（M1）。M/C/Zicsr 变体在 M2 扩展。
///
/// 立即数已按 spec 做符号扩展；寄存器编号为 0..=31 原始编号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    // ---- U 型 ----
    Lui { rd: u8, imm: u32 },
    Auipc { rd: u8, imm: u32 },
    // ---- J 型 ----
    Jal { rd: u8, imm: i32 },
    // ---- I 型 ----
    Jalr { rd: u8, rs1: u8, imm: i32 },
    Addi { rd: u8, rs1: u8, imm: i32 },
    Slti { rd: u8, rs1: u8, imm: i32 },
    Sltiu { rd: u8, rs1: u8, imm: i32 },
    Xori { rd: u8, rs1: u8, imm: i32 },
    Ori { rd: u8, rs1: u8, imm: i32 },
    Andi { rd: u8, rs1: u8, imm: i32 },
    Slli { rd: u8, rs1: u8, shamt: u8 },
    Srli { rd: u8, rs1: u8, shamt: u8 },
    Srai { rd: u8, rs1: u8, shamt: u8 },
    // ---- load ----
    Lb { rd: u8, rs1: u8, imm: i32 },
    Lh { rd: u8, rs1: u8, imm: i32 },
    Lw { rd: u8, rs1: u8, imm: i32 },
    Lbu { rd: u8, rs1: u8, imm: i32 },
    Lhu { rd: u8, rs1: u8, imm: i32 },
    // ---- S 型 store ----
    Sb { rs1: u8, rs2: u8, imm: i32 },
    Sh { rs1: u8, rs2: u8, imm: i32 },
    Sw { rs1: u8, rs2: u8, imm: i32 },
    // ---- B 型 branch ----
    Beq { rs1: u8, rs2: u8, imm: i32 },
    Bne { rs1: u8, rs2: u8, imm: i32 },
    Blt { rs1: u8, rs2: u8, imm: i32 },
    Bge { rs1: u8, rs2: u8, imm: i32 },
    Bltu { rs1: u8, rs2: u8, imm: i32 },
    Bgeu { rs1: u8, rs2: u8, imm: i32 },
    // ---- R 型 ----
    Add { rd: u8, rs1: u8, rs2: u8 },
    Sub { rd: u8, rs1: u8, rs2: u8 },
    Sll { rd: u8, rs1: u8, rs2: u8 },
    Slt { rd: u8, rs1: u8, rs2: u8 },
    Sltu { rd: u8, rs1: u8, rs2: u8 },
    Xor { rd: u8, rs1: u8, rs2: u8 },
    Srl { rd: u8, rs1: u8, rs2: u8 },
    Sra { rd: u8, rs1: u8, rs2: u8 },
    Or { rd: u8, rs1: u8, rs2: u8 },
    And { rd: u8, rs1: u8, rs2: u8 },
    // ---- M extension (RV32M) ----
    Mul { rd: u8, rs1: u8, rs2: u8 },
    Mulh { rd: u8, rs1: u8, rs2: u8 },
    Mulhsu { rd: u8, rs1: u8, rs2: u8 },
    Mulhu { rd: u8, rs1: u8, rs2: u8 },
    Div { rd: u8, rs1: u8, rs2: u8 },
    Divu { rd: u8, rs1: u8, rs2: u8 },
    Rem { rd: u8, rs1: u8, rs2: u8 },
    Remu { rd: u8, rs1: u8, rs2: u8 },
    // ---- misc / system ----
    Fence,
    Ecall,
    Ebreak,
    // ---- Zicsr ----
    Csrrw { rd: u8, csr: u16, rs1: u8 },
    Csrrs { rd: u8, csr: u16, rs1: u8 },
    Csrrc { rd: u8, csr: u16, rs1: u8 },
    Csrrwi { rd: u8, csr: u16, zimm: u8 },
    Csrrsi { rd: u8, csr: u16, zimm: u8 },
    Csrrci { rd: u8, csr: u16, zimm: u8 },
    // ---- privileged ----
    Mret,
    // ---- C extension (RV32C) ----
    CAddi4spn { rd: u8, imm: u32 },
    CLw { rd: u8, rs1: u8, imm: i32 },
    CSw { rs1: u8, rs2: u8, imm: i32 },
    CAddi { rd: u8, imm: i32 },
    CJal { imm: i32 },
    CLi { rd: u8, imm: i32 },
    CAddi16sp { imm: i32 },
    CLui { rd: u8, imm: u32 },
    CSrli { rd: u8, shamt: u8 },
    CSrai { rd: u8, shamt: u8 },
    CAndi { rd: u8, imm: i32 },
    CSub { rd: u8, rs2: u8 },
    CXor { rd: u8, rs2: u8 },
    COr { rd: u8, rs2: u8 },
    CAnd { rd: u8, rs2: u8 },
    CJ { imm: i32 },
    CBeqz { rs1: u8, imm: i32 },
    CBnez { rs1: u8, imm: i32 },
    CSlli { rd: u8, shamt: u8 },
    CLwsp { rd: u8, imm: i32 },
    CJr { rs1: u8 },
    CMv { rd: u8, rs2: u8 },
    CEbreak,
    CJalr { rs1: u8 },
    CAdd { rd: u8, rs2: u8 },
    CSwsp { rs2: u8, imm: i32 },
}

impl core::fmt::Display for Instruction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Instruction::Lui { rd, imm } => write!(f, "lui {}, {:#x}", reg_name(rd), imm),
            Instruction::Auipc { rd, imm } => write!(f, "auipc {}, {:#x}", reg_name(rd), imm),
            Instruction::Jal { rd, imm } => write!(f, "jal {}, {}", reg_name(rd), imm),
            Instruction::Jalr { rd, rs1, imm } => {
                write!(f, "jalr {}, {}({})", reg_name(rd), imm, reg_name(rs1))
            }
            Instruction::Addi { rd, rs1, imm } => {
                write!(f, "addi {}, {}, {}", reg_name(rd), reg_name(rs1), imm)
            }
            Instruction::Slti { rd, rs1, imm } => {
                write!(f, "slti {}, {}, {}", reg_name(rd), reg_name(rs1), imm)
            }
            Instruction::Sltiu { rd, rs1, imm } => {
                write!(f, "sltiu {}, {}, {}", reg_name(rd), reg_name(rs1), imm)
            }
            Instruction::Xori { rd, rs1, imm } => {
                write!(f, "xori {}, {}, {}", reg_name(rd), reg_name(rs1), imm)
            }
            Instruction::Ori { rd, rs1, imm } => {
                write!(f, "ori {}, {}, {}", reg_name(rd), reg_name(rs1), imm)
            }
            Instruction::Andi { rd, rs1, imm } => {
                write!(f, "andi {}, {}, {}", reg_name(rd), reg_name(rs1), imm)
            }
            Instruction::Slli { rd, rs1, shamt } => {
                write!(f, "slli {}, {}, {}", reg_name(rd), reg_name(rs1), shamt)
            }
            Instruction::Srli { rd, rs1, shamt } => {
                write!(f, "srli {}, {}, {}", reg_name(rd), reg_name(rs1), shamt)
            }
            Instruction::Srai { rd, rs1, shamt } => {
                write!(f, "srai {}, {}, {}", reg_name(rd), reg_name(rs1), shamt)
            }
            Instruction::Lb { rd, rs1, imm } => {
                write!(f, "lb {}, {}({})", reg_name(rd), imm, reg_name(rs1))
            }
            Instruction::Lh { rd, rs1, imm } => {
                write!(f, "lh {}, {}({})", reg_name(rd), imm, reg_name(rs1))
            }
            Instruction::Lw { rd, rs1, imm } => {
                write!(f, "lw {}, {}({})", reg_name(rd), imm, reg_name(rs1))
            }
            Instruction::Lbu { rd, rs1, imm } => {
                write!(f, "lbu {}, {}({})", reg_name(rd), imm, reg_name(rs1))
            }
            Instruction::Lhu { rd, rs1, imm } => {
                write!(f, "lhu {}, {}({})", reg_name(rd), imm, reg_name(rs1))
            }
            Instruction::Sb { rs1, rs2, imm } => {
                write!(f, "sb {}, {}({})", reg_name(rs2), imm, reg_name(rs1))
            }
            Instruction::Sh { rs1, rs2, imm } => {
                write!(f, "sh {}, {}({})", reg_name(rs2), imm, reg_name(rs1))
            }
            Instruction::Sw { rs1, rs2, imm } => {
                write!(f, "sw {}, {}({})", reg_name(rs2), imm, reg_name(rs1))
            }
            Instruction::Beq { rs1, rs2, imm } => {
                write!(f, "beq {}, {}, {}", reg_name(rs1), reg_name(rs2), imm)
            }
            Instruction::Bne { rs1, rs2, imm } => {
                write!(f, "bne {}, {}, {}", reg_name(rs1), reg_name(rs2), imm)
            }
            Instruction::Blt { rs1, rs2, imm } => {
                write!(f, "blt {}, {}, {}", reg_name(rs1), reg_name(rs2), imm)
            }
            Instruction::Bge { rs1, rs2, imm } => {
                write!(f, "bge {}, {}, {}", reg_name(rs1), reg_name(rs2), imm)
            }
            Instruction::Bltu { rs1, rs2, imm } => {
                write!(f, "bltu {}, {}, {}", reg_name(rs1), reg_name(rs2), imm)
            }
            Instruction::Bgeu { rs1, rs2, imm } => {
                write!(f, "bgeu {}, {}, {}", reg_name(rs1), reg_name(rs2), imm)
            }
            Instruction::Add { rd, rs1, rs2 } => {
                write!(f, "add {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                write!(f, "sub {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                write!(f, "sll {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                write!(f, "slt {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Sltu { rd, rs1, rs2 } => {
                write!(f, "sltu {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                write!(f, "xor {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                write!(f, "srl {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                write!(f, "sra {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Or { rd, rs1, rs2 } => {
                write!(f, "or {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::And { rd, rs1, rs2 } => {
                write!(f, "and {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2))
            }
            Instruction::Mul { rd, rs1, rs2 } => write!(f, "mul {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Mulh { rd, rs1, rs2 } => write!(f, "mulh {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Mulhsu { rd, rs1, rs2 } => write!(f, "mulhsu {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Mulhu { rd, rs1, rs2 } => write!(f, "mulhu {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Div { rd, rs1, rs2 } => write!(f, "div {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Divu { rd, rs1, rs2 } => write!(f, "divu {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Rem { rd, rs1, rs2 } => write!(f, "rem {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Remu { rd, rs1, rs2 } => write!(f, "remu {}, {}, {}", reg_name(rd), reg_name(rs1), reg_name(rs2)),
            Instruction::Fence => write!(f, "fence"),
            Instruction::Ecall => write!(f, "ecall"),
            Instruction::Ebreak => write!(f, "ebreak"),
            Instruction::Csrrw { rd, csr, rs1 } => write!(f, "csrrw {}, 0x{csr:03x}, {}", reg_name(rd), reg_name(rs1)),
            Instruction::Csrrs { rd, csr, rs1 } => write!(f, "csrrs {}, 0x{csr:03x}, {}", reg_name(rd), reg_name(rs1)),
            Instruction::Csrrc { rd, csr, rs1 } => write!(f, "csrrc {}, 0x{csr:03x}, {}", reg_name(rd), reg_name(rs1)),
            Instruction::Csrrwi { rd, csr, zimm } => write!(f, "csrrwi {}, 0x{csr:03x}, {zimm}", reg_name(rd)),
            Instruction::Csrrsi { rd, csr, zimm } => write!(f, "csrrsi {}, 0x{csr:03x}, {zimm}", reg_name(rd)),
            Instruction::Csrrci { rd, csr, zimm } => write!(f, "csrrci {}, 0x{csr:03x}, {zimm}", reg_name(rd)),
            Instruction::Mret => write!(f, "mret"),
            // ---- C extension ----
            Instruction::CAddi4spn { rd, imm } => write!(f, "c.addi4spn {}, {imm}", reg_name(rd)),
            Instruction::CLw { rd, rs1, imm } => write!(f, "c.lw {}, {}({})", reg_name(rd), imm, reg_name(rs1)),
            Instruction::CSw { rs1, rs2, imm } => write!(f, "c.sw {}, {}({})", reg_name(rs2), imm, reg_name(rs1)),
            Instruction::CAddi { rd, imm } => write!(f, "c.addi {}, {imm}", reg_name(rd)),
            Instruction::CJal { imm } => write!(f, "c.jal {imm}"),
            Instruction::CLi { rd, imm } => write!(f, "c.li {}, {imm}", reg_name(rd)),
            Instruction::CAddi16sp { imm } => write!(f, "c.addi16sp {imm}"),
            Instruction::CLui { rd, imm } => write!(f, "c.lui {}, {:#x}", reg_name(rd), imm),
            Instruction::CSrli { rd, shamt } => write!(f, "c.srli {}, {shamt}", reg_name(rd)),
            Instruction::CSrai { rd, shamt } => write!(f, "c.srai {}, {shamt}", reg_name(rd)),
            Instruction::CAndi { rd, imm } => write!(f, "c.andi {}, {imm}", reg_name(rd)),
            Instruction::CSub { rd, rs2 } => write!(f, "c.sub {}, {}", reg_name(rd), reg_name(rs2)),
            Instruction::CXor { rd, rs2 } => write!(f, "c.xor {}, {}", reg_name(rd), reg_name(rs2)),
            Instruction::COr { rd, rs2 } => write!(f, "c.or {}, {}", reg_name(rd), reg_name(rs2)),
            Instruction::CAnd { rd, rs2 } => write!(f, "c.and {}, {}", reg_name(rd), reg_name(rs2)),
            Instruction::CJ { imm } => write!(f, "c.j {imm}"),
            Instruction::CBeqz { rs1, imm } => write!(f, "c.beqz {}, {imm}", reg_name(rs1)),
            Instruction::CBnez { rs1, imm } => write!(f, "c.bnez {}, {imm}", reg_name(rs1)),
            Instruction::CSlli { rd, shamt } => write!(f, "c.slli {}, {shamt}", reg_name(rd)),
            Instruction::CLwsp { rd, imm } => write!(f, "c.lwsp {}, {}(sp)", reg_name(rd), imm),
            Instruction::CJr { rs1 } => write!(f, "c.jr {}", reg_name(rs1)),
            Instruction::CMv { rd, rs2 } => write!(f, "c.mv {}, {}", reg_name(rd), reg_name(rs2)),
            Instruction::CEbreak => write!(f, "c.ebreak"),
            Instruction::CJalr { rs1 } => write!(f, "c.jalr {}", reg_name(rs1)),
            Instruction::CAdd { rd, rs2 } => write!(f, "c.add {}, {}", reg_name(rd), reg_name(rs2)),
            Instruction::CSwsp { rs2, imm } => write!(f, "c.swsp {}, {}(sp)", reg_name(rs2), imm),
        }
    }
}

/// 寄存器 ABI 别名表（x0..x31）。
pub const ABI_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// 取寄存器 i 的 ABI 别名（越界时回退为 xN）。
pub fn reg_name(i: u8) -> &'static str {
    ABI_NAMES
        .get(usize::from(i))
        .copied()
        .unwrap_or_else(|| unreachable_x(i))
}

// 辅助：寄存器编号越界（正常解码不会出现，防御性兜底）。
#[cold]
#[inline(never)]
fn unreachable_x(i: u8) -> &'static str {
    // 无法返回动态字符串的 &'static，这里以固定字符串兜底，仅在非法输入时出现
    let _ = i;
    "x?"
}

/// CSR 地址常量（最简 M-mode 子集）。
pub mod csr {
    pub const MSTATUS: u16 = 0x300;
    pub const MISA: u16 = 0x301;
    pub const MIE: u16 = 0x304;
    pub const MTVEC: u16 = 0x305;
    pub const MSCRATCH: u16 = 0x340;
    pub const MEPC: u16 = 0x341;
    pub const MCAUSE: u16 = 0x342;
    pub const MTVAL: u16 = 0x343;
    pub const MIP: u16 = 0x344;
    pub const MCYCLE: u16 = 0xB00;
    pub const MINSTRET: u16 = 0xB02;
    pub const MHARTID: u16 = 0xF14; // 只读

    // mstatus 位段
    pub const MSTATUS_MIE: u32 = 1 << 3;
    pub const MSTATUS_MPIE: u32 = 1 << 7;
    pub const MSTATUS_MPP_SHIFT: u32 = 11;
    pub const MSTATUS_MPP_MASK: u32 = 0b11 << MSTATUS_MPP_SHIFT;
    pub const MSTATUS_MPP_U: u32 = 0b00 << MSTATUS_MPP_SHIFT;
    pub const MSTATUS_MPP_M: u32 = 0b11 << MSTATUS_MPP_SHIFT;
}

/// 异常/中断 cause 值（M1 用到的异常码）。
pub mod cause {
    pub const INSTR_MISALIGNED: u32 = 0;
    pub const INSTR_ACCESS_FAULT: u32 = 1;
    pub const ILLEGAL_INSTR: u32 = 2;
    pub const BREAKPOINT: u32 = 3;
    pub const LOAD_MISALIGNED: u32 = 4;
    pub const LOAD_ACCESS_FAULT: u32 = 5;
    pub const STORE_MISALIGNED: u32 = 6;
    pub const STORE_ACCESS_FAULT: u32 = 7;
    pub const ECALL_FROM_U: u32 = 8;
    pub const ECALL_FROM_M: u32 = 11;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    // Display 单测（≥6 例，覆盖各类格式）；直接构造变体（decode 依赖方向是 decode→isa）
    #[test]
    fn display_r_type() {
        assert_eq!(Instruction::Add { rd: 10, rs1: 11, rs2: 12 }.to_string(), "add a0, a1, a2");
        assert_eq!(Instruction::Sub { rd: 5, rs1: 6, rs2: 7 }.to_string(), "sub t0, t1, t2");
    }

    #[test]
    fn display_addi_offset() {
        assert_eq!(Instruction::Addi { rd: 2, rs1: 2, imm: 16 }.to_string(), "addi sp, sp, 16");
        assert_eq!(Instruction::Addi { rd: 2, rs1: 2, imm: -16 }.to_string(), "addi sp, sp, -16");
    }

    #[test]
    fn display_load_store() {
        assert_eq!(Instruction::Lw { rd: 10, rs1: 2, imm: 4 }.to_string(), "lw a0, 4(sp)");
        assert_eq!(Instruction::Sb { rs1: 2, rs2: 11, imm: -8 }.to_string(), "sb a1, -8(sp)");
    }

    #[test]
    fn display_branch_negative_offset() {
        assert_eq!(Instruction::Beq { rs1: 10, rs2: 11, imm: -12 }.to_string(), "beq a0, a1, -12");
        assert_eq!(Instruction::Bgeu { rs1: 1, rs2: 2, imm: 1024 }.to_string(), "bgeu ra, sp, 1024");
    }

    #[test]
    fn display_lui_hex() {
        assert_eq!(Instruction::Lui { rd: 10, imm: 0x12345 }.to_string(), "lui a0, 0x12345");
        assert_eq!(Instruction::Auipc { rd: 0, imm: 0x0 }.to_string(), "auipc zero, 0x0");
    }

    #[test]
    fn display_shift_jal() {
        assert_eq!(Instruction::Srai { rd: 10, rs1: 11, shamt: 31 }.to_string(), "srai a0, a1, 31");
        assert_eq!(Instruction::Jal { rd: 1, imm: 2048 }.to_string(), "jal ra, 2048");
        assert_eq!(Instruction::Jalr { rd: 1, rs1: 5, imm: 4 }.to_string(), "jalr ra, 4(t0)");
    }

    #[test]
    fn display_system() {
        assert_eq!(Instruction::Ecall.to_string(), "ecall");
        assert_eq!(Instruction::Ebreak.to_string(), "ebreak");
        assert_eq!(Instruction::Fence.to_string(), "fence");
    }

    #[test]
    fn display_m_extension() {
        assert_eq!(Instruction::Mul { rd: 10, rs1: 11, rs2: 12 }.to_string(), "mul a0, a1, a2");
        assert_eq!(Instruction::Mulh { rd: 10, rs1: 11, rs2: 12 }.to_string(), "mulh a0, a1, a2");
        assert_eq!(Instruction::Mulhsu { rd: 10, rs1: 11, rs2: 12 }.to_string(), "mulhsu a0, a1, a2");
        assert_eq!(Instruction::Mulhu { rd: 10, rs1: 11, rs2: 12 }.to_string(), "mulhu a0, a1, a2");
        assert_eq!(Instruction::Div { rd: 10, rs1: 11, rs2: 12 }.to_string(), "div a0, a1, a2");
        assert_eq!(Instruction::Divu { rd: 10, rs1: 11, rs2: 12 }.to_string(), "divu a0, a1, a2");
        assert_eq!(Instruction::Rem { rd: 10, rs1: 11, rs2: 12 }.to_string(), "rem a0, a1, a2");
        assert_eq!(Instruction::Remu { rd: 10, rs1: 11, rs2: 12 }.to_string(), "remu a0, a1, a2");
    }

    #[test]
    fn abi_names() {
        assert_eq!(reg_name(0), "zero");
        assert_eq!(reg_name(2), "sp");
        assert_eq!(reg_name(8), "s0");
        assert_eq!(reg_name(10), "a0");
        assert_eq!(reg_name(31), "t6");
    }

    #[test]
    fn display_m_c_zicsr_instructions() {
        // M 扩展（代表性变体：mul / div / remu）
        assert_eq!(Instruction::Mul { rd: 10, rs1: 11, rs2: 12 }.to_string(), "mul a0, a1, a2");
        assert_eq!(Instruction::Div { rd: 10, rs1: 11, rs2: 12 }.to_string(), "div a0, a1, a2");
        assert_eq!(Instruction::Remu { rd: 10, rs1: 11, rs2: 12 }.to_string(), "remu a0, a1, a2");
        // C 扩展
        assert_eq!(Instruction::CLw { rd: 10, rs1: 11, imm: 4 }.to_string(), "c.lw a0, 4(a1)");
        assert_eq!(Instruction::CJ { imm: 100 }.to_string(), "c.j 100");
        assert_eq!(Instruction::CAddi { rd: 2, imm: 16 }.to_string(), "c.addi sp, 16");
        assert_eq!(Instruction::CBeqz { rs1: 10, imm: 8 }.to_string(), "c.beqz a0, 8");
        assert_eq!(Instruction::CSwsp { rs2: 1, imm: 4 }.to_string(), "c.swsp ra, 4(sp)");
        // Zicsr（csrrw 用寄存器源；csrrsi 用 zimm）
        assert_eq!(Instruction::Csrrw { rd: 10, csr: 0x300, rs1: 11 }.to_string(), "csrrw a0, 0x300, a1");
        assert_eq!(Instruction::Csrrsi { rd: 10, csr: 0x340, zimm: 5 }.to_string(), "csrrsi a0, 0x340, 5");
    }
}
