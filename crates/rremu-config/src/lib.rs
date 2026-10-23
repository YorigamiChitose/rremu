//! rremu-config：rremu 侧所有硬编码值与可变参数的**唯一真相源**。
//!
//! - 默认值集中在 `defaults`；其余 crate 不再出现这些字面量。
//! - 解析入口 `Config::resolve`：默认值 + 外部参数覆盖 + 校验 → 统一 `Config`。
//! - 零依赖叶子 crate，**不依赖 clap**（外部参数经纯数据 `Overrides` 传入）。
//!
//! 边界：本 crate **只服务 rremu**；`rremu-am/arch.rs` 为人工镜像，两边一致性
//! 由跨仓库护栏测试锁定（见 `rremu-test/harness`）。

pub mod boot;
pub mod config;
pub mod defaults;
pub mod error;
pub mod layout;

pub use boot::{Boot, Diag, Paths};
pub use config::{Config, Overrides, ResolvedConfig};
pub use defaults::*;
pub use error::ConfigError;
pub use layout::{Device, FbLayout, Layout};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    #[test]
    fn place_dtb_at_ram_top_reserve() {
        let c = Config::default();
        assert_eq!(c.dtb_address().unwrap(), 0x80f0_0000);
        let c = c.with_boot_dtb().unwrap();
        assert_eq!(c.boot.dtb, Some(0x80f0_0000));

        let mut small = Config::default();
        small.layout.ram_size = 0x1000;
        assert!(matches!(small.dtb_address(), Err(ConfigError::DtbReserveTooLarge { .. })));
    }

    #[test]
    fn default_layout_matches_consts() {
        let c = Config::default();
        assert_eq!(c.layout.ram_base, DEFAULT_RAM_BASE);
        assert_eq!(c.layout.ram_size, DEFAULT_RAM_SIZE);
        assert_eq!(c.layout.uart, Some(Device { base: DEFAULT_UART_BASE, size: UART_SIZE }));
        assert_eq!(c.layout.clint, Some(Device { base: DEFAULT_CLINT_BASE, size: CLINT_SIZE }));
        assert_eq!(c.layout.plic, Some(Device { base: DEFAULT_PLIC_BASE, size: PLIC_SIZE }));
        assert_eq!(c.layout.syscon, Some(Device { base: DEFAULT_SYSCON_BASE, size: SYSCON_SIZE }));
        assert_eq!(c.layout.rtc, Some(Device { base: DEFAULT_RTC_BASE, size: RTC_SIZE }));
        assert_eq!(
            c.layout.fb,
            Some(FbLayout {
                base: DEFAULT_FB_BASE,
                width: DEFAULT_FB_WIDTH,
                height: DEFAULT_FB_HEIGHT,
                bpp: FB_BPP
            })
        );
        assert_eq!(c.diag.trace_capacity, DEFAULT_TRACE_CAPACITY);
        assert_eq!(c.diag.event_capacity, DEFAULT_EVENT_CAPACITY);
    }

    #[test]
    fn resolve_applies_overrides() {
        let cfg = Config::resolve(Overrides {
            ram_base: Some(0x9000_0000),
            ram_size: Some(0x1000),
            no_fb: true,
            entry: Some(0x8000_1000),
            elf: Some("prog.elf".into()),
            dump_on_exit: Some("out.json".into()),
            uart_in: Some("uart.in".into()),
        })
        .unwrap();
        assert_eq!(cfg.layout.ram_base, 0x9000_0000);
        assert_eq!(cfg.layout.ram_size, 0x1000);
        assert_eq!(cfg.layout.fb, None);
        assert_eq!(cfg.entry, Some(0x8000_1000));
        assert_eq!(cfg.paths.elf.as_deref(), Some("prog.elf"));
        assert_eq!(cfg.paths.dump_on_exit.as_deref(), Some("out.json"));
        assert_eq!(cfg.paths.uart_in.as_deref(), Some("uart.in"));
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut c = Config::default();
        c.layout.ram_size = 0;
        assert_eq!(c.validate(), Err(ConfigError::ZeroRamSize));

        let mut c = Config::default();
        c.layout.ram_base = u32::MAX;
        c.layout.ram_size = 0x1000;
        assert!(matches!(c.validate(), Err(ConfigError::RamOverflow { .. })));

        let mut c = Config::default();
        c.layout.fb = Some(FbLayout { base: 0, width: 0, height: 480, bpp: 2 });
        assert!(matches!(c.validate(), Err(ConfigError::FbZeroDim { .. })));

        let mut c = Config::default();
        c.layout.fb = Some(FbLayout { base: 0, width: u32::MAX, height: u32::MAX, bpp: 2 });
        assert!(matches!(c.validate(), Err(ConfigError::FbSizeOverflow { .. })));
    }
}
