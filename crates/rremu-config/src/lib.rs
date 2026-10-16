//! rremu-config：rremu 侧所有硬编码值与可变参数的**唯一真相源**。
//!
//! - 默认值集中在下方 `pub const`；其余 crate 不再出现这些字面量。
//! - 解析入口 `Config::resolve`：默认值 + 外部参数覆盖 + 校验 → 统一 `Config`。
//! - 零依赖叶子 crate，**不依赖 clap**（外部参数经纯数据 `Overrides` 传入）。
//!
//! 边界：本 crate **只服务 rremu**；`rremu-am/arch.rs` 为人工镜像，两边一致性
//! 由跨仓库护栏测试锁定（见 `rremu-test/harness`）。

use core::fmt;

// ---------------------------------------------------------------------------
// 硬编码默认值（集中于此）
// ---------------------------------------------------------------------------

/// 默认 RAM 基址。
pub const DEFAULT_RAM_BASE: u32 = 0x8000_0000;
/// 默认 RAM 大小（16 MiB）。
pub const DEFAULT_RAM_SIZE: u32 = 16 * 1024 * 1024;
/// 默认 UART 基址。
pub const DEFAULT_UART_BASE: u32 = 0x1000_0000;
/// 默认 CLINT 基址。
pub const DEFAULT_CLINT_BASE: u32 = 0x0200_0000;
/// 默认 PLIC 基址（qemu-virt）。
pub const DEFAULT_PLIC_BASE: u32 = 0x0c00_0000;
/// 默认 Framebuffer 基址。
pub const DEFAULT_FB_BASE: u32 = 0x2000_0000;
/// 默认 Framebuffer 宽度。
pub const DEFAULT_FB_WIDTH: u32 = 640;
/// 默认 Framebuffer 高度。
pub const DEFAULT_FB_HEIGHT: u32 = 480;

/// UART 设备尺寸（字节）。
pub const UART_SIZE: u32 = 8;
/// CLINT 设备尺寸（覆盖 msip/mtimecmp/mtime）。
pub const CLINT_SIZE: u32 = 0xC000;
/// PLIC 映射尺寸（qemu-virt）。
pub const PLIC_SIZE: u32 = 0x40_0000;
/// Framebuffer 每像素字节数（RGB565）。
pub const FB_BPP: u32 = 2;

/// 默认指令 trace 深度。
pub const DEFAULT_TRACE_CAPACITY: usize = 32;
/// 默认事件环容量。
pub const DEFAULT_EVENT_CAPACITY: usize = 64;
/// 默认 UART 事件环容量。
pub const DEFAULT_UART_EVENT_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// 结构化配置
// ---------------------------------------------------------------------------

/// 一段设备区间（基址 + 尺寸）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Device {
    pub base: u32,
    pub size: u32,
}

/// Framebuffer 布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbLayout {
    pub base: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
}

/// 硬件布局（guest 可见的地址空间）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub ram_base: u32,
    pub ram_size: u32,
    /// `None` 表示不映射 UART。
    pub uart: Option<Device>,
    /// `None` 表示不映射 CLINT。
    pub clint: Option<Device>,
    /// `None` 表示不映射 PLIC。
    pub plic: Option<Device>,
    /// `None` 表示不映射 Framebuffer。
    pub fb: Option<FbLayout>,
}

/// 诊断/调试配置（与硬件无关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diag {
    pub trace_capacity: usize,
    pub event_capacity: usize,
    pub uart_event_capacity: usize,
}

/// 启动上下文（复位后注入到 a0/a1）。
///
/// qemu-virt 约定：`a0 = hartid`、`a1 = dtb 物理地址`。
/// RT-Thread 可能不用；OpenSBI/Linux 必需。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Boot {
    pub hartid: u32,
    pub dtb: Option<u32>,
}

/// host 侧路径（dump/ELF）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    pub elf: Option<String>,
    pub dump_on_exit: Option<String>,
}

/// 统一配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub layout: Layout,
    pub diag: Diag,
    pub paths: Paths,
    pub boot: Boot,
    /// 覆盖 ELF 入口（`None` 时用镜像自身入口）。
    pub entry: Option<u32>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            ram_base: DEFAULT_RAM_BASE,
            ram_size: DEFAULT_RAM_SIZE,
            uart: Some(Device { base: DEFAULT_UART_BASE, size: UART_SIZE }),
            clint: Some(Device { base: DEFAULT_CLINT_BASE, size: CLINT_SIZE }),
            plic: Some(Device { base: DEFAULT_PLIC_BASE, size: PLIC_SIZE }),
            fb: Some(FbLayout {
                base: DEFAULT_FB_BASE,
                width: DEFAULT_FB_WIDTH,
                height: DEFAULT_FB_HEIGHT,
                bpp: FB_BPP,
            }),
        }
    }
}

impl Default for Diag {
    fn default() -> Self {
        Self {
            trace_capacity: DEFAULT_TRACE_CAPACITY,
            event_capacity: DEFAULT_EVENT_CAPACITY,
            uart_event_capacity: DEFAULT_UART_EVENT_CAPACITY,
        }
    }
}

// ---------------------------------------------------------------------------
// 外部参数覆盖（纯数据，bin 从 clap 填充）
// ---------------------------------------------------------------------------

/// 外部参数（覆盖默认值）。字段为 `None` 表示"未指定，保持默认"。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub ram_base: Option<u32>,
    pub ram_size: Option<u32>,
    /// 禁用 Framebuffer 映射。
    pub no_fb: bool,
    pub entry: Option<u32>,
    pub elf: Option<String>,
    pub dump_on_exit: Option<String>,
}

// ---------------------------------------------------------------------------
// 校验与解析
// ---------------------------------------------------------------------------

/// 配置错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    ZeroRamSize,
    RamOverflow { base: u32, size: u32 },
    FbZeroDim { width: u32, height: u32 },
    FbSizeOverflow { width: u32, height: u32, bpp: u32 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::ZeroRamSize => write!(f, "ram size must be non-zero"),
            ConfigError::RamOverflow { base, size } => {
                write!(f, "ram range {base:#x}+{size:#x} overflows address space")
            }
            ConfigError::FbZeroDim { width, height } => {
                write!(f, "framebuffer dimensions must be non-zero ({width}x{height})")
            }
            ConfigError::FbSizeOverflow { width, height, bpp } => {
                write!(f, "framebuffer size overflow ({width}x{height}x{bpp})")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// 应用外部覆盖（不校验）。
    pub fn apply(&mut self, ov: &Overrides) {
        if let Some(b) = ov.ram_base {
            self.layout.ram_base = b;
        }
        if let Some(s) = ov.ram_size {
            self.layout.ram_size = s;
        }
        if ov.no_fb {
            self.layout.fb = None;
        }
        if let Some(e) = ov.entry {
            self.entry = Some(e);
        }
        if let Some(p) = &ov.elf {
            self.paths.elf = Some(p.clone());
        }
        if let Some(p) = &ov.dump_on_exit {
            self.paths.dump_on_exit = Some(p.clone());
        }
    }

    /// 校验配置合法性。
    pub fn validate(&self) -> Result<(), ConfigError> {
        let l = &self.layout;
        if l.ram_size == 0 {
            return Err(ConfigError::ZeroRamSize);
        }
        if l.ram_base.checked_add(l.ram_size).is_none() {
            return Err(ConfigError::RamOverflow { base: l.ram_base, size: l.ram_size });
        }
        if let Some(fb) = &l.fb {
            if fb.width == 0 || fb.height == 0 {
                return Err(ConfigError::FbZeroDim { width: fb.width, height: fb.height });
            }
            if fb
                .width
                .checked_mul(fb.height)
                .and_then(|n| n.checked_mul(fb.bpp))
                .is_none()
            {
                return Err(ConfigError::FbSizeOverflow {
                    width: fb.width,
                    height: fb.height,
                    bpp: fb.bpp,
                });
            }
        }
        Ok(())
    }

    /// 解析入口：默认值 + 外部覆盖 + 校验 → 统一 `Config`。
    pub fn resolve(ov: Overrides) -> Result<Config, ConfigError> {
        let mut cfg = Config::default();
        cfg.apply(&ov);
        cfg.validate()?;
        Ok(cfg)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    #[test]
    fn default_layout_matches_consts() {
        let c = Config::default();
        assert_eq!(c.layout.ram_base, DEFAULT_RAM_BASE);
        assert_eq!(c.layout.ram_size, DEFAULT_RAM_SIZE);
        assert_eq!(c.layout.uart, Some(Device { base: DEFAULT_UART_BASE, size: UART_SIZE }));
        assert_eq!(c.layout.clint, Some(Device { base: DEFAULT_CLINT_BASE, size: CLINT_SIZE }));
        assert_eq!(c.layout.plic, Some(Device { base: DEFAULT_PLIC_BASE, size: PLIC_SIZE }));
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
        })
        .unwrap();
        assert_eq!(cfg.layout.ram_base, 0x9000_0000);
        assert_eq!(cfg.layout.ram_size, 0x1000);
        assert_eq!(cfg.layout.fb, None);
        assert_eq!(cfg.entry, Some(0x8000_1000));
        assert_eq!(cfg.paths.elf.as_deref(), Some("prog.elf"));
        assert_eq!(cfg.paths.dump_on_exit.as_deref(), Some("out.json"));
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
