//! Tiled / LOD choropleth mesh path — Phase 3 beta prototype.

use genegis_geometry::PolygonRing;

use crate::choropleth::{
    build_mesh_from_features, ChoroplethFeature, ChoroplethMap, ChoroplethMesh,
};

/// Grid and LOD settings for tiled choropleth mesh generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TiledLodConfig {
    /// Horizontal tile count over the map bounding box.
    pub grid_x: u32,
    /// Vertical tile count over the map bounding box.
    pub grid_y: u32,
    /// Number of LOD levels (`0` = full detail).
    pub lod_levels: u32,
}

impl Default for TiledLodConfig {
    fn default() -> Self {
        Self {
            grid_x: 2,
            grid_y: 2,
            lod_levels: 3,
        }
    }
}

/// Spatial tile mesh at a specific LOD level.
#[derive(Debug, Clone)]
pub struct ChoroplethTileMesh {
    /// Tile column index.
    pub x: u32,
    /// Tile row index.
    pub y: u32,
    /// Triangulated mesh for this tile.
    pub mesh: ChoroplethMesh,
}

/// Pre-partitioned choropleth features by tile and LOD (geometry in map space).
#[derive(Debug, Clone)]
pub struct ChoroplethTiledLodMap {
    config: TiledLodConfig,
    map_bbox: (f64, f64, f64, f64),
    /// `levels[lod][tile_index]` feature buckets.
    levels: Vec<Vec<Vec<ChoroplethFeature>>>,
}

impl ChoroplethTiledLodMap {
    /// Partition map features into a tile grid with multiple LOD buckets.
    pub fn prepare(map: &ChoroplethMap, config: TiledLodConfig) -> Self {
        let grid_x = config.grid_x.max(1);
        let grid_y = config.grid_y.max(1);
        let lod_levels = config.lod_levels.max(1);
        let map_bbox = map.bbox();
        let tile_count = (grid_x * grid_y) as usize;

        let mut levels = Vec::with_capacity(lod_levels as usize);
        for lod in 0..lod_levels {
            let mut tiles = vec![Vec::new(); tile_count];
            for feature in &map.features {
                let Some((tx, ty)) = tile_for_feature(feature, map_bbox, grid_x, grid_y) else {
                    continue;
                };
                let idx = tile_index(tx, ty, grid_x) as usize;
                tiles[idx].push(feature_at_lod(feature, lod));
            }
            levels.push(tiles);
        }

        Self {
            config: TiledLodConfig {
                grid_x,
                grid_y,
                lod_levels,
            },
            map_bbox,
            levels,
        }
    }

    /// Configured LOD level count.
    pub fn lod_levels(&self) -> u32 {
        self.config.lod_levels
    }

    /// Tile grid dimensions `(columns, rows)`.
    pub fn grid(&self) -> (u32, u32) {
        (self.config.grid_x, self.config.grid_y)
    }

    /// Build per-tile meshes for the selected LOD and viewport.
    pub fn build_tile_meshes(
        &self,
        viewport_w: f32,
        viewport_h: f32,
        lod: u32,
    ) -> Vec<ChoroplethTileMesh> {
        let lod = lod.min(self.config.lod_levels.saturating_sub(1));
        let tiles = &self.levels[lod as usize];
        let mut meshes = Vec::with_capacity(tiles.len());

        for ty in 0..self.config.grid_y {
            for tx in 0..self.config.grid_x {
                let idx = tile_index(tx, ty, self.config.grid_x);
                let features = &tiles[idx as usize];
                let mesh = build_mesh_from_features(
                    features,
                    self.map_bbox,
                    viewport_w,
                    viewport_h,
                );
                meshes.push(ChoroplethTileMesh { x: tx, y: ty, mesh });
            }
        }

        meshes
    }

    /// Merge all tile meshes at the selected LOD into one draw batch.
    pub fn build_merged_mesh(
        &self,
        viewport_w: f32,
        viewport_h: f32,
        lod: u32,
    ) -> ChoroplethMesh {
        ChoroplethMesh::merge(
            self.build_tile_meshes(viewport_w, viewport_h, lod)
                .iter()
                .map(|tile| &tile.mesh)
                .filter(|mesh| !mesh.is_empty()),
        )
    }
}

/// Pick an LOD level from interactive zoom (`1.0` = full detail).
pub fn lod_for_zoom(zoom: f32, lod_levels: u32) -> u32 {
    let max_lod = lod_levels.saturating_sub(1);
    if zoom >= 1.0 {
        0
    } else if zoom >= 0.5 {
        1.min(max_lod)
    } else {
        max_lod
    }
}

fn tile_index(tx: u32, ty: u32, grid_x: u32) -> u32 {
    ty * grid_x + tx
}

fn tile_for_feature(
    feature: &ChoroplethFeature,
    map_bbox: (f64, f64, f64, f64),
    grid_x: u32,
    grid_y: u32,
) -> Option<(u32, u32)> {
    let (cx, cy) = feature_centroid(feature)?;
    Some(tile_for_point(
        cx, cy, map_bbox.0, map_bbox.1, map_bbox.2, map_bbox.3, grid_x, grid_y,
    ))
}

fn tile_for_point(
    x: f64,
    y: f64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    grid_x: u32,
    grid_y: u32,
) -> (u32, u32) {
    let dx = (max_x - min_x).max(1e-9);
    let dy = (max_y - min_y).max(1e-9);
    let tx = (((x - min_x) / dx) * grid_x as f64).floor() as u32;
    let ty = (((y - min_y) / dy) * grid_y as f64).floor() as u32;
    (tx.min(grid_x - 1), ty.min(grid_y - 1))
}

fn feature_centroid(feature: &ChoroplethFeature) -> Option<(f64, f64)> {
    let ring = feature.rings.first()?;
    let coords = ring.exterior();
    if coords.is_empty() {
        return None;
    }
    let (sum_x, sum_y) = coords
        .iter()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let n = coords.len() as f64;
    Some((sum_x / n, sum_y / n))
}

fn feature_at_lod(feature: &ChoroplethFeature, lod: u32) -> ChoroplethFeature {
    ChoroplethFeature {
        rings: feature
            .rings
            .iter()
            .map(|ring| simplify_ring(ring, lod))
            .collect(),
        color: feature.color,
    }
}

fn simplify_ring(ring: &PolygonRing, lod: u32) -> PolygonRing {
    let step = 1usize << lod.min(6);
    let coords = ring.exterior();
    if coords.len() <= 4 || step <= 1 {
        return ring.clone();
    }

    let mut simplified: Vec<(f64, f64)> = coords.iter().step_by(step).copied().collect();
    ensure_closed(&mut simplified);
    if simplified.len() < 4 {
        return ring.clone();
    }
    PolygonRing::new(simplified)
}

fn ensure_closed(coords: &mut Vec<(f64, f64)>) {
    if coords.len() < 2 {
        return;
    }
    let first = coords[0];
    let last = coords[coords.len() - 1];
    if (first.0 - last.0).abs() > f64::EPSILON || (first.1 - last.1).abs() > f64::EPSILON {
        coords.push(first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_style::ChoroplethStyle;

    fn sample_map() -> ChoroplethMap {
        let style = ChoroplethStyle::equal_interval(
            "density",
            "persons/km²",
            &[5000.0, 12000.0, 18000.0],
            3,
        );

        let mut map = ChoroplethMap::default();
        for (min_x, max_x, value) in [(136.88, 136.95, 6000.0), (136.95, 137.02, 15000.0)] {
            map.push_feature(
                vec![PolygonRing::new(vec![
                    (min_x, 35.15),
                    (max_x, 35.15),
                    (max_x, 35.20),
                    (min_x, 35.20),
                    (min_x, 35.15),
                ])],
                style.color_for(value),
            );
        }
        map
    }

    #[test]
    fn prepares_tile_grid_per_lod() {
        let map = sample_map();
        let tiled = ChoroplethTiledLodMap::prepare(
            &map,
            TiledLodConfig {
                grid_x: 2,
                grid_y: 2,
                lod_levels: 3,
            },
        );
        assert_eq!(tiled.grid(), (2, 2));
        assert_eq!(tiled.lod_levels(), 3);
        assert_eq!(tiled.levels.len(), 3);
    }

    #[test]
    fn higher_lod_has_fewer_or_equal_triangles() {
        let map = sample_map();
        let tiled = ChoroplethTiledLodMap::prepare(&map, TiledLodConfig::default());
        let full = tiled.build_merged_mesh(1280.0, 720.0, 0);
        let coarse = tiled.build_merged_mesh(1280.0, 720.0, tiled.lod_levels() - 1);
        assert!(!full.is_empty());
        assert!(coarse.triangle_count() <= full.triangle_count());
    }

    #[test]
    fn nagoya_tiled_path_covers_sixteen_wards() {
        use genegis_analysis::{default_nagoya_data_path, run_nagoya_population_density};

        let analysis = run_nagoya_population_density(default_nagoya_data_path()).expect("analysis");
        let mut map = ChoroplethMap::default();
        for feature in &analysis.features {
            map.push_feature(feature.rings.clone(), feature.color);
        }

        let tiled = ChoroplethTiledLodMap::prepare(&map, TiledLodConfig::default());
        let merged = tiled.build_merged_mesh(1280.0, 720.0, 0);
        assert_eq!(map.features.len(), 16);
        assert!(!merged.is_empty());
        assert!(merged.triangle_count() > 100);
    }
}
