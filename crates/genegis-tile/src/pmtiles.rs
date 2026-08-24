//! PMTiles v3 header, directory, and tile range selection plus local archive writes.

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use genegis_storage::{
    fetch_http_range, is_remote_uri, probe_http_content_length, read_local_range, ByteRange,
    IoRequestEvidence,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use thiserror::Error;

const HEADER_BYTES: u64 = 127;
const ROOT_DIRECTORY_LIMIT: u64 = 16_384;
const MAX_DIRECTORY_BYTES: u64 = 16 * 1024 * 1024;

/// Parsed PMTiles v3 fields required for range selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmTilesHeader {
    /// Specification version; v3 only.
    pub version: u8,
    /// Root directory absolute offset.
    pub root_directory_offset: u64,
    /// Root directory compressed length.
    pub root_directory_length: u64,
    /// Leaf directory section absolute offset.
    pub leaf_directory_offset: u64,
    /// Leaf directory section accumulated length.
    pub leaf_directory_length: u64,
    /// Tile data section absolute offset.
    pub tile_data_offset: u64,
    /// Tile data section accumulated length.
    pub tile_data_length: u64,
    /// Number of addressed tiles before run-length encoding.
    pub addressed_tiles: u64,
    /// Internal directory compression code.
    pub internal_compression: u8,
    /// Tile compression code; returned bytes remain encoded.
    pub tile_compression: u8,
    /// Tile media type code.
    pub tile_type: u8,
    /// Minimum zoom.
    pub minimum_zoom: u8,
    /// Maximum zoom.
    pub maximum_zoom: u8,
}

/// Exact result and requests for one PMTiles tile selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmTilesTileRead {
    /// Parsed archive header.
    pub header: PmTilesHeader,
    /// Requested zoom.
    pub z: u8,
    /// Requested column.
    pub x: u32,
    /// Requested row.
    pub y: u32,
    /// Hilbert tile identity used for lookup.
    pub tile_id: u64,
    /// Exact encoded tile bytes.
    pub bytes: Vec<u8>,
    /// Archive encoded size.
    pub object_bytes: u64,
    /// Exact local seeks or HTTP 206 requests.
    pub requests: Vec<IoRequestEvidence>,
}

/// Fail-closed PMTiles parsing or range error.
#[derive(Debug, Error)]
pub enum PmTilesError {
    /// Local or HTTP storage failure.
    #[error("PMTiles storage failed: {0}")]
    Storage(String),
    /// Header or directory violates PMTiles v3 constraints.
    #[error("invalid PMTiles v3 archive: {0}")]
    Invalid(String),
    /// Internal compression is not supported by the transparent reader.
    #[error("unsupported PMTiles internal compression code {0}")]
    UnsupportedCompression(u8),
    /// The server ignored Range and attempted a whole-object response.
    #[error("PMTiles server ignored byte range and returned status {0}")]
    WholeObjectFallback(u16),
    /// Requested tile is absent.
    #[error("PMTiles tile {z}/{x}/{y} is not present")]
    TileNotFound {
        /// Zoom.
        z: u8,
        /// Column.
        x: u32,
        /// Row.
        y: u32,
    },
}

#[derive(Debug, Clone)]
struct DirectoryEntry {
    tile_id: u64,
    offset: u64,
    length: u64,
    run_length: u64,
}

/// Read one PMTiles v3 tile using only header, directory, leaf, and tile ranges.
pub fn read_pmtiles_tile(
    uri: &str,
    z: u8,
    x: u32,
    y: u32,
) -> Result<PmTilesTileRead, PmTilesError> {
    let object_bytes = if is_remote_uri(uri) {
        probe_http_content_length(uri).map_err(|error| PmTilesError::Storage(error.to_string()))?
    } else {
        std::fs::metadata(uri)
            .map_err(|error| PmTilesError::Storage(error.to_string()))?
            .len()
    };
    let mut requests = Vec::new();
    let header_bytes = read_exact_range(uri, 0, HEADER_BYTES, &mut requests)?;
    let header = parse_header(&header_bytes, object_bytes)?;
    let dimension = 1_u32.checked_shl(z as u32).unwrap_or(0);
    if z < header.minimum_zoom || z > header.maximum_zoom || x >= dimension || y >= dimension {
        return Err(PmTilesError::TileNotFound { z, x, y });
    }
    let tile_id = zxy_to_tile_id(z, x, y)?;
    let root_bytes = read_exact_range(
        uri,
        header.root_directory_offset,
        header.root_directory_length,
        &mut requests,
    )?;
    let root = decode_directory(&root_bytes, header.internal_compression)?;
    let selected = find_entry(&root, tile_id).ok_or(PmTilesError::TileNotFound { z, x, y })?;
    let tile_entry = if selected.run_length == 0 {
        let leaf_bytes = read_exact_range(
            uri,
            header
                .leaf_directory_offset
                .checked_add(selected.offset)
                .ok_or_else(|| PmTilesError::Invalid("leaf offset overflow".into()))?,
            selected.length,
            &mut requests,
        )?;
        let leaf = decode_directory(&leaf_bytes, header.internal_compression)?;
        find_entry(&leaf, tile_id)
            .filter(|entry| entry.run_length > 0)
            .ok_or(PmTilesError::TileNotFound { z, x, y })?
    } else {
        selected
    };
    if tile_id < tile_entry.tile_id || tile_id >= tile_entry.tile_id + tile_entry.run_length {
        return Err(PmTilesError::TileNotFound { z, x, y });
    }
    let bytes = read_exact_range(
        uri,
        header
            .tile_data_offset
            .checked_add(tile_entry.offset)
            .ok_or_else(|| PmTilesError::Invalid("tile offset overflow".into()))?,
        tile_entry.length,
        &mut requests,
    )?;
    Ok(PmTilesTileRead {
        header,
        z,
        x,
        y,
        tile_id,
        bytes,
        object_bytes,
        requests,
    })
}

fn read_exact_range(
    uri: &str,
    start: u64,
    length: u64,
    requests: &mut Vec<IoRequestEvidence>,
) -> Result<Vec<u8>, PmTilesError> {
    if length == 0 || length > MAX_DIRECTORY_BYTES {
        return Err(PmTilesError::Invalid(format!(
            "range length {length} is zero or exceeds {MAX_DIRECTORY_BYTES}"
        )));
    }
    let end = start
        .checked_add(length - 1)
        .ok_or_else(|| PmTilesError::Invalid("range overflow".into()))?;
    let range =
        ByteRange::new(start, end).map_err(|error| PmTilesError::Storage(error.to_string()))?;
    let (bytes, status) = if is_remote_uri(uri) {
        let response = fetch_http_range(uri, &range)
            .map_err(|error| PmTilesError::Storage(error.to_string()))?;
        if response.status != 206 {
            return Err(PmTilesError::WholeObjectFallback(response.status));
        }
        (response.bytes, Some(response.status))
    } else {
        (
            read_local_range(uri, &range)
                .map_err(|error| PmTilesError::Storage(error.to_string()))?,
            None,
        )
    };
    if bytes.len() as u64 != length {
        return Err(PmTilesError::Invalid(format!(
            "range length mismatch: requested {length}, got {}",
            bytes.len()
        )));
    }
    requests.push(IoRequestEvidence {
        start,
        end,
        response_bytes: bytes.len() as u64,
        http_status: status,
    });
    Ok(bytes)
}

fn parse_header(bytes: &[u8], object_bytes: u64) -> Result<PmTilesHeader, PmTilesError> {
    if bytes.len() != HEADER_BYTES as usize || &bytes[..7] != b"PMTiles" || bytes[7] != 3 {
        return Err(PmTilesError::Invalid(
            "magic, length, or version mismatch".into(),
        ));
    }
    let u64_at = |offset: usize| {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("header slice"))
    };
    let header = PmTilesHeader {
        version: bytes[7],
        root_directory_offset: u64_at(8),
        root_directory_length: u64_at(16),
        leaf_directory_offset: u64_at(40),
        leaf_directory_length: u64_at(48),
        tile_data_offset: u64_at(56),
        tile_data_length: u64_at(64),
        addressed_tiles: u64_at(72),
        internal_compression: bytes[97],
        tile_compression: bytes[98],
        tile_type: bytes[99],
        minimum_zoom: bytes[100],
        maximum_zoom: bytes[101],
    };
    let invalid_section = |offset: u64, length: u64, limit: u64| match offset.checked_add(length) {
        Some(end) => end > limit,
        None => true,
    };
    if header.root_directory_offset < HEADER_BYTES
        || invalid_section(
            header.root_directory_offset,
            header.root_directory_length,
            ROOT_DIRECTORY_LIMIT,
        )
        || header.minimum_zoom > header.maximum_zoom
        || invalid_section(
            header.tile_data_offset,
            header.tile_data_length,
            object_bytes,
        )
        || invalid_section(
            header.leaf_directory_offset,
            header.leaf_directory_length,
            object_bytes,
        )
        || invalid_section(
            header.root_directory_offset,
            header.root_directory_length,
            object_bytes,
        )
    {
        return Err(PmTilesError::Invalid(
            "section bounds, root latency limit, or zoom range are invalid".into(),
        ));
    }
    Ok(header)
}

fn decode_directory(bytes: &[u8], compression: u8) -> Result<Vec<DirectoryEntry>, PmTilesError> {
    let decoded = decode_payload(bytes, compression, "directory")?;
    let mut cursor = 0;
    let count = read_varint(&decoded, &mut cursor)? as usize;
    if count == 0 || count > 1_000_000 {
        return Err(PmTilesError::Invalid(
            "directory entry count is invalid".into(),
        ));
    }
    let mut entries = vec![
        DirectoryEntry {
            tile_id: 0,
            offset: 0,
            length: 0,
            run_length: 0,
        };
        count
    ];
    let mut last_id = 0_u64;
    for entry in &mut entries {
        last_id = last_id
            .checked_add(read_varint(&decoded, &mut cursor)?)
            .ok_or_else(|| PmTilesError::Invalid("tile ID overflow".into()))?;
        entry.tile_id = last_id;
    }
    for entry in &mut entries {
        entry.run_length = read_varint(&decoded, &mut cursor)?;
    }
    for entry in &mut entries {
        entry.length = read_varint(&decoded, &mut cursor)?;
        if entry.length == 0 {
            return Err(PmTilesError::Invalid("zero directory length".into()));
        }
    }
    for index in 0..entries.len() {
        let encoded = read_varint(&decoded, &mut cursor)?;
        entries[index].offset = if encoded == 0 && index > 0 {
            entries[index - 1]
                .offset
                .checked_add(entries[index - 1].length)
                .ok_or_else(|| PmTilesError::Invalid("directory offset overflow".into()))?
        } else if encoded > 0 {
            encoded - 1
        } else {
            return Err(PmTilesError::Invalid(
                "first directory offset cannot use zero sentinel".into(),
            ));
        };
    }
    if cursor != decoded.len() {
        return Err(PmTilesError::Invalid("trailing directory bytes".into()));
    }
    Ok(entries)
}

/// Decode one stored tile payload according to the header compression code.
///
/// The range reader returns still-encoded bytes; this helper performs the
/// transparent gzip step for consumers that need the original payload.
pub fn decode_tile_payload(bytes: &[u8], tile_compression: u8) -> Result<Vec<u8>, PmTilesError> {
    decode_payload(bytes, tile_compression, "tile")
}

fn decode_payload(bytes: &[u8], compression: u8, section: &str) -> Result<Vec<u8>, PmTilesError> {
    let decoded = match compression {
        1 => bytes.to_vec(),
        2 => {
            let mut output = Vec::new();
            GzDecoder::new(bytes)
                .take(MAX_DIRECTORY_BYTES + 1)
                .read_to_end(&mut output)
                .map_err(|error| PmTilesError::Invalid(format!("gzip {section}: {error}")))?;
            if output.len() as u64 > MAX_DIRECTORY_BYTES {
                return Err(PmTilesError::Invalid(format!(
                    "{section} decompression budget exceeded"
                )));
            }
            output
        }
        other => return Err(PmTilesError::UnsupportedCompression(other)),
    };
    Ok(decoded)
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, PmTilesError> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes
            .get(*cursor)
            .ok_or_else(|| PmTilesError::Invalid("truncated varint".into()))?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(PmTilesError::Invalid("varint overflow".into()))
}

fn find_entry(entries: &[DirectoryEntry], tile_id: u64) -> Option<DirectoryEntry> {
    entries
        .iter()
        .take_while(|entry| entry.tile_id <= tile_id)
        .last()
        .cloned()
}

/// One tile payload positioned by its tile coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmTilesTileEntry {
    /// Zoom level.
    pub z: u8,
    /// Tile column.
    pub x: u32,
    /// Tile row.
    pub y: u32,
    /// Encoded tile bytes stored verbatim.
    pub bytes: Vec<u8>,
}

/// Options for writing one local PMTiles v3 archive.
#[derive(Debug, Clone)]
pub struct PmTilesWriteOptions {
    /// JSON metadata stored in the archive metadata section.
    pub metadata_json: String,
    /// gzip-compress every tile payload instead of storing raw bytes.
    pub compress_tiles: bool,
}

impl PmTilesWriteOptions {
    /// Options with the supplied metadata and gzip-compressed tiles.
    pub fn compressed(metadata_json: impl Into<String>) -> Self {
        Self {
            metadata_json: metadata_json.into(),
            compress_tiles: true,
        }
    }
}

/// Summary evidence for one written PMTiles v3 archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmTilesWriteReceipt {
    /// Final encoded archive size in bytes.
    pub object_bytes: u64,
    /// Number of addressed tiles (run-length expanded).
    pub addressed_tiles: u64,
    /// Number of directory entries and distinct tile contents.
    pub entry_count: u64,
    /// Internal directory compression code written to the header.
    pub internal_compression: u8,
    /// Tile payload compression code written to the header.
    pub tile_compression: u8,
    /// Minimum zoom derived from entries.
    pub minimum_zoom: u8,
    /// Maximum zoom derived from entries.
    pub maximum_zoom: u8,
}

/// Write a clustered single-root-directory PMTiles v3 archive to a local path.
///
/// The writer emits exactly the subset of PMTiles v3 that
/// [`read_pmtiles_tile`] consumes fail-closed: no leaf directories, clustered
/// contiguous tiles, gzip-compressed internal directory. Tile payloads may be
/// pre-encoded MVT or any opaque media; the archive never interprets them.
pub fn write_pmtiles_archive(
    path: &str,
    entries: &[PmTilesTileEntry],
    options: &PmTilesWriteOptions,
) -> Result<PmTilesWriteReceipt, PmTilesError> {
    if entries.is_empty() {
        return Err(PmTilesError::Invalid(
            "archive requires at least one tile entry".into(),
        ));
    }
    let mut sorted: Vec<(u64, &[u8])> = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.bytes.is_empty() {
            return Err(PmTilesError::Invalid(format!(
                "tile {}/{}/{} has empty payload",
                entry.z, entry.x, entry.y
            )));
        }
        let tile_id = zxy_to_tile_id(entry.z, entry.x, entry.y)?;
        sorted.push((tile_id, &entry.bytes));
    }
    sorted.sort_by_key(|(tile_id, _)| *tile_id);
    let duplicate = sorted.windows(2).any(|pair| pair[0].0 == pair[1].0);
    if duplicate {
        return Err(PmTilesError::Invalid(
            "duplicate tile identity in write request".into(),
        ));
    }

    let tile_compression = if options.compress_tiles { 2 } else { 1 };
    let mut encoded_payloads: Vec<(u64, Vec<u8>)> = Vec::with_capacity(sorted.len());
    let mut directory: Vec<DirectoryEntry> = Vec::with_capacity(sorted.len());
    let mut offset = 0_u64;
    for (tile_id, bytes) in &sorted {
        let encoded = encode_payload(bytes, tile_compression)?;
        let length = encoded.len() as u64;
        encoded_payloads.push((*tile_id, encoded));
        directory.push(DirectoryEntry {
            tile_id: *tile_id,
            offset,
            length,
            run_length: 1,
        });
        offset += length;
    }

    let root_directory = gzip(&encode_directory(&directory)?)?;
    let metadata_bytes = gzip(options.metadata_json.as_bytes())?;
    let mut encoded_tiles: Vec<u8> = Vec::with_capacity(offset as usize);
    for (_, payload) in &encoded_payloads {
        encoded_tiles.extend_from_slice(payload);
    }

    let root_directory_offset = HEADER_BYTES;
    let json_metadata_offset = root_directory_offset + root_directory.len() as u64;
    let tile_data_offset = json_metadata_offset + metadata_bytes.len() as u64;
    let addressed_tiles = directory.len() as u64;
    let minimum_zoom = entries
        .iter()
        .map(|entry| entry.z)
        .min()
        .expect("non-empty");
    let maximum_zoom = entries
        .iter()
        .map(|entry| entry.z)
        .max()
        .expect("non-empty");

    let mut archive: Vec<u8> = Vec::with_capacity(tile_data_offset as usize + encoded_tiles.len());
    let layout = ArchiveLayout {
        root_length: root_directory.len() as u64,
        metadata_length: metadata_bytes.len() as u64,
        tile_data_offset,
        tile_data_length: encoded_tiles.len() as u64,
        addressed_tiles,
        minimum_zoom,
        maximum_zoom,
    };
    archive.extend_from_slice(&build_header(&layout, tile_compression));
    archive.extend_from_slice(&root_directory);
    archive.extend_from_slice(&metadata_bytes);
    archive.extend_from_slice(&encoded_tiles);
    std::fs::write(path, &archive).map_err(|error| PmTilesError::Storage(error.to_string()))?;
    Ok(PmTilesWriteReceipt {
        object_bytes: archive.len() as u64,
        addressed_tiles,
        entry_count: directory.len() as u64,
        internal_compression: 2,
        tile_compression,
        minimum_zoom,
        maximum_zoom,
    })
}

fn build_header(layout: &ArchiveLayout, tile_compression: u8) -> [u8; 127] {
    let mut header = [0_u8; 127];
    header[..7].copy_from_slice(b"PMTiles");
    header[7] = 3;
    let mut put_u64 = |offset: usize, value: u64| {
        header[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    };
    put_u64(8, HEADER_BYTES);
    put_u64(16, layout.root_length);
    put_u64(24, HEADER_BYTES + layout.root_length);
    put_u64(32, layout.metadata_length);
    put_u64(40, 0);
    put_u64(48, 0);
    put_u64(56, layout.tile_data_offset);
    put_u64(64, layout.tile_data_length);
    put_u64(72, layout.addressed_tiles);
    put_u64(80, layout.addressed_tiles);
    put_u64(88, layout.addressed_tiles);
    header[96] = 1;
    header[97] = 2;
    header[98] = tile_compression;
    header[99] = 1;
    header[100] = layout.minimum_zoom;
    header[101] = layout.maximum_zoom;
    header
}

struct ArchiveLayout {
    root_length: u64,
    metadata_length: u64,
    tile_data_offset: u64,
    tile_data_length: u64,
    addressed_tiles: u64,
    minimum_zoom: u8,
    maximum_zoom: u8,
}

fn encode_directory(entries: &[DirectoryEntry]) -> Result<Vec<u8>, PmTilesError> {
    let mut buffer = Vec::new();
    push_varint(&mut buffer, entries.len() as u64);
    let mut last_id = 0_u64;
    for entry in entries {
        push_varint(&mut buffer, entry.tile_id - last_id);
        last_id = entry.tile_id;
    }
    for entry in entries {
        push_varint(&mut buffer, entry.run_length);
    }
    for entry in entries {
        push_varint(&mut buffer, entry.length);
    }
    for (index, entry) in entries.iter().enumerate() {
        if index == 0 {
            push_varint(&mut buffer, entry.offset + 1);
        } else {
            let previous = &entries[index - 1];
            if previous.offset + previous.length == entry.offset {
                push_varint(&mut buffer, 0);
            } else {
                push_varint(&mut buffer, entry.offset + 1);
            }
        }
    }
    Ok(buffer)
}

fn push_varint(buffer: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buffer.push(byte);
            return;
        }
        buffer.push(byte | 0x80);
    }
}

fn encode_payload(bytes: &[u8], compression: u8) -> Result<Vec<u8>, PmTilesError> {
    match compression {
        1 => Ok(bytes.to_vec()),
        2 => gzip(bytes),
        other => Err(PmTilesError::UnsupportedCompression(other)),
    }
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>, PmTilesError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .map_err(|error| PmTilesError::Storage(error.to_string()))?;
    encoder
        .finish()
        .map_err(|error| PmTilesError::Storage(error.to_string()))
}

fn zxy_to_tile_id(z: u8, mut x: u32, mut y: u32) -> Result<u64, PmTilesError> {
    if z > 31 || x >= (1_u32 << z) || y >= (1_u32 << z) {
        return Err(PmTilesError::Invalid(
            "tile coordinate outside Hilbert domain".into(),
        ));
    }
    let base = ((1_u64 << (u32::from(z) * 2)) - 1) / 3;
    let mut distance = 0_u64;
    let mut scale = 1_u32 << z.saturating_sub(1);
    while scale > 0 {
        let rx = u32::from(x & scale != 0);
        let ry = u32::from(y & scale != 0);
        distance += u64::from(scale) * u64::from(scale) * u64::from((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = (1_u32 << z) - 1 - x;
                y = (1_u32 << z) - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        scale /= 2;
    }
    Ok(base + distance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    mod write_round_trip {
        use super::*;

        fn sample_entries() -> Vec<PmTilesTileEntry> {
            vec![
                PmTilesTileEntry {
                    z: 6,
                    x: 30,
                    y: 25,
                    bytes: b"tile-a".to_vec(),
                },
                PmTilesTileEntry {
                    z: 10,
                    x: 908,
                    y: 403,
                    bytes: b"tile-c-payload-longer".to_vec(),
                },
                PmTilesTileEntry {
                    z: 9,
                    x: 454,
                    y: 202,
                    bytes: b"tile-b-payload".to_vec(),
                },
            ]
        }

        fn read_all(path: &str) -> Vec<PmTilesTileRead> {
            sample_entries()
                .iter()
                .map(|entry| read_pmtiles_tile(path, entry.z, entry.x, entry.y).unwrap())
                .collect()
        }

        fn stored_bytes(read: &PmTilesTileRead) -> Vec<u8> {
            match read.header.tile_compression {
                1 => read.bytes.clone(),
                2 => {
                    let mut output = Vec::new();
                    GzDecoder::new(read.bytes.as_slice())
                        .read_to_end(&mut output)
                        .unwrap();
                    output
                }
                other => panic!("unexpected tile compression {other}"),
            }
        }

        #[test]
        fn writes_uncompressed_archive_with_exact_byte_roundtrip() {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("raw.pmtiles");
            let options = PmTilesWriteOptions {
                metadata_json: r#"{"attribution":"GeneGIS"}"#.into(),
                compress_tiles: false,
            };
            let receipt =
                write_pmtiles_archive(path.to_str().unwrap(), &sample_entries(), &options).unwrap();
            assert_eq!(receipt.addressed_tiles, 3);
            assert_eq!(receipt.entry_count, 3);
            assert_eq!(receipt.minimum_zoom, 6);
            assert_eq!(receipt.maximum_zoom, 10);
            assert_eq!(receipt.internal_compression, 2);
            assert_eq!(
                receipt.object_bytes,
                std::fs::metadata(&path).unwrap().len()
            );
            // Metadata section is gzip-compressed per PMTiles v3; decode and compare.
            let raw = std::fs::read(&path).unwrap();
            let root_length = u64::from_le_bytes(raw[16..24].try_into().unwrap());
            let metadata_offset = 127 + root_length as usize;
            let metadata_length = u64::from_le_bytes(raw[32..40].try_into().unwrap()) as usize;
            let decoded_metadata = decode_payload(
                &raw[metadata_offset..metadata_offset + metadata_length],
                2,
                "metadata",
            )
            .unwrap();
            assert_eq!(decoded_metadata, options.metadata_json.as_bytes());

            for read in read_all(path.to_str().unwrap()) {
                assert_eq!(read.requests.len(), 3);
                assert!(read
                    .requests
                    .iter()
                    .all(|request| request.http_status.is_none()));
                let entries = sample_entries();
                let expected = entries
                    .iter()
                    .find(|entry| {
                        zxy_to_tile_id(entry.z, entry.x, entry.y).unwrap() == read.tile_id
                    })
                    .expect("entry for tile id");
                assert_eq!(stored_bytes(&read), expected.bytes);
                assert_eq!(read.header.version, 3);
                assert_eq!(read.header.internal_compression, 2);
                assert_eq!(read.header.minimum_zoom, 6);
                assert_eq!(read.header.maximum_zoom, 10);
            }
        }

        #[test]
        fn writes_gzip_clustered_archive_readable_through_range_selection() {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("gz.pmtiles");
            let mut entries = sample_entries();
            entries[2].bytes = b"compressible-payload-".repeat(512);
            let read_entries = entries.clone();
            let receipt = write_pmtiles_archive(
                path.to_str().unwrap(),
                &entries,
                &PmTilesWriteOptions::compressed("{}"),
            )
            .unwrap();
            assert_eq!(receipt.tile_compression, 2);
            let raw_bytes = write_pmtiles_archive(
                temp.path().join("raw.pmtiles").to_str().unwrap(),
                &entries,
                &PmTilesWriteOptions {
                    metadata_json: "{}".into(),
                    compress_tiles: false,
                },
            )
            .unwrap()
            .object_bytes;
            assert!(std::fs::metadata(path.to_str().unwrap()).unwrap().len() < raw_bytes);
            for entry in &read_entries {
                let read =
                    read_pmtiles_tile(path.to_str().unwrap(), entry.z, entry.x, entry.y).unwrap();
                assert_eq!(read.header.tile_compression, 2);
            }
        }

        #[test]
        fn rejects_empty_entry_lists_and_duplicate_identities() {
            let temp = tempfile::tempdir().unwrap();
            let empty = write_pmtiles_archive(
                temp.path().join("empty.pmtiles").to_str().unwrap(),
                &[],
                &PmTilesWriteOptions::compressed("{}"),
            );
            assert!(matches!(empty, Err(PmTilesError::Invalid(_))));
            let duplicated = write_pmtiles_archive(
                temp.path().join("dupe.pmtiles").to_str().unwrap(),
                &[
                    PmTilesTileEntry {
                        z: 0,
                        x: 0,
                        y: 0,
                        bytes: b"one".to_vec(),
                    },
                    PmTilesTileEntry {
                        z: 0,
                        x: 0,
                        y: 0,
                        bytes: b"two".to_vec(),
                    },
                ],
                &PmTilesWriteOptions::compressed("{}"),
            );
            assert!(matches!(duplicated, Err(PmTilesError::Invalid(_))));
        }

        #[test]
        fn rejects_empty_tile_payloads() {
            let temp = tempfile::tempdir().unwrap();
            let result = write_pmtiles_archive(
                temp.path().join("void.pmtiles").to_str().unwrap(),
                &[PmTilesTileEntry {
                    z: 1,
                    x: 0,
                    y: 0,
                    bytes: Vec::new(),
                }],
                &PmTilesWriteOptions::compressed("{}"),
            );
            assert!(matches!(result, Err(PmTilesError::Invalid(_))));
        }
    }

    fn fixture() -> Vec<u8> {
        let root = [1_u8, 0, 1, 5, 1];
        let metadata = b"{}";
        let tile = b"hello";
        let mut bytes = vec![0_u8; HEADER_BYTES as usize];
        bytes[..7].copy_from_slice(b"PMTiles");
        bytes[7] = 3;
        for (offset, value) in [
            (8, 127_u64),
            (16, root.len() as u64),
            (24, 132),
            (32, 2),
            (40, 134),
            (48, 0),
            (56, 134),
            (64, tile.len() as u64),
            (72, 1),
            (80, 1),
            (88, 1),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[96] = 1;
        bytes[97] = 1;
        bytes[98] = 1;
        bytes[99] = 1;
        bytes[100] = 0;
        bytes[101] = 0;
        bytes.extend(root);
        bytes.extend(metadata);
        bytes.extend(tile);
        bytes
    }

    #[test]
    fn reads_one_tile_with_three_local_ranges() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), fixture()).unwrap();
        let result = read_pmtiles_tile(temp.path().to_str().unwrap(), 0, 0, 0).unwrap();
        assert_eq!(result.bytes, b"hello");
        assert_eq!(result.tile_id, 0);
        assert_eq!(result.requests.len(), 3);
        assert!(result
            .requests
            .iter()
            .all(|request| request.http_status.is_none()));
    }

    #[test]
    fn reads_http_206_and_rejects_bad_magic() {
        let body = fixture();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                serve(&mut stream.unwrap(), &body);
            }
        });
        let result =
            read_pmtiles_tile(&format!("http://{address}/fixture.pmtiles"), 0, 0, 0).unwrap();
        assert_eq!(result.bytes, b"hello");
        assert!(result
            .requests
            .iter()
            .all(|request| request.http_status == Some(206)));

        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), vec![0_u8; 127]).unwrap();
        assert!(matches!(
            read_pmtiles_tile(temp.path().to_str().unwrap(), 0, 0, 0),
            Err(PmTilesError::Invalid(_))
        ));
    }

    fn serve(stream: &mut TcpStream, body: &[u8]) {
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        if request.starts_with("HEAD ") {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            return;
        }
        let range = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("range")
                    .then(|| value.trim().strip_prefix("bytes=").unwrap_or(value.trim()))
            })
            .unwrap();
        let (start, end) = range.trim().split_once('-').unwrap();
        let start: usize = start.parse().unwrap();
        let end: usize = end.parse().unwrap();
        let slice = &body[start..=end];
        write!(
            stream,
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len(),
            slice.len()
        )
        .unwrap();
        stream.write_all(slice).unwrap();
    }
}
