//! Framebuffer 设备：RGB565 像素存储，内存映射写入 + u32 帧输出。
//!
//! 每像素 2 字节（RGB565），支持 byte/half/word 写。
//! - byte 写偶地址 → 像素低字节；奇地址 → 像素高字节。
//! - half 写 → 完整 RGB565 像素。
//! - word 写 → 2 个连续像素。
//!
//! `frame()` 返回 `&[u32]`，每个 u32 由 RGB565 扩展为 `0x00RRGGBB`。
//! 任何写置 `dirty` 标志；`peek_read` 无副作用（不置 dirty）。

use core::any::Any;
use rremu_bus::{AccessSize, BusDevice, BusError, EventSink};

/// RGB565 → 0x00RRGGBB（R5→8位、G6→8位、B5→8位，高位填充低位副本）。
fn rgb565_to_u32(rgb565: u16) -> u32 {
    let r = ((rgb565 >> 11) & 0x1f) as u32;
    let g = ((rgb565 >> 5) & 0x3f) as u32;
    let b = (rgb565 & 0x1f) as u32;
    let r8 = (r << 3) | (r >> 2);
    let g8 = (g << 2) | (g >> 4);
    let b8 = (b << 3) | (b >> 2);
    (r8 << 16) | (g8 << 8) | b8
}

pub struct Framebuffer {
    base: u32,
    width: u32,
    height: u32,
    /// RGB565 原始字节（每像素 2 字节，小端）。
    pixels: Vec<u8>,
    /// 转换后的 u32 帧（供前端渲染）。
    frame: Vec<u32>,
    dirty: bool,
}

impl Framebuffer {
    pub fn new(base: u32, width: u32, height: u32) -> Self {
        let pixel_count = usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0);
        Self {
            base,
            width,
            height,
            pixels: vec![0; pixel_count * 2],
            frame: vec![0; pixel_count],
            dirty: false,
        }
    }

    /// 返回整帧 u32 数据（0x00RRGGBB 格式）。
    pub fn frame(&self) -> &[u32] {
        &self.frame
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn offset(&self, addr: u32) -> u32 {
        addr - self.base
    }

    /// 从 pixels 重建指定像素的 u32 值并写入 frame。
    fn update_pixel(&mut self, pixel_idx: usize) {
        if pixel_idx >= self.frame.len() {
            return;
        }
        let lo = self.pixels[pixel_idx * 2];
        let hi = self.pixels[pixel_idx * 2 + 1];
        let rgb565 = u16::from(lo) | (u16::from(hi) << 8);
        self.frame[pixel_idx] = rgb565_to_u32(rgb565);
    }
}

impl BusDevice for Framebuffer {
    fn read(&mut self, addr: u32, size: AccessSize, _sink: &mut dyn EventSink) -> Result<u32, BusError> {
        let off = self.offset(addr) as usize;
        let n = size.bytes() as usize;
        if off + n > self.pixels.len() {
            return Ok(0);
        }
        let mut v: u32 = 0;
        for i in 0..n {
            v |= u32::from(self.pixels[off + i]) << (8 * i);
        }
        Ok(v)
    }

    fn write(&mut self, addr: u32, size: AccessSize, data: u32, _sink: &mut dyn EventSink) -> Result<(), BusError> {
        let off = self.offset(addr) as usize;
        let n = size.bytes() as usize;
        if off + n > self.pixels.len() {
            return Ok(()); // 越界写忽略
        }
        for i in 0..n {
            self.pixels[off + i] = ((data >> (8 * i)) & 0xff) as u8;
        }
        // 更新受影响的像素
        let start = off / 2;
        let end = (off + n).div_ceil(2);
        for idx in start..end {
            self.update_pixel(idx);
        }
        self.dirty = true;
        Ok(())
    }

    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError> {
        let mut sink = rremu_bus::NullSink;
        self.read(addr, size, &mut sink)
    }

    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError> {
        let off = self.offset(addr) as usize;
        if off + data.len() > self.pixels.len() {
            return Err(BusError::Device {
                addr: self.base,
                msg: "bulk write out of framebuffer range".to_owned(),
            });
        }
        self.pixels[off..off + data.len()].copy_from_slice(data);
        let start = off / 2;
        let end = (off + data.len()).div_ceil(2);
        for idx in start..end.min(self.frame.len()) {
            self.update_pixel(idx);
        }
        self.dirty = true;
        Ok(())
    }

    fn device_name(&self) -> &str {
        "framebuffer"
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    const BASE: u32 = 0x2000_0000;
    const W: u32 = 4;
    const H: u32 = 2;

    fn fb() -> Framebuffer {
        Framebuffer::new(BASE, W, H)
    }

    #[test]
    fn half_write_sets_dirty_and_frame() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        // 像素 0：RGB565 = 0xF800（纯红）
        fb.write(BASE, AccessSize::Half, 0xF800, &mut sink).unwrap();
        assert!(fb.is_dirty());
        // R5=0x1f→8位=0xFF, G6=0→0x00, B5=0→0x00 → 0x00FF0000
        assert_eq!(fb.frame()[0], 0x00FF_0000);
    }

    #[test]
    fn byte_write_assembles_pixel() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        // 逐字节写像素 0：低字节 0x00, 高字节 0xF8 → RGB565 = 0xF800
        fb.write(BASE, AccessSize::Byte, 0x00, &mut sink).unwrap();
        fb.write(BASE + 1, AccessSize::Byte, 0xF8, &mut sink).unwrap();
        assert!(fb.is_dirty());
        assert_eq!(fb.frame()[0], 0x00FF_0000);
    }

    #[test]
    fn word_write_two_pixels() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        // word 写 2 像素：低 16 位 = 像素 0, 高 16 位 = 像素 1
        // 像素 0 = 0x07E0 (纯绿), 像素 1 = 0x001F (纯蓝)
        let data: u32 = 0x001F_07E0;
        fb.write(BASE, AccessSize::Word, data, &mut sink).unwrap();
        assert!(fb.is_dirty());
        // 绿: G6=0x3f→8位=0xFF, R=0, B=0
        assert_eq!(fb.frame()[0], 0x0000_FF00);
        // 蓝: B5=0x1f→8位=0xFF, R=0, G=0
        assert_eq!(fb.frame()[1], 0x0000_00FF);
    }

    #[test]
    fn peek_read_no_side_effects() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        // 先写一个像素
        fb.write(BASE, AccessSize::Half, 0xFFFF, &mut sink).unwrap();
        fb.clear_dirty();
        assert!(!fb.is_dirty());
        // peek 读不置 dirty
        let v = fb.peek_read(BASE, AccessSize::Half).unwrap();
        assert_eq!(v, 0xFFFF);
        assert!(!fb.is_dirty());
    }

    #[test]
    fn read_returns_written_value() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        fb.write(BASE + 4, AccessSize::Half, 0x1234, &mut sink).unwrap();
        let v = fb.read(BASE + 4, AccessSize::Half, &mut sink).unwrap();
        assert_eq!(v, 0x1234);
        // byte 读
        assert_eq!(fb.read(BASE + 4, AccessSize::Byte, &mut sink).unwrap(), 0x34);
        assert_eq!(fb.read(BASE + 5, AccessSize::Byte, &mut sink).unwrap(), 0x12);
    }

    #[test]
    fn write_bytes_bulk() {
        let mut fb = fb();
        // 4 个像素 = 8 字节
        let data: [u8; 8] = [0x00, 0xF8, 0x00, 0x00, 0xE0, 0x07, 0x1F, 0x00];
        // 像素 0 = 0xF800 (红), 像素 1 = 0x0000 (黑), 像素 2 = 0x07E0 (绿), 像素 3 = 0x001F (蓝)
        fb.write_bytes(BASE, &data).unwrap();
        assert!(fb.is_dirty());
        assert_eq!(fb.frame()[0], 0x00FF_0000);
        assert_eq!(fb.frame()[1], 0x0000_0000);
        assert_eq!(fb.frame()[2], 0x0000_FF00);
        assert_eq!(fb.frame()[3], 0x0000_00FF);
    }

    #[test]
    fn out_of_bounds_write_ignored() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        // 总共 4*2*2 = 16 字节；越界写
        fb.write(BASE + 16, AccessSize::Word, 0xDEAD, &mut sink).unwrap();
        // 不 panic 即可
    }

    #[test]
    fn out_of_bounds_read_returns_zero() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        assert_eq!(fb.read(BASE + 100, AccessSize::Word, &mut sink).unwrap(), 0);
    }

    #[test]
    fn clear_dirty_resets_flag() {
        let mut fb = fb();
        let mut sink = rremu_bus::NullSink;
        fb.write(BASE, AccessSize::Half, 0xFFFF, &mut sink).unwrap();
        assert!(fb.is_dirty());
        fb.clear_dirty();
        assert!(!fb.is_dirty());
    }

    #[test]
    fn rgb565_conversion_accuracy() {
        // 纯白 0xFFFF → R=G=B=0xFF (5-bit/6-bit max expands to 0xFF)
        assert_eq!(rgb565_to_u32(0xFFFF), 0x00FF_FFFF);
        // 纯红 0xF800 → R=0xFF, G=0, B=0
        assert_eq!(rgb565_to_u32(0xF800), 0x00FF_0000);
        // 纯绿 0x07E0 → G=0xFF
        assert_eq!(rgb565_to_u32(0x07E0), 0x0000_FF00);
        // 纯蓝 0x001F → B=0xFF
        assert_eq!(rgb565_to_u32(0x001F), 0x0000_00FF);
    }
}
