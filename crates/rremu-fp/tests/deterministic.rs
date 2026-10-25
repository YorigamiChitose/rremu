//! 确定性 softfloat 回归向量。
//!
//! 每类运算使用固定种子生成 100,000 组原始位模式，并与
//! `golden_fp.bin` 中的分块指纹比较。指纹由外部 C 参考程序
//! 在四种硬件舍入模式下生成；RNE 同时与宿主 IEEE 运算交叉验证。
//! 失败时会重新生成出问题的 1,000 组向量，打印操作数、结果位模式和
//! 异常标志。

use std::fmt::Write as _;

use rremu_fp::{
    Fp32, Fp64, RoundingMode, add32, add64, div32, div64, fmadd32, fmadd64, fmsub32, fmsub64,
    fnmadd32, fnmadd64, fnmsub32, fnmsub64, max32, max64, min32, min64, mul32, mul64, sqrt32,
    sqrt64, sub32, sub64,
};

const BLOCK_SIZE: usize = 1_000;
const BLOCKS: usize = 100;
const MODES: [RoundingMode; 4] = [
    RoundingMode::Rne,
    RoundingMode::Rtz,
    RoundingMode::Rdn,
    RoundingMode::Rup,
];
const SEED_BASE: u64 = 0x5252_4650_4743_3031;
const SEED_OP: u64 = 0x9e37_79b9_7f4a_7c15;
const SEED_MODE: u64 = 0xd1b5_4a32_d192_ed03;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const GOLDEN: &[u8] = include_bytes!("golden_fp.bin");
const GOLDEN_MAGIC: &[u8; 8] = b"RRFPGLD1";
const GOLDEN_HEADER: usize = 32;

#[derive(Clone, Copy)]
enum Op {
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32Sqrt,
    F32Min,
    F32Max,
    F32Fmadd,
    F32Fmsub,
    F32Fnmadd,
    F32Fnmsub,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Sqrt,
    F64Min,
    F64Max,
    F64Fmadd,
    F64Fmsub,
    F64Fnmadd,
    F64Fnmsub,
}

const OPS: [Op; 22] = [
    Op::F32Add,
    Op::F32Sub,
    Op::F32Mul,
    Op::F32Div,
    Op::F32Sqrt,
    Op::F32Min,
    Op::F32Max,
    Op::F32Fmadd,
    Op::F32Fmsub,
    Op::F32Fnmadd,
    Op::F32Fnmsub,
    Op::F64Add,
    Op::F64Sub,
    Op::F64Mul,
    Op::F64Div,
    Op::F64Sqrt,
    Op::F64Min,
    Op::F64Max,
    Op::F64Fmadd,
    Op::F64Fmsub,
    Op::F64Fnmadd,
    Op::F64Fnmsub,
];

impl Op {
    const fn name(self) -> &'static str {
        match self {
            Self::F32Add => "f32_add",
            Self::F32Sub => "f32_sub",
            Self::F32Mul => "f32_mul",
            Self::F32Div => "f32_div",
            Self::F32Sqrt => "f32_sqrt",
            Self::F32Min => "f32_min",
            Self::F32Max => "f32_max",
            Self::F32Fmadd => "f32_fmadd",
            Self::F32Fmsub => "f32_fmsub",
            Self::F32Fnmadd => "f32_fnmadd",
            Self::F32Fnmsub => "f32_fnmsub",
            Self::F64Add => "f64_add",
            Self::F64Sub => "f64_sub",
            Self::F64Mul => "f64_mul",
            Self::F64Div => "f64_div",
            Self::F64Sqrt => "f64_sqrt",
            Self::F64Min => "f64_min",
            Self::F64Max => "f64_max",
            Self::F64Fmadd => "f64_fmadd",
            Self::F64Fmsub => "f64_fmsub",
            Self::F64Fnmadd => "f64_fnmadd",
            Self::F64Fnmsub => "f64_fnmsub",
        }
    }

    const fn is_f32(self) -> bool {
        matches!(
            self,
            Self::F32Add
                | Self::F32Sub
                | Self::F32Mul
                | Self::F32Div
                | Self::F32Sqrt
                | Self::F32Min
                | Self::F32Max
                | Self::F32Fmadd
                | Self::F32Fmsub
                | Self::F32Fnmadd
                | Self::F32Fnmsub
        )
    }

    fn eval(self, a: u64, b: u64, c: u64, mode: RoundingMode) -> (u64, u8) {
        let a32 = Fp32::from_bits(a as u32);
        let b32 = Fp32::from_bits(b as u32);
        let c32 = Fp32::from_bits(c as u32);
        let a64 = Fp64::from_bits(a);
        let b64 = Fp64::from_bits(b);
        let c64 = Fp64::from_bits(c);
        match self {
            Self::F32Add => {
                let (value, flags) = add32(a32, b32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Sub => {
                let (value, flags) = sub32(a32, b32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Mul => {
                let (value, flags) = mul32(a32, b32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Div => {
                let (value, flags) = div32(a32, b32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Sqrt => {
                let (value, flags) = sqrt32(a32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Min => {
                let (value, flags) = min32(a32, b32);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Max => {
                let (value, flags) = max32(a32, b32);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Fmadd => {
                let (value, flags) = fmadd32(a32, b32, c32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Fmsub => {
                let (value, flags) = fmsub32(a32, b32, c32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Fnmadd => {
                let (value, flags) = fnmadd32(a32, b32, c32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F32Fnmsub => {
                let (value, flags) = fnmsub32(a32, b32, c32, mode);
                (value.to_bits() as u64, flags.bits())
            }
            Self::F64Add => {
                let (value, flags) = add64(a64, b64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Sub => {
                let (value, flags) = sub64(a64, b64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Mul => {
                let (value, flags) = mul64(a64, b64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Div => {
                let (value, flags) = div64(a64, b64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Sqrt => {
                let (value, flags) = sqrt64(a64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Min => {
                let (value, flags) = min64(a64, b64);
                (value.to_bits(), flags.bits())
            }
            Self::F64Max => {
                let (value, flags) = max64(a64, b64);
                (value.to_bits(), flags.bits())
            }
            Self::F64Fmadd => {
                let (value, flags) = fmadd64(a64, b64, c64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Fmsub => {
                let (value, flags) = fmsub64(a64, b64, c64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Fnmadd => {
                let (value, flags) = fnmadd64(a64, b64, c64, mode);
                (value.to_bits(), flags.bits())
            }
            Self::F64Fnmsub => {
                let (value, flags) = fnmsub64(a64, b64, c64, mode);
                (value.to_bits(), flags.bits())
            }
        }
    }
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn seed(op: usize, mode: usize) -> u64 {
    (SEED_BASE ^ (op as u64).wrapping_mul(SEED_OP) ^ (mode as u64).wrapping_mul(SEED_MODE)) | 1
}

fn hash_byte(mut hash: u64, byte: u8) -> u64 {
    hash ^= byte as u64;
    hash.wrapping_mul(FNV_PRIME)
}

fn hash_result(mut hash: u64, bits: u64, flags: u8, is_f32: bool) -> u64 {
    let bytes = if is_f32 {
        &(bits as u32).to_le_bytes()[..]
    } else {
        &bits.to_le_bytes()[..]
    };
    for &byte in bytes {
        hash = hash_byte(hash, byte);
    }
    hash_byte(hash, flags)
}

fn golden_u32(offset: usize) -> u32 {
    u32::from_le_bytes([
        GOLDEN[offset],
        GOLDEN[offset + 1],
        GOLDEN[offset + 2],
        GOLDEN[offset + 3],
    ])
}

fn golden_u64(offset: usize) -> u64 {
    u64::from_le_bytes([
        GOLDEN[offset],
        GOLDEN[offset + 1],
        GOLDEN[offset + 2],
        GOLDEN[offset + 3],
        GOLDEN[offset + 4],
        GOLDEN[offset + 5],
        GOLDEN[offset + 6],
        GOLDEN[offset + 7],
    ])
}

fn expected_hash(op: usize, mode: usize, block: usize) -> u64 {
    let offset = GOLDEN_HEADER + (((op * MODES.len() + mode) * BLOCKS + block) * 8);
    golden_u64(offset)
}

fn report_failure(
    op_index: usize,
    op: Op,
    mode_index: usize,
    mode: RoundingMode,
    block: usize,
    got: u64,
    expected: u64,
) -> ! {
    let mut message = String::new();
    let _ = writeln!(
        message,
        "deterministic vector mismatch: op={} mode={mode:?} block={block}",
        op.name()
    );
    let _ = writeln!(
        message,
        "seed={:#018x} got={got:#018x} expected={expected:#018x}",
        seed(op_index, mode_index)
    );
    let _ = writeln!(message, "vectors: index a b c result flags");

    let mut rng = Rng::new(seed(op_index, mode_index));
    for _ in 0..block * BLOCK_SIZE {
        let _ = rng.next();
        let _ = rng.next();
        let _ = rng.next();
    }
    for index in 0..BLOCK_SIZE {
        let a = rng.next();
        let b = rng.next();
        let c = rng.next();
        let (bits, flags) = op.eval(a, b, c, mode);
        if op.is_f32() {
            let _ = writeln!(
                message,
                "{index} {a:#010x} {b:#010x} {c:#010x} {bits:#010x} {flags:#04x}"
            );
        } else {
            let _ = writeln!(
                message,
                "{index} {a:#018x} {b:#018x} {c:#018x} {bits:#018x} {flags:#04x}"
            );
        }
    }
    panic!("{message}");
}

#[test]
fn deterministic_softfloat_vectors() {
    assert_eq!(&GOLDEN[..8], GOLDEN_MAGIC, "golden magic");
    let version = golden_u32(8);
    let ops = golden_u32(12);
    let modes = golden_u32(16);
    let block_size = golden_u32(20);
    let blocks = golden_u32(24);
    assert_eq!(version, 1);
    assert_eq!(ops as usize, OPS.len());
    assert_eq!(modes as usize, MODES.len());
    assert_eq!(block_size as usize, BLOCK_SIZE);
    assert_eq!(blocks as usize, BLOCKS);
    assert_eq!(
        GOLDEN.len(),
        GOLDEN_HEADER + OPS.len() * MODES.len() * BLOCKS * 8,
        "golden size"
    );

    for (op_index, op) in OPS.iter().copied().enumerate() {
        for (mode_index, mode) in MODES.iter().copied().enumerate() {
            let mut rng = Rng::new(seed(op_index, mode_index));
            for block in 0..BLOCKS {
                let mut hash = FNV_OFFSET;
                for _ in 0..BLOCK_SIZE {
                    let a = rng.next();
                    let b = rng.next();
                    let c = rng.next();
                    let (bits, flags) = op.eval(a, b, c, mode);
                    hash = hash_result(hash, bits, flags, op.is_f32());
                }
                let expected = expected_hash(op_index, mode_index, block);
                if hash != expected {
                    report_failure(op_index, op, mode_index, mode, block, hash, expected);
                }
            }
        }
    }
}
