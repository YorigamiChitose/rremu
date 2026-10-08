//! rremu-decode：纯函数指令解码（instr_len / decode32 / decode16）。

use rremu_isa::{Address, Instruction, Word};
use thiserror::Error;

/// 指令长度（字节）：低 2 位 == 0b11 为 4 字节，否则 2 字节（RVC，M2 解码）。
#[inline]
pub fn instr_len(first_half: u16) -> usize {
    if first_half & 0b11 == 0b11 { 4 } else { 2 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// 非法编码（opcode/funct 字段不属于任何已实现指令）。
    #[error("illegal instruction {bits:#010x} at pc {pc:#x}")]
    Illegal { bits: Word, pc: Address },
    /// 编码属于尚未实现的指令类别。
    #[error("unimplemented instruction {bits:#010x} at pc {pc:#x}")]
    Unimplemented { bits: Word, pc: Address },
}

/// 解码 32 位指令。
pub fn decode32(word: Word, pc: Address) -> Result<Instruction, DecodeError> {
    let opcode = word & 0x7f;
    let rd = ((word >> 7) & 0x1f) as u8;
    let funct3 = ((word >> 12) & 0b111) as u8;
    let rs1 = ((word >> 15) & 0x1f) as u8;
    let rs2 = ((word >> 20) & 0x1f) as u8;
    let funct7 = word >> 25;
    let illegal = DecodeError::Illegal { bits: word, pc };
    let unimplemented = DecodeError::Unimplemented { bits: word, pc };

    match opcode {
        0b0110111 => Ok(Instruction::Lui { rd, imm: word & 0xffff_f000 }),
        0b0010111 => Ok(Instruction::Auipc { rd, imm: word & 0xffff_f000 }),
        0b1101111 => Ok(Instruction::Jal { rd, imm: imm_j(word) }),
        0b1100111 => {
            if funct3 == 0b000 {
                Ok(Instruction::Jalr { rd, rs1, imm: imm_i(word) })
            } else {
                Err(illegal)
            }
        }
        0b1100011 => {
            let imm = imm_b(word);
            match funct3 {
                0b000 => Ok(Instruction::Beq { rs1, rs2, imm }),
                0b001 => Ok(Instruction::Bne { rs1, rs2, imm }),
                0b100 => Ok(Instruction::Blt { rs1, rs2, imm }),
                0b101 => Ok(Instruction::Bge { rs1, rs2, imm }),
                0b110 => Ok(Instruction::Bltu { rs1, rs2, imm }),
                0b111 => Ok(Instruction::Bgeu { rs1, rs2, imm }),
                _ => Err(illegal),
            }
        }
        0b0000011 => {
            let imm = imm_i(word);
            match funct3 {
                0b000 => Ok(Instruction::Lb { rd, rs1, imm }),
                0b001 => Ok(Instruction::Lh { rd, rs1, imm }),
                0b010 => Ok(Instruction::Lw { rd, rs1, imm }),
                0b100 => Ok(Instruction::Lbu { rd, rs1, imm }),
                0b101 => Ok(Instruction::Lhu { rd, rs1, imm }),
                _ => Err(illegal),
            }
        }
        0b0100011 => {
            let imm = imm_s(word);
            match funct3 {
                0b000 => Ok(Instruction::Sb { rs1, rs2, imm }),
                0b001 => Ok(Instruction::Sh { rs1, rs2, imm }),
                0b010 => Ok(Instruction::Sw { rs1, rs2, imm }),
                _ => Err(illegal),
            }
        }
        0b0010011 => {
            let imm = imm_i(word);
            match funct3 {
                0b000 => Ok(Instruction::Addi { rd, rs1, imm }),
                0b010 => Ok(Instruction::Slti { rd, rs1, imm }),
                0b011 => Ok(Instruction::Sltiu { rd, rs1, imm }),
                0b100 => Ok(Instruction::Xori { rd, rs1, imm }),
                0b110 => Ok(Instruction::Ori { rd, rs1, imm }),
                0b111 => Ok(Instruction::Andi { rd, rs1, imm }),
                0b001 => {
                    if funct7 == 0 {
                        Ok(Instruction::Slli { rd, rs1, shamt: rs2 })
                    } else {
                        Err(illegal)
                    }
                }
                0b101 => match funct7 {
                    0b0000000 => Ok(Instruction::Srli { rd, rs1, shamt: rs2 }),
                    0b0100000 => Ok(Instruction::Srai { rd, rs1, shamt: rs2 }),
                    _ => Err(illegal),
                },
                _ => Err(illegal),
            }
        }
        0b0110011 => {
            match (funct3, funct7) {
                (0b000, 0b0000000) => Ok(Instruction::Add { rd, rs1, rs2 }),
                (0b000, 0b0100000) => Ok(Instruction::Sub { rd, rs1, rs2 }),
                (0b001, 0b0000000) => Ok(Instruction::Sll { rd, rs1, rs2 }),
                (0b010, 0b0000000) => Ok(Instruction::Slt { rd, rs1, rs2 }),
                (0b011, 0b0000000) => Ok(Instruction::Sltu { rd, rs1, rs2 }),
                (0b100, 0b0000000) => Ok(Instruction::Xor { rd, rs1, rs2 }),
                (0b101, 0b0000000) => Ok(Instruction::Srl { rd, rs1, rs2 }),
                (0b101, 0b0100000) => Ok(Instruction::Sra { rd, rs1, rs2 }),
                (0b110, 0b0000000) => Ok(Instruction::Or { rd, rs1, rs2 }),
                (0b111, 0b0000000) => Ok(Instruction::And { rd, rs1, rs2 }),
                _ => Err(illegal),
            }
        }
        0b0001111 => {
            if funct3 == 0b000 {
                Ok(Instruction::Fence)
            } else {
                Err(illegal)
            }
        }
        0b1110011 => {
            // RV32I 中仅 ecall/ebreak；Zicsr 变体在 M2 扩展
            if funct3 == 0b000 && rs1 == 0 && rd == 0 {
                match (word >> 20) & 0xfff {
                    0x000 => Ok(Instruction::Ecall),
                    0x001 => Ok(Instruction::Ebreak),
                    _ => Err(illegal),
                }
            } else {
                Err(unimplemented)
            }
        }
        _ => Err(illegal),
    }
}

/// I 型立即数（符号扩展 bits[31:20]）。
#[inline]
fn imm_i(word: Word) -> i32 {
    (word as i32) >> 20
}

/// S 型立即数 bits[31:25]|bits[11:7]。
#[inline]
fn imm_s(word: Word) -> i32 {
    ((word & 0xfe00_0000) as i32 >> 20) | ((word >> 7) as i32 & 0x1f)
}

/// B 型立即数 bit[31]|bit[7]|bits[30:25]|bits[11:8]<<1。
#[inline]
fn imm_b(word: Word) -> i32 {
    ((word & 0x8000_0000) as i32 >> 19)        // bit12
        | (((word >> 7) as i32 & 1) << 11)     // bit11
        | (((word >> 25) as i32 & 0x3f) << 5)  // bits10:5
        | (((word >> 8) as i32 & 0xf) << 1)    // bits4:1
}

/// J 型立即数 bit[31]|bits[19:12]|bit[20]|bits[30:21]<<1。
#[inline]
fn imm_j(word: Word) -> i32 {
    ((word & 0x8000_0000) as i32 >> 11)         // bit20
        | (((word >> 12) as i32 & 0xff) << 12)  // bits19:12
        | (((word >> 20) as i32 & 1) << 11)     // bit11
        | (((word >> 21) as i32 & 0x3ff) << 1)  // bits10:1
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    // ---- 编码辅助 ----
    fn i_enc(op: u32, rd: u8, f3: u32, rs1: u8, imm: i32) -> Word {
        ((imm as u32 & 0xfff) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | op
    }
    fn r_enc(rd: u8, f3: u32, rs1: u8, rs2: u8, f7: u32) -> Word {
        (f7 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | 0b0110011
    }
    fn s_enc(f3: u32, rs1: u8, rs2: u8, imm: i32) -> Word {
        let i = imm as u32;
        (((i >> 5) & 0x7f) << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | ((i & 0x1f) << 7) | 0b0100011
    }
    fn b_enc(f3: u32, rs1: u8, rs2: u8, off: i32) -> Word {
        let o = off as u32;
        let b12 = (o >> 12) & 1;
        let b11 = (o >> 11) & 1;
        let b10_5 = (o >> 5) & 0x3f;
        let b4_1 = (o >> 1) & 0xf;
        (b12 << 31) | (b10_5 << 25) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (b4_1 << 8) | (b11 << 7) | 0b1100011
    }
    fn u_enc(op: u32, rd: u8, imm20: u32) -> Word {
        (imm20 << 12) | (u32::from(rd) << 7) | op
    }
    fn j_enc(rd: u8, off: i32) -> Word {
        let o = off as u32;
        let b20 = (o >> 20) & 1;
        let b19_12 = (o >> 12) & 0xff;
        let b11 = (o >> 11) & 1;
        let b10_1 = (o >> 1) & 0x3ff;
        (b20 << 31) | (b10_1 << 21) | (b11 << 20) | (b19_12 << 12) | (u32::from(rd) << 7) | 0b1101111
    }

    const PC: Address = 0x8000_0000;

    #[test]
    fn instr_len_32_and_16() {
        assert_eq!(instr_len(0x0003), 4);
        assert_eq!(instr_len(0xffff), 4);
        assert_eq!(instr_len(0x0001), 2); // c.addi4spn 类低位
        assert_eq!(instr_len(0x0002), 2);
    }

    #[test]
    fn decode_u_type() {
        assert_eq!(decode32(u_enc(0b0110111, 10, 0x12345), PC), Ok(Instruction::Lui { rd: 10, imm: 0x1234_5000 }));
        assert_eq!(decode32(u_enc(0b0010111, 2, 0xfffff), PC), Ok(Instruction::Auipc { rd: 2, imm: 0xffff_f000 }));
    }

    #[test]
    fn decode_jal_jalr() {
        assert_eq!(decode32(j_enc(1, 2048), PC), Ok(Instruction::Jal { rd: 1, imm: 2048 }));
        assert_eq!(decode32(j_enc(1, -4096), PC), Ok(Instruction::Jal { rd: 1, imm: -4096 }));
        assert_eq!(decode32(i_enc(0b1100111, 1, 0, 5, -8), PC), Ok(Instruction::Jalr { rd: 1, rs1: 5, imm: -8 }));
        // jalr funct3 != 0 非法
        assert!(decode32(i_enc(0b1100111, 1, 1, 5, 0), PC).is_err());
    }

    #[test]
    fn decode_i_imm_ops() {
        assert_eq!(decode32(i_enc(0b0010011, 2, 0b000, 2, 16), PC), Ok(Instruction::Addi { rd: 2, rs1: 2, imm: 16 }));
        assert_eq!(decode32(i_enc(0b0010011, 3, 0b010, 4, -1), PC), Ok(Instruction::Slti { rd: 3, rs1: 4, imm: -1 }));
        assert_eq!(decode32(i_enc(0b0010011, 3, 0b011, 4, -1), PC), Ok(Instruction::Sltiu { rd: 3, rs1: 4, imm: -1 }));
        assert_eq!(decode32(i_enc(0b0010011, 5, 0b100, 6, 0x7ff), PC), Ok(Instruction::Xori { rd: 5, rs1: 6, imm: 0x7ff }));
        assert_eq!(decode32(i_enc(0b0010011, 5, 0b110, 6, -16), PC), Ok(Instruction::Ori { rd: 5, rs1: 6, imm: -16 }));
        assert_eq!(decode32(i_enc(0b0010011, 5, 0b111, 6, 0x0f), PC), Ok(Instruction::Andi { rd: 5, rs1: 6, imm: 0x0f }));
    }

    #[test]
    fn decode_i_shifts() {
        assert_eq!(decode32(i_enc(0b0010011, 1, 0b001, 2, 3), PC), Ok(Instruction::Slli { rd: 1, rs1: 2, shamt: 3 }));
        assert_eq!(decode32(i_enc(0b0010011, 1, 0b101, 2, 5), PC), Ok(Instruction::Srli { rd: 1, rs1: 2, shamt: 5 }));
        assert_eq!(decode32(i_enc(0b0010011, 1, 0b101, 2, 31) | (0b0100000 << 25), PC), Ok(Instruction::Srai { rd: 1, rs1: 2, shamt: 31 }));
        // slli funct7 != 0 非法
        assert!(decode32(i_enc(0b0010011, 1, 0b001, 2, 3) | (1 << 25), PC).is_err());
    }

    #[test]
    fn decode_loads() {
        assert_eq!(decode32(i_enc(0b0000011, 10, 0b000, 2, -1), PC), Ok(Instruction::Lb { rd: 10, rs1: 2, imm: -1 }));
        assert_eq!(decode32(i_enc(0b0000011, 10, 0b001, 2, 2), PC), Ok(Instruction::Lh { rd: 10, rs1: 2, imm: 2 }));
        assert_eq!(decode32(i_enc(0b0000011, 10, 0b010, 2, 4), PC), Ok(Instruction::Lw { rd: 10, rs1: 2, imm: 4 }));
        assert_eq!(decode32(i_enc(0b0000011, 10, 0b100, 2, 0), PC), Ok(Instruction::Lbu { rd: 10, rs1: 2, imm: 0 }));
        assert_eq!(decode32(i_enc(0b0000011, 10, 0b101, 2, 6), PC), Ok(Instruction::Lhu { rd: 10, rs1: 2, imm: 6 }));
    }

    #[test]
    fn decode_stores() {
        assert_eq!(decode32(s_enc(0b000, 2, 11, -8), PC), Ok(Instruction::Sb { rs1: 2, rs2: 11, imm: -8 }));
        assert_eq!(decode32(s_enc(0b001, 2, 11, 4), PC), Ok(Instruction::Sh { rs1: 2, rs2: 11, imm: 4 }));
        assert_eq!(decode32(s_enc(0b010, 2, 11, 2047), PC), Ok(Instruction::Sw { rs1: 2, rs2: 11, imm: 2047 }));
    }

    #[test]
    fn decode_branches() {
        assert_eq!(decode32(b_enc(0b000, 1, 2, -4096), PC), Ok(Instruction::Beq { rs1: 1, rs2: 2, imm: -4096 }));
        assert_eq!(decode32(b_enc(0b001, 1, 2, 4094), PC), Ok(Instruction::Bne { rs1: 1, rs2: 2, imm: 4094 }));
        assert_eq!(decode32(b_enc(0b100, 1, 2, -2), PC), Ok(Instruction::Blt { rs1: 1, rs2: 2, imm: -2 }));
        assert_eq!(decode32(b_enc(0b101, 1, 2, 8), PC), Ok(Instruction::Bge { rs1: 1, rs2: 2, imm: 8 }));
        assert_eq!(decode32(b_enc(0b110, 1, 2, 16), PC), Ok(Instruction::Bltu { rs1: 1, rs2: 2, imm: 16 }));
        assert_eq!(decode32(b_enc(0b111, 1, 2, -16), PC), Ok(Instruction::Bgeu { rs1: 1, rs2: 2, imm: -16 }));
    }

    #[test]
    fn decode_r_type_all_10() {
        assert_eq!(decode32(r_enc(3, 0b000, 1, 2, 0b0000000), PC), Ok(Instruction::Add { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b000, 1, 2, 0b0100000), PC), Ok(Instruction::Sub { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b001, 1, 2, 0b0000000), PC), Ok(Instruction::Sll { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b010, 1, 2, 0b0000000), PC), Ok(Instruction::Slt { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b011, 1, 2, 0b0000000), PC), Ok(Instruction::Sltu { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b100, 1, 2, 0b0000000), PC), Ok(Instruction::Xor { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b101, 1, 2, 0b0000000), PC), Ok(Instruction::Srl { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b101, 1, 2, 0b0100000), PC), Ok(Instruction::Sra { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b110, 1, 2, 0b0000000), PC), Ok(Instruction::Or { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(r_enc(3, 0b111, 1, 2, 0b0000000), PC), Ok(Instruction::And { rd: 3, rs1: 1, rs2: 2 }));
    }

    #[test]
    fn decode_misc_system() {
        assert_eq!(decode32(i_enc(0b0001111, 0, 0b000, 0, 0), PC), Ok(Instruction::Fence));
        // ecall: imm=0
        let ecall = i_enc(0b1110011, 0, 0b000, 0, 0);
        assert_eq!(decode32(ecall, PC), Ok(Instruction::Ecall));
        // ebreak: imm=1
        let ebreak = i_enc(0b1110011, 0, 0b000, 0, 1);
        assert_eq!(decode32(ebreak, PC), Ok(Instruction::Ebreak));
        // CSR 空间在 M1 未实现 → Unimplemented
        let csrrw = i_enc(0b1110011, 5, 0b001, 6, 0x300);
        assert_eq!(decode32(csrrw, PC), Err(DecodeError::Unimplemented { bits: csrrw, pc: PC }));
    }

    #[test]
    fn decode_illegal() {
        // 未定义 opcode
        let w = 0b1111111 | (1 << 7);
        assert_eq!(decode32(w, PC), Err(DecodeError::Illegal { bits: w, pc: PC }));
        // load funct3=011 非法
        assert!(decode32(i_enc(0b0000011, 1, 0b011, 2, 0), PC).is_err());
        // branch funct3=010 非法
        assert!(decode32(b_enc(0b010, 1, 2, 0), PC).is_err());
        // store funct3=011 非法
        assert!(decode32(s_enc(0b011, 1, 2, 0), PC).is_err());
        // OP funct7=1 非法（M 扩展在 M2）
        assert!(decode32(r_enc(1, 0b000, 2, 3, 0b0000001), PC).is_err());
    }
}
