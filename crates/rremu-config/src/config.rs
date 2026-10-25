//! 统一配置、覆盖与解析入口。

use crate::boot::{Boot, Diag, Paths};
use crate::defaults::DTB_RESERVE;
use crate::error::ConfigError;
use crate::layout::{FbLayout, Layout};

/// 统一配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub(crate) layout: Layout,
    pub(crate) diag: Diag,
    pub(crate) paths: Paths,
    pub(crate) boot: Boot,
    /// 覆盖 ELF 入口（`None` 时用镜像自身入口）。
    pub(crate) entry: Option<u32>,
}

/// 解析后的配置类型别名。字段对外只读，修改必须走消费式 builder。
pub type ResolvedConfig = Config;

impl Config {
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
    pub fn diag(&self) -> &Diag {
        &self.diag
    }
    pub fn paths(&self) -> &Paths {
        &self.paths
    }
    pub fn boot(&self) -> &Boot {
        &self.boot
    }
    pub fn entry(&self) -> Option<u32> {
        self.entry
    }

    pub fn with_layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }

    pub fn with_fb(mut self, fb: Option<FbLayout>) -> Self {
        self.layout.fb = fb;
        self
    }

    pub fn with_boot_hartid(mut self, hartid: u32) -> Self {
        self.boot.hartid = hartid;
        self
    }

    pub fn with_boot_dtb_addr(mut self, dtb: Option<u32>) -> Self {
        self.boot.dtb = dtb;
        self
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
    /// UART 输入源（文件/FIFO）；`None` 表示不自动读取输入。
    pub uart_in: Option<String>,
}

impl Config {
    /// 应用外部覆盖（不校验）。
    fn apply(&mut self, ov: &Overrides) {
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
        if let Some(p) = &ov.uart_in {
            self.paths.uart_in = Some(p.clone());
        }
    }

    /// 校验配置合法性。
    pub fn validate(&self) -> Result<(), ConfigError> {
        let l = &self.layout;
        if l.ram_size == 0 {
            return Err(ConfigError::ZeroRamSize);
        }
        if l.ram_base.checked_add(l.ram_size).is_none() {
            return Err(ConfigError::RamOverflow {
                base: l.ram_base,
                size: l.ram_size,
            });
        }
        if let Some(fb) = &l.fb {
            if fb.width == 0 || fb.height == 0 {
                return Err(ConfigError::FbZeroDim {
                    width: fb.width,
                    height: fb.height,
                });
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

    /// 计算 DTB 地址，不修改配置。
    /// 地址位于 RAM 顶部下方 `DTB_RESERVE` 处，8 字节对齐。
    pub fn dtb_address(&self) -> Result<u32, ConfigError> {
        let ram_end = self
            .layout
            .ram_base
            .checked_add(self.layout.ram_size)
            .ok_or(ConfigError::RamOverflow {
                base: self.layout.ram_base,
                size: self.layout.ram_size,
            })?;
        if self.layout.ram_size < DTB_RESERVE {
            return Err(ConfigError::DtbReserveTooLarge {
                ram_size: self.layout.ram_size,
            });
        }
        Ok((ram_end - DTB_RESERVE) & !7)
    }

    /// 返回带 DTB 放置地址的新配置；原配置不被修改。
    pub fn with_boot_dtb(mut self) -> Result<Config, ConfigError> {
        self.boot.dtb = Some(self.dtb_address()?);
        Ok(self)
    }

    /// 解析入口：默认值 + 外部覆盖 + 校验 → 统一 `Config`。
    pub fn resolve(ov: Overrides) -> Result<Config, ConfigError> {
        let mut cfg = Config::default();
        cfg.apply(&ov);
        cfg.validate()?;
        Ok(cfg)
    }
}
