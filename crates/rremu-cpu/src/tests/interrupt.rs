use super::*;

/// 测试 IRQ 设备：返回可配置的中断 cause 列表。
pub(super) struct MockIrq {
    causes: Vec<u32>,
}

impl MockIrq {
    pub(super) fn new(cause: u32) -> Self {
        Self {
            causes: vec![cause],
        }
    }
    pub(super) fn multi(causes: Vec<u32>) -> Self {
        Self { causes }
    }
}

impl BusDevice for MockIrq {
    fn read(
        &mut self,
        _addr: u32,
        _size: AccessSize,
        _sink: &mut dyn EventSink,
    ) -> Result<u32, BusError> {
        Ok(0)
    }
    fn write(
        &mut self,
        _addr: u32,
        _size: AccessSize,
        _data: u32,
        _sink: &mut dyn EventSink,
    ) -> Result<(), BusError> {
        Ok(())
    }
    fn peek_read(&mut self, _addr: u32, _size: AccessSize) -> Result<u32, BusError> {
        Ok(0)
    }
    fn write_bytes(&mut self, _addr: u32, _data: &[u8]) -> Result<(), BusError> {
        Ok(())
    }
    fn device_name(&self) -> &str {
        "mock-irq"
    }
    fn check_interrupt(&self) -> Vec<u32> {
        self.causes.clone()
    }
}

#[test]
fn interrupt_taken_when_mie_and_mtie_enabled() {
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_TI)),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
    cpu.csr.mie = csr::MIE_MTIE;
    cpu.csr.mtvec = BASE + 0x800;

    let mut sink = Sink(Vec::new());
    let outcome = cpu.step(&mut bus, &mut sink).unwrap();
    assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_TI });
    assert_eq!(cpu.csr_state().mepc, BASE); // mepc = 旧 pc（未执行的指令）
    assert_eq!(cpu.pc(), BASE + 0x800); // pc = mtvec
    // trap 后 MIE=0（防嵌套），MPIE=旧MIE=1，MPP=M
    let ms = cpu.csr_state().mstatus;
    assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 0);
    assert_eq!((ms >> csr::MSTATUS_MPIE.trailing_zeros()) & 1, 1);
    assert_eq!((ms >> csr::MSTATUS_MPP_SHIFT) & 0b11, 0b11);
}

#[test]
fn interrupt_not_taken_when_mstatus_mie_disabled() {
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_TI)),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MPP_M; // MIE=0
    cpu.csr.mie = csr::MIE_MTIE;
    cpu.csr.mtvec = BASE + 0x800;
    // 放一条 NOP
    let mut h = Harness {
        cpu,
        bus,
        sink: Sink(Vec::new()),
    };
    h.load(BASE, &[0x0000_0013]); // addi x0, x0, 0 (NOP)
    let outcome = h.step();
    // 中断未取，NOP 正常执行
    assert!(matches!(outcome, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), BASE + 4);
}

#[test]
fn interrupt_not_taken_when_mie_mtie_disabled() {
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_TI)),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M; // MIE=1
    cpu.csr.mie = 0; // MTIE=0
    cpu.csr.mtvec = BASE + 0x800;
    let mut h = Harness {
        cpu,
        bus,
        sink: Sink(Vec::new()),
    };
    h.load(BASE, &[0x0000_0013]); // NOP
    let outcome = h.step();
    assert!(matches!(outcome, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), BASE + 4);
}

#[test]
fn interrupt_mret_restores_mie_and_resumes() {
    // 中断 → trap → handler 第一条 mret → 恢复 MIE → pc = mepc
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_TI)),
    )
    .unwrap();

    let handler = BASE + 0x800;
    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
    cpu.csr.mie = csr::MIE_MTIE;
    cpu.csr.mtvec = handler;

    let mut h = Harness {
        cpu,
        bus,
        sink: Sink(Vec::new()),
    };
    // 主程序放 NOP；handler 放 mret
    h.load(BASE, &[0x0000_0013]); // NOP at BASE
    h.load(handler, &[0x3020_0073]); // mret at handler

    // 第 1 步：中断触发，trap 到 handler
    let out1 = h.step();
    assert_eq!(out1, StepOutcome::Trapped { cause: cause::M_TI });
    assert_eq!(h.cpu.pc(), handler);
    assert_eq!(h.cpu.csr_state().mepc, BASE);

    // 第 2 步：执行 mret → pc = mepc = BASE，MIE 恢复为 1
    let out2 = h.step();
    assert!(matches!(out2, StepOutcome::Executed { .. }));
    assert_eq!(h.cpu.pc(), BASE);
    let ms = h.cpu.csr_state().mstatus;
    assert_eq!((ms >> csr::MSTATUS_MIE.trailing_zeros()) & 1, 1);

    // 第 3 步：中断再次 pending → 再次 trap（MockIrq 恒 pending）
    let out3 = h.step();
    assert_eq!(out3, StepOutcome::Trapped { cause: cause::M_TI });
}

#[test]
fn msi_serviced_when_mti_pending_but_unenabled() {
    // MSI 使能 + MTI 未使能 + 两者同时 pending → 必须进入 MSI handler。
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    // 单个设备同时 pending MSI + MTI
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::multi(vec![cause::M_SI, cause::M_TI])),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
    cpu.csr.mie = csr::MIE_MSIE; // 仅 MSIE，MTIE=0
    cpu.csr.mtvec = BASE + 0x800;

    let mut sink = Sink(Vec::new());
    let outcome = cpu.step(&mut bus, &mut sink).unwrap();
    assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_SI });
    assert_eq!(cpu.csr_state().mcause, cause::M_SI);
}

#[test]
fn mei_highest_priority_over_msi_and_mti() {
    // spec 序 MEI > MSI > MTI：三者同时 pending 且全部使能 → 取 MEI。
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::multi(vec![cause::M_EI, cause::M_SI, cause::M_TI])),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
    cpu.csr.mie = csr::MIE_MEIE | csr::MIE_MSIE | csr::MIE_MTIE;
    cpu.csr.mtvec = BASE + 0x800;

    let mut sink = Sink(Vec::new());
    let outcome = cpu.step(&mut bus, &mut sink).unwrap();
    assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_EI });
}

#[test]
fn msi_priority_over_mti_when_both_enabled() {
    // spec 序 MSI > MTI：MSI 与 MTI 同时 pending 且均使能 → 取 MSI。
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::multi(vec![cause::M_SI, cause::M_TI])),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
    cpu.csr.mie = csr::MIE_MSIE | csr::MIE_MTIE;
    cpu.csr.mtvec = BASE + 0x800;

    let mut sink = Sink(Vec::new());
    let outcome = cpu.step(&mut bus, &mut sink).unwrap();
    assert_eq!(outcome, StepOutcome::Trapped { cause: cause::M_SI });
}

// ---- audit 修复回归测试 ----

#[test]
fn trap_sets_mtval_for_load_fault() {
    // lw t0, 0x234(x0) → 地址 0x234 未映射 → LOAD_ACCESS_FAULT，mtval=0x234
    let mut h = Harness::new();
    h.load(BASE, &[i_enc(LOAD, 5, 0b010, 0, 0x234)]);
    let out = h.step();
    assert_eq!(
        out,
        StepOutcome::Trapped {
            cause: cause::LOAD_ACCESS_FAULT
        }
    );
    assert_eq!(h.cpu.debug_csr_read(csr::MTVAL).unwrap(), 0x234);
}

#[test]
fn trap_sets_mtval_for_illegal_instr() {
    let mut h = Harness::new();
    h.load(BASE, &[0xffff_ffff]);
    let out = h.step();
    assert_eq!(
        out,
        StepOutcome::Trapped {
            cause: cause::ILLEGAL_INSTR
        }
    );
    assert_eq!(h.cpu.debug_csr_read(csr::MTVAL).unwrap(), 0xffff_ffff);
}

#[test]
fn exit_ecall_counts_cycle_and_instret() {
    // lui a7, 0xCAFE0 ; addi a0, x0, 42 ; ecall → 三条指令都计入 cycle/instret
    let mut h = Harness::new();
    h.load(
        BASE,
        &[
            u_enc(0b0110111, 17, 0xCAFE0),
            i_enc(OP_IMM, 10, 0b000, 0, 0x2a),
            i_enc(0b1110011, 0, 0b000, 0, 0),
        ],
    );
    h.step();
    h.step();
    let out = h.step();
    assert_eq!(out, StepOutcome::Exited { code: 42 });
    assert_eq!(h.cpu.cycle(), 3);
    assert_eq!(h.cpu.instret(), 3);
}

#[test]
fn fetch_misaligned_has_priority_over_interrupt() {
    // 奇数 pc + MTI pending + 使能 → 取指 misaligned 优先于中断
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_TI)),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE + 1); // 奇数 pc
    cpu.csr.mstatus = csr::MSTATUS_MIE | csr::MSTATUS_MPP_M;
    cpu.csr.mie = csr::MIE_MTIE;
    cpu.csr.mtvec = BASE + 0x800;

    let mut sink = Sink(Vec::new());
    let outcome = cpu.step(&mut bus, &mut sink).unwrap();
    assert_eq!(
        outcome,
        StepOutcome::Trapped {
            cause: cause::INSTR_MISALIGNED
        }
    );
}

#[test]
fn mip_reflects_device_pending() {
    // MSI pending（但 MIE=0 不触发）→ step 后 mip.MSIP 应被投影为 1
    let mut bus = BusRouter::new();
    bus.map(BASE..BASE + 0x1000, Box::new(MockRam::new(BASE, 0x1000)))
        .unwrap();
    bus.map(
        0x0200_0000..0x0200_0010,
        Box::new(MockIrq::new(cause::M_SI)),
    )
    .unwrap();

    let mut cpu = Cpu::new(BASE);
    cpu.csr.mstatus = csr::MSTATUS_MPP_M; // MIE=0
    let mut h = Harness {
        cpu,
        bus,
        sink: Sink(Vec::new()),
    };
    h.load(BASE, &[0x0000_0013]); // NOP
    h.step();
    assert_eq!(
        h.cpu.debug_csr_read(csr::MIP).unwrap() & csr::MIP_MSIP,
        csr::MIP_MSIP
    );
}

// ---- A extension tests ----
