use copc_streaming::{ByteSource, CopcError};
use genegis_storage::{fetch_http_range, probe_http_content_length, ByteRange};

/// HTTP(S) byte source backed by `genegis-storage` range reads.
pub struct HttpByteSource {
    url: String,
    size: u64,
}

impl HttpByteSource {
    /// Probe object size and prepare a range-read source for the URL.
    pub fn open(url: impl Into<String>) -> Result<Self, CopcError> {
        let url = url.into();
        let size = probe_http_content_length(&url).map_err(map_storage_error)?;
        Ok(Self { url, size })
    }
}

impl ByteSource for HttpByteSource {
    async fn read_range(&self, offset: u64, length: u64) -> Result<Vec<u8>, CopcError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let end = offset
            .checked_add(length - 1)
            .ok_or_else(|| CopcError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("read_range overflow: offset={offset}, length={length}"),
            )))?;
        let range = ByteRange::new(offset, end).map_err(map_storage_error)?;
        let result = fetch_http_range(&self.url, &range).map_err(map_storage_error)?;
        Ok(result.bytes)
    }

    async fn size(&self) -> Result<Option<u64>, CopcError> {
        Ok(Some(self.size))
    }
}

fn map_storage_error(err: genegis_storage::StorageError) -> CopcError {
    CopcError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        err.to_string(),
    ))
}
