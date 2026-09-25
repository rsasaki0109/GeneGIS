//! 令和2年国勢調査 地域メッシュ統計 from e-Stat 統計GIS (no application ID).
//!
//! The statistical GIS publishes population per standard grid square
//! (地域メッシュ) as one zipped CP932 CSV per first-level mesh. Grid squares
//! are defined by their codes, so geometry is computed rather than
//! downloaded.
//!
//! Secrecy (秘匿処理, HTKSYORI/HTKSAKI/GASSAN) suppresses only the detailed
//! breakdown columns; 人口（総数） is published for every cell. This was
//! established from the data itself: summing every row's total gives the
//! same population at 1 km, 500 m, and 250 m for each first-level mesh,
//! while dropping 秘匿地域 rows does not. Each cell therefore keeps its own
//! total and its own square, and the cross-level totals must match exactly.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use geo_types::{Geometry, Polygon};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::layer::{sha256_bytes, CrsStatus, Feature, Layer};
use crate::place::Fetcher;

/// License of e-Stat statistical data.
pub const LICENSE: &str = "政府標準利用規約（第2.0版）準拠 e-Stat 利用規約";
/// Required source statement.
pub const ATTRIBUTION: &str =
    "出典：政府統計の総合窓口(e-Stat)（https://www.e-stat.go.jp/）令和2年国勢調査 地域メッシュ統計を加工して作成";

/// Grid-square level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeshLevel {
    /// 3次メッシュ (~1 km).
    #[serde(rename = "1km")]
    Km1,
    /// 4次メッシュ (~500 m).
    #[serde(rename = "500m")]
    M500,
    /// 5次メッシュ (~250 m).
    #[serde(rename = "250m")]
    M250,
}

impl MeshLevel {
    /// e-Stat statistics ID for 令和2年国勢調査 人口等基本集計 at this level.
    pub fn stats_id(self) -> &'static str {
        match self {
            Self::Km1 => "T001140",
            Self::M500 => "T001141",
            Self::M250 => "T001142",
        }
    }

    /// Mesh code length.
    pub fn digits(self) -> usize {
        match self {
            Self::Km1 => 8,
            Self::M500 => 9,
            Self::M250 => 10,
        }
    }

    /// Parse `1km`, `500m`, `250m`.
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_lowercase().replace(' ', "").as_str() {
            "1km" | "1000m" | "3次" => Ok(Self::Km1),
            "500m" | "0.5km" | "4次" => Ok(Self::M500),
            "250m" | "0.25km" | "5次" => Ok(Self::M250),
            other => Err(ToolkitError::parameter(
                "census_mesh",
                format!("unknown mesh level {other}; use 1km, 500m, or 250m"),
            )),
        }
    }

    /// Human label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Km1 => "1kmメッシュ",
            Self::M500 => "500mメッシュ",
            Self::M250 => "250mメッシュ",
        }
    }
}

/// Download URL of one first-level mesh file.
pub fn download_url(level: MeshLevel, first_mesh: &str) -> String {
    format!(
        "https://www.e-stat.go.jp/gis/statmap-search/data?statsId={}&code={first_mesh}&downloadType=2",
        level.stats_id()
    )
}

fn digit(code: &str, i: usize) -> Result<u32> {
    code.as_bytes()
        .get(i)
        .filter(|b| b.is_ascii_digit())
        .map(|b| (b - b'0') as u32)
        .ok_or_else(|| ToolkitError::import("estat", format!("invalid mesh code {code}")))
}

/// `[min_lon, min_lat, max_lon, max_lat]` of a standard grid square (JGD2011).
pub fn mesh_bounds(code: &str) -> Result<[f64; 4]> {
    if !(4..=10).contains(&code.len()) || code.len() == 5 || code.len() == 7 {
        return Err(ToolkitError::import(
            "estat",
            format!("invalid mesh code {code}"),
        ));
    }
    let p = (digit(code, 0)? * 10 + digit(code, 1)?) as f64;
    let u = (digit(code, 2)? * 10 + digit(code, 3)?) as f64;
    let mut lat = p / 1.5;
    let mut lon = u + 100.0;
    let (mut dlat, mut dlon) = (2.0 / 3.0, 1.0);
    if code.len() >= 6 {
        dlat /= 8.0;
        dlon /= 8.0;
        lat += digit(code, 4)? as f64 * dlat;
        lon += digit(code, 5)? as f64 * dlon;
    }
    if code.len() >= 8 {
        dlat /= 10.0;
        dlon /= 10.0;
        lat += digit(code, 6)? as f64 * dlat;
        lon += digit(code, 7)? as f64 * dlon;
    }
    for i in 8..code.len() {
        // Quadrants: 1 SW, 2 SE, 3 NW, 4 NE.
        let q = digit(code, i)?;
        if !(1..=4).contains(&q) {
            return Err(ToolkitError::import(
                "estat",
                format!("invalid quadrant in mesh code {code}"),
            ));
        }
        dlat /= 2.0;
        dlon /= 2.0;
        if q >= 3 {
            lat += dlat;
        }
        if q == 2 || q == 4 {
            lon += dlon;
        }
    }
    Ok([lon, lat, lon + dlon, lat + dlat])
}

/// First-level mesh codes covering a lon/lat bounding box.
pub fn first_meshes(bbox: [f64; 4]) -> Vec<String> {
    let lat0 = (bbox[1] * 1.5).floor() as i64;
    let lat1 = (bbox[3] * 1.5).floor() as i64;
    let lon0 = bbox[0].floor() as i64 - 100;
    let lon1 = bbox[2].floor() as i64 - 100;
    let mut out = Vec::new();
    for p in lat0..=lat1 {
        for u in lon0..=lon1 {
            if (30..=68).contains(&p) && (22..=53).contains(&u) {
                out.push(format!("{p:02}{u:02}"));
            }
        }
    }
    out
}

fn rectangle(b: [f64; 4]) -> Polygon<f64> {
    Polygon::new(
        vec![
            (b[0], b[1]),
            (b[2], b[1]),
            (b[2], b[3]),
            (b[0], b[3]),
            (b[0], b[1]),
        ]
        .into(),
        vec![],
    )
}

/// One parsed statistics row.
#[derive(Debug, Clone)]
#[allow(dead_code)] // secrecy links are parsed for completeness
struct Row {
    code: String,
    secrecy: u8,
    merged: Vec<String>,
    total: Option<i64>,
    male: Option<i64>,
    female: Option<i64>,
}

fn number(value: &str) -> Option<i64> {
    match value.trim() {
        "" | "*" => None,
        "-" => Some(0),
        v => v.parse().ok(),
    }
}

/// Parse a zipped statistics file.
fn parse_zip(bytes: &[u8], level: MeshLevel) -> Result<Vec<Row>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ToolkitError::import("estat", format!("not a zip archive: {e}")))?;
    let mut raw = Vec::new();
    {
        let mut entry = archive
            .by_index(0)
            .map_err(|e| ToolkitError::import("estat", e.to_string()))?;
        entry.read_to_end(&mut raw)?;
    }
    let (text, _, _) = encoding_rs::SHIFT_JIS.decode(&raw);
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .ok_or_else(|| ToolkitError::import("estat", "empty file"))?
        .split(',')
        .collect();
    let column = |suffix: &str| {
        header
            .iter()
            .position(|h| h.trim() == format!("{}{suffix}", level.stats_id()))
            .ok_or_else(|| {
                ToolkitError::import(
                    "estat",
                    format!("column {}{suffix} missing", level.stats_id()),
                )
            })
    };
    let (total, male, female) = (column("001")?, column("002")?, column("003")?);
    let key = header
        .iter()
        .position(|h| h.trim() == "KEY_CODE")
        .unwrap_or(0);
    let secrecy = header.iter().position(|h| h.trim() == "HTKSYORI");
    let gassan = header.iter().position(|h| h.trim() == "GASSAN");
    lines.next(); // second header row: Japanese labels
    let mut rows = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        let Some(code) = cells
            .get(key)
            .map(|c| c.trim())
            .filter(|c| c.len() == level.digits())
        else {
            continue;
        };
        let get = |i: usize| cells.get(i).copied().unwrap_or("");
        rows.push(Row {
            code: code.to_string(),
            secrecy: secrecy
                .and_then(|i| get(i).trim().parse().ok())
                .unwrap_or(0),
            merged: gassan
                .map(|i| {
                    get(i)
                        .split(';')
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            total: number(get(total)),
            male: number(get(male)),
            female: number(get(female)),
        });
    }
    if rows.is_empty() {
        return Err(ToolkitError::import(
            "estat",
            "no mesh rows at the requested level",
        ));
    }
    Ok(rows)
}

/// Downloaded first-level mesh file with its identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshFile {
    /// First-level mesh code.
    pub first_mesh: String,
    /// Download URL.
    pub url: String,
    /// `sha256:` of the zip.
    pub sha256: String,
    /// Bytes.
    pub bytes: usize,
    /// Whether it came from the local cache.
    pub cached: bool,
}

/// Result of building a census mesh layer.
#[derive(Debug, Clone)]
pub struct CensusMesh {
    /// Mesh layer (cells intersecting the requested box).
    pub layer: Layer,
    /// Source files.
    pub files: Vec<MeshFile>,
    /// Population of whole downloaded first meshes, summed from raw rows.
    pub raw_total: i64,
    /// Population of whole downloaded first meshes, summed from the built
    /// features (before clipping to the box).
    pub built_total: i64,
    /// Cells with secrecy flags (breakdown columns suppressed).
    pub secret_cells: usize,
}

/// Build the mesh layer from downloaded files (pure; used by tests).
pub fn build_census_mesh(
    level: MeshLevel,
    bbox: [f64; 4],
    files: &[(MeshFile, Vec<u8>)],
) -> Result<CensusMesh> {
    let mut layer = Layer::new(
        format!("令和2年国勢調査 人口{}", level.label()),
        "EPSG:6668",
        CrsStatus::Declared,
    );
    let (mut raw_total, mut built_total, mut secret_cells, mut unknown) =
        (0i64, 0i64, 0usize, 0usize);
    let mut next_id = 0u64;
    for (_, bytes) in files {
        for row in parse_zip(bytes, level)? {
            if row.secrecy != 0 {
                secret_cells += 1;
            }
            match row.total {
                Some(total) => raw_total += total,
                None => unknown += 1,
            }
            let b = mesh_bounds(&row.code)?;
            built_total += row.total.unwrap_or(0);
            if b[0] > bbox[2] || b[2] < bbox[0] || b[1] > bbox[3] || b[3] < bbox[1] {
                continue;
            }
            layer.features.push(Feature {
                id: next_id,
                geometry: Some(Geometry::Polygon(rectangle(b))),
                properties: BTreeMap::from([
                    ("mesh_code".to_string(), Value::from(row.code.clone())),
                    (
                        "population".to_string(),
                        row.total.map(Value::from).unwrap_or(Value::Null),
                    ),
                    (
                        "male".to_string(),
                        row.male.map(Value::from).unwrap_or(Value::Null),
                    ),
                    (
                        "female".to_string(),
                        row.female.map(Value::from).unwrap_or(Value::Null),
                    ),
                ]),
            });
            next_id += 1;
        }
    }
    layer.refresh_schema();
    for field in ["population", "male", "female"] {
        layer.set_field_unit(field, "persons");
    }
    layer.provenance.license = Some(LICENSE.into());
    layer.provenance.attribution = Some(ATTRIBUTION.into());
    layer.provenance.format = "estat-mesh".into();
    layer.provenance.source_uri = files
        .first()
        .map(|(f, _)| f.url.clone())
        .unwrap_or_default();
    layer.provenance.source_sha256 = Some(sha256_bytes(
        files
            .iter()
            .map(|(f, _)| f.sha256.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
    ));
    layer.provenance.notes = vec![
        format!("令和2年国勢調査 人口等基本集計 {}（statsId {}）", level.label(), level.stats_id()),
        format!(
            "秘匿処理のある {secret_cells} セルも人口総数は公表値をそのまま使用（秘匿は内訳の列のみ）{}",
            if unknown > 0 { format!("; 総数が非公表のセル {unknown}") } else { String::new() }
        ),
        "メッシュは JGD2011 経緯度で定義（WGS 84 と同一とみなす）".into(),
    ];
    Ok(CensusMesh {
        layer,
        files: files.iter().map(|(f, _)| f.clone()).collect(),
        raw_total,
        built_total,
        secret_cells,
    })
}

/// Cache-first download of one first-level mesh file.
pub fn fetch_mesh_file(
    level: MeshLevel,
    first_mesh: &str,
    fetcher: &dyn Fetcher,
    cache: Option<&PathBuf>,
) -> Result<(MeshFile, Vec<u8>)> {
    let url = download_url(level, first_mesh);
    let cached_path = cache.map(|dir| dir.join(format!("{}_{first_mesh}.zip", level.stats_id())));
    if let Some(path) = &cached_path {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok((
                MeshFile {
                    first_mesh: first_mesh.into(),
                    url,
                    sha256: sha256_bytes(&bytes),
                    bytes: bytes.len(),
                    cached: true,
                },
                bytes,
            ));
        }
    }
    let bytes = fetcher.get(&url)?;
    if !bytes.starts_with(b"PK") {
        return Err(ToolkitError::Provider(format!(
            "e-Stat returned no data for mesh {first_mesh} ({url})"
        )));
    }
    if let Some(path) = &cached_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &bytes)?;
    }
    Ok((
        MeshFile {
            first_mesh: first_mesh.into(),
            url,
            sha256: sha256_bytes(&bytes),
            bytes: bytes.len(),
            cached: false,
        },
        bytes,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// Synthetic 1km (3次) file for first mesh 5236 in the published layout.
    pub(crate) fn fixture_zip(level: MeshLevel, rows: &[(&str, u8, &str, &str, i64)]) -> Vec<u8> {
        let id = level.stats_id();
        let mut text = format!("KEY_CODE,HTKSYORI,HTKSAKI,GASSAN,{id}001,{id}002,{id}003\n,,,,　人口（総数）,　人口（総数）　男,　人口（総数）　女\n");
        for (code, secrecy, saki, gassan, total) in rows {
            text.push_str(&format!(
                "{code},{secrecy},{saki},{gassan},{total},{},{}\n",
                total / 2,
                total - total / 2
            ));
        }
        let (encoded, _, _) = encoding_rs::SHIFT_JIS.encode(&text);
        let mut out = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
            zip.start_file(
                format!("tbl{id}X5236.txt"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&encoded).unwrap();
            zip.finish().unwrap();
        }
        out
    }

    #[test]
    fn mesh_codes_map_to_published_squares() {
        // 53394611 is the 3次 mesh containing 東京駅 (139.7671E, 35.6812N).
        let b = mesh_bounds("53394611").unwrap();
        assert!(
            (b[0] - 139.7625).abs() < 1e-9 && (b[1] - 35.675).abs() < 1e-9,
            "{b:?}"
        );
        assert!((b[2] - b[0] - 0.0125).abs() < 1e-12 && (b[3] - b[1] - 1.0 / 120.0).abs() < 1e-12);
        let quarter = mesh_bounds("5339461111").unwrap(); // SW quadrant of the SW quadrant
        assert!((quarter[2] - quarter[0] - 0.0125 / 4.0).abs() < 1e-12);
        assert!((quarter[0] - b[0]).abs() < 1e-12 && (quarter[1] - b[1]).abs() < 1e-12);
        let ne = mesh_bounds("533946114").unwrap();
        assert!(
            (ne[0] - (b[0] + 0.0125 / 2.0)).abs() < 1e-12
                && (ne[1] - (b[1] + 1.0 / 240.0)).abs() < 1e-12
        );
        assert!(mesh_bounds("53394615").is_ok());
        assert!(
            mesh_bounds("533946115").is_err(),
            "quadrant 5 does not exist"
        );
        assert_eq!(
            first_meshes([136.8, 35.0, 137.1, 35.3]),
            vec!["5236".to_string(), "5237".to_string()]
        );
    }

    #[test]
    fn every_cell_keeps_its_own_published_total() {
        let file = fixture_zip(
            MeshLevel::Km1,
            &[
                ("52365610", 0, "", "", 1000),
                ("52365611", 1, "", "52365612;52365613", 42),
                ("52365612", 2, "52365611", "", 3),
                ("52365613", 2, "52365611", "", 1),
            ],
        );
        let meta = MeshFile {
            first_mesh: "5236".into(),
            url: "fixture".into(),
            sha256: sha256_bytes(&file),
            bytes: file.len(),
            cached: true,
        };
        let mesh =
            build_census_mesh(MeshLevel::Km1, [136.0, 34.0, 138.0, 36.0], &[(meta, file)]).unwrap();
        assert_eq!(mesh.layer.features.len(), 4);
        assert_eq!(mesh.raw_total, 1046);
        assert_eq!(mesh.built_total, 1046);
        assert_eq!(mesh.secret_cells, 3);
        assert!(
            mesh.layer.invalid_feature_ids().is_empty(),
            "every cell is a plain square"
        );
        assert_eq!(
            mesh.layer.field("population").unwrap().unit.as_deref(),
            Some("persons")
        );
    }
}
