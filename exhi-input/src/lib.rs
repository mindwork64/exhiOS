#![no_std]

use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

pub const SCANCODE_QUEUE_CAPACITY: usize = 64;
const QUEUE_MASK: usize = SCANCODE_QUEUE_CAPACITY - 1;

/// Single-producer/single-consumer queue shared with the assembly IRQ1 stub.
///
/// Assembly ABI (repr(C)): head=0, tail=8, dropped=16, slots=24.
/// One slot remains empty to distinguish a full queue from an empty queue.
#[repr(C)]
pub struct ScanCodeQueue {
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
    slots: [AtomicU8; SCANCODE_QUEUE_CAPACITY],
}

impl ScanCodeQueue {
    pub const fn new() -> Self {
        Self {
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            slots: [const { AtomicU8::new(0) }; SCANCODE_QUEUE_CAPACITY],
        }
    }

    /// Adds one byte under the single-producer contract.
    pub fn push(&self, scancode: u8) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & QUEUE_MASK;
        if next == self.tail.load(Ordering::Acquire) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.slots[head].store(scancode, Ordering::Relaxed);
        self.head.store(next, Ordering::Release);
        true
    }

    /// Removes one byte under the single-consumer contract.
    pub fn try_pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        let value = self.slots[tail].load(Ordering::Relaxed);
        self.tail.store((tail + 1) & QUEUE_MASK, Ordering::Release);
        Some(value)
    }

    /// Reset only while the producer IRQ is masked.
    pub fn clear(&self) {
        self.head.store(0, Ordering::Relaxed);
        self.tail.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Default for ScanCodeQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Exported for direct, allocation-free writes from the IRQ1 assembly stub.
#[no_mangle]
pub static EXHIOS_KEYBOARD_SCANCODE_QUEUE: ScanCodeQueue = ScanCodeQueue::new();

/// USB HID keyboard/keypad usage ID. Keeping physical key identity independent
/// of characters allows future USB keyboards and layout translation to reuse it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyCode(pub u16);

impl KeyCode {
    pub const A: Self = Self(0x04);
    pub const B: Self = Self(0x05);
    pub const C: Self = Self(0x06);
    pub const D: Self = Self(0x07);
    pub const E: Self = Self(0x08);
    pub const F: Self = Self(0x09);
    pub const G: Self = Self(0x0A);
    pub const H: Self = Self(0x0B);
    pub const I: Self = Self(0x0C);
    pub const J: Self = Self(0x0D);
    pub const K: Self = Self(0x0E);
    pub const L: Self = Self(0x0F);
    pub const M: Self = Self(0x10);
    pub const N: Self = Self(0x11);
    pub const O: Self = Self(0x12);
    pub const P: Self = Self(0x13);
    pub const Q: Self = Self(0x14);
    pub const R: Self = Self(0x15);
    pub const S: Self = Self(0x16);
    pub const T: Self = Self(0x17);
    pub const U: Self = Self(0x18);
    pub const V: Self = Self(0x19);
    pub const W: Self = Self(0x1A);
    pub const X: Self = Self(0x1B);
    pub const Y: Self = Self(0x1C);
    pub const Z: Self = Self(0x1D);
    pub const DIGIT_1: Self = Self(0x1E);
    pub const DIGIT_2: Self = Self(0x1F);
    pub const DIGIT_3: Self = Self(0x20);
    pub const DIGIT_4: Self = Self(0x21);
    pub const DIGIT_5: Self = Self(0x22);
    pub const DIGIT_6: Self = Self(0x23);
    pub const DIGIT_7: Self = Self(0x24);
    pub const DIGIT_8: Self = Self(0x25);
    pub const DIGIT_9: Self = Self(0x26);
    pub const DIGIT_0: Self = Self(0x27);
    pub const ENTER: Self = Self(0x28);
    pub const ESCAPE: Self = Self(0x29);
    pub const BACKSPACE: Self = Self(0x2A);
    pub const TAB: Self = Self(0x2B);
    pub const SPACE: Self = Self(0x2C);
    pub const MINUS: Self = Self(0x2D);
    pub const EQUAL: Self = Self(0x2E);
    pub const LEFT_BRACKET: Self = Self(0x2F);
    pub const RIGHT_BRACKET: Self = Self(0x30);
    pub const BACKSLASH: Self = Self(0x31);
    pub const SEMICOLON: Self = Self(0x33);
    pub const APOSTROPHE: Self = Self(0x34);
    pub const GRAVE: Self = Self(0x35);
    pub const COMMA: Self = Self(0x36);
    pub const PERIOD: Self = Self(0x37);
    pub const SLASH: Self = Self(0x38);
    pub const CAPS_LOCK: Self = Self(0x39);
    pub const F1: Self = Self(0x3A);
    pub const F2: Self = Self(0x3B);
    pub const F3: Self = Self(0x3C);
    pub const F4: Self = Self(0x3D);
    pub const F5: Self = Self(0x3E);
    pub const F6: Self = Self(0x3F);
    pub const F7: Self = Self(0x40);
    pub const F8: Self = Self(0x41);
    pub const F9: Self = Self(0x42);
    pub const F10: Self = Self(0x43);
    pub const F11: Self = Self(0x44);
    pub const F12: Self = Self(0x45);
    pub const PRINT_SCREEN: Self = Self(0x46);
    pub const SCROLL_LOCK: Self = Self(0x47);
    pub const INSERT: Self = Self(0x49);
    pub const HOME: Self = Self(0x4A);
    pub const PAGE_UP: Self = Self(0x4B);
    pub const DELETE: Self = Self(0x4C);
    pub const END: Self = Self(0x4D);
    pub const PAGE_DOWN: Self = Self(0x4E);
    pub const RIGHT: Self = Self(0x4F);
    pub const LEFT: Self = Self(0x50);
    pub const DOWN: Self = Self(0x51);
    pub const UP: Self = Self(0x52);
    pub const NUM_LOCK: Self = Self(0x53);
    pub const KEYPAD_SLASH: Self = Self(0x54);
    pub const KEYPAD_MULTIPLY: Self = Self(0x55);
    pub const KEYPAD_MINUS: Self = Self(0x56);
    pub const KEYPAD_PLUS: Self = Self(0x57);
    pub const KEYPAD_ENTER: Self = Self(0x58);
    pub const KEYPAD_1: Self = Self(0x59);
    pub const KEYPAD_2: Self = Self(0x5A);
    pub const KEYPAD_3: Self = Self(0x5B);
    pub const KEYPAD_4: Self = Self(0x5C);
    pub const KEYPAD_5: Self = Self(0x5D);
    pub const KEYPAD_6: Self = Self(0x5E);
    pub const KEYPAD_7: Self = Self(0x5F);
    pub const KEYPAD_8: Self = Self(0x60);
    pub const KEYPAD_9: Self = Self(0x61);
    pub const KEYPAD_0: Self = Self(0x62);
    pub const KEYPAD_DECIMAL: Self = Self(0x63);
    pub const APPLICATION: Self = Self(0x65);
    pub const LEFT_CONTROL: Self = Self(0xE0);
    pub const LEFT_SHIFT: Self = Self(0xE1);
    pub const LEFT_ALT: Self = Self(0xE2);
    pub const LEFT_META: Self = Self(0xE3);
    pub const RIGHT_CONTROL: Self = Self(0xE4);
    pub const RIGHT_SHIFT: Self = Self(0xE5);
    pub const RIGHT_ALT: Self = Self(0xE6);
    pub const RIGHT_META: Self = Self(0xE7);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub scroll_lock: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub pressed: bool,
    pub modifiers: Modifiers,
}

/// Decoder for i8042 translated Set 1 make/break bytes.
pub struct KeyboardDecoder {
    extended: bool,
    pause_bytes_remaining: u8,
    left_shift: bool,
    right_shift: bool,
    left_control: bool,
    right_control: bool,
    left_alt: bool,
    right_alt: bool,
    left_meta: bool,
    right_meta: bool,
    caps_lock: bool,
    num_lock: bool,
    scroll_lock: bool,
}

impl KeyboardDecoder {
    pub const fn new() -> Self {
        Self {
            extended: false,
            pause_bytes_remaining: 0,
            left_shift: false,
            right_shift: false,
            left_control: false,
            right_control: false,
            left_alt: false,
            right_alt: false,
            left_meta: false,
            right_meta: false,
            caps_lock: false,
            num_lock: false,
            scroll_lock: false,
        }
    }

    pub fn feed(&mut self, scancode: u8) -> Option<KeyEvent> {
        if self.pause_bytes_remaining != 0 {
            self.pause_bytes_remaining -= 1;
            return None;
        }
        match scancode {
            0xE0 => {
                self.extended = true;
                return None;
            }
            // Pause is E1 1D 45 E1 9D C5 and has no independent break byte.
            0xE1 => {
                self.extended = false;
                self.pause_bytes_remaining = 5;
                return None;
            }
            _ => {}
        }

        let extended = core::mem::replace(&mut self.extended, false);
        let pressed = scancode & 0x80 == 0;
        let key = map_set1_scancode(extended, scancode & 0x7F)?;
        self.update_modifiers(key, pressed);
        Some(KeyEvent {
            key,
            pressed,
            modifiers: self.modifiers(),
        })
    }

    pub fn modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.left_shift || self.right_shift,
            control: self.left_control || self.right_control,
            alt: self.left_alt || self.right_alt,
            meta: self.left_meta || self.right_meta,
            caps_lock: self.caps_lock,
            num_lock: self.num_lock,
            scroll_lock: self.scroll_lock,
        }
    }

    fn update_modifiers(&mut self, key: KeyCode, pressed: bool) {
        match key {
            KeyCode::LEFT_SHIFT => self.left_shift = pressed,
            KeyCode::RIGHT_SHIFT => self.right_shift = pressed,
            KeyCode::LEFT_CONTROL => self.left_control = pressed,
            KeyCode::RIGHT_CONTROL => self.right_control = pressed,
            KeyCode::LEFT_ALT => self.left_alt = pressed,
            KeyCode::RIGHT_ALT => self.right_alt = pressed,
            KeyCode::LEFT_META => self.left_meta = pressed,
            KeyCode::RIGHT_META => self.right_meta = pressed,
            KeyCode::CAPS_LOCK if pressed => self.caps_lock = !self.caps_lock,
            KeyCode::NUM_LOCK if pressed => self.num_lock = !self.num_lock,
            KeyCode::SCROLL_LOCK if pressed => self.scroll_lock = !self.scroll_lock,
            _ => {}
        }
    }
}

impl Default for KeyboardDecoder {
    fn default() -> Self {
        Self::new()
    }
}

fn map_set1_scancode(extended: bool, code: u8) -> Option<KeyCode> {
    if extended {
        return match code {
            0x1C => Some(KeyCode::KEYPAD_ENTER),
            0x1D => Some(KeyCode::RIGHT_CONTROL),
            0x35 => Some(KeyCode::KEYPAD_SLASH),
            0x38 => Some(KeyCode::RIGHT_ALT),
            0x37 => Some(KeyCode::PRINT_SCREEN),
            0x47 => Some(KeyCode::HOME),
            0x48 => Some(KeyCode::UP),
            0x49 => Some(KeyCode::PAGE_UP),
            0x4B => Some(KeyCode::LEFT),
            0x4D => Some(KeyCode::RIGHT),
            0x4F => Some(KeyCode::END),
            0x50 => Some(KeyCode::DOWN),
            0x51 => Some(KeyCode::PAGE_DOWN),
            0x52 => Some(KeyCode::INSERT),
            0x53 => Some(KeyCode::DELETE),
            0x5B => Some(KeyCode::LEFT_META),
            0x5C => Some(KeyCode::RIGHT_META),
            0x5D => Some(KeyCode::APPLICATION),
            _ => None,
        };
    }

    match code {
        0x01 => Some(KeyCode::ESCAPE),
        0x02 => Some(KeyCode::DIGIT_1),
        0x03 => Some(KeyCode::DIGIT_2),
        0x04 => Some(KeyCode::DIGIT_3),
        0x05 => Some(KeyCode::DIGIT_4),
        0x06 => Some(KeyCode::DIGIT_5),
        0x07 => Some(KeyCode::DIGIT_6),
        0x08 => Some(KeyCode::DIGIT_7),
        0x09 => Some(KeyCode::DIGIT_8),
        0x0A => Some(KeyCode::DIGIT_9),
        0x0B => Some(KeyCode::DIGIT_0),
        0x0C => Some(KeyCode::MINUS),
        0x0D => Some(KeyCode::EQUAL),
        0x0E => Some(KeyCode::BACKSPACE),
        0x0F => Some(KeyCode::TAB),
        0x10 => Some(KeyCode::Q),
        0x11 => Some(KeyCode::W),
        0x12 => Some(KeyCode::E),
        0x13 => Some(KeyCode::R),
        0x14 => Some(KeyCode::T),
        0x15 => Some(KeyCode::Y),
        0x16 => Some(KeyCode::U),
        0x17 => Some(KeyCode::I),
        0x18 => Some(KeyCode::O),
        0x19 => Some(KeyCode::P),
        0x1A => Some(KeyCode::LEFT_BRACKET),
        0x1B => Some(KeyCode::RIGHT_BRACKET),
        0x1C => Some(KeyCode::ENTER),
        0x1D => Some(KeyCode::LEFT_CONTROL),
        0x1E => Some(KeyCode::A),
        0x1F => Some(KeyCode::S),
        0x20 => Some(KeyCode::D),
        0x21 => Some(KeyCode::F),
        0x22 => Some(KeyCode::G),
        0x23 => Some(KeyCode::H),
        0x24 => Some(KeyCode::J),
        0x25 => Some(KeyCode::K),
        0x26 => Some(KeyCode::L),
        0x27 => Some(KeyCode::SEMICOLON),
        0x28 => Some(KeyCode::APOSTROPHE),
        0x29 => Some(KeyCode::GRAVE),
        0x2A => Some(KeyCode::LEFT_SHIFT),
        0x2B => Some(KeyCode::BACKSLASH),
        0x2C => Some(KeyCode::Z),
        0x2D => Some(KeyCode::X),
        0x2E => Some(KeyCode::C),
        0x2F => Some(KeyCode::V),
        0x30 => Some(KeyCode::B),
        0x31 => Some(KeyCode::N),
        0x32 => Some(KeyCode::M),
        0x33 => Some(KeyCode::COMMA),
        0x34 => Some(KeyCode::PERIOD),
        0x35 => Some(KeyCode::SLASH),
        0x36 => Some(KeyCode::RIGHT_SHIFT),
        0x37 => Some(KeyCode::KEYPAD_MULTIPLY),
        0x38 => Some(KeyCode::LEFT_ALT),
        0x39 => Some(KeyCode::SPACE),
        0x3A => Some(KeyCode::CAPS_LOCK),
        0x3B => Some(KeyCode::F1),
        0x3C => Some(KeyCode::F2),
        0x3D => Some(KeyCode::F3),
        0x3E => Some(KeyCode::F4),
        0x3F => Some(KeyCode::F5),
        0x40 => Some(KeyCode::F6),
        0x41 => Some(KeyCode::F7),
        0x42 => Some(KeyCode::F8),
        0x43 => Some(KeyCode::F9),
        0x44 => Some(KeyCode::F10),
        0x45 => Some(KeyCode::NUM_LOCK),
        0x46 => Some(KeyCode::SCROLL_LOCK),
        0x47 => Some(KeyCode::KEYPAD_7),
        0x48 => Some(KeyCode::KEYPAD_8),
        0x49 => Some(KeyCode::KEYPAD_9),
        0x4A => Some(KeyCode::KEYPAD_MINUS),
        0x4B => Some(KeyCode::KEYPAD_4),
        0x4C => Some(KeyCode::KEYPAD_5),
        0x4D => Some(KeyCode::KEYPAD_6),
        0x4E => Some(KeyCode::KEYPAD_PLUS),
        0x4F => Some(KeyCode::KEYPAD_1),
        0x50 => Some(KeyCode::KEYPAD_2),
        0x51 => Some(KeyCode::KEYPAD_3),
        0x52 => Some(KeyCode::KEYPAD_0),
        0x53 => Some(KeyCode::KEYPAD_DECIMAL),
        0x57 => Some(KeyCode::F11),
        0x58 => Some(KeyCode::F12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_is_fifo_and_counts_overflow() {
        let queue = ScanCodeQueue::new();
        for value in 0..(SCANCODE_QUEUE_CAPACITY - 1) {
            assert!(queue.push(value as u8));
        }
        assert!(!queue.push(0xAA));
        assert_eq!(queue.dropped_count(), 1);
        for value in 0..(SCANCODE_QUEUE_CAPACITY - 1) {
            assert_eq!(queue.try_pop(), Some(value as u8));
        }
        assert_eq!(queue.try_pop(), None);
    }

    #[test]
    fn decodes_make_and_break() {
        let mut decoder = KeyboardDecoder::new();
        let down = decoder.feed(0x1E).unwrap();
        let up = decoder.feed(0x9E).unwrap();
        assert_eq!(down.key, KeyCode::A);
        assert!(down.pressed);
        assert_eq!(up.key, KeyCode::A);
        assert!(!up.pressed);
    }

    #[test]
    fn tracks_both_shift_keys_without_early_release() {
        let mut decoder = KeyboardDecoder::new();
        decoder.feed(0x2A);
        decoder.feed(0x36);
        decoder.feed(0xAA);
        assert!(decoder.modifiers().shift);
        decoder.feed(0xB6);
        assert!(!decoder.modifiers().shift);
    }

    #[test]
    fn decodes_extended_navigation_and_right_control() {
        let mut decoder = KeyboardDecoder::new();
        assert_eq!(decoder.feed(0xE0), None);
        let up = decoder.feed(0x48).unwrap();
        assert_eq!(up.key, KeyCode::UP);
        assert!(up.pressed);

        decoder.feed(0xE0);
        let control = decoder.feed(0x1D).unwrap();
        assert_eq!(control.key, KeyCode::RIGHT_CONTROL);
        assert!(control.modifiers.control);
    }

    #[test]
    fn lock_keys_toggle_only_on_make() {
        let mut decoder = KeyboardDecoder::new();
        assert!(decoder.feed(0x3A).unwrap().modifiers.caps_lock);
        assert!(decoder.feed(0xBA).unwrap().modifiers.caps_lock);
        assert!(!decoder.feed(0x3A).unwrap().modifiers.caps_lock);
    }

    #[test]
    fn pause_sequence_is_consumed_without_fake_keys() {
        let mut decoder = KeyboardDecoder::new();
        for byte in [0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5] {
            assert_eq!(decoder.feed(byte), None);
        }
        assert_eq!(decoder.feed(0x1E).unwrap().key, KeyCode::A);
    }
}
