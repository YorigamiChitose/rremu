//! Machine 构造：配置、设备映射与镜像加载。

use std::collections::VecDeque;

use rremu_bus::{Bus, BusRouter};
use rremu_config::Config;
use rremu_cpu::Cpu;
use rremu_devices::{Clint, Framebuffer, Plic, Ram, Rtc, Syscon, Uart};
use rremu_loader::Image;

use crate::term::Terminal;
use crate::{DebugController, FramebufferAccessor, Machine, MachineError, RingCollector, RunState};

impl Machine {
    /// 从配置 + 可选镜像构造机器（配置的唯一入口）。
    ///
    /// 配置合法性与校验已由 `rremu_config::Config`（`resolve`/`validate`）完成；
    /// 此处仅做依赖镜像的段范围校验与设备映射。
    pub fn from_config(cfg: &Config, image: Option<Image>) -> Result<Machine, MachineError> {
        let ram_base = cfg.layout().ram_base;
        let ram_size = cfg.layout().ram_size;
        // 防御性：配置应已校验；overflow 不可能（保留兜底）
        let ram_end = ram_base
            .checked_add(ram_size)
            .ok_or_else(|| MachineError::InvalidConfig("ram range overflows".into()))?;

        let mut bus = BusRouter::new();
        bus.map(ram_base..ram_end, Box::new(Ram::new(ram_base, ram_size)))?;
        if let Some(d) = cfg.layout().uart {
            bus.map(
                d.base..d.base.wrapping_add(d.size),
                Box::new(Uart::new(d.base)),
            )?;
        }
        if let Some(d) = cfg.layout().clint {
            // CLINT 范围覆盖 msip(0x0) / mtimecmp(0x4000) / mtime(0xBFF8)
            bus.map(
                d.base..d.base.wrapping_add(d.size),
                Box::new(Clint::new(d.base)),
            )?;
        }
        if let Some(d) = cfg.layout().plic {
            bus.map(
                d.base..d.base.wrapping_add(d.size),
                Box::new(Plic::new(d.base)),
            )?;
        }
        if let Some(d) = cfg.layout().syscon {
            bus.map(
                d.base..d.base.wrapping_add(d.size),
                Box::new(Syscon::new(d.base)),
            )?;
        }
        if let Some(d) = cfg.layout().rtc {
            bus.map(
                d.base..d.base.wrapping_add(d.size),
                Box::new(Rtc::new(d.base)),
            )?;
        }
        if let Some(f) = cfg.layout().fb {
            let fb_size = f
                .width
                .checked_mul(f.height)
                .and_then(|n| n.checked_mul(f.bpp))
                .ok_or_else(|| MachineError::InvalidConfig("framebuffer size overflow".into()))?;
            bus.map(
                f.base..f.base.wrapping_add(fb_size),
                Box::new(Framebuffer::new(f.base, f.width, f.height)),
            )?;
        }

        if let Some(img) = &image {
            for seg in &img.segments {
                let end = seg.addr.checked_add(seg.data.len() as u32).ok_or(
                    MachineError::SegmentOutOfRange {
                        addr: seg.addr,
                        len: seg.data.len(),
                        ram_base,
                        ram_end,
                    },
                )?;
                if seg.addr < ram_base || end > ram_end {
                    return Err(MachineError::SegmentOutOfRange {
                        addr: seg.addr,
                        len: seg.data.len(),
                        ram_base,
                        ram_end,
                    });
                }
            }
            for seg in &img.segments {
                bus.write_bytes(seg.addr, &seg.data)?;
            }
        }

        let entry = cfg
            .entry()
            .or_else(|| image.as_ref().map(|i| i.entry))
            .ok_or(MachineError::NoEntry)?;

        let fb = match cfg.layout().fb {
            Some(f) => FramebufferAccessor::new(Some(f.base), f.width, f.height),
            None => FramebufferAccessor::new(None, 0, 0),
        };

        let mut machine = Machine {
            cpu: Cpu::new(entry),
            bus,
            collector: RingCollector::new(),
            image,
            ram_base,
            uart_base: cfg.layout().uart.map(|d| d.base),
            clint_base: cfg.layout().clint.map(|d| d.base),
            entry,
            state: RunState::Idle,
            terminal: Terminal::new(),
            uart_sink: None,
            trace: VecDeque::new(),
            trace_capacity: cfg.diag().trace_capacity,
            last_trap: None,
            trace_at_trap: None,
            dbg: DebugController::new(),
            fb,
        };
        machine.set_event_ring_capacity(cfg.diag().event_capacity, cfg.diag().uart_event_capacity);
        // 启动参数注入：a0=hartid、a1=dtb（qemu-virt 约定）
        machine
            .cpu
            .set_args(cfg.boot().hartid, cfg.boot().dtb.unwrap_or(0));
        Ok(machine)
    }
}
