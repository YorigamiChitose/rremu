//! PMP 物理地址访问检查。

use rremu_isa::Priv;

use super::{AccessType, Cpu};

impl Cpu {
    /// PMP 物理地址检查。返回 true 表示允许访问。
    ///
    /// - 第一个匹配项决定权限（低编号优先）
    /// - M-mode 默认绕过未锁定项；匹配到锁定项时必须满足权限
    /// - M-mode 无匹配且存在锁定项时拒绝；S/U 无匹配时拒绝
    pub(crate) fn pmp_check(&self, addr: u32, access: AccessType) -> bool {
        let addr = u64::from(addr);
        let mut any_locked = false;
        for i in 0..16usize {
            let cfg = ((self.csr.pmpcfg[i / 4] >> ((i % 4) * 8)) & 0xff) as u8;
            let locked = cfg & 0x80 != 0;
            any_locked |= locked;
            let a = (cfg >> 3) & 0x3;
            if a == 0 {
                continue;
            }
            let (start, end) = match a {
                1 => {
                    // TOR
                    let start = if i == 0 {
                        0
                    } else {
                        u64::from(self.csr.pmpaddr[i - 1]) << 2
                    };
                    (start, u64::from(self.csr.pmpaddr[i]) << 2)
                }
                2 => {
                    // NA4
                    let start = u64::from(self.csr.pmpaddr[i]) << 2;
                    (start, start + 4)
                }
                _ => {
                    // NAPOT：pmpaddr 低位连续的 1 编码区间大小
                    let p = self.csr.pmpaddr[i];
                    let ones = p.trailing_ones();
                    if ones >= 32 {
                        (0, 1u64 << 35)
                    } else {
                        let mask = (1u64 << ones) - 1;
                        let start = (u64::from(p) & !mask) << 2;
                        (start, start + (1u64 << (ones + 3)))
                    }
                }
            };
            if addr < start || addr >= end {
                continue;
            }
            if self.priv_ == Priv::Machine && !locked {
                return true;
            }
            return match access {
                AccessType::Fetch => cfg & 0x04 != 0,
                AccessType::Load => cfg & 0x01 != 0,
                AccessType::Store => cfg & 0x02 != 0,
            };
        }
        if self.priv_ == Priv::Machine {
            !any_locked
        } else {
            false
        }
    }
}
