//! Minimal ISO/OGC WKB reader and writer (2D; Z/M ordinates are dropped).

use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};

use crate::error::{Result, ToolkitError};

/// Encode a geometry as little-endian 2D WKB.
pub fn write(geometry: &Geometry<f64>) -> Vec<u8> {
    let mut out = Vec::new();
    write_geometry(&mut out, geometry);
    out
}

fn header(out: &mut Vec<u8>, kind: u32) {
    out.push(1);
    out.extend_from_slice(&kind.to_le_bytes());
}

fn coord(out: &mut Vec<u8>, c: Coord<f64>) {
    out.extend_from_slice(&c.x.to_le_bytes());
    out.extend_from_slice(&c.y.to_le_bytes());
}

fn count(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_le_bytes());
}

fn ring(out: &mut Vec<u8>, line: &LineString<f64>) {
    count(out, line.0.len());
    for c in &line.0 {
        coord(out, *c);
    }
}

fn polygon_body(out: &mut Vec<u8>, polygon: &Polygon<f64>) {
    count(out, 1 + polygon.interiors().len());
    ring(out, polygon.exterior());
    for interior in polygon.interiors() {
        ring(out, interior);
    }
}

fn write_geometry(out: &mut Vec<u8>, geometry: &Geometry<f64>) {
    match geometry {
        Geometry::Point(p) => {
            header(out, 1);
            coord(out, p.0);
        }
        Geometry::LineString(line) => {
            header(out, 2);
            ring(out, line);
        }
        Geometry::Line(line) => {
            header(out, 2);
            count(out, 2);
            coord(out, line.start);
            coord(out, line.end);
        }
        Geometry::Polygon(polygon) => {
            header(out, 3);
            polygon_body(out, polygon);
        }
        Geometry::Rect(rect) => write_geometry(out, &Geometry::Polygon(rect.to_polygon())),
        Geometry::Triangle(tri) => write_geometry(out, &Geometry::Polygon(tri.to_polygon())),
        Geometry::MultiPoint(multi) => {
            header(out, 4);
            count(out, multi.0.len());
            for p in &multi.0 {
                write_geometry(out, &Geometry::Point(*p));
            }
        }
        Geometry::MultiLineString(multi) => {
            header(out, 5);
            count(out, multi.0.len());
            for line in &multi.0 {
                write_geometry(out, &Geometry::LineString(line.clone()));
            }
        }
        Geometry::MultiPolygon(multi) => {
            header(out, 6);
            count(out, multi.0.len());
            for polygon in &multi.0 {
                header(out, 3);
                polygon_body(out, polygon);
            }
        }
        Geometry::GeometryCollection(collection) => {
            header(out, 7);
            count(out, collection.0.len());
            for part in &collection.0 {
                write_geometry(out, part);
            }
        }
    }
}

/// Decode ISO, EWKB-flagged, or plain WKB (Z/M ordinates are dropped).
pub fn read(bytes: &[u8]) -> Result<Geometry<f64>> {
    let mut reader = Reader { bytes, pos: 0 };
    let geometry = reader.geometry()?;
    Ok(geometry)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

fn err(reason: impl Into<String>) -> ToolkitError {
    ToolkitError::import("wkb", reason)
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| err("length overflow"))?;
        if end > self.bytes.len() {
            return Err(err("truncated geometry"));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self, little: bool) -> Result<u32> {
        let b: [u8; 4] = self.take(4)?.try_into().expect("4 bytes");
        Ok(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    fn f64(&mut self, little: bool) -> Result<f64> {
        let b: [u8; 8] = self.take(8)?.try_into().expect("8 bytes");
        Ok(if little {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }

    fn coord(&mut self, little: bool, dims: usize) -> Result<Coord<f64>> {
        let x = self.f64(little)?;
        let y = self.f64(little)?;
        for _ in 2..dims {
            self.f64(little)?;
        }
        Ok(Coord { x, y })
    }

    fn line(&mut self, little: bool, dims: usize) -> Result<LineString<f64>> {
        let n = self.u32(little)? as usize;
        if n > self.bytes.len() / 16 + 1 {
            return Err(err("implausible coordinate count"));
        }
        (0..n)
            .map(|_| self.coord(little, dims))
            .collect::<Result<Vec<_>>>()
            .map(LineString)
    }

    fn polygon(&mut self, little: bool, dims: usize) -> Result<Polygon<f64>> {
        let rings = self.u32(little)? as usize;
        if rings == 0 {
            return Ok(Polygon::new(LineString(vec![]), vec![]));
        }
        let exterior = self.line(little, dims)?;
        let interiors = (1..rings)
            .map(|_| self.line(little, dims))
            .collect::<Result<Vec<_>>>()?;
        Ok(Polygon::new(exterior, interiors))
    }

    fn geometry(&mut self) -> Result<Geometry<f64>> {
        let order = self.take(1)?[0];
        let little = match order {
            0 => false,
            1 => true,
            other => return Err(err(format!("invalid byte order {other}"))),
        };
        let raw = self.u32(little)?;
        // EWKB flags.
        let ewkb_z = raw & 0x8000_0000 != 0;
        let ewkb_m = raw & 0x4000_0000 != 0;
        if raw & 0x2000_0000 != 0 {
            self.u32(little)?; // embedded SRID
        }
        let code = raw & 0x0FFF_FFFF;
        let base = code % 1000;
        let iso = code / 1000;
        let dims = 2
            + usize::from(ewkb_z || iso == 1 || iso == 3)
            + usize::from(ewkb_m || iso == 2 || iso == 3);
        let child = |reader: &mut Self, expect: u32| -> Result<Geometry<f64>> {
            let geometry = reader.geometry()?;
            let ok = matches!(
                (expect, &geometry),
                (1, Geometry::Point(_)) | (2, Geometry::LineString(_)) | (3, Geometry::Polygon(_))
            );
            if !ok {
                return Err(err("multi-geometry member has the wrong type"));
            }
            Ok(geometry)
        };
        Ok(match base {
            1 => Geometry::Point(Point(self.coord(little, dims)?)),
            2 => Geometry::LineString(self.line(little, dims)?),
            3 => Geometry::Polygon(self.polygon(little, dims)?),
            4 => {
                let n = self.u32(little)? as usize;
                let mut points = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    if let Geometry::Point(p) = child(self, 1)? {
                        points.push(p);
                    }
                }
                Geometry::MultiPoint(MultiPoint(points))
            }
            5 => {
                let n = self.u32(little)? as usize;
                let mut lines = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    if let Geometry::LineString(l) = child(self, 2)? {
                        lines.push(l);
                    }
                }
                Geometry::MultiLineString(MultiLineString(lines))
            }
            6 => {
                let n = self.u32(little)? as usize;
                let mut polygons = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    if let Geometry::Polygon(p) = child(self, 3)? {
                        polygons.push(p);
                    }
                }
                Geometry::MultiPolygon(MultiPolygon(polygons))
            }
            7 => {
                let n = self.u32(little)? as usize;
                let mut parts = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    parts.push(self.geometry()?);
                }
                Geometry::GeometryCollection(GeometryCollection(parts))
            }
            other => return Err(err(format!("unsupported WKB geometry type {other}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::{line_string, polygon};

    #[test]
    fn round_trips_common_geometries() {
        let geometries = vec![
            Geometry::Point(Point::new(1.0, 2.0)),
            Geometry::LineString(line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 1.0)]),
            Geometry::Polygon(polygon![
                exterior: [(x: 0.0, y: 0.0), (x: 4.0, y: 0.0), (x: 4.0, y: 4.0), (x: 0.0, y: 0.0)],
                interiors: [[(x: 1.0, y: 1.0), (x: 2.0, y: 1.0), (x: 2.0, y: 2.0), (x: 1.0, y: 1.0)]],
            ]),
            Geometry::MultiPoint(MultiPoint(vec![Point::new(1.0, 1.0), Point::new(2.0, 2.0)])),
        ];
        for geometry in geometries {
            assert_eq!(read(&write(&geometry)).unwrap(), geometry);
        }
    }

    #[test]
    fn drops_z_from_iso_wkb() {
        let mut bytes = vec![1];
        bytes.extend_from_slice(&1001u32.to_le_bytes());
        for v in [1.0f64, 2.0, 3.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(read(&bytes).unwrap(), Geometry::Point(Point::new(1.0, 2.0)));
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(read(&[1, 1, 0, 0, 0, 0]).is_err());
    }
}
