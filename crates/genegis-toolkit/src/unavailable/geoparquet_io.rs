//! GeoParquet stand-in for builds without the `native` feature (e.g. the
//! browser): the format needs a C library, so it fails closed with a clear
//! reason instead of being silently skipped.

use crate::error::{Result, ToolkitError};
use crate::import::{ImportOptions, ImportReport};
use crate::layer::Layer;

const REASON: &str = "GeoParquet is not available in this build (requires the native feature)";

pub(crate) fn read(
    _bytes: &[u8],
    _options: &ImportOptions,
    _report: &mut ImportReport,
) -> Result<Layer> {
    Err(ToolkitError::import("geoparquet", REASON))
}

pub fn write(_layer: &Layer) -> Result<Vec<u8>> {
    Err(ToolkitError::import("geoparquet", REASON))
}
