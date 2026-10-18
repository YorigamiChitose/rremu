//! Base integer and M-extension execution helpers.

use rremu_isa::Instruction;

use super::Cpu;

impl Cpu {
    pub(crate) fn exec_alu(&mut self, pc: u32, len: u8, instr: Instruction) -> Option<u32> {
        let x = |c: &Self, i: u8| c.x[usize::from(i)];
        let mut next_pc = pc.wrapping_add(len as u32);
        macro_rules! set {
            ($rd:expr, $v:expr) => {{
                let v: u32 = $v;
                if $rd != 0 {
                    self.x[usize::from($rd)] = v;
                }
            }};
        }
        match instr {
        Instruction::Lui { rd, imm } => set!(rd, imm),
        Instruction::Auipc { rd, imm } => set!(rd, pc.wrapping_add(imm)),

        Instruction::Jal { rd, imm } => {
                set!(rd, pc.wrapping_add(4));
                next_pc = pc.wrapping_add(imm as u32);
            }
        Instruction::Jalr { rd, rs1, imm } => {
                let t = pc.wrapping_add(4);
                next_pc = x(self, rs1).wrapping_add(imm as u32) & !1;
                set!(rd, t);
            }

        Instruction::Beq { rs1, rs2, imm } => {
                if x(self, rs1) == x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
        Instruction::Bne { rs1, rs2, imm } => {
                if x(self, rs1) != x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
        Instruction::Blt { rs1, rs2, imm } => {
                if (x(self, rs1) as i32) < (x(self, rs2) as i32) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
        Instruction::Bge { rs1, rs2, imm } => {
                if (x(self, rs1) as i32) >= (x(self, rs2) as i32) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
        Instruction::Bltu { rs1, rs2, imm } => {
                if x(self, rs1) < x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }
        Instruction::Bgeu { rs1, rs2, imm } => {
                if x(self, rs1) >= x(self, rs2) {
                    next_pc = pc.wrapping_add(imm as u32);
                }
            }

        Instruction::Addi { rd, rs1, imm } => set!(rd, x(self, rs1).wrapping_add(imm as u32)),
        Instruction::Slti { rd, rs1, imm } => set!(rd, u32::from((x(self, rs1) as i32) < imm)),
        Instruction::Sltiu { rd, rs1, imm } => set!(rd, u32::from(x(self, rs1) < imm as u32)),
        Instruction::Xori { rd, rs1, imm } => set!(rd, x(self, rs1) ^ imm as u32),
        Instruction::Ori { rd, rs1, imm } => set!(rd, x(self, rs1) | imm as u32),
        Instruction::Andi { rd, rs1, imm } => set!(rd, x(self, rs1) & imm as u32),
        Instruction::Slli { rd, rs1, shamt } => set!(rd, x(self, rs1) << shamt),
        Instruction::Srli { rd, rs1, shamt } => set!(rd, x(self, rs1) >> shamt),
        Instruction::Srai { rd, rs1, shamt } => set!(rd, ((x(self, rs1) as i32) >> shamt) as u32),

        Instruction::Add { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_add(x(self, rs2))),
        Instruction::Sub { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_sub(x(self, rs2))),
        Instruction::Sll { rd, rs1, rs2 } => set!(rd, x(self, rs1) << (x(self, rs2) & 0x1f)),
        Instruction::Slt { rd, rs1, rs2 } => set!(rd, u32::from((x(self, rs1) as i32) < (x(self, rs2) as i32))),
        Instruction::Sltu { rd, rs1, rs2 } => set!(rd, u32::from(x(self, rs1) < x(self, rs2))),
        Instruction::Xor { rd, rs1, rs2 } => set!(rd, x(self, rs1) ^ x(self, rs2)),
        Instruction::Srl { rd, rs1, rs2 } => set!(rd, x(self, rs1) >> (x(self, rs2) & 0x1f)),
        Instruction::Sra { rd, rs1, rs2 } => set!(rd, ((x(self, rs1) as i32) >> (x(self, rs2) & 0x1f)) as u32),
        Instruction::Or { rd, rs1, rs2 } => set!(rd, x(self, rs1) | x(self, rs2)),
        Instruction::And { rd, rs1, rs2 } => set!(rd, x(self, rs1) & x(self, rs2)),

            // ---- M extension (RV32M) ----
        Instruction::Mul { rd, rs1, rs2 } => set!(rd, x(self, rs1).wrapping_mul(x(self, rs2))),
        Instruction::Mulh { rd, rs1, rs2 } => {
                let a = (x(self, rs1) as i32) as i64;
                let b = (x(self, rs2) as i32) as i64;
                set!(rd, ((a * b) >> 32) as u32);
            }
        Instruction::Mulhsu { rd, rs1, rs2 } => {
                let a = (x(self, rs1) as i32) as i64;
                let b = x(self, rs2) as i64; // rs2 无符号，零扩展
                set!(rd, ((a * b) >> 32) as u32);
            }
        Instruction::Mulhu { rd, rs1, rs2 } => {
                let a = x(self, rs1) as u64;
                let b = x(self, rs2) as u64;
                set!(rd, ((a * b) >> 32) as u32);
            }
        Instruction::Div { rd, rs1, rs2 } => {
                let a = x(self, rs1) as i32;
                let b = x(self, rs2) as i32;
                let result = a.checked_div(b).map(|v| v as u32).unwrap_or_else(|| {
                    // 除零返回 -1；INT_MIN / -1 溢出返回 INT_MIN
                    if a == i32::MIN && b == -1 { i32::MIN as u32 } else { u32::MAX }
                });
                set!(rd, result);
            }
        Instruction::Divu { rd, rs1, rs2 } => {
                let divisor = x(self, rs2);
                let result = x(self, rs1).checked_div(divisor).unwrap_or(u32::MAX);
                set!(rd, result);
            }
        Instruction::Rem { rd, rs1, rs2 } => {
                let a = x(self, rs1) as i32;
                let b = x(self, rs2) as i32;
                let result = a.checked_rem(b).map(|v| v as u32).unwrap_or_else(|| {
                    // 除零返回 rs1；INT_MIN % -1 溢出返回 0
                    if a == i32::MIN && b == -1 { 0 } else { x(self, rs1) }
                });
                set!(rd, result);
            }
        Instruction::Remu { rd, rs1, rs2 } => {
                let divisor = x(self, rs2);
                let result = x(self, rs1).checked_rem(divisor).unwrap_or_else(|| x(self, rs1));
                set!(rd, result);
            }

            _ => return None,
        }
        Some(next_pc)
    }
}
