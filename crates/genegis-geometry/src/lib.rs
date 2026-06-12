//! Geometry primitives for GeneGIS spatial engine.

pub mod area;
pub mod bbox;
pub mod point;
pub mod polygon;

pub use area::{planar_area_km2_rings, planar_area_km2_wgs84, AreaMethod};
pub use bbox::BoundingBox;
pub use point::{Coord, Point};
pub use polygon::PolygonRing;
