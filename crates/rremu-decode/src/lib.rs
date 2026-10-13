//! rremu-decode：纯函数指令解码（instr_len / decode32 / decode16）。

use rremu_isa::{Address, Instruction, Word};
use thiserror::Error;

/// 指令长度（字节）：低 2 位 == 0b11 为 4 字节，否则 2 字节（RVC 压缩指令）。
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
                // ---- M extension (funct7 = 0000001) ----
                (0b000, 0b0000001) => Ok(Instruction::Mul { rd, rs1, rs2 }),
                (0b001, 0b0000001) => Ok(Instruction::Mulh { rd, rs1, rs2 }),
                (0b010, 0b0000001) => Ok(Instruction::Mulhsu { rd, rs1, rs2 }),
                (0b011, 0b0000001) => Ok(Instruction::Mulhu { rd, rs1, rs2 }),
                (0b100, 0b0000001) => Ok(Instruction::Div { rd, rs1, rs2 }),
                (0b101, 0b0000001) => Ok(Instruction::Divu { rd, rs1, rs2 }),
                (0b110, 0b0000001) => Ok(Instruction::Rem { rd, rs1, rs2 }),
                (0b111, 0b0000001) => Ok(Instruction::Remu { rd, rs1, rs2 }),
                _ => Err(illegal),
            }
        }
        0b0101111 => {
            // AMO：A 扩展（RV32A）。funct5 区分具体操作；aq/rl 位忽略。
            let funct5 = word >> 27;
            match (funct3, funct5) {
                (0b010, 0b00010) => Ok(Instruction::LrW { rd, rs1 }),
                (0b010, 0b00011) => Ok(Instruction::ScW { rd, rs1, rs2 }),
                (0b010, 0b00001) => Ok(Instruction::AmoswapW { rd, rs1, rs2 }),
                (0b010, 0b00000) => Ok(Instruction::AmoaddW { rd, rs1, rs2 }),
                (0b010, 0b00100) => Ok(Instruction::AmoxorW { rd, rs1, rs2 }),
                (0b010, 0b01100) => Ok(Instruction::AmoandW { rd, rs1, rs2 }),
                (0b010, 0b01000) => Ok(Instruction::AmoorW { rd, rs1, rs2 }),
                (0b010, 0b10000) => Ok(Instruction::AmominW { rd, rs1, rs2 }),
                (0b010, 0b10100) => Ok(Instruction::AmomaxW { rd, rs1, rs2 }),
                (0b010, 0b11000) => Ok(Instruction::AmominuW { rd, rs1, rs2 }),
                (0b010, 0b11100) => Ok(Instruction::AmomaxuW { rd, rs1, rs2 }),
                _ => Err(illegal),
            }
        }
        0b0001111 => {
            match funct3 {
                0b000 => Ok(Instruction::Fence),
                0b001 => Ok(Instruction::FenceI),
                _ => Err(illegal),
            }
        }
        0b1110011 => {
            // SYSTEM：ecall/ebreak/mret + Zicsr CSR 读写
            let csr_addr = ((word >> 20) & 0xfff) as u16;
            match funct3 {
                0b000 if rs1 == 0 && rd == 0 => match (word >> 20) & 0xfff {
                    0x000 => Ok(Instruction::Ecall),
                    0x001 => Ok(Instruction::Ebreak),
                    0x302 => Ok(Instruction::Mret),
                    _ => Err(illegal),
                },
                0b001 => Ok(Instruction::Csrrw { rd, csr: csr_addr, rs1 }),
                0b010 => Ok(Instruction::Csrrs { rd, csr: csr_addr, rs1 }),
                0b011 => Ok(Instruction::Csrrc { rd, csr: csr_addr, rs1 }),
                0b101 => Ok(Instruction::Csrrwi { rd, csr: csr_addr, zimm: rs1 }),
                0b110 => Ok(Instruction::Csrrsi { rd, csr: csr_addr, zimm: rs1 }),
                0b111 => Ok(Instruction::Csrrci { rd, csr: csr_addr, zimm: rs1 }),
                _ => Err(illegal),
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

// =====================================================================
// RV32C (compressed) decoding
// =====================================================================

/// 压缩寄存器号：3 位编码映射到 x8..x15。
#[inline]
fn creg(n: u8) -> u8 {
    n + 8
}

/// c.addi4spn 的 nzuimm（10 位无符号，bits[1:0]=0）。
#[inline]
fn c_imm_addi4spn(h: u16) -> u32 {
    ((h as u32 >> 11) & 0x3) << 4
        | ((h as u32 >> 7) & 0xf) << 6
        | ((h as u32 >> 6) & 0x1) << 2
        | ((h as u32 >> 5) & 0x1) << 3
}

/// 从 top_bit 位（0-based）符号扩展：把 raw 视为 top_bit+1 位有符号数。
#[inline]
fn sext(raw: u32, top_bit: u32) -> i32 {
    let shift = 31 - top_bit;
    ((raw as i32) << shift) >> shift
}

/// CB/CI 编码的 6 位移位量：shamt[5]=bit12，shamt[4:0]=bits[6:2]。
#[inline]
fn c_shamt(h: u16) -> u8 {
    (((h >> 12) & 0x1) << 5 | ((h >> 2) & 0x1f)) as u8
}

/// c.lw/c.sw 的 uimm（7 位无符号，bits[1:0]=0）。
#[inline]
fn c_imm_lw(h: u16) -> i32 {
    let uimm = ((h as u32 >> 10) & 0x7) << 3
        | ((h as u32 >> 6) & 0x1) << 2
        | ((h as u32 >> 5) & 0x1) << 6;
    uimm as i32
}

/// CI 格式 6 位符号扩展立即数（c.addi / c.li / c.slli/srli/srai shamt / c.andi）。
#[inline]
fn c_imm_ci(h: u16) -> i32 {
    let raw = u32::from(h >> 2 & 0x1f) | u32::from((h >> 12) & 1) << 5;
    sext(raw, 5)
}

/// c.addi16sp 的符号扩展立即数（10 位，bits[1:0]=0）。
#[inline]
fn c_imm_addi16sp(h: u16) -> i32 {
    let raw = ((h as u32 >> 12) & 0x1) << 9
        | ((h as u32 >> 6) & 0x1) << 4
        | ((h as u32 >> 5) & 0x1) << 6
        | ((h as u32 >> 3) & 0x3) << 7
        | ((h as u32 >> 2) & 0x1) << 5
        | ((h as u32 >> 11) & 0x1) << 3
        | u32::from((h >> 10) & 1) << 2;
    // 符号扩展从 bit9
    sext(raw, 9)
}

/// c.lui 的立即数（20 位 upper，sign-extended）。
#[inline]
fn c_imm_lui(h: u16) -> u32 {
    // NZimm[5:0] = {bit12, bits[6:2]}, value = sign_extend(NZimm) << 12
    let nzimm = u32::from((h >> 12) & 1) << 5 | u32::from((h >> 2) & 0x1f);
    // sign-extend from bit 5
    (sext(nzimm, 5) as u32) << 12
}

/// CJ 格式跳转偏移（c.j / c.jal，12 位符号扩展，bit0=0）。
#[inline]
fn c_imm_j(h: u16) -> i32 {
    let raw = ((h as u32 >> 12) & 0x1) << 11
        | ((h as u32 >> 11) & 0x1) << 4
        | ((h as u32 >> 9) & 0x3) << 8
        | ((h as u32 >> 8) & 0x1) << 10
        | ((h as u32 >> 7) & 0x1) << 6
        | ((h as u32 >> 6) & 0x1) << 7
        | ((h as u32 >> 3) & 0x7) << 1
        | u32::from((h >> 2) & 1) << 5;
    sext(raw, 11)
}

/// CB 格式分支偏移（c.beqz/c.bnez，9 位符号扩展，bit0=0）。
#[inline]
fn c_imm_b(h: u16) -> i32 {
    let raw = ((h as u32 >> 12) & 0x1) << 8
        | ((h as u32 >> 10) & 0x3) << 3
        | ((h as u32 >> 5) & 0x3) << 6
        | ((h as u32 >> 3) & 0x3) << 1
        | u32::from((h >> 2) & 1) << 5;
    sext(raw, 8)
}

/// c.lwsp 的 uimm（8 位无符号，bits[1:0]=0）。
/// spec: uimm[5]=bit12, uimm[4:2]=bits[6:4], uimm[7:6]=bits[3:2]
#[inline]
fn c_imm_lwsp(h: u16) -> i32 {
    let uimm = ((h as u32 >> 12) & 0x1) << 5
        | ((h as u32 >> 4) & 0x7) << 2
        | ((h as u32 >> 2) & 0x3) << 6;
    uimm as i32
}

/// c.swsp 的 uimm（8 位无符号，bits[1:0]=0）。
#[inline]
fn c_imm_swsp(h: u16) -> i32 {
    let uimm = ((h as u32 >> 9) & 0xf) << 2
        | ((h as u32 >> 7) & 0x3) << 6;
    uimm as i32
}

/// 解码 16 位压缩指令（RV32C）。
///
/// 调用方需保证 `instr_len(half) == 2`（即 bits[1:0] != 11）。
pub fn decode16(half: u16, pc: Address) -> Result<Instruction, DecodeError> {
    let illegal = DecodeError::Illegal { bits: half as Word, pc };
    let op = (half & 0b11) as u8;
    let funct3 = ((half >> 13) & 0b111) as u8;

    match (op, funct3) {
        // ---- Quadrant 00 (CIW/CL/CSS) ----
        (0b00, 0b000) => {
            // c.addi4spn: rd' = bits[4:2], nzuimm from bits[12:5]
            let rd = creg(((half >> 2) & 0x7) as u8);
            let imm = c_imm_addi4spn(half);
            // nzuimm 不能为 0（保留编码）
            if imm == 0 {
                return Err(illegal);
            }
            Ok(Instruction::CAddi4spn { rd, imm })
        }
        (0b00, 0b010) => {
            // c.lw: rd'=bits[4:2], rs1'=bits[9:7], uimm from bits[12:10|6|5]
            let rd = creg(((half >> 2) & 0x7) as u8);
            let rs1 = creg(((half >> 7) & 0x7) as u8);
            let imm = c_imm_lw(half);
            Ok(Instruction::CLw { rd, rs1, imm })
        }
        (0b00, 0b110) => {
            // c.sw: rs1'=bits[9:7], rs2'=bits[4:2], uimm same as c.lw
            let rs1 = creg(((half >> 7) & 0x7) as u8);
            let rs2 = creg(((half >> 2) & 0x7) as u8);
            let imm = c_imm_lw(half);
            Ok(Instruction::CSw { rs1, rs2, imm })
        }
        (0b00, _) => Err(illegal), // 001(flw)/011/101/111 reserved

        // ---- Quadrant 01 ----
        (0b01, 0b000) => {
            // c.addi (also c.nop when rd=0, imm=0)
            let rd = ((half >> 7) & 0x1f) as u8;
            let imm = c_imm_ci(half);
            Ok(Instruction::CAddi { rd, imm })
        }
        (0b01, 0b001) => {
            // c.jal
            let imm = c_imm_j(half);
            Ok(Instruction::CJal { imm })
        }
        (0b01, 0b010) => {
            // c.li
            let rd = ((half >> 7) & 0x1f) as u8;
            let imm = c_imm_ci(half);
            Ok(Instruction::CLi { rd, imm })
        }
        (0b01, 0b011) => {
            // c.addi16sp 与 c.lui 共用 CI 编码空间，由 rd 字段区分：
            // rd==x2(00010) → c.addi16sp（rd 隐含为 sp，bits[11:7]=00010）
            // rd∉{x0,x2}     → c.lui（rd 字段为目标寄存器）
            let rd = ((half >> 7) & 0x1f) as u8;
            if rd == 2 {
                // c.addi16sp
                let imm = c_imm_addi16sp(half);
                Ok(Instruction::CAddi16sp { imm })
            } else {
                // c.lui
                let imm = c_imm_lui(half);
                if imm == 0 {
                    // HINT（NOP）：nzimm==0（或 rd==x0）为保留/HINT 编码
                    Ok(Instruction::CLui { rd: 0, imm: 0 })
                } else {
                    Ok(Instruction::CLui { rd, imm })
                }
            }
        }
        (0b01, 0b100) => {
            // CB / CA 格式
            let funct2 = (half >> 10) & 0b11;
            let rd = creg(((half >> 7) & 0x7) as u8);
            match funct2 {
                0b00 => {
                    // c.srli: 6 位 shamt
                    Ok(Instruction::CSrli { rd, shamt: c_shamt(half) })
                }
                0b01 => {
                    // c.srai
                    Ok(Instruction::CSrai { rd, shamt: c_shamt(half) })
                }
                0b10 => {
                    // c.andi
                    let imm = c_imm_ci(half);
                    Ok(Instruction::CAndi { rd, imm })
                }
                0b11 => {
                    // CA: c.sub / c.xor / c.or / c.and
                    let funct2_sub = (half >> 5) & 0b11;
                    let rs2 = creg(((half >> 2) & 0x7) as u8);
                    match funct2_sub {
                        0b00 => Ok(Instruction::CSub { rd, rs2 }),
                        0b01 => Ok(Instruction::CXor { rd, rs2 }),
                        0b10 => Ok(Instruction::COr { rd, rs2 }),
                        0b11 => Ok(Instruction::CAnd { rd, rs2 }),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }
        }
        (0b01, 0b101) => {
            // c.j
            let imm = c_imm_j(half);
            Ok(Instruction::CJ { imm })
        }
        (0b01, 0b110) => {
            // c.beqz
            let rs1 = creg(((half >> 7) & 0x7) as u8);
            let imm = c_imm_b(half);
            Ok(Instruction::CBeqz { rs1, imm })
        }
        (0b01, 0b111) => {
            // c.bnez
            let rs1 = creg(((half >> 7) & 0x7) as u8);
            let imm = c_imm_b(half);
            Ok(Instruction::CBnez { rs1, imm })
        }

        // ---- Quadrant 10 ----
        (0b10, 0b000) => {
            // c.slli: rd=bits[11:7], 6 位 shamt
            let rd = ((half >> 7) & 0x1f) as u8;
            Ok(Instruction::CSlli { rd, shamt: c_shamt(half) })
        }
        (0b10, 0b010) => {
            // c.lwsp: rd=bits[11:7], uimm from bits[12|6:4|3:2]
            let rd = ((half >> 7) & 0x1f) as u8;
            let imm = c_imm_lwsp(half);
            Ok(Instruction::CLwsp { rd, imm })
        }
        (0b10, 0b100) => {
            // CR 格式
            let rs1 = ((half >> 7) & 0x1f) as u8;
            let rs2 = ((half >> 2) & 0x1f) as u8;
            let bit12 = (half >> 12) & 0x1;
            if bit12 == 0 {
                if rs2 == 0 {
                    if rs1 == 0 {
                        return Err(illegal); // c.jr x0 保留编码
                    }
                    // c.jr
                    Ok(Instruction::CJr { rs1 })
                } else {
                    // c.mv
                    Ok(Instruction::CMv { rd: rs1, rs2 })
                }
            } else {
                if rs2 == 0 {
                    if rs1 == 0 {
                        // c.ebreak
                        Ok(Instruction::CEbreak)
                    } else {
                        // c.jalr
                        Ok(Instruction::CJalr { rs1 })
                    }
                } else {
                    // c.add
                    Ok(Instruction::CAdd { rd: rs1, rs2 })
                }
            }
        }
        (0b10, 0b110) => {
            // c.swsp: rs2=bits[6:2], uimm from bits[12:9|8:7]
            let rs2 = ((half >> 2) & 0x1f) as u8;
            let imm = c_imm_swsp(half);
            Ok(Instruction::CSwsp { rs2, imm })
        }
        (0b10, _) => Err(illegal), // 001(flwsp)/011/101/111 reserved

        // op=11 是 32 位指令，不应进入 decode16
        (0b11, _) => Err(illegal),
        // op=4..=255 不可能出现（op 只有 2 位），但需穷尽 u8
        _ => Err(illegal),
    }
}

// ---------------------------------------------------------------------------
// C extension encoder (与 decode16 互为逆运算，供测试 round-trip 使用)
// ---------------------------------------------------------------------------

fn creg_inv(r: u8) -> u16 { (r - 8) as u16 }

pub fn encode_c(instr: &Instruction) -> u16 {
    match *instr {
        Instruction::CAddi4spn { rd, imm } => {
            let rd_p = creg_inv(rd);
            let b12_11 = ((imm >> 4) & 0x3) as u16;
            let b10_7 = ((imm >> 6) & 0xf) as u16;
            let b6 = ((imm >> 2) & 0x1) as u16;
            let b5 = ((imm >> 3) & 0x1) as u16;
            (b12_11 << 11) | (b10_7 << 7) | (b6 << 6) | (b5 << 5) | (rd_p << 2)
        }
        Instruction::CLw { rd, rs1, imm } => {
            let rd_p = creg_inv(rd);
            let rs1_p = creg_inv(rs1);
            let uimm = imm as u32;
            let b12_10 = ((uimm >> 3) & 0x7) as u16;
            let b6 = ((uimm >> 2) & 0x1) as u16;
            let b5 = ((uimm >> 6) & 0x1) as u16;
            (0b010u16 << 13) | (b12_10 << 10) | (rs1_p << 7) | (b6 << 6) | (b5 << 5) | (rd_p << 2)
        }
        Instruction::CSw { rs1, rs2, imm } => {
            let rs1_p = creg_inv(rs1);
            let rs2_p = creg_inv(rs2);
            let uimm = imm as u32;
            let b12_10 = ((uimm >> 3) & 0x7) as u16;
            let b6 = ((uimm >> 2) & 0x1) as u16;
            let b5 = ((uimm >> 6) & 0x1) as u16;
            (0b110u16 << 13) | (b12_10 << 10) | (rs1_p << 7) | (b6 << 6) | (b5 << 5) | (rs2_p << 2)
        }
        Instruction::CAddi { rd, imm } => {
            let i = imm as u32 & 0x3f;
            let b12 = ((i >> 5) & 1) as u16;
            let lo = (i & 0x1f) as u16;
            (b12 << 12) | ((rd as u16) << 7) | (lo << 2) | 0b01
        }
        Instruction::CJal { imm } => encode_cj(imm, 0b001),
        Instruction::CLi { rd, imm } => {
            let i = imm as u32 & 0x3f;
            let b12 = ((i >> 5) & 1) as u16;
            let lo = (i & 0x1f) as u16;
            (0b010u16 << 13) | (b12 << 12) | ((rd as u16) << 7) | (lo << 2) | 0b01
        }
        Instruction::CAddi16sp { imm } => {
                let raw = imm as u32;
                let b9 = ((raw >> 9) & 1) as u16;
                let b4 = ((raw >> 4) & 1) as u16;
                let b6 = ((raw >> 6) & 1) as u16;
                let b8_7 = ((raw >> 7) & 0x3) as u16;
                let b5 = ((raw >> 5) & 1) as u16;
                let b3 = ((raw >> 3) & 1) as u16;
                let b2 = ((raw >> 2) & 1) as u16;
                (0b011u16 << 13) | (b9 << 12) | (2u16 << 7) | (b4 << 6) | (b6 << 5) | (b8_7 << 3) | (b5 << 2) | (b3 << 11) | (b2 << 10) | 0b01
            }
        Instruction::CLui { rd, imm } => {
            let v = imm >> 12;
            let b12 = ((v >> 5) & 1) as u16;
            let lo = (v & 0x1f) as u16;
            (0b011u16 << 13) | (b12 << 12) | ((rd as u16) << 7) | (lo << 2) | 0b01
        }
        Instruction::CSrli { rd, shamt } => encode_cb_shift(rd, shamt, 0b00),
        Instruction::CSrai { rd, shamt } => encode_cb_shift(rd, shamt, 0b01),
        Instruction::CAndi { rd, imm } => {
            let rd_p = creg_inv(rd);
            let i = imm as u32 & 0x3f;
            let b12 = ((i >> 5) & 1) as u16;
            let lo = (i & 0x1f) as u16;
            (0b100u16 << 13) | (0b10u16 << 10) | (b12 << 12) | (rd_p << 7) | (lo << 2) | 0b01
        }
        Instruction::CSub { rd, rs2 } => encode_ca(rd, rs2, 0b00),
        Instruction::CXor { rd, rs2 } => encode_ca(rd, rs2, 0b01),
        Instruction::COr { rd, rs2 } => encode_ca(rd, rs2, 0b10),
        Instruction::CAnd { rd, rs2 } => encode_ca(rd, rs2, 0b11),
        Instruction::CJ { imm } => encode_cj(imm, 0b101),
        Instruction::CBeqz { rs1, imm } => encode_cb_branch(rs1, imm, 0b110),
        Instruction::CBnez { rs1, imm } => encode_cb_branch(rs1, imm, 0b111),
        Instruction::CSlli { rd, shamt } => {
            let s = shamt as u32 & 0x3f;
            let b12 = ((s >> 5) & 1) as u16;
            let lo = (s & 0x1f) as u16;
            (b12 << 12) | ((rd as u16) << 7) | (lo << 2) | 0b10
        }
        Instruction::CLwsp { rd, imm } => {
            let uimm = imm as u32;
            let b5 = ((uimm >> 5) & 1) as u16;
            let b4_2 = ((uimm >> 2) & 0x7) as u16;
            let b7_6 = ((uimm >> 6) & 0x3) as u16;
            (0b010u16 << 13) | (b5 << 12) | ((rd as u16) << 7) | (b4_2 << 4) | (b7_6 << 2) | 0b10
        }
        Instruction::CJr { rs1 } => (0b100u16 << 13) | ((rs1 as u16) << 7) | 0b10,
        Instruction::CMv { rd, rs2 } => (0b100u16 << 13) | ((rd as u16) << 7) | ((rs2 as u16) << 2) | 0b10,
        Instruction::CEbreak => (0b100u16 << 13) | (1 << 12) | 0b10,
        Instruction::CJalr { rs1 } => (0b100u16 << 13) | (1 << 12) | ((rs1 as u16) << 7) | 0b10,
        Instruction::CAdd { rd, rs2 } => (0b100u16 << 13) | (1 << 12) | ((rd as u16) << 7) | ((rs2 as u16) << 2) | 0b10,
        Instruction::CSwsp { rs2, imm } => {
            let uimm = imm as u32;
            let b5_2 = ((uimm >> 2) & 0xf) as u16;
            let b7_6 = ((uimm >> 6) & 0x3) as u16;
            (0b110u16 << 13) | (b5_2 << 9) | (b7_6 << 7) | ((rs2 as u16) << 2) | 0b10
        }
        _ => panic!("not a C instruction"),
    }
}

fn encode_cj(imm: i32, funct3: u16) -> u16 {
    let raw = imm as u32 & 0xffe;
    let b11 = ((raw >> 11) & 1) as u16;
    let b4 = ((raw >> 4) & 1) as u16;
    let b9_8 = ((raw >> 8) & 0x3) as u16;
    let b10 = ((raw >> 10) & 1) as u16;
    let b6 = ((raw >> 6) & 1) as u16;
    let b7 = ((raw >> 7) & 1) as u16;
    let b3_1 = ((raw >> 1) & 0x7) as u16;
    let b5 = ((raw >> 5) & 1) as u16;
    (funct3 << 13) | (b11 << 12) | (b4 << 11) | (b9_8 << 9) | (b10 << 8) | (b6 << 7) | (b7 << 6) | (b3_1 << 3) | (b5 << 2) | 0b01
}

fn encode_cb_shift(rd: u8, shamt: u8, funct2: u16) -> u16 {
    let rd_p = creg_inv(rd);
    let s = shamt as u32 & 0x3f;
    let b12 = ((s >> 5) & 1) as u16;
    let lo = (s & 0x1f) as u16;
    (0b100u16 << 13) | (funct2 << 10) | (b12 << 12) | (rd_p << 7) | (lo << 2) | 0b01
}

fn encode_ca(rd: u8, rs2: u8, funct2_sub: u16) -> u16 {
    let rd_p = creg_inv(rd);
    let rs2_p = creg_inv(rs2);
    (0b100u16 << 13) | (0b11u16 << 10) | (rd_p << 7) | (funct2_sub << 5) | (rs2_p << 2) | 0b01
}

fn encode_cb_branch(rs1: u8, imm: i32, funct3: u16) -> u16 {
    let rs1_p = creg_inv(rs1);
    let raw = imm as u32 & 0x1fe;
    let b8 = ((raw >> 8) & 1) as u16;
    let b4_3 = ((raw >> 3) & 0x3) as u16;
    let b7_6 = ((raw >> 6) & 0x3) as u16;
    let b2_1 = ((raw >> 1) & 0x3) as u16;
    let b5 = ((raw >> 5) & 1) as u16;
    (funct3 << 13) | (b8 << 12) | (b4_3 << 10) | (rs1_p << 7) | (b7_6 << 5) | (b2_1 << 3) | (b5 << 2) | 0b01
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
        // CSR 指令解码
        let csrrw = i_enc(0b1110011, 5, 0b001, 6, 0x300);
        assert_eq!(decode32(csrrw, PC), Ok(Instruction::Csrrw { rd: 5, csr: 0x300, rs1: 6 }));
        let csrrs = i_enc(0b1110011, 7, 0b010, 8, 0x305);
        assert_eq!(decode32(csrrs, PC), Ok(Instruction::Csrrs { rd: 7, csr: 0x305, rs1: 8 }));
        let csrwi = i_enc(0b1110011, 0, 0b101, 3, 0x304);
        assert_eq!(decode32(csrwi, PC), Ok(Instruction::Csrrwi { rd: 0, csr: 0x304, zimm: 3 }));
        // mret = 0x30200073
        assert_eq!(decode32(0x3020_0073, PC), Ok(Instruction::Mret));
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
        // OP funct7=0000010 非法
        assert!(decode32(r_enc(1, 0b000, 2, 3, 0b0000010), PC).is_err());
    }

    #[test]
    fn decode_m_extension() {
        let m = |f3: u32| r_enc(3, f3, 1, 2, 0b0000001);
        assert_eq!(decode32(m(0b000), PC), Ok(Instruction::Mul { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b001), PC), Ok(Instruction::Mulh { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b010), PC), Ok(Instruction::Mulhsu { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b011), PC), Ok(Instruction::Mulhu { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b100), PC), Ok(Instruction::Div { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b101), PC), Ok(Instruction::Divu { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b110), PC), Ok(Instruction::Rem { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(m(0b111), PC), Ok(Instruction::Remu { rd: 3, rs1: 1, rs2: 2 }));
    }

    fn amo_enc(f5: u32, f3: u32, rs1: u8, rs2: u8, rd: u8) -> Word {
        (f5 << 27) | (u32::from(rs2) << 20) | (u32::from(rs1) << 15) | (f3 << 12) | (u32::from(rd) << 7) | 0b0101111
    }

    #[test]
    fn decode_a_extension() {
        let amo = |f5, f3| amo_enc(f5, f3, 1, 2, 3);
        assert_eq!(decode32(amo(0b00010, 0b010), PC), Ok(Instruction::LrW { rd: 3, rs1: 1 }));
        assert_eq!(decode32(amo(0b00011, 0b010), PC), Ok(Instruction::ScW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b00001, 0b010), PC), Ok(Instruction::AmoswapW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b00000, 0b010), PC), Ok(Instruction::AmoaddW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b00100, 0b010), PC), Ok(Instruction::AmoxorW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b01100, 0b010), PC), Ok(Instruction::AmoandW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b01000, 0b010), PC), Ok(Instruction::AmoorW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b10000, 0b010), PC), Ok(Instruction::AmominW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b10100, 0b010), PC), Ok(Instruction::AmomaxW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b11000, 0b010), PC), Ok(Instruction::AmominuW { rd: 3, rs1: 1, rs2: 2 }));
        assert_eq!(decode32(amo(0b11100, 0b010), PC), Ok(Instruction::AmomaxuW { rd: 3, rs1: 1, rs2: 2 }));
        // 非法 AMO：funct3=011（RV64 .d 变体）与未知 funct5
        assert!(decode32(amo(0b11000, 0b011), PC).is_err());
        assert!(decode32(amo(0b11111, 0b010), PC).is_err());
    }

    #[test]
    fn decode_fence_i() {
        assert_eq!(decode32(0x0000_100F, PC), Ok(Instruction::FenceI));
        // fence（funct3=000）仍正确
        assert_eq!(decode32(0x0000_000F, PC), Ok(Instruction::Fence));
    }

    // ---- C extension tests ----

    fn ciw_enc(rd_prime: u8, nzuimm: u32) -> u16 {
        // c.addi4spn: funct3=000, op=00
        // nzuimm[5:4]=bits[12:11], nzuimm[9:6]=bits[10:7], nzuimm[2]=bit6, nzuimm[3]=bit5
        let b12_11 = (nzuimm >> 4) & 0x3;
        let b10_7 = (nzuimm >> 6) & 0xf;
        let b6 = (nzuimm >> 2) & 0x1;
        let b5 = (nzuimm >> 3) & 0x1;
        (b12_11 << 11) as u16 | (b10_7 << 7) as u16 | (b6 << 6) as u16 | (b5 << 5) as u16 | (u16::from(rd_prime) << 2)
    }

    #[test]
    fn decode_c_addi4spn() {
        // c.addi4spn x8(x0'), 4  → rd'=0 → x8, nzuimm=4
        let h = ciw_enc(0, 4);
        assert_eq!(decode16(h, PC), Ok(Instruction::CAddi4spn { rd: 8, imm: 4 }));
        // nzuimm=0 非法
        assert!(decode16(ciw_enc(0, 0), PC).is_err());
    }

    #[test]
    fn decode_c_lw_sw() {
        // c.lw x8, 0(x8): rd'=0, rs1'=0, uimm=0
        let lw = 0b010u16 << 13; // funct3=010, op=00
        assert_eq!(decode16(lw, PC), Ok(Instruction::CLw { rd: 8, rs1: 8, imm: 0 }));
        // c.sw x9, 4(x8): rs1'=0 (x8) at bits[9:7], rs2'=1 (x9) at bits[4:2], uimm=4
        // uimm[5:3]=bits[12:10]=0, uimm[2]=bit6=1, uimm[6]=bit5=0 → uimm=4
        let sw = (0b110u16 << 13) | (1 << 6) | (1 << 2); // funct3=110, bit6=1, rs2'=1 (bits[4:2])
        assert_eq!(decode16(sw, PC), Ok(Instruction::CSw { rs1: 8, rs2: 9, imm: 4 }));
    }

    #[test]
    fn decode_c_addi() {
        // c.addi x1, 5: rd=1, imm=5
        // CI: imm[5]=bit12, imm[4:0]=bits[6:2]
        let h = (1 << 7) | (5 << 2) | 0b01; // funct3=000, op=01
        assert_eq!(decode16(h, PC), Ok(Instruction::CAddi { rd: 1, imm: 5 }));
        // c.nop: rd=0, imm=0
        let nop = 0b01;
        assert_eq!(decode16(nop, PC), Ok(Instruction::CAddi { rd: 0, imm: 0 }));
    }

    #[test]
    fn decode_c_li() {
        // c.li x5, -1: rd=5, imm=-1 → imm[5]=1, imm[4:0]=0x1f
        let h = (0b010u16 << 13) | (1 << 12) | (5 << 7) | (0x1f << 2) | 0b01;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLi { rd: 5, imm: -1 }));
    }

    #[test]
    fn decode_c_j_jal() {
        // c.jal 0: imm=0
        let jal = (0b001u16 << 13) | 0b01;
        assert_eq!(decode16(jal, PC), Ok(Instruction::CJal { imm: 0 }));
        // c.j 0
        let j = (0b101u16 << 13) | 0b01;
        assert_eq!(decode16(j, PC), Ok(Instruction::CJ { imm: 0 }));
    }

    #[test]
    fn decode_c_slli() {
        // c.slli x1, 3: rd=1, shamt=3
        let h = (1 << 7) | (3 << 2) | 0b10;
        assert_eq!(decode16(h, PC), Ok(Instruction::CSlli { rd: 1, shamt: 3 }));
    }

    #[test]
    fn decode_c_mv_add_jr_jalr_ebreak() {
        // c.mv x1, x2: rs1=1, rs2=2, bit12=0, op=10, funct3=100
        let mv = (0b100u16 << 13) | (1 << 7) | (2 << 2) | 0b10;
        assert_eq!(decode16(mv, PC), Ok(Instruction::CMv { rd: 1, rs2: 2 }));
        // c.add x1, x2: bit12=1
        let add = (0b100u16 << 13) | (1 << 12) | (1 << 7) | (2 << 2) | 0b10;
        assert_eq!(decode16(add, PC), Ok(Instruction::CAdd { rd: 1, rs2: 2 }));
        // c.jr x1: rs2=0
        let jr = (0b100u16 << 13) | (1 << 7) | 0b10;
        assert_eq!(decode16(jr, PC), Ok(Instruction::CJr { rs1: 1 }));
        // c.jalr x1: bit12=1, rs2=0
        let jalr = (0b100u16 << 13) | (1 << 12) | (1 << 7) | 0b10;
        assert_eq!(decode16(jalr, PC), Ok(Instruction::CJalr { rs1: 1 }));
        // c.ebreak: rs1=0, rs2=0, bit12=1
        let ebreak = (0b100u16 << 13) | (1 << 12) | 0b10;
        assert_eq!(decode16(ebreak, PC), Ok(Instruction::CEbreak));
    }

    #[test]
    fn decode_c_lwsp_swsp() {
        // c.lwsp x1, 0: rd=1, uimm=0, funct3=010, op=10
        let lwsp = (0b010u16 << 13) | (1 << 7) | 0b10;
        assert_eq!(decode16(lwsp, PC), Ok(Instruction::CLwsp { rd: 1, imm: 0 }));
        // c.swsp x1, 0: rs2=1, funct3=110, op=10
        let swsp = (0b110u16 << 13) | (1 << 2) | 0b10;
        assert_eq!(decode16(swsp, PC), Ok(Instruction::CSwsp { rs2: 1, imm: 0 }));
    }

    #[test]
    fn decode_c_lwsp_nonzero_offset() {
        // 从 rv32uc-p-rvc.elf 的真实编码验证：
        // 0x4532 = c.lwsp a0, 12(sp): bits[6:4]=011 → uimm[4:2]=3, <<2=12
        let h = 0x4532u16;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLwsp { rd: 10, imm: 12 }));

        // 0x4632 = c.lwsp a2, 12(sp): rd=12(a2), same offset
        let h = 0x4632u16;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLwsp { rd: 12, imm: 12 }));

        // 手工构造 c.lwsp a0, 4(sp): uimm=4 (uimm[2]=1)
        // uimm[2] → inst[4], so inst[4]=1; bits[6:2]=00010
        let h = (0b010u16 << 13) | (10u16 << 7) | (1u16 << 4) | 0b10;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLwsp { rd: 10, imm: 4 }));

        // c.lwsp a0, 64(sp): uimm=64 (uimm[6]=1)
        // uimm[6] → inst[2], so inst[2]=1; bits[6:2]=00001
        let h = (0b010u16 << 13) | (10u16 << 7) | (1u16 << 2) | 0b10;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLwsp { rd: 10, imm: 64 }));

        // c.lwsp a0, 128(sp): uimm=128 (uimm[7]=1)
        // uimm[7] → inst[3], so inst[3]=1; bits[6:2]=00010... wait
        // uimm[7] → inst[3], so inst[3]=1; bits[6:2]=00010
        let h = (0b010u16 << 13) | (10u16 << 7) | (1u16 << 3) | 0b10;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLwsp { rd: 10, imm: 128 }));

        // c.lwsp a0, 32(sp): uimm=32 (uimm[5]=1)
        // uimm[5] → inst[12], so inst[12]=1
        let h = (0b010u16 << 13) | (1u16 << 12) | (10u16 << 7) | 0b10;
        assert_eq!(decode16(h, PC), Ok(Instruction::CLwsp { rd: 10, imm: 32 }));
    }

    #[test]
    fn decode_c_alu() {
        // c.srli x8, 1: rd'=0(x8), shamt=1, funct3=100, funct2=00, op=01
        let srli = (0b100u16 << 13) | (1 << 2) | 0b01;
        assert_eq!(decode16(srli, PC), Ok(Instruction::CSrli { rd: 8, shamt: 1 }));
        // c.sub x8, x9: rd'=0, rs2'=1, funct2=11, funct2_sub=00
        let sub = (0b100u16 << 13) | (0b11 << 10) | (1 << 2) | 0b01;
        assert_eq!(decode16(sub, PC), Ok(Instruction::CSub { rd: 8, rs2: 9 }));
        // c.xor x8, x9: funct2_sub=01
        let xor = (0b100u16 << 13) | (0b11 << 10) | (0b01 << 5) | (1 << 2) | 0b01;
        assert_eq!(decode16(xor, PC), Ok(Instruction::CXor { rd: 8, rs2: 9 }));
    }

    #[test]
    fn c_extension_roundtrip_all() {
        // 对每条 C 指令编码后解码，验证与原始指令一致
        let cases: Vec<Instruction> = vec![
            Instruction::CAddi4spn { rd: 8, imm: 4 },
            Instruction::CLw { rd: 8, rs1: 9, imm: 4 },
            Instruction::CSw { rs1: 8, rs2: 9, imm: 8 },
            Instruction::CAddi { rd: 1, imm: 5 },
            Instruction::CAddi { rd: 3, imm: -7 },
            Instruction::CJal { imm: 42 },
            Instruction::CJal { imm: -8 },
            Instruction::CLi { rd: 5, imm: -1 },
            Instruction::CLi { rd: 2, imm: 31 },
            Instruction::CAddi16sp { imm: -16 },
            Instruction::CLui { rd: 6, imm: 0x1f000 },
            Instruction::CSrli { rd: 8, shamt: 5 },
            Instruction::CSrai { rd: 9, shamt: 31 },
            Instruction::CAndi { rd: 10, imm: -16 },
            Instruction::CSub { rd: 8, rs2: 9 },
            Instruction::CXor { rd: 10, rs2: 11 },
            Instruction::COr { rd: 12, rs2: 13 },
            Instruction::CAnd { rd: 14, rs2: 15 },
            Instruction::CJ { imm: 100 },
            Instruction::CJ { imm: -50 },
            Instruction::CBeqz { rs1: 8, imm: 16 },
            Instruction::CBnez { rs1: 9, imm: -8 },
            Instruction::CSlli { rd: 3, shamt: 12 },
            Instruction::CLwsp { rd: 4, imm: 32 },
            Instruction::CJr { rs1: 5 },
            Instruction::CMv { rd: 6, rs2: 7 },
            Instruction::CEbreak,
            Instruction::CJalr { rs1: 8 },
            Instruction::CAdd { rd: 9, rs2: 10 },
            Instruction::CSwsp { rs2: 11, imm: 64 },
        ];
        for instr in &cases {
            let h = encode_c(instr);
            let decoded = decode16(h, PC).unwrap_or_else(|e| panic!("decode failed for {instr:?}: {e}"));
            assert_eq!(decoded, *instr, "round-trip failed for {instr:?}: encoded={h:#06x}, decoded={decoded:?}");
        }
    }

    /// 回归测试：c.lui rd=x10(a0) 不得被误判为 c.addi16sp。
    ///
    /// C.LUI 与 C.ADDI16SP 共用 CI 编码空间（op=01, funct3=011），由 rd 字段区分：
    /// rd==x2 → c.addi16sp；rd∉{x0,x2} → c.lui。早期实现只取 bits[9:7] 做 3 位
    /// 比较（==0b010），同时命中 x2(00010) 与 x10(01010)，导致 c.lui a0 被错误
    /// 解码为 c.addi16sp，使 a0 保持为 0 而非装入 0xC000。
    #[test]
    fn c_lui_rd_a0_not_misidentified_as_addi16sp() {
        // c.lui a0, 12（imm=0xC000）的编码：rd=10 → bits[11:7]=01010
        // bits[9:7]=010，恰为旧实现误匹配 c.addi16sp 的模式。
        let h: u16 = (0b011 << 13) | (10u16 << 7) | (12u16 << 2) | 0b01;
        assert_eq!(h, 0x6531);
        assert_eq!(
            decode16(h, PC),
            Ok(Instruction::CLui { rd: 10, imm: 0xC000 }),
            "c.lui a0 被误判为 c.addi16sp（rd 字段 bits[9:7] 比较缺陷）"
        );
    }

    #[test]
    fn decode_c_hint_and_reserved_encodings() {
        // HINT 编码应解码为 NOP 等价物；保留编码应 illegal。
        // c.lui nzimm==0（rd=8）→ HINT（CLui{rd:0,imm:0}，执行时不写寄存器）
        let lui_hint: u16 = (0b011 << 13) | (8u16 << 7) | 0b01;
        assert_eq!(decode16(lui_hint, PC), Ok(Instruction::CLui { rd: 0, imm: 0 }));
        // c.addi16sp nzimm==0（rd=2）→ HINT（CAddi16sp{imm:0}，sp 不变）
        let addi16sp_hint: u16 = (0b011 << 13) | (2u16 << 7) | 0b01;
        assert_eq!(decode16(addi16sp_hint, PC), Ok(Instruction::CAddi16sp { imm: 0 }));
        // c.slli rd==x0 → HINT（CSlli{rd:0}，写 x0 被忽略）
        let slli_hint: u16 = 0b10;
        assert_eq!(decode16(slli_hint, PC), Ok(Instruction::CSlli { rd: 0, shamt: 0 }));
        // c.jr x0 → 保留编码（illegal）
        let jr_x0: u16 = (0b100 << 13) | 0b10;
        assert!(decode16(jr_x0, PC).is_err());
    }
}
