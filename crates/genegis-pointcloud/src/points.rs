//! Point extraction from LAS / COPC point clouds (UC-5 epoch diff).
//!
//! Change detection needs raw XYZ samples, not just metadata. This module
//! exposes one uniform reader: COPC files stream through the copc hierarchy
//! (all nodes), everything else goes through the `las` crate. Points are
//! returned in source coordinates — callers record the CRS separately.

use crate::error::PointcloudError;

/// A dense-enough XYZ sample of a point cloud.
#[derive(Debug, Clone, PartialEq)]
pub struct PointCloud {
    /// `[x, y, z]` triples in source CRS units.
    pub points: Vec<[f64; 3]>,
}

impl PointCloud {
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// Axis-aligned bounds as `[min_x, min_y, min_z, max_x, max_y, max_z]`.
    pub fn bounds(&self) -> Option<[f64; 6]> {
        let mut iter = self.points.iter();
        let first = *iter.next()?;
        let mut bounds = [first[0], first[1], first[2], first[0], first[1], first[2]];
        for p in iter {
            bounds[0] = bounds[0].min(p[0]);
            bounds[1] = bounds[1].min(p[1]);
            bounds[2] = bounds[2].min(p[2]);
            bounds[3] = bounds[3].max(p[0]);
            bounds[4] = bounds[4].max(p[1]);
            bounds[5] = bounds[5].max(p[2]);
        }
        Some(bounds)
    }
}

/// Read all points from a `.copc.laz` or plain `.las` file.
pub fn read_point_cloud_path(path: &str) -> Result<PointCloud, PointcloudError> {
    if path.ends_with(".copc.laz") {
        read_copc_points(path)
    } else {
        read_las_points(path)
    }
}

fn read_las_points(path: &str) -> Result<PointCloud, PointcloudError> {
    let mut reader = las::Reader::from_path(path)
        .map_err(|error| PointcloudError::Copc(format!("las open {path}: {error}")))?;
    let points = reader
        .points()
        .filter_map(|result| result.ok())
        .map(|point| [point.x, point.y, point.z])
        .collect();
    Ok(PointCloud { points })
}

fn read_copc_points(path: &str) -> Result<PointCloud, PointcloudError> {
    let source = copc_streaming::FileSource::open(path)
        .map_err(|error| PointcloudError::Copc(error.to_string()))?;
    let mut reader = crate::runtime::block_on(copc_streaming::CopcStreamingReader::open(source))
        .map_err(|error| PointcloudError::Copc(error.to_string()))?;
    crate::runtime::block_on(reader.load_all_hierarchy())
        .map_err(|error| PointcloudError::Copc(error.to_string()))?;

    let mut points = Vec::new();
    for (key, _entry) in reader.entries().collect::<Vec<_>>() {
        let chunk = crate::runtime::block_on(reader.fetch_chunk(key))
            .map_err(|error| PointcloudError::Copc(error.to_string()))?;
        let node_points = reader
            .read_points(&chunk)
            .map_err(|error| PointcloudError::Copc(error.to_string()))?;
        points.extend(node_points.into_iter().map(|p| [p.x, p.y, p.z]));
    }
    if points.is_empty() {
        return Err(PointcloudError::Copc(format!(
            "COPC {path} contained no decodable points"
        )));
    }
    Ok(PointCloud { points })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;

    fn write_tmp_las(path: &std::path::Path) {
        let header = las::Header::default();
        let mut writer = las::Writer::new(
            BufWriter::new(std::fs::File::create(path).expect("create")),
            header,
        )
        .expect("writer");
        for i in 0..8_i64 {
            writer
                .write(las::Point {
                    x: i as f64,
                    y: (i % 3) as f64,
                    z: i as f64 * 0.5,
                    ..Default::default()
                })
                .expect("write point");
        }
    }

    #[test]
    fn roundtrips_plain_las_points() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("tiny.las");
        write_tmp_las(&path);
        let cloud = read_point_cloud_path(path.to_str().unwrap()).expect("read");
        assert_eq!(cloud.point_count(), 8);
        let bounds = cloud.bounds().expect("bounds");
        assert_eq!(bounds[0], 0.0);
        assert_eq!(bounds[3], 7.0);
        assert!((bounds[5] - 3.5).abs() < 1e-9);
    }

    #[test]
    fn rejects_missing_files() {
        assert!(read_point_cloud_path("/nonexistent/tiny.las").is_err());
    }
}
