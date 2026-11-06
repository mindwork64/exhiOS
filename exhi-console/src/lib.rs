#![no_std]

//! Allocation-free, pixel-format-aware GOP text console.

#[cfg(test)]
extern crate std;

use core::convert::TryFrom;
use core::ptr;

const CELL_W: usize = 12;
const CELL_H: usize = 16;
const GLYPH_W: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Color {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsoleError {
    InvalidDimensions,
    InvalidStride,
    BufferTooSmall,
    UnsupportedPixelFormat(u32),
}

impl ConsoleError {
    pub const fn message(self) -> &'static [u8] {
        match self {
            Self::InvalidDimensions => b"invalid framebuffer dimensions",
            Self::InvalidStride => b"scanline stride is smaller than width",
            Self::BufferTooSmall => b"framebuffer slice is too small",
            Self::UnsupportedPixelFormat(_) => b"unsupported GOP pixel format",
        }
    }
}

#[derive(Clone, Copy)]
enum PixelFormat {
    RgbReserved,
    BgrReserved,
}

pub struct FramebufferConsole<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    foreground: Color,
    background: Color,
    cursor_x: usize,
    cursor_y: usize,
}

impl<'a> FramebufferConsole<'a> {
    /// GOP format 0 is RGB-reserved byte order; format 1 is BGR-reserved.
    /// BitMask and BltOnly modes are rejected until explicitly supported.
    pub fn new(
        pixels: &'a mut [u32],
        width: u32,
        height: u32,
        pixels_per_scan_line: u32,
        pixel_format: u32,
    ) -> Result<Self, ConsoleError> {
        if width < CELL_W as u32 || height < CELL_H as u32 {
            return Err(ConsoleError::InvalidDimensions);
        }
        if pixels_per_scan_line < width {
            return Err(ConsoleError::InvalidStride);
        }
        let required = u64::from(pixels_per_scan_line)
            .checked_mul(u64::from(height))
            .ok_or(ConsoleError::BufferTooSmall)?;
        let required = usize::try_from(required).map_err(|_| ConsoleError::BufferTooSmall)?;
        if pixels.len() < required {
            return Err(ConsoleError::BufferTooSmall);
        }
        let format = match pixel_format {
            0 => PixelFormat::RgbReserved,
            1 => PixelFormat::BgrReserved,
            value => return Err(ConsoleError::UnsupportedPixelFormat(value)),
        };
        Ok(Self {
            pixels,
            width: width as usize,
            height: height as usize,
            stride: pixels_per_scan_line as usize,
            format,
            foreground: Color::new(220, 235, 255),
            background: Color::new(8, 12, 20),
            cursor_x: 0,
            cursor_y: 0,
        })
    }

    pub fn set_colors(&mut self, foreground: Color, background: Color) {
        self.foreground = foreground;
        self.background = background;
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }

    pub fn clear(&mut self) {
        let pixel = self.pack(self.background);
        for y in 0..self.height {
            for x in 0..self.width {
                self.put_pixel(x, y, pixel);
            }
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_byte(byte);
        }
    }

    pub fn write_u32(&mut self, mut value: u32) {
        let mut digits = [0u8; 10];
        let mut count = 0usize;
        loop {
            digits[count] = b'0' + (value % 10) as u8;
            count += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for index in (0..count).rev() {
            self.write_glyph(digits[index]);
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\r' => self.cursor_x = 0,
            b'\n' => self.next_line(),
            b'\t' => {
                let stop = ((self.cursor_x / 4) + 1) * 4;
                while self.cursor_x < stop {
                    self.write_glyph(b' ');
                }
            }
            0x20..=0x7e => self.write_glyph(byte),
            _ => {}
        }
    }

    fn write_glyph(&mut self, byte: u8) {
        let x0 = self.cursor_x * CELL_W;
        let y0 = self.cursor_y * CELL_H;
        let bg = self.pack(self.background);
        let fg = self.pack(self.foreground);
        for y in 0..CELL_H {
            for x in 0..CELL_W {
                self.put_pixel(x0 + x, y0 + y, bg);
            }
        }
        for (gy, bits) in glyph(byte).iter().copied().enumerate() {
            for gx in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - gx)) != 0 {
                    let x = x0 + 1 + gx * 2;
                    let y = y0 + 1 + gy * 2;
                    self.put_pixel(x, y, fg);
                    self.put_pixel(x + 1, y, fg);
                    self.put_pixel(x, y + 1, fg);
                    self.put_pixel(x + 1, y + 1, fg);
                }
            }
        }
        self.cursor_x += 1;
        if self.cursor_x >= self.columns() {
            self.next_line();
        }
    }

    fn columns(&self) -> usize {
        self.width / CELL_W
    }

    fn rows(&self) -> usize {
        self.height / CELL_H
    }

    fn next_line(&mut self) {
        self.cursor_x = 0;
        self.cursor_y += 1;
        if self.cursor_y >= self.rows() {
            self.scroll();
            self.cursor_y = self.rows() - 1;
        }
    }

    fn scroll(&mut self) {
        let copy_height = self.height - CELL_H;
        for y in 0..copy_height {
            for x in 0..self.width {
                let src = (y + CELL_H) * self.stride + x;
                let dst = y * self.stride + x;
                // SAFETY: both indices are within the constructor-validated slice.
                let value = unsafe { ptr::read_volatile(self.pixels.as_ptr().add(src)) };
                // SAFETY: dst is within the same validated framebuffer slice.
                unsafe { ptr::write_volatile(self.pixels.as_mut_ptr().add(dst), value) };
            }
        }
        let bg = self.pack(self.background);
        for y in copy_height..self.height {
            for x in 0..self.width {
                self.put_pixel(x, y, bg);
            }
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, value: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = y * self.stride + x;
        // SAFETY: dimensions, stride and backing-slice length were checked in new().
        unsafe { ptr::write_volatile(self.pixels.as_mut_ptr().add(index), value) };
    }

    fn pack(&self, color: Color) -> u32 {
        let r = u32::from(color.red);
        let g = u32::from(color.green);
        let b = u32::from(color.blue);
        match self.format {
            // Little-endian memory bytes: R, G, B, Reserved.
            PixelFormat::RgbReserved => r | (g << 8) | (b << 16),
            // Little-endian memory bytes: B, G, R, Reserved.
            PixelFormat::BgrReserved => b | (g << 8) | (r << 16),
        }
    }
}

fn glyph(byte: u8) -> [u8; 7] {
    match byte.to_ascii_uppercase() {
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'D' => [30, 17, 17, 17, 17, 17, 30],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'F' => [31, 16, 16, 30, 16, 16, 16],
        b'G' => [14, 17, 16, 23, 17, 17, 15],
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'I' => [14, 4, 4, 4, 4, 4, 14],
        b'J' => [7, 2, 2, 2, 18, 18, 12],
        b'K' => [17, 18, 20, 24, 20, 18, 17],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'M' => [17, 27, 21, 21, 17, 17, 17],
        b'N' => [17, 25, 21, 19, 17, 17, 17],
        b'O' => [14, 17, 17, 17, 17, 17, 14],
        b'P' => [30, 17, 17, 30, 16, 16, 16],
        b'Q' => [14, 17, 17, 17, 21, 18, 13],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'S' => [15, 16, 16, 14, 1, 1, 30],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        b'U' => [17, 17, 17, 17, 17, 17, 14],
        b'V' => [17, 17, 17, 17, 17, 10, 4],
        b'W' => [17, 17, 17, 21, 21, 21, 10],
        b'X' => [17, 17, 10, 4, 10, 17, 17],
        b'Y' => [17, 17, 10, 4, 4, 4, 4],
        b'Z' => [31, 1, 2, 4, 8, 16, 31],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b'.' => [0, 0, 0, 0, 0, 12, 12],
        b',' => [0, 0, 0, 0, 12, 12, 8],
        b':' => [0, 12, 12, 0, 12, 12, 0],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 31],
        b'+' => [0, 4, 4, 31, 4, 4, 0],
        b'=' => [0, 0, 31, 0, 31, 0, 0],
        b'/' => [1, 2, 2, 4, 8, 8, 16],
        b'\\' => [16, 8, 8, 4, 2, 2, 1],
        b'(' => [2, 4, 8, 8, 8, 4, 2],
        b')' => [8, 4, 2, 2, 2, 4, 8],
        b'<' => [2, 4, 8, 16, 8, 4, 2],
        b'>' => [8, 4, 2, 1, 2, 4, 8],
        b'?' => [14, 17, 1, 2, 4, 0, 4],
        b'!' => [4, 4, 4, 4, 4, 0, 4],
        b'#' => [10, 10, 31, 10, 31, 10, 10],
        b' ' => [0; 7],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn rejects_bad_stride_format_and_short_buffer() {
        let mut pixels = vec![0u32; 16 * 16];
        assert_eq!(FramebufferConsole::new(&mut pixels, 12, 16, 11, 0).err(), Some(ConsoleError::InvalidStride));
        assert_eq!(FramebufferConsole::new(&mut pixels, 12, 16, 12, 5).err(), Some(ConsoleError::UnsupportedPixelFormat(5)));
        let mut short = vec![0u32; 12 * 15];
        assert_eq!(FramebufferConsole::new(&mut short, 12, 16, 12, 0).err(), Some(ConsoleError::BufferTooSmall));
    }

    #[test]
    fn rgb_byte_order_and_padding_are_correct() {
        let sentinel = 0xDEAD_BEEFu32;
        let mut pixels = vec![sentinel; 16 * 16];
        let mut console = FramebufferConsole::new(&mut pixels, 12, 16, 16, 0).unwrap();
        console.set_colors(Color::new(0x12, 0x34, 0x56), Color::new(0, 0, 0));
        console.clear();
        console.write(b"A");
        drop(console);
        assert_eq!(pixels[16 + 3], 0x0056_3412);
        assert!(pixels[12..16].iter().all(|pixel| *pixel == sentinel));
    }

    #[test]
    fn bgr_byte_order_is_correct() {
        let mut pixels = vec![0u32; 12 * 16];
        let mut console = FramebufferConsole::new(&mut pixels, 12, 16, 12, 1).unwrap();
        console.set_colors(Color::new(0x12, 0x34, 0x56), Color::new(0, 0, 0));
        console.write(b"A");
        drop(console);
        assert_eq!(pixels[16 + 3], 0x0012_3456);
    }

    #[test]
    fn wraps_and_scrolls() {
        let mut pixels = vec![0u32; 12 * 32];
        let mut console = FramebufferConsole::new(&mut pixels, 12, 32, 12, 0).unwrap();
        console.write(b"ABC");
        assert_eq!(console.cursor_position(), (0, 1));
    }

    #[test]
    fn decimal_output_uses_no_heap() {
        let mut pixels = vec![0u32; 120 * 32];
        let mut console = FramebufferConsole::new(&mut pixels, 120, 32, 120, 0).unwrap();
        console.write_u32(2026);
        assert_eq!(console.cursor_position(), (4, 0));
    }
}
