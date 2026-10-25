//! CLINT（Core Local Interruptor）设备：msip / mtimecmp / mtime。
//!
//! | offset  | 寄存器    | 宽度 | 行为 |
//! |---------|-----------|------|------|
//! | 0x0000  | msip      | 4B   | RW，bit0 为软件中断 pending（写 1 置位，写 0 清除） |
//! | 0x4000  | mtimecmp  | 4B   | RW，定时器比较值（RV32 低 32 位模型） |
//! | 0xBFF8  | mtime     | 4B   | RO，由 `tick(cycle)` 更新（= cycle as u32） |
//!
//! | `check_interrupt()` 返回当前全部 pending 的中断 cause 列表（含 bit31）：
//! | msip != 0 → MSI (0x8000_0003)
//! | mtime >= mtimecmp → MTI (0x8000_0007)
//! | 同时 pending 时两者都返回（不裁剪），由 CPU 按 spec 序裁决

use rremu_bus::{AccessSize, BusDevice, BusError, EventSink, Resetable};

const REG_MSIP: u32 = 0x0000;
const REG_MTIMECMP: u32 = 0x4000;
const REG_MTIME: u32 = 0xBFF8;

// 寄存器范围结束偏移（独占）
const REG_MSIP_END: u32 = REG_MSIP + 4;
const REG_MTIMECMP_END: u32 = REG_MTIMECMP + 4;
const REG_MTIME_END: u32 = REG_MTIME + 4;

/// 中断 cause 码（bit31 置位表示中断）。
const CAUSE_MSI: u32 = 0x8000_0003;
const CAUSE_MTI: u32 = 0x8000_0007;

pub struct Clint {
    base: u32,
    msip: u32,
    mtimecmp: u32,
    mtime: u32,
}

impl Clint {
    pub fn new(base: u32) -> Self {
        // mtimecmp 初始 u32::MAX，避免复位后 mtime(0) >= mtimecmp(0) 立即产生虚假 MTI
        Self {
            base,
            msip: 0,
            mtimecmp: u32::MAX,
            mtime: 0,
        }
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }

    /// 定位寄存器：返回 (寄存器起始偏移, 当前值)；未知偏移返回 None。
    fn reg_at(&self, off: u32) -> Option<(u32, u32)> {
        if off < REG_MSIP_END {
            Some((REG_MSIP, self.msip))
        } else if (REG_MTIMECMP..REG_MTIMECMP_END).contains(&off) {
            Some((REG_MTIMECMP, self.mtimecmp))
        } else if (REG_MTIME..REG_MTIME_END).contains(&off) {
            Some((REG_MTIME, self.mtime))
        } else {
            None
        }
    }

    /// 从 32 位寄存器值中按偏移和宽度提取数据。
    fn extract(off: u32, reg_off: u32, val: u32, size: AccessSize) -> u32 {
        let byte_idx = (off - reg_off) as usize;
        let shift = 8 * byte_idx;
        match size {
            AccessSize::Byte => (val >> shift) & 0xff,
            AccessSize::Half => (val >> shift) & 0xffff,
            AccessSize::Word => val,
        }
    }

    /// 部分写合并：按 reg 内字节偏移与宽度，把 data 的对应字节并入 old。
    fn merge_reg(old: u32, byte_idx: usize, size: AccessSize, data: u32) -> u32 {
        let shift = 8 * byte_idx;
        let mask = match size {
            AccessSize::Byte => 0xff_u32 << shift,
            AccessSize::Half => 0xffff << shift,
            AccessSize::Word => 0xffff_ffff,
        };
        (old & !mask) | ((data << shift) & mask)
    }
}

impl BusDevice for Clint {
    fn read(
        &mut self,
        addr: u32,
        size: AccessSize,
        _sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        let off = self.offset(addr);
        let (reg_off, val) = self.reg_at(off).unwrap_or((off, 0));
        Ok(Self::extract(off, reg_off, val, size))
    }

    fn write(
        &mut self,
        addr: u32,
        size: AccessSize,
        data: u32,
        _sink: &mut dyn EventSink,
    ) -> Result<(), BusError> {
        let off = self.offset(addr);
        if off < REG_MSIP_END {
            let byte_idx = (off - REG_MSIP) as usize;
            self.msip = Self::merge_reg(self.msip, byte_idx, size, data) & 1;
        } else if (REG_MTIMECMP..REG_MTIMECMP_END).contains(&off) {
            let byte_idx = (off - REG_MTIMECMP) as usize;
            self.mtimecmp = Self::merge_reg(self.mtimecmp, byte_idx, size, data);
        }
        // mtime 和未知偏移：写忽略
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        let mut sink = rremu_bus::NullSink;
        self.read(addr, size, &mut sink)
    }

    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Err(BusError::Device {
            addr: self.base,
            msg: "cannot bulk-write to clint".to_owned(),
        })
    }

    fn device_name(&self) -> &str {
        "clint"
    }

    fn check_interrupt(&self) -> Vec<u32> {
        let mut causes = Vec::new();
        if self.msip & 1 != 0 {
            causes.push(CAUSE_MSI);
        }
        if self.mtime >= self.mtimecmp {
            causes.push(CAUSE_MTI);
        }
        causes
    }

    fn tick(&mut self, cycle: u64) {
        self.mtime = cycle as u32;
    }

    fn as_resetable(&mut self) -> Option<&mut dyn Resetable> {
        Some(self)
    }
}

impl Resetable for Clint {
    fn reset(&mut self) {
        self.msip = 0;
        self.mtimecmp = u32::MAX;
        self.mtime = 0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    const BASE: u32 = 0x0200_0000;

    #[test]
    fn mtime_below_mtimecmp_no_interrupt() {
        let mut clint = Clint::new(BASE);
        clint.mtimecmp = 100;
        clint.mtime = 50;
        assert!(clint.check_interrupt().is_empty());
    }

    #[test]
    fn mtime_ge_mtimecmp_returns_timer_interrupt() {
        let mut clint = Clint::new(BASE);
        clint.mtimecmp = 100;
        clint.mtime = 100; // >=
        assert_eq!(clint.check_interrupt(), vec![0x8000_0007]);
        clint.mtime = 150;
        assert_eq!(clint.check_interrupt(), vec![0x8000_0007]);
    }

    #[test]
    fn msip_set_returns_software_interrupt() {
        let mut clint = Clint::new(BASE);
        clint.msip = 1;
        clint.mtimecmp = u32::MAX; // 确保 timer 不 pending
        clint.mtime = 0;
        assert_eq!(clint.check_interrupt(), vec![0x8000_0003]);
    }

    #[test]
    fn both_pending_returns_both_causes() {
        // 设备侧不裁剪：MSI + MTI 同时 pending 时两者都返回
        let mut clint = Clint::new(BASE);
        clint.msip = 1;
        clint.mtimecmp = 10;
        clint.mtime = 20;
        let causes = clint.check_interrupt();
        assert!(causes.contains(&0x8000_0003)); // MSI
        assert!(causes.contains(&0x8000_0007)); // MTI
        assert_eq!(causes.len(), 2);
    }

    #[test]
    fn tick_updates_mtime_from_cycle() {
        let mut clint = Clint::new(BASE);
        assert_eq!(clint.mtime, 0);
        clint.tick(42);
        assert_eq!(clint.mtime, 42);
        clint.tick(100);
        assert_eq!(clint.mtime, 100);
    }

    #[test]
    fn new_state_has_no_pending_interrupt() {
        // 复位后 mtimecmp=u32::MAX，mtime=0 < mtimecmp，不应有虚假 MTI
        let clint = Clint::new(BASE);
        assert!(clint.check_interrupt().is_empty());
    }

    #[test]
    fn read_write_msip_word() {
        let mut clint = Clint::new(BASE);
        let mut sink = rremu_bus::NullSink;
        // 写 msip = 1
        clint
            .write(BASE + REG_MSIP, AccessSize::Word, 1, &mut sink)
            .unwrap();
        assert_eq!(clint.msip, 1);
        assert_eq!(
            clint
                .read(BASE + REG_MSIP, AccessSize::Word, &mut sink)
                .unwrap(),
            1
        );
        // 写 msip = 0 清除
        clint
            .write(BASE + REG_MSIP, AccessSize::Word, 0, &mut sink)
            .unwrap();
        assert_eq!(clint.msip, 0);
    }

    #[test]
    fn read_write_mtimecmp_word() {
        let mut clint = Clint::new(BASE);
        let mut sink = rremu_bus::NullSink;
        clint
            .write(
                BASE + REG_MTIMECMP,
                AccessSize::Word,
                0x1234_5678,
                &mut sink,
            )
            .unwrap();
        assert_eq!(clint.mtimecmp, 0x1234_5678);
        assert_eq!(
            clint
                .read(BASE + REG_MTIMECMP, AccessSize::Word, &mut sink)
                .unwrap(),
            0x1234_5678
        );
    }

    #[test]
    fn read_mtime_word() {
        let mut clint = Clint::new(BASE);
        let mut sink = rremu_bus::NullSink;
        clint.tick(99);
        assert_eq!(
            clint
                .read(BASE + REG_MTIME, AccessSize::Word, &mut sink)
                .unwrap(),
            99
        );
    }

    #[test]
    fn mtime_write_ignored() {
        let mut clint = Clint::new(BASE);
        let mut sink = rremu_bus::NullSink;
        clint.tick(50);
        // 尝试写 mtime → 应被忽略
        clint
            .write(BASE + REG_MTIME, AccessSize::Word, 0, &mut sink)
            .unwrap();
        assert_eq!(clint.mtime, 50); // 未变
    }

    #[test]
    fn byte_access_to_mtimecmp() {
        let mut clint = Clint::new(BASE);
        let mut sink = rremu_bus::NullSink;
        // 写 mtimecmp 的低字节
        clint
            .write(
                BASE + REG_MTIMECMP,
                AccessSize::Word,
                0x00FF_00AA,
                &mut sink,
            )
            .unwrap();
        assert_eq!(clint.mtimecmp, 0x00FF_00AA);
        // 读低字节
        assert_eq!(
            clint
                .read(BASE + REG_MTIMECMP, AccessSize::Byte, &mut sink)
                .unwrap(),
            0xAA
        );
        // 读第二字节
        assert_eq!(
            clint
                .read(BASE + REG_MTIMECMP + 1, AccessSize::Byte, &mut sink)
                .unwrap(),
            0x00
        );
        assert_eq!(
            clint
                .read(BASE + REG_MTIMECMP + 2, AccessSize::Byte, &mut sink)
                .unwrap(),
            0xFF
        );
        assert_eq!(
            clint
                .read(BASE + REG_MTIMECMP + 3, AccessSize::Byte, &mut sink)
                .unwrap(),
            0x00
        );
    }

    #[test]
    fn unknown_register_reads_zero() {
        let mut clint = Clint::new(BASE);
        let mut sink = rremu_bus::NullSink;
        assert_eq!(
            clint
                .read(BASE + 0x1000, AccessSize::Word, &mut sink)
                .unwrap(),
            0
        );
        clint
            .write(BASE + 0x1000, AccessSize::Word, 0xDEAD, &mut sink)
            .unwrap();
        assert_eq!(
            clint
                .read(BASE + 0x1000, AccessSize::Word, &mut sink)
                .unwrap(),
            0
        );
    }

    #[test]
    fn peek_read_equivalent_to_read() {
        let mut clint = Clint::new(BASE);
        clint.mtimecmp = 0xBEEF;
        clint.tick(42);
        assert_eq!(
            clint
                .peek_read(BASE + REG_MTIMECMP, AccessSize::Word)
                .unwrap(),
            0xBEEF
        );
        assert_eq!(
            clint.peek_read(BASE + REG_MTIME, AccessSize::Word).unwrap(),
            42
        );
    }
}
