//! Unified, allocation-free writer for early kernel text output.
//!
//! The writer mirrors bytes to COM1 and, after GOP initialization, to the
//! framebuffer console. It is intentionally single-core and polling-only:
//! callers must serialize access and keep interrupts disabled until the
//! interrupt-safe console path is implemented.

use core::fmt;

use crate::arch::x86_64::serial;
use exhi_console::FramebufferConsole;

pub struct KernelConsole {
    framebuffer: Option<FramebufferConsole<'static>>,
}

impl KernelConsole {
    pub const fn new(framebuffer: Option<FramebufferConsole<'static>>) -> Self {
        Self { framebuffer }
    }

    /// Returns true when GOP framebuffer output is available.
    pub const fn has_framebuffer(&self) -> bool {
        self.framebuffer.is_some()
    }

    /// Sends the original byte sequence to COM1 and the framebuffer.
    ///
    /// The framebuffer renderer ignores bytes it does not support, while
    /// COM1 receives the complete stream for diagnostics and terminal logs.
    pub fn write(&mut self, bytes: &[u8]) {
        serial::COM1.write(bytes);
        if let Some(framebuffer) = self.framebuffer.as_mut() {
            framebuffer.write(bytes);
        }
    }

    /// Writes formatted text without allocating from the kernel heap.
    pub fn write_fmt(&mut self, arguments: fmt::Arguments<'_>) {
        let _ = fmt::write(self, arguments);
    }
}

impl fmt::Write for KernelConsole {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write(value.as_bytes());
        Ok(())
    }
}
