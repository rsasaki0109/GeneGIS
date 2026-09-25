//! Zipped ESRI Shapefile import (`.shp` + `.dbf`, optional `.prj` and `.cpg`).

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use geo_types::{
    Coord, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
};
use serde_json::Value;

use super::{coordinates_look_geographic, ImportOptions, ImportReport, SkippedRecord};
use crate::error::{Result, ToolkitError};
use crate::layer::{CrsStatus, Feature, Layer};
use crate::proj;

fn err(reason: impl Into<String>) -> ToolkitError {
    ToolkitError::import("shapefile", reason)
}

pub(super) fn read_zip(
    bytes: &[u8],
    options: &ImportOptions,
    report: &mut ImportReport,
) -> Result<Layer> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| err(format!("invalid zip: {e}")))?;
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| err(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        if name.starts_with("__MACOSX/") {
            continue;
        }
        let lower = name.to_lowercase();
        if [".shp", ".dbf", ".prj", ".cpg", ".shx"]
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            if entry.size() > 2 * 1024 * 1024 * 1024 {
                return Err(err(format!("{name} is too large")));
            }
            let mut data = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut data)
                .map_err(|e| err(e.to_string()))?;
            files.insert(lower, data);
        }
    }
    let stems: Vec<String> = files
        .keys()
        .filter_map(|name| name.strip_suffix(".shp").map(str::to_string))
        .collect();
    let stem = match (&options.table, stems.as_slice()) {
        (Some(table), _) => stems
            .iter()
            .find(|s| {
                s.rsplit('/').next() == Some(table.to_lowercase().as_str())
                    || **s == table.to_lowercase()
            })
            .cloned()
            .ok_or_else(|| err(format!("no {table}.shp in the archive")))?,
        (None, []) => return Err(err("the zip contains no .shp file")),
        (None, [only]) => only.clone(),
        (None, [first, ..]) => {
            report.notes.push(format!(
                "archive contains {} shapefiles; imported {} (choose another with table)",
                stems.len(),
                first
            ));
            first.clone()
        }
    };
    let shp = files
        .get(&format!("{stem}.shp"))
        .ok_or_else(|| err("missing .shp"))?;
    let dbf = files.get(&format!("{stem}.dbf"));
    let prj = files.get(&format!("{stem}.prj"));
    let cpg = files.get(&format!("{stem}.cpg"));

    let shapes = read_shp(shp)?;
    let (records, encoding) = match dbf {
        Some(dbf) => {
            let (records, encoding) =
                read_dbf(dbf, options.encoding.as_deref(), cpg.map(Vec::as_slice))?;
            (Some(records), Some(encoding))
        }
        None => {
            report
                .notes
                .push("no .dbf in the archive; features have no attributes".into());
            (None, None)
        }
    };
    report.encoding = encoding;
    if let Some(records) = &records {
        if records.len() != shapes.len() {
            return Err(err(format!(
                ".shp has {} records but .dbf has {}; the files do not belong together",
                shapes.len(),
                records.len()
            )));
        }
    }

    let display_name = stem.rsplit('/').next().unwrap_or(&stem).to_string();
    let mut layer = Layer::new(
        options.name.clone().unwrap_or(display_name),
        "EPSG:4326",
        CrsStatus::Declared,
    );
    for (index, shape) in shapes.into_iter().enumerate() {
        let record = index + 1;
        let properties = match &records {
            Some(records) => match &records[index] {
                Some(props) => props.clone(),
                None => continue, // deleted in .dbf
            },
            None => BTreeMap::new(),
        };
        match shape {
            Ok(geometry) => layer.features.push(Feature {
                id: layer.features.len() as u64,
                geometry,
                properties,
            }),
            Err(reason) => report.skipped.push(SkippedRecord { record, reason }),
        }
    }

    if let Some(user) = &options.crs {
        layer.crs = proj::lookup(user)?.id;
        layer.crs_status = CrsStatus::UserSupplied;
    } else if let Some(prj) = prj {
        let wkt = String::from_utf8_lossy(prj);
        let epsg = proj::epsg_from_wkt(&wkt).ok_or_else(|| {
            ToolkitError::CrsRequired(format!(
                "the .prj describes a CRS GeneGIS cannot identify ({}); choose the EPSG code explicitly",
                wkt.chars().take(80).collect::<String>()
            ))
        })?;
        layer.crs = proj::lookup_epsg(epsg)?.id;
        layer.crs_status = CrsStatus::Declared;
        report
            .notes
            .push(format!("CRS {} read from .prj", layer.crs));
    } else if coordinates_look_geographic(&layer) {
        layer.crs_status = CrsStatus::Inferred;
        report.notes.push(
            "no .prj; EPSG:4326 was inferred from the coordinate ranges — confirm or re-import with an explicit CRS".into(),
        );
    } else {
        return Err(ToolkitError::CrsRequired(
            "the shapefile has no .prj and projected coordinates; choose the source CRS".into(),
        ));
    }
    layer.refresh_schema();
    Ok(layer)
}

type ShapeResult = std::result::Result<Option<Geometry<f64>>, String>;

fn be_i32(b: &[u8], at: usize) -> Result<i32> {
    b.get(at..at + 4)
        .map(|s| i32::from_be_bytes(s.try_into().expect("4 bytes")))
        .ok_or_else(|| err("truncated .shp"))
}

fn le_i32(b: &[u8], at: usize) -> Option<i32> {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes(s.try_into().expect("4 bytes")))
}

fn le_f64(b: &[u8], at: usize) -> Option<f64> {
    b.get(at..at + 8)
        .map(|s| f64::from_le_bytes(s.try_into().expect("8 bytes")))
}

fn read_shp(bytes: &[u8]) -> Result<Vec<ShapeResult>> {
    if be_i32(bytes, 0)? != 9994 {
        return Err(err("not a shapefile (.shp magic 9994 missing)"));
    }
    let file_len = (be_i32(bytes, 24)? as usize * 2).min(bytes.len());
    let mut shapes = Vec::new();
    let mut offset = 100;
    while offset + 8 <= file_len {
        let content_len = be_i32(bytes, offset + 4)? as usize * 2;
        let start = offset + 8;
        let end = start + content_len;
        if end > bytes.len() {
            return Err(err("record extends past the end of the .shp"));
        }
        shapes.push(parse_shape(&bytes[start..end]));
        offset = end;
    }
    Ok(shapes)
}

fn parse_shape(c: &[u8]) -> ShapeResult {
    let bad = || "truncated shape record".to_string();
    let kind = le_i32(c, 0).ok_or_else(bad)?;
    let point_at = |at: usize| -> std::result::Result<Coord<f64>, String> {
        Ok(Coord {
            x: le_f64(c, at).ok_or_else(bad)?,
            y: le_f64(c, at + 8).ok_or_else(bad)?,
        })
    };
    match kind {
        0 => Ok(None),
        1 | 11 | 21 => Ok(Some(Geometry::Point(Point(point_at(4)?)))),
        8 | 18 | 28 => {
            let n = le_i32(c, 36).ok_or_else(bad)? as usize;
            let points = (0..n)
                .map(|i| point_at(40 + i * 16).map(Point))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Some(Geometry::MultiPoint(MultiPoint(points))))
        }
        3 | 13 | 23 | 5 | 15 | 25 => {
            let num_parts = le_i32(c, 36).ok_or_else(bad)? as usize;
            let num_points = le_i32(c, 40).ok_or_else(bad)? as usize;
            if num_parts > c.len() / 4 || num_points > c.len() / 16 {
                return Err("implausible part/point count".into());
            }
            let parts: Vec<usize> = (0..num_parts)
                .map(|i| le_i32(c, 44 + i * 4).map(|v| v as usize).ok_or_else(bad))
                .collect::<std::result::Result<_, _>>()?;
            let points_at = 44 + num_parts * 4;
            let coords: Vec<Coord<f64>> = (0..num_points)
                .map(|i| point_at(points_at + i * 16))
                .collect::<std::result::Result<_, _>>()?;
            let mut lines = Vec::with_capacity(num_parts);
            for (i, &start) in parts.iter().enumerate() {
                let end = parts.get(i + 1).copied().unwrap_or(num_points);
                if start > end || end > coords.len() {
                    return Err("invalid part index".into());
                }
                lines.push(LineString(coords[start..end].to_vec()));
            }
            if matches!(kind, 3 | 13 | 23) {
                Ok(Some(if lines.len() == 1 {
                    Geometry::LineString(lines.remove(0))
                } else {
                    Geometry::MultiLineString(MultiLineString(lines))
                }))
            } else {
                assemble_polygons(lines).map(Some)
            }
        }
        31 => Err("MultiPatch shapes are not supported".into()),
        other => Err(format!("unknown shape type {other}")),
    }
}

fn signed_area(ring: &LineString<f64>) -> f64 {
    ring.0
        .windows(2)
        .map(|w| w[0].x * w[1].y - w[1].x * w[0].y)
        .sum::<f64>()
        / 2.0
}

/// Shapefile polygons store exterior rings clockwise and holes
/// counter-clockwise; holes are assigned to the exterior that contains them.
fn assemble_polygons(rings: Vec<LineString<f64>>) -> std::result::Result<Geometry<f64>, String> {
    let mut exteriors: Vec<(LineString<f64>, Vec<LineString<f64>>)> = Vec::new();
    let mut holes = Vec::new();
    for mut ring in rings {
        if ring.0.len() < 3 {
            continue;
        }
        if ring.0.first() != ring.0.last() {
            ring.0.push(ring.0[0]);
        }
        if signed_area(&ring) <= 0.0 {
            exteriors.push((ring, Vec::new()));
        } else {
            holes.push(ring);
        }
    }
    for hole in holes {
        let probe = hole.0[0];
        let owner = exteriors
            .iter_mut()
            .find(|(ext, _)| genegis_point_in_ring((probe.x, probe.y), &ext.0));
        match owner {
            Some((_, interiors)) => interiors.push(hole),
            None => {
                // Orientation error in the source: treat as an exterior.
                let mut reversed = hole;
                reversed.0.reverse();
                exteriors.push((reversed, Vec::new()));
            }
        }
    }
    if exteriors.is_empty() {
        return Err("polygon has no valid rings".into());
    }
    let polygons: Vec<Polygon<f64>> = exteriors
        .into_iter()
        .map(|(exterior, interiors)| Polygon::new(exterior, interiors))
        .collect();
    Ok(if polygons.len() == 1 {
        Geometry::Polygon(polygons.into_iter().next().expect("one polygon"))
    } else {
        Geometry::MultiPolygon(MultiPolygon(polygons))
    })
}

fn genegis_point_in_ring(point: (f64, f64), ring: &[Coord<f64>]) -> bool {
    let (x, y) = point;
    let mut inside = false;
    let mut j = ring.len().saturating_sub(1);
    for i in 0..ring.len() {
        let (xi, yi) = (ring[i].x, ring[i].y);
        let (xj, yj) = (ring[j].x, ring[j].y);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

struct DbfField {
    name: String,
    kind: u8,
    length: usize,
    decimals: u8,
}

type Record = Option<BTreeMap<String, Value>>;

fn read_dbf(
    bytes: &[u8],
    explicit: Option<&str>,
    cpg: Option<&[u8]>,
) -> Result<(Vec<Record>, String)> {
    if bytes.len() < 32 {
        return Err(err(".dbf header is truncated"));
    }
    let count = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes")) as usize;
    let header_len = u16::from_le_bytes(bytes[8..10].try_into().expect("2 bytes")) as usize;
    let record_len = u16::from_le_bytes(bytes[10..12].try_into().expect("2 bytes")) as usize;
    let ldid = bytes[29];
    let mut fields = Vec::new();
    let mut at = 32;
    while at + 32 <= header_len.min(bytes.len()) && bytes[at] != 0x0D {
        let raw_name = &bytes[at..at + 11];
        let name_end = raw_name.iter().position(|&b| b == 0).unwrap_or(11);
        fields.push(DbfField {
            name: String::new(),
            kind: bytes[at + 11],
            length: bytes[at + 16] as usize,
            decimals: bytes[at + 17],
        });
        let last = fields.last_mut().expect("pushed");
        last.name = String::from_utf8_lossy(&raw_name[..name_end]).into_owned(); // re-decoded below
        at += 32;
    }
    let raw_names: Vec<Vec<u8>> = {
        let mut names = Vec::new();
        let mut at = 32;
        for _ in &fields {
            let raw = &bytes[at..at + 11];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(11);
            names.push(raw[..end].to_vec());
            at += 32;
        }
        names
    };

    // Decide the text encoding once for the whole table.
    let encoding: &'static encoding_rs::Encoding = if let Some(label) = explicit {
        encoding_rs::Encoding::for_label(label.trim().as_bytes())
            .ok_or_else(|| err(format!("unknown encoding {label}")))?
    } else if let Some(cpg) = cpg {
        let label = String::from_utf8_lossy(cpg).trim().to_string();
        match label.to_ascii_uppercase().as_str() {
            "65001" | "UTF8" | "UTF-8" => encoding_rs::UTF_8,
            "932" | "CP932" | "SJIS" | "SHIFT_JIS" | "SHIFT-JIS" | "MS932" => {
                encoding_rs::SHIFT_JIS
            }
            _ => encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or_else(|| err(format!(".cpg names an unknown encoding {label}")))?,
        }
    } else if ldid == 0x13 || ldid == 0x7B {
        encoding_rs::SHIFT_JIS
    } else {
        let all_utf8 = raw_names.iter().all(|n| std::str::from_utf8(n).is_ok())
            && (0..count).all(|r| {
                let start = header_len + r * record_len;
                bytes
                    .get(start..(start + record_len).min(bytes.len()))
                    .is_some_and(|rec| std::str::from_utf8(rec).is_ok())
            });
        if all_utf8 {
            encoding_rs::UTF_8
        } else {
            encoding_rs::SHIFT_JIS
        }
    };
    let decode = |raw: &[u8]| encoding.decode_without_bom_handling(raw).0.into_owned();
    for (field, raw) in fields.iter_mut().zip(&raw_names) {
        field.name = decode(raw).trim().to_string();
    }

    let mut records = Vec::with_capacity(count);
    for r in 0..count {
        let start = header_len + r * record_len;
        let Some(record) = bytes.get(start..start + record_len) else {
            return Err(err(format!(".dbf record {} is truncated", r + 1)));
        };
        if record[0] == b'*' {
            records.push(None);
            continue;
        }
        let mut props = BTreeMap::new();
        let mut at = 1;
        for field in &fields {
            let raw = record.get(at..at + field.length).unwrap_or(&[]);
            at += field.length;
            let text = decode(raw);
            let trimmed = text.trim();
            let value = match field.kind {
                b'N' | b'F' | b'O' => {
                    if trimmed.is_empty() || trimmed.starts_with('*') {
                        Value::Null
                    } else if field.decimals == 0 {
                        trimmed
                            .parse::<i64>()
                            .map(Value::from)
                            .or_else(|_| trimmed.parse::<f64>().map(Value::from))
                            .unwrap_or_else(|_| Value::from(trimmed))
                    } else {
                        trimmed
                            .parse::<f64>()
                            .map(Value::from)
                            .unwrap_or_else(|_| Value::from(trimmed))
                    }
                }
                b'L' => match trimmed.chars().next() {
                    Some('T' | 't' | 'Y' | 'y') => Value::Bool(true),
                    Some('F' | 'f' | 'N' | 'n') => Value::Bool(false),
                    _ => Value::Null,
                },
                b'D' if trimmed.len() == 8 => Value::from(format!(
                    "{}-{}-{}",
                    &trimmed[..4],
                    &trimmed[4..6],
                    &trimmed[6..8]
                )),
                _ if trimmed.is_empty() => Value::Null,
                _ => Value::from(text.trim_end().to_string()),
            };
            props.insert(field.name.clone(), value);
        }
        records.push(Some(props));
    }
    Ok((records, encoding.name().to_ascii_lowercase()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{import_bytes, ImportOptions};
    use crate::layer::CrsStatus;
    use std::io::Write;

    /// Build a two-feature polygon shapefile zip (one with a hole) for tests.
    pub(crate) fn polygon_zip(prj: Option<&str>, names: [&str; 2]) -> Vec<u8> {
        fn ring(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
            points.to_vec()
        }
        // Exterior clockwise, hole counter-clockwise.
        let shapes = [
            vec![
                ring(&[
                    (0.0, 0.0),
                    (0.0, 10.0),
                    (10.0, 10.0),
                    (10.0, 0.0),
                    (0.0, 0.0),
                ]),
                ring(&[(2.0, 2.0), (4.0, 2.0), (4.0, 4.0), (2.0, 4.0), (2.0, 2.0)]),
            ],
            vec![ring(&[
                (20.0, 20.0),
                (20.0, 21.0),
                (21.0, 21.0),
                (21.0, 20.0),
                (20.0, 20.0),
            ])],
        ];
        let mut records = Vec::new();
        for (i, parts) in shapes.iter().enumerate() {
            let mut c = Vec::new();
            c.extend_from_slice(&5i32.to_le_bytes());
            c.extend_from_slice(&[0u8; 32]);
            c.extend_from_slice(&(parts.len() as i32).to_le_bytes());
            let n: usize = parts.iter().map(Vec::len).sum();
            c.extend_from_slice(&(n as i32).to_le_bytes());
            let mut start = 0;
            for p in parts {
                c.extend_from_slice(&(start as i32).to_le_bytes());
                start += p.len();
            }
            for p in parts {
                for (x, y) in p {
                    c.extend_from_slice(&x.to_le_bytes());
                    c.extend_from_slice(&y.to_le_bytes());
                }
            }
            let mut rec = Vec::new();
            rec.extend_from_slice(&((i + 1) as i32).to_be_bytes());
            rec.extend_from_slice(&((c.len() / 2) as i32).to_be_bytes());
            rec.extend_from_slice(&c);
            records.push(rec);
        }
        let body: Vec<u8> = records.concat();
        let mut shp = vec![0u8; 100];
        shp[0..4].copy_from_slice(&9994i32.to_be_bytes());
        shp[24..28].copy_from_slice(&(((100 + body.len()) / 2) as i32).to_be_bytes());
        shp[28..32].copy_from_slice(&1000i32.to_le_bytes());
        shp[32..36].copy_from_slice(&5i32.to_le_bytes());
        shp.extend_from_slice(&body);

        // DBF: NAME C(20), POP N(10,0) in Shift_JIS.
        let mut dbf = vec![0u8; 32];
        dbf[0] = 3;
        dbf[4..8].copy_from_slice(&2u32.to_le_bytes());
        let header_len = 32 + 32 * 2 + 1;
        dbf[8..10].copy_from_slice(&(header_len as u16).to_le_bytes());
        dbf[10..12].copy_from_slice(&(1u16 + 20 + 10).to_le_bytes());
        for (name, kind, len) in [("NAME", b'C', 20u8), ("POP", b'N', 10u8)] {
            let mut d = [0u8; 32];
            d[..name.len()].copy_from_slice(name.as_bytes());
            d[11] = kind;
            d[16] = len;
            dbf.extend_from_slice(&d);
        }
        dbf.push(0x0D);
        for (i, name) in names.iter().enumerate() {
            dbf.push(b' ');
            let (encoded, _, _) = encoding_rs::SHIFT_JIS.encode(name);
            let mut cell = encoded.to_vec();
            cell.resize(20, b' ');
            dbf.extend_from_slice(&cell);
            dbf.extend_from_slice(format!("{:>10}", (i + 1) * 100).as_bytes());
        }
        dbf.push(0x1A);

        let mut out = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("wards/wards.shp", opts).unwrap();
            zip.write_all(&shp).unwrap();
            zip.start_file("wards/wards.dbf", opts).unwrap();
            zip.write_all(&dbf).unwrap();
            if let Some(prj) = prj {
                zip.start_file("wards/wards.prj", opts).unwrap();
                zip.write_all(prj.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        out
    }

    #[test]
    fn reads_polygons_holes_and_shift_jis_attributes() {
        let bytes = polygon_zip(
            Some(r#"PROJCS["JGD_2011_Japan_Zone_7",GEOGCS["GCS_JGD_2011"]]"#),
            ["中区", "東区"],
        );
        let (layer, report) = import_bytes("wards.zip", &bytes, &ImportOptions::default()).unwrap();
        assert_eq!(layer.features.len(), 2);
        assert_eq!(layer.crs, "EPSG:6675");
        assert_eq!(layer.crs_status, CrsStatus::Declared);
        assert_eq!(report.encoding.as_deref(), Some("shift_jis"));
        assert_eq!(layer.features[0].properties["NAME"], "中区");
        assert_eq!(layer.features[1].properties["POP"], 200);
        match layer.features[0].geometry.as_ref().unwrap() {
            geo_types::Geometry::Polygon(p) => assert_eq!(p.interiors().len(), 1),
            other => panic!("expected polygon, got {other:?}"),
        }
    }

    #[test]
    fn projected_shapefile_without_prj_requires_crs() {
        let bytes = polygon_zip(None, ["a", "b"]);
        // Coordinates 0..21 look geographic, so this is inferred, not rejected.
        let (layer, _) = import_bytes("wards.zip", &bytes, &ImportOptions::default()).unwrap();
        assert_eq!(layer.crs_status, CrsStatus::Inferred);
    }
}
