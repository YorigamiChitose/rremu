//! 集成测试：UART RX 中断经 BusRouter 路由到 PLIC → MEI。
#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行

use rremu_bus::{AccessSize, Bus, BusRouter, NullSink};
use rremu_devices::{Plic, Uart};

const UART_BASE: u32 = 0x1000_0000;
const PLIC_BASE: u32 = 0x0c00_0000;
const CAUSE_M_EI: u32 = 0x8000_000b;

#[test]
fn uart_irq_routed_to_plic_when_rx_enabled() {
    let mut bus = BusRouter::new();
    bus.map(UART_BASE..UART_BASE + 8, Box::new(Uart::new(UART_BASE)))
        .unwrap();
    bus.map(
        PLIC_BASE..PLIC_BASE + 0x40_0000,
        Box::new(Plic::new(PLIC_BASE)),
    )
    .unwrap();
    let mut sink = NullSink;

    // 未使能 RX 中断：注入数据也不产生 MEI
    bus.device_mut(UART_BASE)
        .unwrap()
        .as_input_sink()
        .unwrap()
        .push_input(b'A');
    assert!(
        !bus.service_interrupts().contains(&CAUSE_M_EI),
        "no MEI before enabling RX"
    );

    // 使能 UART RX 中断（IER bit0）与 PLIC source 10（ctx0 word0 bit10）
    bus.data_write(UART_BASE + 1, AccessSize::Byte, 0x01, &mut sink)
        .unwrap();
    bus.data_write(PLIC_BASE + 0x2000, AccessSize::Word, 1 << 10, &mut sink)
        .unwrap();

    assert!(
        bus.service_interrupts().contains(&CAUSE_M_EI),
        "UART RX should raise MEI via PLIC"
    );
}
