//! Uniform grid spatial index over bounding boxes.
//!
//! A dependency-free alternative to an R-tree that is fast for the roughly
//! uniform, city-scale layers GeneGIS works with. Items are registered in
//! every cell their box overlaps; queries return each candidate once.

/// Grid index over item bounding boxes `[min_x, min_y, max_x, max_y]`.
#[derive(Debug, Clone)]
pub struct GridIndex {
    origin: (f64, f64),
    cell: f64,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
    boxes: Vec<[f64; 4]>,
}

impl GridIndex {
    /// Build an index with roughly `target_per_cell` items per cell.
    pub fn build(boxes: Vec<[f64; 4]>, target_per_cell: usize) -> Self {
        let mut extent = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for b in &boxes {
            extent = [
                extent[0].min(b[0]),
                extent[1].min(b[1]),
                extent[2].max(b[2]),
                extent[3].max(b[3]),
            ];
        }
        if boxes.is_empty() {
            extent = [0.0, 0.0, 1.0, 1.0];
        }
        let width = (extent[2] - extent[0]).max(f64::EPSILON);
        let height = (extent[3] - extent[1]).max(f64::EPSILON);
        let target_cells = (boxes.len() / target_per_cell.max(1)).clamp(1, 1 << 20) as f64;
        let cell = ((width * height) / target_cells)
            .sqrt()
            .max(width.max(height) / 4096.0);
        let columns = ((width / cell).ceil() as usize).clamp(1, 4096);
        let rows = ((height / cell).ceil() as usize).clamp(1, 4096);
        let mut index = Self {
            origin: (extent[0], extent[1]),
            cell,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
            boxes,
        };
        for i in 0..index.boxes.len() {
            let b = index.boxes[i];
            let (c0, r0, c1, r1) = index.cell_range(b);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    index.cells[r * columns + c].push(i);
                }
            }
        }
        index
    }

    fn clamp_col(&self, x: f64) -> usize {
        (((x - self.origin.0) / self.cell).floor().max(0.0) as usize).min(self.columns - 1)
    }

    fn clamp_row(&self, y: f64) -> usize {
        (((y - self.origin.1) / self.cell).floor().max(0.0) as usize).min(self.rows - 1)
    }

    fn cell_range(&self, b: [f64; 4]) -> (usize, usize, usize, usize) {
        (
            self.clamp_col(b[0]),
            self.clamp_row(b[1]),
            self.clamp_col(b[2]),
            self.clamp_row(b[3]),
        )
    }

    /// Items whose box intersects `query` expanded by `pad`, ascending, unique.
    pub fn query(&self, query: [f64; 4], pad: f64) -> Vec<usize> {
        let q = [
            query[0] - pad,
            query[1] - pad,
            query[2] + pad,
            query[3] + pad,
        ];
        let (c0, r0, c1, r1) = self.cell_range(q);
        let mut out = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                for &i in &self.cells[r * self.columns + c] {
                    let b = self.boxes[i];
                    if b[0] <= q[2] && q[0] <= b[2] && b[1] <= q[3] && q[1] <= b[3] {
                        out.push(i);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Nearest item to `point` under `distance` (exact distance to the item),
    /// searching rings of cells outward and stopping once no unseen cell can
    /// hold a closer item.
    pub fn nearest(
        &self,
        point: (f64, f64),
        mut distance: impl FnMut(usize) -> f64,
    ) -> Option<(usize, f64)> {
        if self.boxes.is_empty() {
            return None;
        }
        let (pc, pr) = (self.clamp_col(point.0), self.clamp_row(point.1));
        let mut best: Option<(usize, f64)> = None;
        let mut seen = vec![false; self.boxes.len()];
        let max_ring = self.columns.max(self.rows);
        for ring in 0..=max_ring {
            let (r0, r1) = (pr.saturating_sub(ring), (pr + ring).min(self.rows - 1));
            let (c0, c1) = (pc.saturating_sub(ring), (pc + ring).min(self.columns - 1));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let on_ring = r == r0 || r == r1 || c == c0 || c == c1;
                    if !on_ring {
                        continue;
                    }
                    for &i in &self.cells[r * self.columns + c] {
                        if std::mem::replace(&mut seen[i], true) {
                            continue;
                        }
                        let d = distance(i);
                        if best.is_none_or(|(_, b)| d < b) {
                            best = Some((i, d));
                        }
                    }
                }
            }
            // Anything not yet seen lies at least `ring` whole cells away
            // from the point's cell (the point may sit anywhere inside it).
            if let Some((_, b)) = best {
                if b <= ring as f64 * self.cell {
                    break;
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn query_matches_brute_force() {
        let mut seed = 7;
        let boxes: Vec<[f64; 4]> = (0..2000)
            .map(|_| {
                let (x, y) = (lcg(&mut seed) * 100.0, lcg(&mut seed) * 100.0);
                let (w, h) = (lcg(&mut seed) * 3.0, lcg(&mut seed) * 3.0);
                [x, y, x + w, y + h]
            })
            .collect();
        let index = GridIndex::build(boxes.clone(), 4);
        for _ in 0..200 {
            let (x, y) = (lcg(&mut seed) * 100.0, lcg(&mut seed) * 100.0);
            let q = [x, y, x + 5.0, y + 5.0];
            let pad = lcg(&mut seed) * 2.0;
            let brute: Vec<usize> = (0..boxes.len())
                .filter(|&i| {
                    let b = boxes[i];
                    b[0] <= q[2] + pad
                        && q[0] - pad <= b[2]
                        && b[1] <= q[3] + pad
                        && q[1] - pad <= b[3]
                })
                .collect();
            assert_eq!(index.query(q, pad), brute);
        }
    }

    #[test]
    fn nearest_matches_brute_force() {
        let mut seed = 11;
        let points: Vec<(f64, f64)> = (0..3000)
            .map(|_| (lcg(&mut seed) * 1000.0, lcg(&mut seed) * 500.0))
            .collect();
        let index = GridIndex::build(points.iter().map(|p| [p.0, p.1, p.0, p.1]).collect(), 4);
        for _ in 0..300 {
            let q = (
                lcg(&mut seed) * 1200.0 - 100.0,
                lcg(&mut seed) * 700.0 - 100.0,
            );
            let dist =
                |i: usize| ((points[i].0 - q.0).powi(2) + (points[i].1 - q.1).powi(2)).sqrt();
            let brute = (0..points.len())
                .map(|i| (i, dist(i)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap();
            let (_, d) = index.nearest(q, dist).unwrap();
            assert!((d - brute.1).abs() < 1e-9, "grid {d} vs brute {}", brute.1);
        }
    }
}
