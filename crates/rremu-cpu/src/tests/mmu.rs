use super::*;

const MMU_VA: u32 = 0x4000_0000;
const ROOT_PA: u32 = BASE + 0x1000;
const L1_PA: u32 = BASE + 0x2000;
const DATA_PA: u32 = BASE + 0x3000;

fn write_u32(h: &mut Harness, addr: u32, val: u32) {
    h.bus.write_bytes(addr, &val.to_le_bytes()).unwrap();
}

fn read_u32(h: &mut Harness, addr: u32) -> u32 {
    h.bus.peek(addr, AccessSize::Word).unwrap()
}

fn pte_ppn(pa: u32) -> u32 {
    (pa >> 12) & csr::SATP_PPN_MASK
}

fn enable_sv32(h: &mut Harness, root: u32) {
    h.cpu.priv_ = Priv::Supervisor;
    h.cpu
        .csr_write(csr::SATP, csr::SATP_MODE_SV32 | pte_ppn(root));
}

/// 建立 4KiB 映射，返回第二级 PTE 地址。
fn map_4k(h: &mut Harness, va: u32, pa: u32, flags: u32) -> u32 {
    let root_pte = ROOT_PA + ((va >> 22) & 0x3ff) * 4;
    write_u32(h, root_pte, PTE_V | (pte_ppn(L1_PA) << 10));
    let leaf = L1_PA + ((va >> 12) & 0x3ff) * 4;
    write_u32(h, leaf, PTE_V | flags | (pte_ppn(pa) << 10));
    leaf
}

#[test]
fn sv32_fetch_translates_and_sets_accessed() {
    let mut h = Harness::new_with_ram(0x10000);
    let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
    enable_sv32(&mut h, ROOT_PA);
    h.load(DATA_PA, &[0x0000_0013]); // nop
    h.cpu.pc = MMU_VA;
    let out = h.step();
    assert!(matches!(out, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), MMU_VA + 4);
    assert_ne!(read_u32(&mut h, leaf) & PTE_A, 0);
}

#[test]
fn sv32_invalid_pte_raises_instruction_page_fault() {
    let mut h = Harness::new_with_ram(0x10000);
    let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
    write_u32(&mut h, leaf, 0);
    enable_sv32(&mut h, ROOT_PA);
    h.cpu.pc = MMU_VA;
    let out = h.step();
    assert_eq!(
        out,
        StepOutcome::Trapped {
            cause: cause::INSTR_PAGE_FAULT
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, MMU_VA);
}

#[test]
fn sv32_load_sets_accessed_and_store_requires_write() {
    let mut h = Harness::new_with_ram(0x10000);
    let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R);
    enable_sv32(&mut h, ROOT_PA);
    h.load(DATA_PA, &[0x1234_5678]);
    let v = h
        .cpu
        .load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word)
        .unwrap();
    assert_eq!(v, 0x1234_5678);
    assert_ne!(read_u32(&mut h, leaf) & PTE_A, 0);
    assert_eq!(read_u32(&mut h, leaf) & PTE_D, 0);

    let err = h
        .cpu
        .store(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word, 1)
        .err()
        .unwrap();
    assert_eq!(err.cause, cause::STORE_PAGE_FAULT);
    assert_eq!(err.mtval, MMU_VA);
}

#[test]
fn sv32_sum_controls_supervisor_access_to_user_page() {
    let mut h = Harness::new_with_ram(0x10000);
    map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_U);
    enable_sv32(&mut h, ROOT_PA);
    h.load(DATA_PA, &[0x55]);
    let err = h
        .cpu
        .load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word)
        .err()
        .unwrap();
    assert_eq!(err.cause, cause::LOAD_PAGE_FAULT);

    h.cpu.csr_write(csr::SSTATUS, csr::MSTATUS_SUM);
    let v = h
        .cpu
        .load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word)
        .unwrap();
    assert_eq!(v, 0x55);
}

#[test]
fn sv32_superpage_maps_4mib() {
    let mut h = Harness::new_with_ram(0x10000);
    let root_pte = ROOT_PA + ((MMU_VA >> 22) & 0x3ff) * 4;
    write_u32(
        &mut h,
        root_pte,
        PTE_V | PTE_R | PTE_W | PTE_X | (pte_ppn(BASE) << 10),
    );
    enable_sv32(&mut h, ROOT_PA);
    h.load(BASE, &[0x0000_0013]);
    h.cpu.pc = MMU_VA;
    let out = h.step();
    assert!(matches!(out, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), MMU_VA + 4);
}

#[test]
fn sv32_32bit_instruction_crossing_page_uses_two_translations() {
    let mut h = Harness::new_with_ram(0x10000);
    let va = MMU_VA + 0xffe;
    map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
    map_4k(
        &mut h,
        MMU_VA + 0x1000,
        BASE + 0x4000,
        PTE_R | PTE_W | PTE_X,
    );
    // addi x1, x0, 1 = 0x00100093：低半字在页尾，高半字在下一页。
    h.bus
        .write_bytes(DATA_PA + 0xffe, &0x0093u16.to_le_bytes())
        .unwrap();
    h.bus
        .write_bytes(BASE + 0x4000, &0x0010u16.to_le_bytes())
        .unwrap();
    enable_sv32(&mut h, ROOT_PA);
    h.cpu.pc = va;
    let out = h.step();
    assert!(matches!(out, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.reg(1), 1);
    assert_eq!(h.cpu.pc(), MMU_VA + 0x1002);
}

#[test]
fn sv32_compressed_instruction_at_page_end_does_not_fetch_next_page() {
    let mut h = Harness::new_with_ram(0x10000);
    let va = MMU_VA + 0xffe;
    map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_W | PTE_X);
    // c.nop = 0x0001，下一页未映射也不应被读取。
    h.bus
        .write_bytes(DATA_PA + 0xffe, &0x0001u16.to_le_bytes())
        .unwrap();
    enable_sv32(&mut h, ROOT_PA);
    h.cpu.pc = va;
    let out = h.step();
    assert!(matches!(out, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), MMU_VA + 0x1000);
}

#[test]
fn sv32_supervisor_cannot_fetch_user_page() {
    let mut h = Harness::new_with_ram(0x10000);
    map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_X | PTE_U);
    enable_sv32(&mut h, ROOT_PA);
    h.load(DATA_PA, &[0x0000_0013]); // nop
    h.cpu.pc = MMU_VA;
    let out = h.step();
    assert_eq!(
        out,
        StepOutcome::Trapped {
            cause: cause::INSTR_PAGE_FAULT
        }
    );
    assert_eq!(h.cpu.csr_state().mtval, MMU_VA);
}

#[test]
fn sv32_supervisor_fetch_user_page_faults_even_with_sum() {
    let mut h = Harness::new_with_ram(0x10000);
    map_4k(&mut h, MMU_VA, DATA_PA, PTE_R | PTE_X | PTE_U);
    enable_sv32(&mut h, ROOT_PA);
    h.cpu.csr_write(csr::SSTATUS, csr::MSTATUS_SUM);
    h.load(DATA_PA, &[0x0000_0013]);
    h.cpu.pc = MMU_VA;
    assert_eq!(
        h.step(),
        StepOutcome::Trapped {
            cause: cause::INSTR_PAGE_FAULT
        }
    );
}

#[test]
fn mprv_uses_mpp_for_m_mode_data_translation() {
    let mut h = Harness::new_with_ram(0x10000);
    let leaf = map_4k(&mut h, MMU_VA, DATA_PA, PTE_R);
    enable_sv32(&mut h, ROOT_PA);
    h.load(DATA_PA, &[0x1234_5678]);

    // MPRV=1, MPP=S: M-mode data access must use Sv32 translation.
    h.cpu.priv_ = Priv::Machine;
    h.cpu.csr.mstatus = csr::MSTATUS_MPRV | csr::MSTATUS_MPP_S;
    let v = h
        .cpu
        .load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word)
        .unwrap();
    assert_eq!(v, 0x1234_5678);
    assert_ne!(read_u32(&mut h, leaf) & PTE_A, 0);

    // MPRV=0: M-mode access is bare, so the same VA is not mapped.
    h.cpu.csr.mstatus = csr::MSTATUS_MPP_S;
    let err = h
        .cpu
        .load(&mut h.bus, &mut h.sink, MMU_VA, AccessSize::Word)
        .err()
        .unwrap();
    assert_eq!(err.cause, cause::LOAD_ACCESS_FAULT);
}

#[test]
fn mprv_sum_mxr_matrix_for_m_mode_data_access() {
    let mut h = Harness::new_with_ram(0x10000);
    let va_user = MMU_VA;
    let va_exec = MMU_VA + 0x1000;
    map_4k(&mut h, va_user, DATA_PA, PTE_R | PTE_U);
    map_4k(&mut h, va_exec, DATA_PA, PTE_X);
    enable_sv32(&mut h, ROOT_PA);
    h.load(DATA_PA, &[0x1234_5678]);

    h.cpu.priv_ = Priv::Machine;
    h.cpu.csr.mstatus = csr::MSTATUS_MPRV | csr::MSTATUS_MPP_S;

    // SUM=0: user page is inaccessible for M-mode load through MPRV.
    let err = h
        .cpu
        .load(&mut h.bus, &mut h.sink, va_user, AccessSize::Word)
        .err()
        .unwrap();
    assert_eq!(err.cause, cause::LOAD_PAGE_FAULT);
    h.cpu.csr.mstatus |= csr::MSTATUS_SUM;
    let v = h
        .cpu
        .load(&mut h.bus, &mut h.sink, va_user, AccessSize::Word)
        .unwrap();
    assert_eq!(v, 0x1234_5678);

    // MXR=0: execute-only page cannot be loaded; MXR=1 makes it readable.
    h.cpu.csr.mstatus = csr::MSTATUS_MPRV | csr::MSTATUS_MPP_S;
    let err = h
        .cpu
        .load(&mut h.bus, &mut h.sink, va_exec, AccessSize::Word)
        .err()
        .unwrap();
    assert_eq!(err.cause, cause::LOAD_PAGE_FAULT);
    h.cpu.csr.mstatus |= csr::MSTATUS_MXR;
    let v = h
        .cpu
        .load(&mut h.bus, &mut h.sink, va_exec, AccessSize::Word)
        .unwrap();
    assert_eq!(v, 0x1234_5678);
}
