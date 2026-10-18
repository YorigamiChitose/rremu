//! RV32I/M/A/C/Zicsr 指令枚举与显示。

use crate::types::reg_name;

/// RV32I 指令枚举；M/C/Zicsr 变体见下方分组。
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
    // ---- A extension (RV32A) ----
    LrW { rd: u8, rs1: u8 },
    ScW { rd: u8, rs1: u8, rs2: u8 },
    AmoswapW { rd: u8, rs1: u8, rs2: u8 },
    AmoaddW { rd: u8, rs1: u8, rs2: u8 },
    AmoxorW { rd: u8, rs1: u8, rs2: u8 },
    AmoandW { rd: u8, rs1: u8, rs2: u8 },
    AmoorW { rd: u8, rs1: u8, rs2: u8 },
    AmominW { rd: u8, rs1: u8, rs2: u8 },
    AmomaxW { rd: u8, rs1: u8, rs2: u8 },
    AmominuW { rd: u8, rs1: u8, rs2: u8 },
    AmomaxuW { rd: u8, rs1: u8, rs2: u8 },
    // ---- misc / system ----
    Fence,
    FenceI,
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
    Sret,
    Wfi,
    SfenceVma,
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
            Instruction::LrW { rd, rs1 } => write!(f, "lr.w {}, ({})", reg_name(rd), reg_name(rs1)),
            Instruction::ScW { rd, rs1, rs2 } => write!(f, "sc.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmoswapW { rd, rs1, rs2 } => write!(f, "amoswap.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmoaddW { rd, rs1, rs2 } => write!(f, "amoadd.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmoxorW { rd, rs1, rs2 } => write!(f, "amoxor.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmoandW { rd, rs1, rs2 } => write!(f, "amoand.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmoorW { rd, rs1, rs2 } => write!(f, "amoor.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmominW { rd, rs1, rs2 } => write!(f, "amomin.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmomaxW { rd, rs1, rs2 } => write!(f, "amomax.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmominuW { rd, rs1, rs2 } => write!(f, "amominu.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::AmomaxuW { rd, rs1, rs2 } => write!(f, "amomaxu.w {}, {}, ({})", reg_name(rd), reg_name(rs2), reg_name(rs1)),
            Instruction::Fence => write!(f, "fence"),
            Instruction::FenceI => write!(f, "fence.i"),
            Instruction::Ecall => write!(f, "ecall"),
            Instruction::Ebreak => write!(f, "ebreak"),
            Instruction::Csrrw { rd, csr, rs1 } => write!(f, "csrrw {}, 0x{csr:03x}, {}", reg_name(rd), reg_name(rs1)),
            Instruction::Csrrs { rd, csr, rs1 } => write!(f, "csrrs {}, 0x{csr:03x}, {}", reg_name(rd), reg_name(rs1)),
            Instruction::Csrrc { rd, csr, rs1 } => write!(f, "csrrc {}, 0x{csr:03x}, {}", reg_name(rd), reg_name(rs1)),
            Instruction::Csrrwi { rd, csr, zimm } => write!(f, "csrrwi {}, 0x{csr:03x}, {zimm}", reg_name(rd)),
            Instruction::Csrrsi { rd, csr, zimm } => write!(f, "csrrsi {}, 0x{csr:03x}, {zimm}", reg_name(rd)),
            Instruction::Csrrci { rd, csr, zimm } => write!(f, "csrrci {}, 0x{csr:03x}, {zimm}", reg_name(rd)),
            Instruction::Mret => write!(f, "mret"),
            Instruction::Sret => write!(f, "sret"),
            Instruction::Wfi => write!(f, "wfi"),
            Instruction::SfenceVma => write!(f, "sfence.vma"),
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

