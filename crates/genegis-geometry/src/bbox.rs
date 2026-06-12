use serde::{Deserialize, Serialize};

use crate::Coord;

/// Axis-aligned bounding box: min_x, min_y, max_x, max_y.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: Coord,
    pub max: Coord,
}

impl BoundingBox {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min: Coord {
                x: min_x,
                y: min_y,
                z: None,
            },
            max: Coord {
                x: max_x,
                y: max_y,
                z: None,
            },
        }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min.x && x <= self.max.x && y >= self.min.y && y <= self.max.y
    }
}
