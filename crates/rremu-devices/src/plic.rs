//! PLIC（Platform-Level Interrupt Controller）设备 —— qemu-virt 兼容子集。
//!
//! 寄存器布局（SiFive 标准）：
//! | offset              | 寄存器 |
//! |---------------------|--------|
//! | `0x0000 + 4*src`    | priority（每个 source 一个 32 位优先级；src 0 保留） |
//! | `0x1000 + 4*(src/32)` | pending（只读位图） |
//! | `0x2000 + ctx*0x80 + 4*(src/32)` | enable（每 context 位图） |
//! | `0x200000 + ctx*0x1000` | threshold |
//! | `0x200004 + ctx*0x1000` | claim（读）/ complete（写） |
//!
//! context：单 hart 假设下 ctx0=M、ctx1=S（与 qemu-virt 一致）。
//! 设备侧可经 `set_pending` 抬高/清除某 source 的 pending；
//! `check_interrupt` 分别检查 M/S context，返回 MEI / SEI。

use rremu_bus::{AccessSize, BusDevice, BusError, EventSink};

const NUM_SOURCES: usize = 1024;
const WORDS: usize = NUM_SOURCES / 32;
const NUM_CONTEXTS: usize = 2;

const PENDING_BASE: u32 = 0x1000;
const ENABLE_BASE: u32 = 0x2000;
const ENABLE_STRIDE: u32 = 0x80;
const CONTEXT_BASE: u32 = 0x200_000;
const CONTEXT_STRIDE: u32 = 0x1000;

/// 设备映射尺寸（qemu-virt 使用 0x400000）。
pub const PLIC_SIZE: u32 = 0x40_0000;

/// 机器外部中断 cause（M_EI，bit31 置位）。
const CAUSE_M_EI: u32 = 0x8000_000b;
/// 监督者外部中断 cause（S_EI，bit31 置位）。
const CAUSE_S_EI: u32 = 0x8000_0009;

pub struct Plic {
    base: u32,
    priority: [u32; NUM_SOURCES],
    pending: [u32; WORDS],
    enable: [[u32; WORDS]; NUM_CONTEXTS],
    threshold: [u32; NUM_CONTEXTS],
}

impl Plic {
    pub fn new(base: u32) -> Self {
        Self {
            base,
            priority: [0; NUM_SOURCES],
            pending: [0; WORDS],
            enable: [[0; WORDS]; NUM_CONTEXTS],
            threshold: [0; NUM_CONTEXTS],
        }
    }

    /// 抬高/清除某 source 的 pending（供设备侧调用）。
    pub fn set_pending(&mut self, source: u32, pending: bool) {
        if source == 0 || source as usize >= NUM_SOURCES {
            return;
        }
        let w = source as usize / 32;
        let b = source % 32;
        if pending {
            self.pending[w] |= 1 << b;
        } else {
            self.pending[w] &= !(1 << b);
        }
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }

    /// claim：返回该 context 可用的最高优先级 source（无则 0），并清除其 pending。
    fn claim(&mut self, ctx: usize) -> u32 {
        let mut best = 0u32;
        let mut best_prio = 0u32;
        for w in 0..WORDS {
            let mut bits = self.pending[w] & self.enable[ctx][w];
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                let src = (w * 32) as u32 + b;
                let prio = self.priority[src as usize];
                if prio > best_prio && prio > self.threshold[ctx] {
                    best_prio = prio;
                    best = src;
                }
            }
        }
        if best != 0 {
            self.set_pending(best, false);
        }
        best
    }

    /// 读取 offset 处的 32 位寄存器值。
    fn read_reg(&self, off: u32) -> u32 {
        match off {
            o if o < PENDING_BASE => {
                // priority
                self.priority[(o / 4) as usize % NUM_SOURCES]
            }
            o if (PENDING_BASE..PENDING_BASE + (WORDS as u32) * 4).contains(&o) => {
                self.pending[((o - PENDING_BASE) / 4) as usize]
            }
            o if (ENABLE_BASE..CONTEXT_BASE).contains(&o) => {
                let ctx = ((o - ENABLE_BASE) / ENABLE_STRIDE) as usize;
                let word = (((o - ENABLE_BASE) % ENABLE_STRIDE) / 4) as usize;
                if ctx < NUM_CONTEXTS && word < WORDS {
                    self.enable[ctx][word]
                } else {
                    0
                }
            }
            o if o >= CONTEXT_BASE => {
                let ctx = ((o - CONTEXT_BASE) / CONTEXT_STRIDE) as usize;
                let reg = (o - CONTEXT_BASE) % CONTEXT_STRIDE;
                if ctx < NUM_CONTEXTS {
                    match reg {
                        0 => self.threshold[ctx],
                        4 => 0, // claim 需 &mut，见 read()
                        _ => 0,
                    }
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

impl BusDevice for Plic {
    fn read(&mut self, addr: u32, size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
        let off = self.offset(addr);
        // claim（读）有副作用
        if off >= CONTEXT_BASE {
            let ctx = ((off - CONTEXT_BASE) / CONTEXT_STRIDE) as usize;
            let reg = (off - CONTEXT_BASE) % CONTEXT_STRIDE;
            if reg == 4 && ctx < NUM_CONTEXTS {
                return Ok(self.claim(ctx));
            }
        }
        let word = self.read_reg(off);
        Ok(extract(off, word, size))
    }

    fn write(&mut self, addr: u32, size: AccessSize, data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
        let off = self.offset(addr);
        let word_mask = match size {
            AccessSize::Byte => 0xff,
            AccessSize::Half => 0xffff,
            AccessSize::Word => 0xffff_ffff,
        };
        let data = data & word_mask;

        match off {
            o if o < PENDING_BASE => {
                self.priority[(o / 4) as usize % NUM_SOURCES] = data;
            }
            o if (ENABLE_BASE..CONTEXT_BASE).contains(&o) => {
                let ctx = ((o - ENABLE_BASE) / ENABLE_STRIDE) as usize;
                let word = (((o - ENABLE_BASE) % ENABLE_STRIDE) / 4) as usize;
                if ctx < NUM_CONTEXTS && word < WORDS {
                    self.enable[ctx][word] = data;
                }
            }
            o if o >= CONTEXT_BASE => {
                let ctx = ((o - CONTEXT_BASE) / CONTEXT_STRIDE) as usize;
                let reg = (o - CONTEXT_BASE) % CONTEXT_STRIDE;
                if ctx < NUM_CONTEXTS && reg == 0 {
                    self.threshold[ctx] = data;
                }
                // reg==4：complete 写，当前无 per-claim 跟踪，忽略
            }
            _ => {} // pending 区只读；其余忽略
        }
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        // 无副作用：claim 处返回 0
        let off = self.offset(addr);
        let word = self.read_reg(off);
        Ok(extract(off, word, size))
    }

    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Err(BusError::Device { addr: self.base, msg: "cannot bulk-write to plic".to_owned() })
    }

    fn device_name(&self) -> &str {
        "plic"
    }

    fn check_interrupt(&self) -> Vec<u32> {
        let mut causes = Vec::new();
        // ctx0 = M，ctx1 = S；两个 context 的使能/pending 独立。
        for (ctx, cause) in [(0, CAUSE_M_EI), (1, CAUSE_S_EI)] {
            if (0..WORDS).any(|w| self.pending[w] & self.enable[ctx][w] != 0) {
                causes.push(cause);
            }
        }
        causes
    }

    /// 接收设备 IRQ 断言：置对应 source 的 pending。
    fn irq_assert(&mut self, source: u32) {
        self.set_pending(source, true);
    }
}

/// 从 32 位寄存器值按 offset 与宽度提取。
fn extract(off: u32, word: u32, size: AccessSize) -> u32 {
    let shift = (off % 4) * 8;
    match size {
        AccessSize::Byte => (word >> shift) & 0xff,
        AccessSize::Half => (word >> shift) & 0xffff,
        AccessSize::Word => word,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    const BASE: u32 = 0x0c00_0000;

    #[test]
    fn priority_and_threshold_roundtrip() {
        let mut p = Plic::new(BASE);
        let mut sink = rremu_bus::NullSink;
        // priority[10] = 5
        p.write(BASE + 10 * 4, AccessSize::Word, 5, &mut sink).unwrap();
        assert_eq!(p.read(BASE + 10 * 4, AccessSize::Word, &mut sink).unwrap(), 5);
        // threshold ctx0
        p.write(BASE + CONTEXT_BASE, AccessSize::Word, 1, &mut sink).unwrap();
        assert_eq!(p.read(BASE + CONTEXT_BASE, AccessSize::Word, &mut sink).unwrap(), 1);
    }

    #[test]
    fn claim_returns_enabled_pending() {
        let mut p = Plic::new(BASE);
        let mut sink = rremu_bus::NullSink;
        // enable source 10 for ctx0（source 10 位于 enable 位图 word 0）
        p.write(BASE + ENABLE_BASE, AccessSize::Word, 1 << 10, &mut sink).unwrap();
        // priority[10]=7, threshold=0
        p.write(BASE + 10 * 4, AccessSize::Word, 7, &mut sink).unwrap();
        // raise pending, then claim
        p.set_pending(10, true);
        assert_eq!(p.check_interrupt(), vec![CAUSE_M_EI]);
        let claim = p.read(BASE + CONTEXT_BASE + 4, AccessSize::Word, &mut sink).unwrap();
        assert_eq!(claim, 10);
        // 已被清除
        assert!(p.check_interrupt().is_empty());
    }

    #[test]
    fn unknown_offset_no_panic() {
        let mut p = Plic::new(BASE);
        let mut sink = rremu_bus::NullSink;
        assert_eq!(p.read(BASE + 0x3000, AccessSize::Word, &mut sink).unwrap(), 0);
        p.write(BASE + 0x3000, AccessSize::Word, 0xDEAD, &mut sink).unwrap();
    }
    #[test]
    fn s_context_claim_returns_s_ei() {
        let mut p = Plic::new(BASE);
        let mut sink = rremu_bus::NullSink;
        // 只 enable S context(1) 的 source 10
        p.write(BASE + ENABLE_BASE + ENABLE_STRIDE, AccessSize::Word, 1 << 10, &mut sink).unwrap();
        p.write(BASE + 10 * 4, AccessSize::Word, 7, &mut sink).unwrap();
        p.set_pending(10, true);
        assert_eq!(p.check_interrupt(), vec![CAUSE_S_EI]);
        let claim = p
            .read(BASE + CONTEXT_BASE + CONTEXT_STRIDE + 4, AccessSize::Word, &mut sink)
            .unwrap();
        assert_eq!(claim, 10);
        assert!(p.check_interrupt().is_empty());
    }

}
