//! 设备中断投影与优先级仲裁。

use rremu_isa::{cause, csr, Priv};

use super::Cpu;

impl Cpu {
    /// 将设备 pending 中断投影到 mip 的设备位。
    /// SSIP/STIP 由软件写入（如 OpenSBI 注入 S timer），不被设备覆盖。
    pub(crate) fn sync_mip(&mut self, pending: &[u32]) {
        let mut dev = 0u32;
        for &c in pending {
            dev |= match c {
                cause::M_SI => csr::MIP_MSIP,
                cause::M_TI => csr::MIP_MTIP,
                cause::M_EI => csr::MIP_MEIP,
                cause::S_EI => csr::MIP_SEIP,
                _ => 0,
            };
        }
        let mask = csr::MIP_MSIP | csr::MIP_MTIP | csr::MIP_MEIP | csr::MIP_SEIP;
        self.csr.mip = (self.csr.mip & !mask) | dev;
    }

    /// 按规范优先级选择当前可接收的中断。
    ///
    /// 目标为 M 的中断：当前 `< M` 时全局使能；当前为 M 时需 MIE。
    /// 目标为 S 的中断：当前 `< S` 时全局使能；当前为 S 时需 SIE；
    /// 当前为 M 时不接收已委派给 S 的中断。
    pub(crate) fn pending_interrupt(&self) -> Option<u32> {
        const PRIORITY: [u32; 6] = [
            cause::M_EI,
            cause::M_SI,
            cause::M_TI,
            cause::S_EI,
            cause::S_SI,
            cause::S_TI,
        ];
        for &cause in &PRIORITY {
            let code = cause & 0x7fff_ffff;
            if (self.csr.mip >> code) & 1 == 0 || (self.csr.mie >> code) & 1 == 0 {
                continue;
            }
            let delegated = (self.csr.mideleg >> code) & 1 == 1;
            let enabled = if delegated {
                match self.priv_ {
                    Priv::Machine => false,
                    Priv::Supervisor => (self.csr.mstatus & csr::MSTATUS_SIE) != 0,
                    Priv::User => true,
                }
            } else {
                match self.priv_ {
                    Priv::Machine => (self.csr.mstatus & csr::MSTATUS_MIE) != 0,
                    _ => true,
                }
            };
            if enabled {
                return Some(cause);
            }
        }
        None
    }
}
