//! Point-containment predicates for polygon parts.

use crate::PolygonRing;

/// Even-odd ray-cast containment for one closed ring.
///
/// The ring may or may not repeat its first vertex at the end; both forms
/// produce identical results because the closing segment wraps to index 0.
pub fn point_in_ring(point: (f64, f64), ring: &[(f64, f64)]) -> bool {
    let (x, y) = point;
    let mut inside = false;
    let mut previous = match ring.last() {
        Some(last) => *last,
        None => return false,
    };
    for &current in ring {
        let (x1, y1) = previous;
        let (x2, y2) = current;
        if (y1 > y) != (y2 > y) {
            let crossing_x = (x2 - x1) * (y - y1) / (y2 - y1) + x1;
            if x < crossing_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// Containment across polygon parts whose interior rings are holes.
///
/// A point is contained when it lies inside some part's exterior ring and
/// outside every hole of that same part. Parts are treated independently;
/// overlapping parts are caller error and resolve to simple disjunction.
pub fn point_in_polygon_parts(point: (f64, f64), parts: &[PolygonRing]) -> bool {
    parts.iter().any(|part| {
        if !point_in_ring(point, part.exterior()) {
            return false;
        }
        part.holes().iter().all(|hole| !point_in_ring(point, hole))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn detects_points_inside_and_outside_a_square() {
        let ring = square(0.0, 0.0, 10.0, 10.0);
        assert!(point_in_ring((5.0, 5.0), &ring));
        assert!(point_in_ring((0.5, 9.9), &ring));
        assert!(!point_in_ring((15.0, 5.0), &ring));
        assert!(!point_in_ring((-0.1, 5.0), &ring));
    }

    #[test]
    fn accepts_open_and_closed_rings_identically() {
        let open = square(0.0, 0.0, 10.0, 10.0);
        let mut closed = open.clone();
        closed.push(closed[0]);
        assert_eq!(
            point_in_ring((7.0, 3.0), &open),
            point_in_ring((7.0, 3.0), &closed)
        );
        assert!(point_in_ring((7.0, 3.0), &closed));
    }

    #[test]
    fn subtracts_holes_from_containment() {
        let part = PolygonRing::with_holes(
            square(0.0, 0.0, 10.0, 10.0),
            vec![square(4.0, 4.0, 6.0, 6.0)],
        );
        assert!(point_in_polygon_parts((2.0, 2.0), &[part.clone()]));
        assert!(!point_in_polygon_parts((5.0, 5.0), &[part]));
    }

    #[test]
    fn handles_multipolygon_disjunction() {
        let left = PolygonRing::new(square(0.0, 0.0, 2.0, 2.0));
        let right = PolygonRing::new(square(8.0, 8.0, 10.0, 10.0));
        assert!(point_in_polygon_parts(
            (1.0, 1.0),
            &[left.clone(), right.clone()]
        ));
        assert!(point_in_polygon_parts((9.0, 9.0), &[left, right]));
    }

    #[test]
    fn concave_notch_is_respected() {
        let notch = vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (6.0, 10.0),
            (6.0, 4.0),
            (0.0, 4.0),
        ];
        assert!(point_in_ring((2.0, 2.0), &notch));
        assert!(!point_in_ring((2.0, 8.0), &notch));
        assert!(point_in_ring((8.0, 8.0), &notch));
        assert!(point_in_ring((8.0, 2.0), &notch));
    }
}
