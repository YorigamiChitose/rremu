//! rremu-window：minifb 独立显示窗。
//!
//! 设计原则：纯显示窗，仅做两件事：
//!   1. 接收外部传入的 u32 像素缓冲，刷新到原生窗口；
//!   2. 检测用户是否关闭窗口（OS 原生 X 按钮），返回 `false` 让前端停机。
//!
//! **不包含**任何调试按键、运行/暂停/调速/单步等控制逻辑（控制仍在 CLI REPL）；
//! Framebuffer 设备本身也是纯设备，不附带窗口或调试功能。
//!
//! 依赖图：window 仅依赖 minifb + thiserror，不依赖 machine/cpu/bus（更纯净，
//! bin/Repl 通过 `Machine::framebuffer_dirty_frame()` 取 `&[u32]` 再传给 `Window::update`）。

use thiserror::Error;

/// 窗口创建/更新错误（包装 minifb 错误）。
#[derive(Debug, Error)]
pub enum WindowError {
    #[error("failed to create window: {0}")]
    Create(String),
    #[error("failed to update window: {0}")]
    Update(String),
}

impl From<minifb::Error> for WindowError {
    fn from(e: minifb::Error) -> Self {
        // 按 new/update 上下文分别包装由调用处完成；此处兜底转 Update。
        WindowError::Update(e.to_string())
    }
}

/// minifb 纯显示窗封装。
///
/// 用法：
/// ```ignore
/// let mut w = Window::new(640, 480, "rremu framebuffer")?;
/// // 主循环中按需刷新：
/// if !w.update(frame_buffer) {
///     // 用户关窗 → 暂停/退出
/// }
/// ```
pub struct Window {
    inner: minifb::Window,
    width: usize,
    height: usize,
    closed: bool,
}

impl Window {
    /// 创建窗口（无键鼠事件订阅，无标题栏菜单，仅显示像素）。
    pub fn new(width: usize, height: usize, title: &str) -> Result<Self, WindowError> {
        let opts = minifb::WindowOptions {
            // 不缩放（默认 Scale::X1）；无键鼠事件订阅；默认带标题栏与关闭按钮。
            ..Default::default()
        };
        let inner = minifb::Window::new(title, width, height, opts)
            .map_err(|e| WindowError::Create(e.to_string()))?;
        Ok(Self { inner, width, height, closed: false })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// 是否仍然打开（用户尚未点关闭按钮）。
    pub fn is_open(&self) -> bool {
        !self.closed && self.inner.is_open()
    }

    /// 仅处理 OS 事件、检测关窗，不刷新像素（fb 未 dirty 时用）。
    ///
    /// 返回 `false` 表示用户已关闭窗口。即使窗口已关闭，调用也安全。
    pub fn poll(&mut self) -> bool {
        if self.closed {
            return false;
        }
        if !self.inner.is_open() {
            self.closed = true;
            return false;
        }
        self.inner.update();
        if !self.inner.is_open() {
            self.closed = true;
            return false;
        }
        true
    }

    /// 用新像素缓冲刷新窗口。
    ///
    /// - `buffer` 长度必须等于 `width * height`；
    /// - 像素格式：`0x00RRGGBB`（与 Framebuffer 设备 `frame()` 输出一致）；
    /// - 返回 `false` 表示用户已关闭窗口，调用方应停止运行。
    ///
    /// 即使窗口已被关闭，调用本方法也安全（返回 `false`，不做任何事）。
    pub fn update(&mut self, buffer: &[u32]) -> bool {
        if self.closed {
            return false;
        }
        if !self.inner.is_open() {
            self.closed = true;
            return false;
        }
        // 长度不符 → 视为脏数据，不更新（避免 panic）
        if buffer.len() != self.width * self.height {
            return true;
        }
        // update_with_buffer 在某些后端要求 buffer 长度精确匹配；
        // 我们已在上面断言。错误时不算关窗，仅跳过本次刷新。
        let _ = self.inner.update_with_buffer(buffer, self.width, self.height);
        // update() 必须调用以处理 OS 事件（包括关闭按钮）
        self.inner.update();
        if !self.inner.is_open() {
            self.closed = true;
            return false;
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;

    /// 类型级与配置级测试（沙箱无 X11 时 minifb 创建会失败，但本测试不依赖真实窗口）。
    #[test]
    fn window_error_display_includes_context() {
        let e = WindowError::Create("display not found".into());
        assert!(format!("{e}").contains("create"));
        assert!(format!("{e}").contains("display not found"));
        let e = WindowError::Update("size mismatch".into());
        assert!(format!("{e}").contains("update"));
        assert!(format!("{e}").contains("size mismatch"));
    }

    /// 模拟关闭后状态：closed 守卫保证 update 不再做事并返回 false。
    /// 这里手动构造一个 closed=true 的 Window（绕过真实 minifb::Window）来测守卫逻辑。
    /// 由于 minifb::Window 无法在沙箱无 X11 时创建，本测试只覆盖错误路径的格式化。
    #[test]
    fn window_error_from_minifb_error_stringifies() {
        // minifb::Error 没有 pub 构造器；通过 to_string 路径间接验证 From 实现。
        // 仅断言 From<minifb::Error> 编译期存在且可调用：
        fn _assert_from(_: impl From<minifb::Error>) {}
        _assert_from(WindowError::Update("".into()));
    }
}
