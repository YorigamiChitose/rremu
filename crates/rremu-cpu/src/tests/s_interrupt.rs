use super::interrupt::MockIrq;
use super::*;

#[test]
fn supervisor_software_interrupt_delegated_to_s() {
    let mut h = Harness::new();
    h.load(BASE, &[0x0000_0013]); // nop
    h.cpu.priv_ = Priv::Supervisor;
    h.cpu.csr_write(csr::MSTATUS, csr::MSTATUS_SIE);
    h.cpu.csr_write(csr::MIE, csr::SIE_SSIE);
    h.cpu.csr_write(csr::MIDELEG, 1 << 1);
    h.cpu.csr_write(csr::MIP, csr::MIP_SSIP);
    h.cpu.csr_write(csr::STVEC, BASE + 0x100);
    let out = h.step();
    assert_eq!(out, StepOutcome::Trapped { cause: cause::S_SI });
    assert_eq!(h.cpu.pc(), BASE + 0x100);
    assert_eq!(h.cpu.debug_csr_read(csr::SCAUSE).unwrap(), cause::S_SI);
    assert_eq!(h.cpu.debug_csr_read(csr::STVAL).unwrap(), 0);
    assert_eq!(h.cpu.priv_(), Priv::Supervisor);
    assert_eq!(
        h.cpu.debug_csr_read(csr::SSTATUS).unwrap() & csr::MSTATUS_SPP,
        csr::MSTATUS_SPP
    );
}

#[test]
fn delegated_s_interrupt_is_not_taken_in_m() {
    let mut h = Harness::new();
    h.load(BASE, &[0x0000_0013]); // nop
    h.cpu.csr_write(csr::MSTATUS, csr::MSTATUS_MIE);
    h.cpu.csr_write(csr::MIE, csr::SIE_SSIE);
    h.cpu.csr_write(csr::MIDELEG, 1 << 1);
    h.cpu.csr_write(csr::MIP, csr::MIP_SSIP);
    let out = h.step();
    assert!(matches!(out, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), BASE + 4);
}

#[test]
fn delegated_s_interrupt_taken_in_u_without_sie() {
    let mut h = Harness::new();
    h.load(BASE, &[0x0000_0013]); // nop
    h.cpu.priv_ = Priv::User;
    h.cpu.csr_write(csr::MIE, csr::SIE_SSIE);
    h.cpu.csr_write(csr::MIDELEG, 1 << 1);
    h.cpu.csr_write(csr::MIP, csr::MIP_SSIP);
    h.cpu.csr_write(csr::STVEC, BASE + 0x100);
    let out = h.step();
    assert_eq!(out, StepOutcome::Trapped { cause: cause::S_SI });
    assert_eq!(h.cpu.priv_(), Priv::Supervisor);
    assert_eq!(
        h.cpu.debug_csr_read(csr::SSTATUS).unwrap() & csr::MSTATUS_SPP,
        0
    );
}

#[test]
fn m_interrupt_taken_in_s_without_mie() {
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_TI)),
    )
    .unwrap();
    let mut cpu = Cpu::new(BASE);
    cpu.csr.pmpaddr[0] = 0x1fff_ffff;
    cpu.csr.pmpcfg[0] = 0x1f;
    cpu.priv_ = Priv::Supervisor;
    cpu.csr_write(csr::MIE, csr::MIE_MTIE);
    cpu.csr_write(csr::MTVEC, BASE + 0x100);
    let mut h = Harness {
        cpu,
        bus,
        sink: Sink(Vec::new()),
    };
    h.load(BASE, &[0x0000_0013]); // nop
    let out = h.step();
    assert_eq!(out, StepOutcome::Trapped { cause: cause::M_TI });
    assert_eq!(h.cpu.priv_(), Priv::Machine);
    assert_eq!(
        h.cpu.debug_csr_read(csr::MSTATUS).unwrap() & csr::MSTATUS_MPP_MASK,
        csr::MSTATUS_MPP_S
    );
}

#[test]
fn plic_external_irq_injected_as_s_ei() {
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::S_EI)),
    )
    .unwrap();
    let mut cpu = Cpu::new(BASE);
    cpu.csr.pmpaddr[0] = 0x1fff_ffff;
    cpu.csr.pmpcfg[0] = 0x1f;
    cpu.priv_ = Priv::Supervisor;
    cpu.csr_write(csr::MSTATUS, csr::MSTATUS_SIE);
    cpu.csr_write(csr::MIE, csr::SIE_SEIE);
    cpu.csr_write(csr::MIDELEG, 1 << 9);
    cpu.csr_write(csr::STVEC, BASE + 0x100);
    let mut h = Harness {
        cpu,
        bus,
        sink: Sink(Vec::new()),
    };
    h.load(BASE, &[0x0000_0013]); // nop
    let out = h.step();
    assert_eq!(out, StepOutcome::Trapped { cause: cause::S_EI });
    assert_eq!(
        h.cpu.debug_csr_read(csr::SIP).unwrap() & csr::SIE_SEIE,
        csr::SIE_SEIE
    );
}

// ---- PMP tests ----
