//! Geometry primitives for GeneGIS spatial engine.

pub mod area;
pub mod bbox;
pub mod point;
pub mod polygon;
pub mod predicate;

pub use area::{
    area_km2_for_crs, area_km2_rings_for_crs, ellipsoidal_area_km2_rings,
    ellipsoidal_area_km2_wgs84, geodesic_area_km2_rings, geodesic_area_km2_wgs84,
    planar_area_km2_meters, planar_area_km2_rings, planar_area_km2_wgs84, polygon_area_km2_for_crs,
    polygon_parts_area_km2_for_crs, AreaError, AreaMethod,
};
pub use bbox::BoundingBox;
pub use point::{Coord, Point};
pub use polygon::PolygonRing;
pub use predicate::{point_in_polygon_parts, point_in_ring};
