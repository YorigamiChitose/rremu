//! rremu-isa：ISA 层 —— 基础类型、指令枚举、CSR 常量、特权级定义。
//! 零外部依赖、零 IO 的叶子 crate。

pub mod cause;
pub mod csr;
pub mod instruction;
pub mod types;

pub use instruction::Instruction;
pub use types::{ABI_NAMES, Address, FP_ABI_NAMES, Priv, Word, freg_name, reg_name};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    // Display 单测（≥6 例，覆盖各类格式）；直接构造变体（decode 依赖方向是 decode→isa）
    #[test]
    fn display_r_type() {
        assert_eq!(
            Instruction::Add {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "add a0, a1, a2"
        );
        assert_eq!(
            Instruction::Sub {
                rd: 5,
                rs1: 6,
                rs2: 7
            }
            .to_string(),
            "sub t0, t1, t2"
        );
    }

    #[test]
    fn display_addi_offset() {
        assert_eq!(
            Instruction::Addi {
                rd: 2,
                rs1: 2,
                imm: 16
            }
            .to_string(),
            "addi sp, sp, 16"
        );
        assert_eq!(
            Instruction::Addi {
                rd: 2,
                rs1: 2,
                imm: -16
            }
            .to_string(),
            "addi sp, sp, -16"
        );
    }

    #[test]
    fn display_load_store() {
        assert_eq!(
            Instruction::Lw {
                rd: 10,
                rs1: 2,
                imm: 4
            }
            .to_string(),
            "lw a0, 4(sp)"
        );
        assert_eq!(
            Instruction::Sb {
                rs1: 2,
                rs2: 11,
                imm: -8
            }
            .to_string(),
            "sb a1, -8(sp)"
        );
    }

    #[test]
    fn display_branch_negative_offset() {
        assert_eq!(
            Instruction::Beq {
                rs1: 10,
                rs2: 11,
                imm: -12
            }
            .to_string(),
            "beq a0, a1, -12"
        );
        assert_eq!(
            Instruction::Bgeu {
                rs1: 1,
                rs2: 2,
                imm: 1024
            }
            .to_string(),
            "bgeu ra, sp, 1024"
        );
    }

    #[test]
    fn display_lui_hex() {
        assert_eq!(
            Instruction::Lui {
                rd: 10,
                imm: 0x12345
            }
            .to_string(),
            "lui a0, 0x12345"
        );
        assert_eq!(
            Instruction::Auipc { rd: 0, imm: 0x0 }.to_string(),
            "auipc zero, 0x0"
        );
    }

    #[test]
    fn display_shift_jal() {
        assert_eq!(
            Instruction::Srai {
                rd: 10,
                rs1: 11,
                shamt: 31
            }
            .to_string(),
            "srai a0, a1, 31"
        );
        assert_eq!(
            Instruction::Jal { rd: 1, imm: 2048 }.to_string(),
            "jal ra, 2048"
        );
        assert_eq!(
            Instruction::Jalr {
                rd: 1,
                rs1: 5,
                imm: 4
            }
            .to_string(),
            "jalr ra, 4(t0)"
        );
    }

    #[test]
    fn display_fp_arithmetic() {
        assert_eq!(
            Instruction::FaddS {
                rd: 10,
                rs1: 11,
                rs2: 12,
                rm: 0
            }
            .to_string(),
            "fadd.s fa0, fa1, fa2, rne"
        );
        assert_eq!(
            Instruction::FdivD {
                rd: 10,
                rs1: 11,
                rs2: 12,
                rm: 4
            }
            .to_string(),
            "fdiv.d fa0, fa1, fa2, rmm"
        );
        assert_eq!(
            Instruction::FsqrtS {
                rd: 10,
                rs1: 11,
                rm: 7
            }
            .to_string(),
            "fsqrt.s fa0, fa1, dyn"
        );
    }

    #[test]
    fn display_fp_minmax() {
        assert_eq!(
            Instruction::FminS {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "fmin.s fa0, fa1, fa2"
        );
        assert_eq!(
            Instruction::FmaxD {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "fmax.d fa0, fa1, fa2"
        );
    }

    #[test]
    fn display_fp_fma() {
        assert_eq!(
            Instruction::FmaddS {
                rd: 10,
                rs1: 11,
                rs2: 12,
                rs3: 13,
                rm: 0
            }
            .to_string(),
            "fmadd.s fa0, fa1, fa2, fa3, rne"
        );
        assert_eq!(
            Instruction::FnmsubD {
                rd: 10,
                rs1: 11,
                rs2: 12,
                rs3: 13,
                rm: 7
            }
            .to_string(),
            "fnmsub.d fa0, fa1, fa2, fa3, dyn"
        );
    }

    #[test]
    fn display_fp_conversions() {
        assert_eq!(
            Instruction::FcvtWS {
                rd: 10,
                rs1: 11,
                rm: 0
            }
            .to_string(),
            "fcvt.w.s a0, fa1, rne"
        );
        assert_eq!(
            Instruction::FcvtWuD {
                rd: 10,
                rs1: 11,
                rm: 7
            }
            .to_string(),
            "fcvt.wu.d a0, fa1, dyn"
        );
        assert_eq!(
            Instruction::FcvtSWu {
                rd: 10,
                rs1: 11,
                rm: 1
            }
            .to_string(),
            "fcvt.s.wu fa0, a1, rtz"
        );
        assert_eq!(
            Instruction::FcvtSD {
                rd: 10,
                rs1: 11,
                rm: 4
            }
            .to_string(),
            "fcvt.s.d fa0, fa1, rmm"
        );
    }

    #[test]
    fn display_system() {
        assert_eq!(Instruction::Ecall.to_string(), "ecall");
        assert_eq!(Instruction::Ebreak.to_string(), "ebreak");
        assert_eq!(Instruction::Fence.to_string(), "fence");
    }

    #[test]
    fn display_m_extension() {
        assert_eq!(
            Instruction::Mul {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "mul a0, a1, a2"
        );
        assert_eq!(
            Instruction::Mulh {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "mulh a0, a1, a2"
        );
        assert_eq!(
            Instruction::Mulhsu {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "mulhsu a0, a1, a2"
        );
        assert_eq!(
            Instruction::Mulhu {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "mulhu a0, a1, a2"
        );
        assert_eq!(
            Instruction::Div {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "div a0, a1, a2"
        );
        assert_eq!(
            Instruction::Divu {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "divu a0, a1, a2"
        );
        assert_eq!(
            Instruction::Rem {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "rem a0, a1, a2"
        );
        assert_eq!(
            Instruction::Remu {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "remu a0, a1, a2"
        );
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
        assert_eq!(
            Instruction::Mul {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "mul a0, a1, a2"
        );
        assert_eq!(
            Instruction::Div {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "div a0, a1, a2"
        );
        assert_eq!(
            Instruction::Remu {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "remu a0, a1, a2"
        );
        // C 扩展
        assert_eq!(
            Instruction::CLw {
                rd: 10,
                rs1: 11,
                imm: 4
            }
            .to_string(),
            "c.lw a0, 4(a1)"
        );
        assert_eq!(Instruction::CJ { imm: 100 }.to_string(), "c.j 100");
        assert_eq!(
            Instruction::CAddi { rd: 2, imm: 16 }.to_string(),
            "c.addi sp, 16"
        );
        assert_eq!(
            Instruction::CBeqz { rs1: 10, imm: 8 }.to_string(),
            "c.beqz a0, 8"
        );
        assert_eq!(
            Instruction::CSwsp { rs2: 1, imm: 4 }.to_string(),
            "c.swsp ra, 4(sp)"
        );
        // Zicsr（csrrw 用寄存器源；csrrsi 用 zimm）
        assert_eq!(
            Instruction::Csrrw {
                rd: 10,
                csr: 0x300,
                rs1: 11
            }
            .to_string(),
            "csrrw a0, 0x300, a1"
        );
        assert_eq!(
            Instruction::Csrrsi {
                rd: 10,
                csr: 0x340,
                zimm: 5
            }
            .to_string(),
            "csrrsi a0, 0x340, 5"
        );
    }

    #[test]
    fn cause_name_known() {
        assert_eq!(
            cause::cause_name(cause::ILLEGAL_INSTR),
            "illegal instruction"
        );
        assert_eq!(
            cause::cause_name(cause::LOAD_ACCESS_FAULT),
            "load access fault"
        );
        assert_eq!(
            cause::cause_name(cause::ECALL_FROM_M),
            "environment call from M-mode"
        );
        assert_eq!(
            cause::cause_name(cause::ECALL_FROM_S),
            "environment call from S-mode"
        );
        assert_eq!(
            cause::cause_name(cause::INSTR_PAGE_FAULT),
            "instruction page fault"
        );
        assert_eq!(cause::cause_name(cause::LOAD_PAGE_FAULT), "load page fault");
        assert_eq!(
            cause::cause_name(cause::STORE_PAGE_FAULT),
            "store/AMO page fault"
        );
        assert_eq!(cause::cause_name(cause::M_TI), "machine timer interrupt");
        assert_eq!(cause::cause_name(cause::M_SI), "machine software interrupt");
        assert_eq!(cause::cause_name(cause::S_TI), "supervisor timer interrupt");
        assert_eq!(
            cause::cause_name(cause::S_SI),
            "supervisor software interrupt"
        );
        assert_eq!(
            cause::cause_name(cause::S_EI),
            "supervisor external interrupt"
        );
        assert_eq!(cause::cause_name(99), "unknown exception");
        assert_eq!(cause::cause_name(0x8000_0000 | 99), "unknown interrupt");
    }

    #[test]
    fn fp_abi_names() {
        assert_eq!(FP_ABI_NAMES.len(), 32);
        assert_eq!(freg_name(0), "ft0");
        assert_eq!(freg_name(8), "fs0");
        assert_eq!(freg_name(10), "fa0");
        assert_eq!(freg_name(27), "fs11");
        assert_eq!(freg_name(31), "ft11");
        assert_eq!(freg_name(32), "f?");
    }

    #[test]
    fn display_a_extension_and_fence_i() {
        assert_eq!(
            Instruction::LrW { rd: 10, rs1: 11 }.to_string(),
            "lr.w a0, (a1)"
        );
        assert_eq!(
            Instruction::ScW {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "sc.w a0, a2, (a1)"
        );
        assert_eq!(
            Instruction::AmoswapW {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "amoswap.w a0, a2, (a1)"
        );
        assert_eq!(
            Instruction::AmoaddW {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "amoadd.w a0, a2, (a1)"
        );
        assert_eq!(
            Instruction::AmominuW {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "amominu.w a0, a2, (a1)"
        );
        assert_eq!(
            Instruction::AmomaxuW {
                rd: 10,
                rs1: 11,
                rs2: 12
            }
            .to_string(),
            "amomaxu.w a0, a2, (a1)"
        );
        assert_eq!(Instruction::FenceI.to_string(), "fence.i");
    }
}
