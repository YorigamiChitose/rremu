//! CSR instruction helpers.

use super::{Cpu, TrapErr};

impl Cpu {
    fn write_rd(&mut self, rd: u8, val: u32) {
        if rd != 0 {
            self.x[usize::from(rd)] = val;
        }
    }

    pub(crate) fn exec_csrrw(
        &mut self,
        rd: u8,
        csr: u16,
        rs1: u8,
        raw: u32,
    ) -> Result<(), TrapErr> {
        self.check_csr_access(csr, true, raw)?;
        let old = self.architectural_csr_read(csr)?;
        let val = self.x[usize::from(rs1)];
        self.csr_write(csr, val);
        self.write_rd(rd, old);
        Ok(())
    }

    pub(crate) fn exec_csrrs(
        &mut self,
        rd: u8,
        csr: u16,
        rs1: u8,
        raw: u32,
    ) -> Result<(), TrapErr> {
        let mask = self.x[usize::from(rs1)];
        self.check_csr_access(csr, mask != 0, raw)?;
        let old = self.architectural_csr_read(csr)?;
        if mask != 0 {
            self.csr_write(csr, old | mask);
        }
        self.write_rd(rd, old);
        Ok(())
    }

    pub(crate) fn exec_csrrc(
        &mut self,
        rd: u8,
        csr: u16,
        rs1: u8,
        raw: u32,
    ) -> Result<(), TrapErr> {
        let mask = self.x[usize::from(rs1)];
        self.check_csr_access(csr, mask != 0, raw)?;
        let old = self.architectural_csr_read(csr)?;
        if mask != 0 {
            self.csr_write(csr, old & !mask);
        }
        self.write_rd(rd, old);
        Ok(())
    }

    pub(crate) fn exec_csrrwi(
        &mut self,
        rd: u8,
        csr: u16,
        zimm: u8,
        raw: u32,
    ) -> Result<(), TrapErr> {
        self.check_csr_access(csr, true, raw)?;
        let old = self.architectural_csr_read(csr)?;
        self.csr_write(csr, u32::from(zimm));
        self.write_rd(rd, old);
        Ok(())
    }

    pub(crate) fn exec_csrrsi(
        &mut self,
        rd: u8,
        csr: u16,
        zimm: u8,
        raw: u32,
    ) -> Result<(), TrapErr> {
        let mask = u32::from(zimm);
        self.check_csr_access(csr, mask != 0, raw)?;
        let old = self.architectural_csr_read(csr)?;
        if mask != 0 {
            self.csr_write(csr, old | mask);
        }
        self.write_rd(rd, old);
        Ok(())
    }

    pub(crate) fn exec_csrrci(
        &mut self,
        rd: u8,
        csr: u16,
        zimm: u8,
        raw: u32,
    ) -> Result<(), TrapErr> {
        let mask = u32::from(zimm);
        self.check_csr_access(csr, mask != 0, raw)?;
        let old = self.architectural_csr_read(csr)?;
        if mask != 0 {
            self.csr_write(csr, old & !mask);
        }
        self.write_rd(rd, old);
        Ok(())
    }
}
