#![no_std]

//! Bounds-checked ELF64 validation for the initial ExhiOS x86-64 kernel format.
//!
//! This crate deliberately does not load segments or perform relocations. It
//! validates the executable image before the bootloader starts allocating
//! physical memory.

pub const ELF_HEADER_SIZE: usize = 64;
pub const ELF64_PROGRAM_HEADER_SIZE: usize = 56;
pub const MAX_PROGRAM_HEADERS: usize = 128;

pub const ET_EXEC: u16 = 2;
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    TruncatedHeader,
    InvalidMagic,
    UnsupportedClass,
    UnsupportedEndianness,
    UnsupportedIdentVersion,
    NotExecutable,
    WrongMachine,
    UnsupportedElfVersion,
    InvalidHeaderSize,
    InvalidProgramHeaderSize,
    NoProgramHeaders,
    TooManyProgramHeaders,
    ProgramHeaderTableOutOfBounds,
    UnsupportedDynamicLinking,
    InvalidSegmentFileSize,
    SegmentFileRangeOutOfBounds,
    InvalidSegmentMemorySize,
    SegmentAddressOverflow,
    InvalidSegmentAlignment,
    WritableExecutableSegment,
    OverlappingLoadSegments,
    NoLoadableSegments,
    EntryPointNotExecutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramHeader {
    pub kind: u32,
    pub flags: u32,
    pub offset: u64,
    pub virtual_address: u64,
    pub physical_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
}

impl ProgramHeader {
    pub const fn is_loadable(&self) -> bool {
        self.kind == PT_LOAD
    }

    pub const fn is_executable(&self) -> bool {
        self.flags & PF_X != 0
    }

    pub const fn is_writable(&self) -> bool {
        self.flags & PF_W != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfFile<'a> {
    bytes: &'a [u8],
    program_header_offset: usize,
    program_header_count: usize,
    entry_point: u64,
}

impl<'a> ElfFile<'a> {
    /// Validate an x86-64 little-endian ELF executable and all its load segments.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
        if bytes.len() < ELF_HEADER_SIZE {
            return Err(ElfError::TruncatedHeader);
        }

        if bytes.get(0..4) != Some(b"\x7FELF") {
            return Err(ElfError::InvalidMagic);
        }
        if bytes[4] != 2 {
            return Err(ElfError::UnsupportedClass);
        }
        if bytes[5] != 1 {
            return Err(ElfError::UnsupportedEndianness);
        }
        if bytes[6] != 1 {
            return Err(ElfError::UnsupportedIdentVersion);
        }

        if read_u16(bytes, 16) != Some(ET_EXEC) {
            return Err(ElfError::NotExecutable);
        }
        if read_u16(bytes, 18) != Some(EM_X86_64) {
            return Err(ElfError::WrongMachine);
        }
        if read_u32(bytes, 20) != Some(1) {
            return Err(ElfError::UnsupportedElfVersion);
        }
        if read_u16(bytes, 52) != Some(ELF_HEADER_SIZE as u16) {
            return Err(ElfError::InvalidHeaderSize);
        }
        if read_u16(bytes, 54) != Some(ELF64_PROGRAM_HEADER_SIZE as u16) {
            return Err(ElfError::InvalidProgramHeaderSize);
        }

        let program_header_count = usize::from(
            read_u16(bytes, 56).ok_or(ElfError::TruncatedHeader)?,
        );

        if program_header_count == 0 {
            return Err(ElfError::NoProgramHeaders);
        }
        if program_header_count > MAX_PROGRAM_HEADERS {
            return Err(ElfError::TooManyProgramHeaders);
        }

        let program_header_offset = usize::try_from(
            read_u64(bytes, 32).ok_or(ElfError::TruncatedHeader)?,
        )
        .map_err(|_| ElfError::ProgramHeaderTableOutOfBounds)?;

        let table_size = program_header_count
            .checked_mul(ELF64_PROGRAM_HEADER_SIZE)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?;
        let table_end = program_header_offset
            .checked_add(table_size)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?;

        if table_end > bytes.len() {
            return Err(ElfError::ProgramHeaderTableOutOfBounds);
        }

        let entry_point = read_u64(bytes, 24).ok_or(ElfError::TruncatedHeader)?;
        let mut loadable_count = 0usize;
        let mut entry_is_executable = false;

        for index in 0..program_header_count {
            let header = parse_program_header(bytes, program_header_offset, index)?;

            if header.kind == PT_DYNAMIC || header.kind == PT_INTERP {
                return Err(ElfError::UnsupportedDynamicLinking);
            }

            if !header.is_loadable() {
                continue;
            }

            loadable_count += 1;

            if header.file_size > header.memory_size {
                return Err(ElfError::InvalidSegmentFileSize);
            }
            if header.memory_size == 0 {
                return Err(ElfError::InvalidSegmentMemorySize);
            }

            let file_offset = usize::try_from(header.offset)
                .map_err(|_| ElfError::SegmentFileRangeOutOfBounds)?;
            let file_size = usize::try_from(header.file_size)
                .map_err(|_| ElfError::SegmentFileRangeOutOfBounds)?;
            let file_end = file_offset
                .checked_add(file_size)
                .ok_or(ElfError::SegmentFileRangeOutOfBounds)?;

            if file_end > bytes.len() {
                return Err(ElfError::SegmentFileRangeOutOfBounds);
            }

            let memory_end = header
                .virtual_address
                .checked_add(header.memory_size)
                .ok_or(ElfError::SegmentAddressOverflow)?;

            if header.alignment > 1 {
                if !header.alignment.is_power_of_two()
                    || header.offset % header.alignment
                        != header.virtual_address % header.alignment
                {
                    return Err(ElfError::InvalidSegmentAlignment);
                }
            }

            if header.is_writable() && header.is_executable() {
                return Err(ElfError::WritableExecutableSegment);
            }

            if header.is_executable()
                && entry_point >= header.virtual_address
                && entry_point < memory_end
            {
                entry_is_executable = true;
            }

            // Reject byte-range overlap. The initial kernel linker script emits
            // distinct text/rodata and data ranges, so no overlap is required.
            for previous_index in 0..index {
                let previous =
                    parse_program_header(bytes, program_header_offset, previous_index)?;

                if previous.is_loadable() {
                    let previous_end = previous
                        .virtual_address
                        .checked_add(previous.memory_size)
                        .ok_or(ElfError::SegmentAddressOverflow)?;

                    if header.virtual_address < previous_end
                        && previous.virtual_address < memory_end
                    {
                        return Err(ElfError::OverlappingLoadSegments);
                    }
                }
            }
        }

        if loadable_count == 0 {
            return Err(ElfError::NoLoadableSegments);
        }
        if !entry_is_executable {
            return Err(ElfError::EntryPointNotExecutable);
        }

        Ok(Self {
            bytes,
            program_header_offset,
            program_header_count,
            entry_point,
        })
    }

    pub const fn entry_point(&self) -> u64 {
        self.entry_point
    }

    pub const fn program_header_count(&self) -> usize {
        self.program_header_count
    }

    pub fn program_headers(&self) -> ProgramHeaders<'a> {
        ProgramHeaders {
            bytes: self.bytes,
            offset: self.program_header_offset,
            count: self.program_header_count,
            next_index: 0,
        }
    }
}

pub struct ProgramHeaders<'a> {
    bytes: &'a [u8],
    offset: usize,
    count: usize,
    next_index: usize,
}

impl Iterator for ProgramHeaders<'_> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= self.count {
            return None;
        }

        let index = self.next_index;
        self.next_index += 1;
        parse_program_header(self.bytes, self.offset, index).ok()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.count - self.next_index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ProgramHeaders<'_> {}

fn parse_program_header(
    bytes: &[u8],
    table_offset: usize,
    index: usize,
) -> Result<ProgramHeader, ElfError> {
    let entry_offset = index
        .checked_mul(ELF64_PROGRAM_HEADER_SIZE)
        .and_then(|offset| table_offset.checked_add(offset))
        .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?;

    Ok(ProgramHeader {
        kind: read_u32(bytes, entry_offset).ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        flags: read_u32(bytes, entry_offset + 4)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        offset: read_u64(bytes, entry_offset + 8)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        virtual_address: read_u64(bytes, entry_offset + 16)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        physical_address: read_u64(bytes, entry_offset + 24)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        file_size: read_u64(bytes, entry_offset + 32)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        memory_size: read_u64(bytes, entry_offset + 40)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
        alignment: read_u64(bytes, entry_offset + 48)
            .ok_or(ElfError::ProgramHeaderTableOutOfBounds)?,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(read_array::<2>(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(read_array::<4>(bytes, offset)?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(read_array::<8>(bytes, offset)?))
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    let end = offset.checked_add(N)?;
    bytes.get(offset..end)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM_HEADER_OFFSET: usize = ELF_HEADER_SIZE;
    const SEGMENT_OFFSET: usize = ELF_HEADER_SIZE + ELF64_PROGRAM_HEADER_SIZE;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn write_load_header(
        bytes: &mut [u8],
        offset: usize,
        file_offset: u64,
        virtual_address: u64,
        flags: u32,
    ) {
        put_u32(bytes, offset, PT_LOAD);
        put_u32(bytes, offset + 4, flags);
        put_u64(bytes, offset + 8, file_offset);
        put_u64(bytes, offset + 16, virtual_address);
        put_u64(bytes, offset + 24, virtual_address);
        put_u64(bytes, offset + 32, 8);
        put_u64(bytes, offset + 40, 16);
        put_u64(bytes, offset + 48, 8);
    }

    fn valid_elf() -> allocless::Bytes {
        let mut bytes = allocless::Bytes([0; 128]);
        let image = &mut bytes.0;

        image[0..4].copy_from_slice(b"\x7FELF");
        image[4] = 2; // ELFCLASS64
        image[5] = 1; // ELFDATA2LSB
        image[6] = 1; // EV_CURRENT
        put_u16(image, 16, ET_EXEC);
        put_u16(image, 18, EM_X86_64);
        put_u32(image, 20, 1);
        put_u64(image, 24, 0x10_0000);
        put_u64(image, 32, PROGRAM_HEADER_OFFSET as u64);
        put_u16(image, 52, ELF_HEADER_SIZE as u16);
        put_u16(image, 54, ELF64_PROGRAM_HEADER_SIZE as u16);
        put_u16(image, 56, 1);

        write_load_header(
            image,
            PROGRAM_HEADER_OFFSET,
            SEGMENT_OFFSET as u64,
            0x10_0000,
            PF_X | 4,
        );
        image[SEGMENT_OFFSET..SEGMENT_OFFSET + 8].copy_from_slice(b"EXHIKERN");
        bytes
    }

    mod allocless {
        pub struct Bytes(pub [u8; 128]);
    }

    #[test]
    fn accepts_valid_x86_64_executable() {
        let bytes = valid_elf();
        let elf = ElfFile::parse(&bytes.0).unwrap();

        assert_eq!(elf.entry_point(), 0x10_0000);
        assert_eq!(elf.program_header_count(), 1);

        let mut headers = elf.program_headers();
        let first = headers.next().unwrap();
        assert!(first.is_loadable());
        assert_eq!(first.file_size, 8);
        assert_eq!(first.memory_size, 16);
        assert!(headers.next().is_none());
    }

    #[test]
    fn rejects_invalid_magic() {
        let mut bytes = valid_elf();
        bytes.0[0] = 0;
        assert_eq!(ElfFile::parse(&bytes.0), Err(ElfError::InvalidMagic));
    }

    #[test]
    fn rejects_wrong_machine() {
        let mut bytes = valid_elf();
        put_u16(&mut bytes.0, 18, 3);
        assert_eq!(ElfFile::parse(&bytes.0), Err(ElfError::WrongMachine));
    }

    #[test]
    fn rejects_program_header_table_outside_file() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes.0, 32, u64::MAX);
        assert_eq!(
            ElfFile::parse(&bytes.0),
            Err(ElfError::ProgramHeaderTableOutOfBounds)
        );
    }

    #[test]
    fn rejects_file_size_larger_than_memory_size() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes.0, PROGRAM_HEADER_OFFSET + 32, 17);
        assert_eq!(
            ElfFile::parse(&bytes.0),
            Err(ElfError::InvalidSegmentFileSize)
        );
    }

    #[test]
    fn rejects_invalid_segment_alignment() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes.0, PROGRAM_HEADER_OFFSET + 48, 3);
        assert_eq!(
            ElfFile::parse(&bytes.0),
            Err(ElfError::InvalidSegmentAlignment)
        );
    }

    #[test]
    fn rejects_writable_executable_segment() {
        let mut bytes = valid_elf();
        put_u32(
            &mut bytes.0,
            PROGRAM_HEADER_OFFSET + 4,
            PF_X | PF_W,
        );
        assert_eq!(
            ElfFile::parse(&bytes.0),
            Err(ElfError::WritableExecutableSegment)
        );
    }

    #[test]
    fn rejects_entry_point_outside_executable_segment() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes.0, 24, 0x20_0000);
        assert_eq!(
            ElfFile::parse(&bytes.0),
            Err(ElfError::EntryPointNotExecutable)
        );
    }

    #[test]
    fn rejects_load_segment_file_range_outside_file() {
        let mut bytes = valid_elf();
        put_u64(
            &mut bytes.0,
            PROGRAM_HEADER_OFFSET + 8,
            u64::MAX - 4,
        );
        assert_eq!(
            ElfFile::parse(&bytes.0),
            Err(ElfError::SegmentFileRangeOutOfBounds)
        );
    }

    #[test]
    fn rejects_overlapping_load_segments() {
        let mut bytes = [0u8; 192];
        bytes[0..4].copy_from_slice(b"\x7FELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        put_u16(&mut bytes, 16, ET_EXEC);
        put_u16(&mut bytes, 18, EM_X86_64);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 24, 0x10_0000);
        put_u64(&mut bytes, 32, PROGRAM_HEADER_OFFSET as u64);
        put_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16);
        put_u16(&mut bytes, 54, ELF64_PROGRAM_HEADER_SIZE as u16);
        put_u16(&mut bytes, 56, 2);

        write_load_header(
            &mut bytes,
            PROGRAM_HEADER_OFFSET,
            176,
            0x10_0000,
            PF_X,
        );
        write_load_header(
            &mut bytes,
            PROGRAM_HEADER_OFFSET + ELF64_PROGRAM_HEADER_SIZE,
            184,
            0x10_0008,
            4,
        );
        assert_eq!(
            ElfFile::parse(&bytes),
            Err(ElfError::OverlappingLoadSegments)
        );
    }
}
