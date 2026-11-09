#![no_std]

//! Bounded parser for the initial ExhiOS key=value configuration format.
//!
//! Lines are UTF-8, comments begin with '#', and the first '=' separates key
//! from value. Parsing never allocates and rejects oversized or malformed input.

#[cfg(test)]
extern crate std;

pub const MAX_LINE_BYTES: usize = 128;
pub const MAX_KEY_BYTES: usize = 32;
pub const MAX_VALUE_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigErrorKind {
    LineTooLong,
    MissingSeparator,
    EmptyKey,
    KeyTooLong,
    InvalidKey,
    ValueTooLong,
    InvalidValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigError {
    pub line: usize,
    pub kind: ConfigErrorKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigEntry<'a> {
    pub key: &'a str,
    pub value: &'a str,
}

pub struct ConfigParser<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
}

impl<'a> ConfigParser<'a> {
    pub const fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 0,
        }
    }

    pub const fn line_number(&self) -> usize {
        self.line
    }
}

impl<'a> Iterator for ConfigParser<'a> {
    type Item = Result<ConfigEntry<'a>, ConfigError>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.offset < self.source.len() {
            let start = self.offset;
            let bytes = self.source.as_bytes();
            let remaining = &bytes[start..];
            let line_length = remaining
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(remaining.len());
            let line_end = start + line_length;
            self.offset = line_end + usize::from(line_end < self.source.len());
            self.line += 1;

            let mut raw = &self.source[start..line_end];
            if raw.as_bytes().last() == Some(&b'\r') {
                raw = &raw[..raw.len() - 1];
            }
            if raw.len() > MAX_LINE_BYTES {
                return Some(Err(self.error(ConfigErrorKind::LineTooLong)));
            }

            let line = trim_ascii(raw);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some(separator) = line.as_bytes().iter().position(|byte| *byte == b'=') else {
                return Some(Err(self.error(ConfigErrorKind::MissingSeparator)));
            };
            let key = trim_ascii(&line[..separator]);
            let value = trim_ascii(&line[separator + 1..]);

            if key.is_empty() {
                return Some(Err(self.error(ConfigErrorKind::EmptyKey)));
            }
            if key.len() > MAX_KEY_BYTES {
                return Some(Err(self.error(ConfigErrorKind::KeyTooLong)));
            }
            if !key.as_bytes().iter().copied().all(is_key_byte) {
                return Some(Err(self.error(ConfigErrorKind::InvalidKey)));
            }
            if value.len() > MAX_VALUE_BYTES {
                return Some(Err(self.error(ConfigErrorKind::ValueTooLong)));
            }
            if value
                .as_bytes()
                .iter()
                .any(|byte| *byte < 0x20 || *byte == 0x7F)
            {
                return Some(Err(self.error(ConfigErrorKind::InvalidValue)));
            }

            return Some(Ok(ConfigEntry { key, value }));
        }
        None
    }
}

impl ConfigParser<'_> {
    fn error(&self, kind: ConfigErrorKind) -> ConfigError {
        ConfigError {
            line: self.line,
            kind,
        }
    }
}

fn trim_ascii(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && is_space(bytes[start]) {
        start += 1;
    }
    while end > start && is_space(bytes[end - 1]) {
        end -= 1;
    }
    &value[start..end]
}

const fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

const fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::format;
    use std::string::String;

    #[test]
    fn parses_comments_blank_lines_whitespace_and_crlf() {
        let source = "# system config\r\n\r\n hostname = exhios \r\nprompt=exhios> \r\n";
        let mut parser = ConfigParser::new(source);
        assert_eq!(
            parser.next(),
            Some(Ok(ConfigEntry {
                key: "hostname",
                value: "exhios",
            }))
        );
        assert_eq!(
            parser.next(),
            Some(Ok(ConfigEntry {
                key: "prompt",
                value: "exhios>",
            }))
        );
        assert_eq!(parser.next(), None);
        assert_eq!(parser.line_number(), 4);
    }

    #[test]
    fn preserves_equals_inside_values() {
        let mut parser = ConfigParser::new("motd=kernel=a=b\n");
        assert_eq!(
            parser.next(),
            Some(Ok(ConfigEntry {
                key: "motd",
                value: "kernel=a=b",
            }))
        );
    }

    #[test]
    fn reports_missing_separator_and_empty_key_with_line_numbers() {
        let mut parser = ConfigParser::new("ok=yes\nbroken\n=value\n");
        assert_eq!(
            parser.next(),
            Some(Ok(ConfigEntry {
                key: "ok",
                value: "yes",
            }))
        );
        assert_eq!(
            parser.next(),
            Some(Err(ConfigError {
                line: 2,
                kind: ConfigErrorKind::MissingSeparator,
            }))
        );
        assert_eq!(
            parser.next(),
            Some(Err(ConfigError {
                line: 3,
                kind: ConfigErrorKind::EmptyKey,
            }))
        );
    }

    #[test]
    fn rejects_invalid_keys_and_control_characters_in_values() {
        let mut invalid_key = ConfigParser::new("bad/key=value\n");
        assert_eq!(
            invalid_key.next(),
            Some(Err(ConfigError {
                line: 1,
                kind: ConfigErrorKind::InvalidKey,
            }))
        );

        let mut invalid_value = ConfigParser::new("key=bad\u{0001}value\n");
        assert_eq!(
            invalid_value.next(),
            Some(Err(ConfigError {
                line: 1,
                kind: ConfigErrorKind::InvalidValue,
            }))
        );
    }

    #[test]
    fn rejects_oversized_lines_keys_and_values() {
        let line = "x".repeat(MAX_LINE_BYTES + 1);
        let mut parser = ConfigParser::new(&line);
        assert_eq!(
            parser.next(),
            Some(Err(ConfigError {
                line: 1,
                kind: ConfigErrorKind::LineTooLong,
            }))
        );

        let key = "k".repeat(MAX_KEY_BYTES + 1);
        let mut parser = ConfigParser::new(&format!("{key}=v\n"));
        assert_eq!(
            parser.next(),
            Some(Err(ConfigError {
                line: 1,
                kind: ConfigErrorKind::KeyTooLong,
            }))
        );

        let value = "v".repeat(MAX_VALUE_BYTES + 1);
        let mut parser = ConfigParser::new(&format!("k={value}\n"));
        assert_eq!(
            parser.next(),
            Some(Err(ConfigError {
                line: 1,
                kind: ConfigErrorKind::ValueTooLong,
            }))
        );
    }
}
