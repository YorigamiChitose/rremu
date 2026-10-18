//! Sv32 地址翻译与页表权限检查。

use rremu_bus::{AccessSize, Bus, EventSink};
use rremu_isa::{cause, csr, Priv};

use super::{Cpu, TrapErr};

/// 地址翻译的访问类型（决定 page fault cause 与权限检查）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessType {
    Fetch,
    Load,
    Store,
}

impl AccessType {
    fn page_fault(self) -> u32 {
        match self {
            AccessType::Fetch => cause::INSTR_PAGE_FAULT,
            AccessType::Load => cause::LOAD_PAGE_FAULT,
            AccessType::Store => cause::STORE_PAGE_FAULT,
        }
    }

    fn access_fault(self) -> u32 {
        match self {
            AccessType::Fetch => cause::INSTR_ACCESS_FAULT,
            AccessType::Load => cause::LOAD_ACCESS_FAULT,
            AccessType::Store => cause::STORE_ACCESS_FAULT,
        }
    }
}

// Sv32 PTE 位。
pub(crate) const PTE_V: u32 = 1 << 0;
pub(crate) const PTE_R: u32 = 1 << 1;
pub(crate) const PTE_W: u32 = 1 << 2;
pub(crate) const PTE_X: u32 = 1 << 3;
pub(crate) const PTE_U: u32 = 1 << 4;
pub(crate) const PTE_A: u32 = 1 << 6;
pub(crate) const PTE_D: u32 = 1 << 7;

impl Cpu {
    /// 虚拟地址 → 物理地址翻译的**唯一收口**。
    ///
    /// 支持 Sv32：S/U（以及 MPRV 下的 M 数据访问）在 `satp.MODE=1` 时走
    /// 两级页表；其余情况保持 bare 直通。所有取指/访存路径都经过此函数。
    #[inline]
    pub(crate) fn translate<B: Bus>(
        &self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        va: u32,
        access: AccessType,
    ) -> Result<u32, TrapErr> {
        let priv_ = self.effective_priv(access);
        if priv_ == Priv::Machine || (self.csr.satp & csr::SATP_MODE_MASK) == 0 {
            return Ok(va);
        }

        let root_ppn = self.csr.satp & csr::SATP_PPN_MASK;
        let root = root_ppn << 12;
        let vpn1 = (va >> 22) & 0x3ff;
        let pte_addr1 = root.wrapping_add(vpn1 << 2);
        let pte1 = self.read_pte(bus, sink, pte_addr1, va, access)?;
        if pte1 & PTE_V == 0 {
            return Err(TrapErr { cause: access.page_fault(), mtval: va });
        }

        if pte1 & (PTE_R | PTE_W | PTE_X) == 0 {
            // 非叶 PTE：走第二级。
            let ppn = (pte1 >> 10) & csr::SATP_PPN_MASK;
            let vpn0 = (va >> 12) & 0x3ff;
            let pte_addr0 = (ppn << 12).wrapping_add(vpn0 << 2);
            let pte0 = self.read_pte(bus, sink, pte_addr0, va, access)?;
            if pte0 & PTE_V == 0 || pte0 & (PTE_R | PTE_W | PTE_X) == 0 {
                return Err(TrapErr { cause: access.page_fault(), mtval: va });
            }
            return self.translate_leaf(bus, sink, va, access, priv_, pte_addr0, pte0, false);
        }

        // 一级叶 PTE：4MiB 大页。
        self.translate_leaf(bus, sink, va, access, priv_, pte_addr1, pte1, true)
    }

    /// MPRV 下 M-mode 数据访问按 MPP 指定的特权级翻译；取指始终按当前特权级。
    fn effective_priv(&self, access: AccessType) -> Priv {
        if self.priv_ == Priv::Machine
            && access != AccessType::Fetch
            && (self.csr.mstatus & csr::MSTATUS_MPRV) != 0
        {
            return match (self.csr.mstatus >> csr::MSTATUS_MPP_SHIFT) & 0b11 {
                0b00 => Priv::User,
                0b01 => Priv::Supervisor,
                _ => Priv::Machine,
            };
        }
        self.priv_
    }

    fn read_pte<B: Bus>(
        &self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        addr: u32,
        va: u32,
        access: AccessType,
    ) -> Result<u32, TrapErr> {
        bus.data_read(addr, AccessSize::Word, sink)
            .map_err(|_| TrapErr { cause: access.access_fault(), mtval: va })
    }

    #[allow(clippy::too_many_arguments)]
    fn translate_leaf<B: Bus>(
        &self,
        bus: &mut B,
        sink: &mut dyn EventSink,
        va: u32,
        access: AccessType,
        priv_: Priv,
        pte_addr: u32,
        pte: u32,
        superpage: bool,
    ) -> Result<u32, TrapErr> {
        let r = pte & PTE_R != 0;
        let w = pte & PTE_W != 0;
        let x = pte & PTE_X != 0;
        if (w && !r) || (!r && !w && !x) {
            return Err(TrapErr { cause: access.page_fault(), mtval: va });
        }

        let user = pte & PTE_U != 0;
        if priv_ == Priv::User && !user {
            return Err(TrapErr { cause: access.page_fault(), mtval: va });
        }
        if priv_ == Priv::Supervisor && user {
            if access == AccessType::Fetch {
                // S-mode 永不执行 U 页；SUM 只影响 load/store。
                return Err(TrapErr { cause: access.page_fault(), mtval: va });
            }
            if (self.csr.mstatus & csr::MSTATUS_SUM) == 0 {
                return Err(TrapErr { cause: access.page_fault(), mtval: va });
            }
        }

        let mxr = self.csr.mstatus & csr::MSTATUS_MXR != 0;
        let allowed = match access {
            AccessType::Fetch => x,
            AccessType::Load => r || (mxr && x),
            AccessType::Store => w,
        };
        if !allowed {
            return Err(TrapErr { cause: access.page_fault(), mtval: va });
        }

        let ppn = (pte >> 10) & csr::SATP_PPN_MASK;
        let pa = if superpage {
            // 4MiB 大页要求 PPN[0] 为零。
            if ppn & 0x3ff != 0 {
                return Err(TrapErr { cause: access.page_fault(), mtval: va });
            }
            (ppn << 12) | (va & 0x3f_ffff)
        } else {
            (ppn << 12) | (va & 0xfff)
        };

        // A/D 硬件更新；写 PTE 失败按 page fault 处理。
        let mut new_pte = pte | PTE_A;
        if access == AccessType::Store {
            new_pte |= PTE_D;
        }
        if new_pte != pte {
            bus.data_write(pte_addr, AccessSize::Word, new_pte, sink)
                .map_err(|_| TrapErr { cause: access.page_fault(), mtval: va })?;
        }
        Ok(pa)
    }

}
