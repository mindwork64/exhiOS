//! Unified early-kernel text output for the framebuffer and COM1.
//!
//! This writer is single-core and polling-only. Keep interrupts disabled and
//! serialize access until an interrupt-safe console backend is introduced.

use core::fmt;

use crate::arch::x86_64::serial;
use exhi_console::FramebufferConsole;

/// Writes text to COM1 and, when available, the GOP framebuffer console.
pub struct KernelConsole {
    framebuffer: Option<FramebufferConsole<'static>>,
}

impl KernelConsole {
    pub const fn new(framebuffer: Option<FramebufferConsole<'static>>) -> Self {
        Self { framebuffer }
    }

    /// Writes raw bytes to both output devices.
    ///
    /// FramebufferConsole renders printable ASCII and its supported control
    /// characters; COM1 receives the original byte sequence.
    pub fn write(&mut self, bytes: &[u8]) {
        serial::COM1.write(bytes);
        if let Some(framebuffer) = self.framebuffer.as_mut() {
            framebuffer.write(bytes);
        }
    }

    /// Formats text without heap allocation and mirrors it to both devices.
    pub fn write_fmt(&mut self, arguments: fmt::Arguments<'_>) {
        let _ = fmt::write(self, arguments);
    }

    pub const fn has_framebuffer(&self) -> bool {
        self.framebuffer.is_some()
    }
}

impl fmt::Write for KernelConsole {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write(value.as_bytes());
        Ok(())
    }
}
