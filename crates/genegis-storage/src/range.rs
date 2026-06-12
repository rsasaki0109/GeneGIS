use crate::error::StorageError;

/// Inclusive HTTP byte range (`bytes=start-end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// First byte offset (inclusive).
    pub start: u64,
    /// Last byte offset (inclusive).
    pub end: u64,
}

impl ByteRange {
    /// Create an inclusive byte range.
    pub fn new(start: u64, end: u64) -> Result<Self, StorageError> {
        if end < start {
            return Err(StorageError::InvalidRange(format!(
                "end ({end}) must be >= start ({start})"
            )));
        }
        Ok(Self { start, end })
    }

    /// Prefix range `0..=end` for header/metadata probes.
    pub fn prefix(end: u64) -> Result<Self, StorageError> {
        Self::new(0, end)
    }

    /// Number of bytes covered by the range.
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Render the HTTP `Range` header value (`bytes=start-end`).
    pub fn header_value(&self) -> String {
        format!("bytes={}-{}", self.start, self.end)
    }

    /// Parse `START-END` or `bytes=START-END`.
    pub fn parse(input: &str) -> Result<Self, StorageError> {
        let trimmed = input.trim();
        let spec = trimmed
            .strip_prefix("bytes=")
            .unwrap_or(trimmed);
        let (start, end) = spec.split_once('-').ok_or_else(|| {
            StorageError::InvalidRange(format!("expected START-END, got {input:?}"))
        })?;
        Self::new(parse_u64(start)?, parse_u64(end)?)
    }
}

fn parse_u64(value: &str) -> Result<u64, StorageError> {
    value.trim().parse::<u64>().map_err(|err| {
        StorageError::InvalidRange(format!("invalid offset {value:?}: {err}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_range_spec() {
        let range = ByteRange::parse("0-4095").expect("range");
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 4095);
        assert_eq!(range.len(), 4096);
    }

    #[test]
    fn rejects_inverted_range() {
        assert!(ByteRange::new(10, 5).is_err());
    }
}
