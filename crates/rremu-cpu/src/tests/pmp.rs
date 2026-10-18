use super::*;

    fn clear_pmp(h: &mut Harness) {
        h.cpu.csr.pmpcfg = [0; 4];
        h.cpu.csr.pmpaddr = [0; 16];
    }

    #[test]
    fn pmp_without_entries_denies_s_but_allows_m() {
        let mut h = Harness::new();
        clear_pmp(&mut h);
        h.load(BASE, &[0x0000_0013]); // nop
        h.cpu.priv_ = Priv::Supervisor;
        assert_eq!(h.step(), StepOutcome::Trapped { cause: cause::INSTR_ACCESS_FAULT });

        let mut h = Harness::new();
        clear_pmp(&mut h);
        h.load(BASE, &[0x0000_0013]);
        assert!(matches!(h.step(), StepOutcome::Executed { .. }));
    }

    #[test]
    fn pmp_napot_region_and_outside() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        // NAPOT 4KiB，覆盖 BASE..BASE+0x1000。
        h.cpu.csr.pmpaddr[0] = (BASE >> 2) | ((1 << 9) - 1);
        h.cpu.csr.pmpcfg[0] = 0x1f; // R/W/X + NAPOT
        h.cpu.priv_ = Priv::Supervisor;
        assert!(h.cpu.pmp_check(BASE, AccessType::Fetch));
        assert!(h.cpu.pmp_check(BASE + 0xfff, AccessType::Load));
        assert!(!h.cpu.pmp_check(BASE + 0x1000, AccessType::Load));
    }

    #[test]
    fn pmp_tor_uses_previous_address() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        let start = BASE + 0x800;
        let end = BASE + 0x1000;
        h.cpu.csr.pmpaddr[0] = start >> 2;
        h.cpu.csr.pmpaddr[1] = end >> 2;
        h.cpu.csr.pmpcfg[0] = 0x0f00; // entry1: TOR + R/W/X；entry0=OFF
        h.cpu.priv_ = Priv::Supervisor;
        assert!(!h.cpu.pmp_check(start - 1, AccessType::Load));
        assert!(h.cpu.pmp_check(start, AccessType::Load));
        assert!(h.cpu.pmp_check(end - 1, AccessType::Load));
        assert!(!h.cpu.pmp_check(end, AccessType::Load));
    }

    #[test]
    fn pmp_na4_matches_single_word() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        let addr = BASE + 0x100;
        h.cpu.csr.pmpaddr[0] = addr >> 2;
        h.cpu.csr.pmpcfg[0] = 0x17; // NA4 + R/W/X
        h.cpu.priv_ = Priv::Supervisor;
        assert!(h.cpu.pmp_check(addr, AccessType::Load));
        assert!(h.cpu.pmp_check(addr + 3, AccessType::Load));
        assert!(!h.cpu.pmp_check(addr + 4, AccessType::Load));
    }

    #[test]
    fn pmp_locked_entry_applies_to_m() {
        let mut h = Harness::new_with_ram(0x10000);
        clear_pmp(&mut h);
        h.cpu.csr.pmpaddr[0] = (BASE >> 2) | ((1 << 9) - 1);
        h.cpu.csr.pmpcfg[0] = 0x99; // NAPOT + R + L（无 W/X）
        h.cpu.priv_ = Priv::Machine;
        assert!(h.cpu.pmp_check(BASE, AccessType::Load));
        assert!(!h.cpu.pmp_check(BASE, AccessType::Store));
        assert!(!h.cpu.pmp_check(BASE, AccessType::Fetch));
        // 有锁定项且无匹配 → M 也拒绝
        assert!(!h.cpu.pmp_check(BASE + 0x1000, AccessType::Load));
    }
