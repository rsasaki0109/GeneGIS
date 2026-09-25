//! Built-in coordinate transformations.
//!
//! GeneGIS keeps projection support explicit rather than delegating to an
//! opaque system library: every supported EPSG code is listed here with its
//! datum assumption, and anything else fails closed. JGD2000, JGD2011, and
//! WGS 84 are treated as coincident (differences are below ~1 m at the scale
//! of city analysis); the assumption is reported with every transform.

use geo::MapCoords;
use geo_types::{Coord, Geometry};
use serde::{Deserialize, Serialize};

use crate::error::{Result, ToolkitError};

/// GRS80 semi-major axis in metres (JGD2000/JGD2011).
const GRS80_A: f64 = 6_378_137.0;
/// GRS80 flattening. WGS 84 differs only in the 1e-11 range.
const GRS80_F: f64 = 1.0 / 298.257_222_101;

/// Assumption recorded whenever coordinates change datum family.
pub const DATUM_ASSUMPTION: &str =
    "WGS 84, JGD2000, and JGD2011 are treated as coincident (sub-metre to ~1 m difference)";

/// Axis unit family of a CRS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisUnit {
    /// Longitude/latitude in decimal degrees.
    Degrees,
    /// Projected easting/northing in metres.
    Metres,
}

/// Projection method behind a supported EPSG code.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Projection {
    /// Geographic longitude/latitude.
    Geographic,
    /// Spherical Web Mercator (EPSG:3857).
    WebMercator,
    /// Ellipsoidal transverse Mercator (Krüger series).
    TransverseMercator {
        /// Latitude of origin in degrees.
        lat0: f64,
        /// Central meridian in degrees.
        lon0: f64,
        /// Scale factor on the central meridian.
        k0: f64,
        /// False easting in metres.
        false_easting: f64,
        /// False northing in metres.
        false_northing: f64,
    },
}

/// A CRS GeneGIS can transform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrsInfo {
    /// EPSG code.
    pub epsg: u32,
    /// `EPSG:<code>` identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Axis unit.
    pub unit: AxisUnit,
    /// Projection method.
    pub projection: Projection,
}

impl CrsInfo {
    /// Whether the CRS uses degrees.
    pub fn is_geographic(&self) -> bool {
        self.unit == AxisUnit::Degrees
    }
}

/// Origins (latitude, longitude in degrees) of the Japan Plane Rectangular
/// coordinate systems I–XIX (国土交通省告示第九号).
const JAPAN_PLANE_ORIGINS: [(f64, f64); 19] = [
    (33.0, 129.5),
    (33.0, 131.0),
    (36.0, 132.0 + 1.0 / 6.0),
    (33.0, 133.5),
    (36.0, 134.0 + 1.0 / 3.0),
    (36.0, 136.0),
    (36.0, 137.0 + 1.0 / 6.0),
    (36.0, 138.5),
    (36.0, 139.0 + 5.0 / 6.0),
    (40.0, 140.0 + 5.0 / 6.0),
    (44.0, 140.25),
    (44.0, 142.25),
    (44.0, 144.25),
    (26.0, 142.0),
    (26.0, 127.5),
    (26.0, 124.0),
    (26.0, 131.0),
    (20.0, 136.0),
    (26.0, 154.0),
];

const ROMAN: [&str; 19] = [
    "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII", "XIII", "XIV", "XV",
    "XVI", "XVII", "XVIII", "XIX",
];

/// Parse `EPSG:6675`, `epsg:6675`, `6675`, or an OGC CRS URN/URL.
pub fn parse_epsg(value: &str) -> Result<u32> {
    let trimmed = value.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.contains("CRS84") {
        return Ok(4326);
    }
    let digits = upper
        .rsplit(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())
        .unwrap_or("");
    let looks_epsg = upper.starts_with("EPSG")
        || upper.contains("/EPSG/")
        || upper.contains(":EPSG:")
        || trimmed.chars().all(|c| c.is_ascii_digit());
    if !looks_epsg || digits.is_empty() {
        return Err(ToolkitError::UnsupportedCrs(value.to_string()));
    }
    digits
        .parse()
        .map_err(|_| ToolkitError::UnsupportedCrs(value.to_string()))
}

/// Look up a supported CRS by identifier.
pub fn lookup(value: &str) -> Result<CrsInfo> {
    lookup_epsg(parse_epsg(value)?)
}

/// Look up a supported CRS by EPSG code.
pub fn lookup_epsg(epsg: u32) -> Result<CrsInfo> {
    let info = |name: String, unit: AxisUnit, projection: Projection| CrsInfo {
        epsg,
        id: format!("EPSG:{epsg}"),
        name,
        unit,
        projection,
    };
    let tm = |lat0: f64, lon0: f64, k0: f64, false_easting: f64, false_northing: f64| {
        Projection::TransverseMercator {
            lat0,
            lon0,
            k0,
            false_easting,
            false_northing,
        }
    };
    match epsg {
        4326 => Ok(info(
            "WGS 84".into(),
            AxisUnit::Degrees,
            Projection::Geographic,
        )),
        4612 => Ok(info(
            "JGD2000".into(),
            AxisUnit::Degrees,
            Projection::Geographic,
        )),
        6668 => Ok(info(
            "JGD2011".into(),
            AxisUnit::Degrees,
            Projection::Geographic,
        )),
        3857 | 900913 => Ok(info(
            "WGS 84 / Pseudo-Mercator".into(),
            AxisUnit::Metres,
            Projection::WebMercator,
        )),
        6669..=6687 | 2443..=2461 => {
            let (base, datum) = if epsg >= 6669 {
                (6669, "JGD2011")
            } else {
                (2443, "JGD2000")
            };
            let zone = (epsg - base) as usize;
            let (lat0, lon0) = JAPAN_PLANE_ORIGINS[zone];
            Ok(info(
                format!("{datum} / Japan Plane Rectangular CS {}", ROMAN[zone]),
                AxisUnit::Metres,
                tm(lat0, lon0, 0.9999, 0.0, 0.0),
            ))
        }
        32601..=32660 => {
            let zone = epsg - 32600;
            Ok(info(
                format!("WGS 84 / UTM zone {zone}N"),
                AxisUnit::Metres,
                tm(0.0, utm_central_meridian(zone), 0.9996, 500_000.0, 0.0),
            ))
        }
        32701..=32760 => {
            let zone = epsg - 32700;
            Ok(info(
                format!("WGS 84 / UTM zone {zone}S"),
                AxisUnit::Metres,
                tm(
                    0.0,
                    utm_central_meridian(zone),
                    0.9996,
                    500_000.0,
                    10_000_000.0,
                ),
            ))
        }
        3097..=3101 | 6688..=6692 => {
            let (base, datum) = if epsg >= 6688 {
                (6688, "JGD2011")
            } else {
                (3097, "JGD2000")
            };
            let zone = 51 + epsg - base;
            Ok(info(
                format!("{datum} / UTM zone {zone}N"),
                AxisUnit::Metres,
                tm(0.0, utm_central_meridian(zone), 0.9996, 500_000.0, 0.0),
            ))
        }
        4301 | 30161..=30179 => Err(ToolkitError::UnsupportedCrs(format!(
            "EPSG:{epsg} (Tokyo datum requires a grid shift that GeneGIS does not apply)"
        ))),
        other => Err(ToolkitError::UnsupportedCrs(format!("EPSG:{other}"))),
    }
}

fn utm_central_meridian(zone: u32) -> f64 {
    zone as f64 * 6.0 - 183.0
}

/// Choose a metric working CRS (UTM) for data centred at `lon`, `lat`.
pub fn metric_crs_for(lon: f64, lat: f64) -> CrsInfo {
    let zone = (((lon + 180.0) / 6.0).floor() as i64).clamp(0, 59) as u32 + 1;
    let epsg = if lat >= 0.0 {
        32600 + zone
    } else {
        32700 + zone
    };
    lookup_epsg(epsg).expect("UTM zones are registered")
}

/// Transform a single coordinate between two supported CRSs.
pub fn transform_coord(from: &CrsInfo, to: &CrsInfo, x: f64, y: f64) -> Result<(f64, f64)> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ToolkitError::InvalidCoordinate(format!("({x}, {y})")));
    }
    if from.epsg == to.epsg {
        return Ok((x, y));
    }
    let (lon, lat) = to_geographic(from, x, y)?;
    from_geographic(to, lon, lat)
}

/// Transform every coordinate of a geometry.
pub fn transform_geometry(
    from: &CrsInfo,
    to: &CrsInfo,
    geometry: &Geometry<f64>,
) -> Result<Geometry<f64>> {
    if from.epsg == to.epsg {
        return Ok(geometry.clone());
    }
    geometry.try_map_coords(|Coord { x, y }| {
        transform_coord(from, to, x, y).map(|(x, y)| Coord { x, y })
    })
}

/// Convert CRS coordinates to longitude/latitude degrees.
pub fn to_geographic(crs: &CrsInfo, x: f64, y: f64) -> Result<(f64, f64)> {
    match crs.projection {
        Projection::Geographic => {
            validate_lon_lat(x, y)?;
            Ok((x, y))
        }
        Projection::WebMercator => {
            let lon = (x / GRS80_A).to_degrees();
            let lat = (2.0 * (y / GRS80_A).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
            Ok((lon, lat))
        }
        Projection::TransverseMercator {
            lat0,
            lon0,
            k0,
            false_easting,
            false_northing,
        } => Ok(tm_inverse(
            x,
            y,
            lat0,
            lon0,
            k0,
            false_easting,
            false_northing,
        )),
    }
}

/// Convert longitude/latitude degrees to CRS coordinates.
pub fn from_geographic(crs: &CrsInfo, lon: f64, lat: f64) -> Result<(f64, f64)> {
    validate_lon_lat(lon, lat)?;
    match crs.projection {
        Projection::Geographic => Ok((lon, lat)),
        Projection::WebMercator => {
            let lat = lat.clamp(-85.051_128_78, 85.051_128_78);
            let x = GRS80_A * lon.to_radians();
            let y = GRS80_A
                * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0)
                    .tan()
                    .ln();
            Ok((x, y))
        }
        Projection::TransverseMercator {
            lat0,
            lon0,
            k0,
            false_easting,
            false_northing,
        } => Ok(tm_forward(
            lon,
            lat,
            lat0,
            lon0,
            k0,
            false_easting,
            false_northing,
        )),
    }
}

fn validate_lon_lat(lon: f64, lat: f64) -> Result<()> {
    if !lon.is_finite()
        || !lat.is_finite()
        || !(-180.0..=180.0).contains(&lon)
        || !(-90.0..=90.0).contains(&lat)
    {
        return Err(ToolkitError::InvalidCoordinate(format!(
            "longitude/latitude ({lon}, {lat}) is outside the geographic domain"
        )));
    }
    Ok(())
}

struct KruegerSeries {
    a_hat: f64,
    alpha: [f64; 4],
    beta: [f64; 4],
    delta: [f64; 4],
    e2n: f64,
}

fn krueger() -> KruegerSeries {
    let n = GRS80_F / (2.0 - GRS80_F);
    let n2 = n * n;
    let n3 = n2 * n;
    let n4 = n3 * n;
    KruegerSeries {
        a_hat: GRS80_A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0),
        alpha: [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0,
            49561.0 * n4 / 161280.0,
        ],
        beta: [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0,
            4397.0 * n4 / 161280.0,
        ],
        delta: [
            2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3 + 116.0 * n4 / 45.0,
            7.0 * n2 / 3.0 - 8.0 * n3 / 5.0 - 227.0 * n4 / 45.0,
            56.0 * n3 / 15.0 - 136.0 * n4 / 35.0,
            4279.0 * n4 / 630.0,
        ],
        e2n: 2.0 * n.sqrt() / (1.0 + n),
    }
}

fn conformal_t(series: &KruegerSeries, lat: f64) -> f64 {
    let s = lat.sin();
    (s.atanh() - series.e2n * (series.e2n * s).atanh()).sinh()
}

/// Meridian arc (scaled by `a_hat`) from the equator to `lat` (radians).
fn meridian_arc(series: &KruegerSeries, lat: f64) -> f64 {
    let xi = conformal_t(series, lat).atan();
    let mut sum = xi;
    for (j, alpha) in series.alpha.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        sum += alpha * (k * xi).sin();
    }
    series.a_hat * sum
}

fn tm_forward(
    lon: f64,
    lat: f64,
    lat0: f64,
    lon0: f64,
    k0: f64,
    false_easting: f64,
    false_northing: f64,
) -> (f64, f64) {
    let series = krueger();
    let phi = lat.to_radians();
    let dlambda = (lon - lon0).to_radians();
    let t = conformal_t(&series, phi);
    let xi_p = t.atan2(dlambda.cos());
    let eta_p = (dlambda.sin() / (1.0 + t * t).sqrt()).atanh();
    let mut xi = xi_p;
    let mut eta = eta_p;
    for (j, alpha) in series.alpha.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi += alpha * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += alpha * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    let origin = meridian_arc(&series, lat0.to_radians());
    let x = false_easting + k0 * series.a_hat * eta;
    let y = false_northing + k0 * (series.a_hat * xi - origin);
    (x, y)
}

fn tm_inverse(
    x: f64,
    y: f64,
    lat0: f64,
    lon0: f64,
    k0: f64,
    false_easting: f64,
    false_northing: f64,
) -> (f64, f64) {
    let series = krueger();
    let origin = meridian_arc(&series, lat0.to_radians());
    let xi = ((y - false_northing) / k0 + origin) / series.a_hat;
    let eta = (x - false_easting) / (k0 * series.a_hat);
    let mut xi_p = xi;
    let mut eta_p = eta;
    for (j, beta) in series.beta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi_p -= beta * (k * xi).sin() * (k * eta).cosh();
        eta_p -= beta * (k * xi).cos() * (k * eta).sinh();
    }
    let chi = (xi_p.sin() / eta_p.cosh()).asin();
    let mut phi = chi;
    for (j, delta) in series.delta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        phi += delta * (k * chi).sin();
    }
    let lambda = eta_p.sinh().atan2(xi_p.cos());
    (lon0 + lambda.to_degrees(), phi.to_degrees())
}

/// OGC WKT1 definition of a supported CRS (for GeoPackage and `.prj` output).
pub fn to_wkt(crs: &CrsInfo) -> String {
    let geographic = |epsg: u32| -> String {
        let (name, datum, spheroid, flattening) = match epsg {
            6668 => (
                "JGD2011",
                "Japanese_Geodetic_Datum_2011",
                "GRS 1980",
                "298.257222101",
            ),
            4612 => (
                "JGD2000",
                "Japanese_Geodetic_Datum_2000",
                "GRS 1980",
                "298.257222101",
            ),
            _ => ("WGS 84", "WGS_1984", "WGS 84", "298.257223563"),
        };
        format!(
            "GEOGCS[\"{name}\",DATUM[\"{datum}\",SPHEROID[\"{spheroid}\",6378137,{flattening}]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433],AUTHORITY[\"EPSG\",\"{epsg}\"]]"
        )
    };
    let base = match crs.epsg {
        6669..=6692 => 6668,
        2443..=2461 | 3097..=3101 => 4612,
        _ => 4326,
    };
    match crs.projection {
        Projection::Geographic => geographic(crs.epsg),
        Projection::WebMercator => format!(
            "PROJCS[\"{}\",{},PROJECTION[\"Mercator_1SP\"],PARAMETER[\"central_meridian\",0],PARAMETER[\"scale_factor\",1],PARAMETER[\"false_easting\",0],PARAMETER[\"false_northing\",0],UNIT[\"metre\",1],EXTENSION[\"PROJ4\",\"+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +no_defs\"],AUTHORITY[\"EPSG\",\"{}\"]]",
            crs.name,
            geographic(4326),
            crs.epsg
        ),
        Projection::TransverseMercator {
            lat0,
            lon0,
            k0,
            false_easting,
            false_northing,
        } => format!(
            "PROJCS[\"{}\",{},PROJECTION[\"Transverse_Mercator\"],PARAMETER[\"latitude_of_origin\",{lat0}],PARAMETER[\"central_meridian\",{lon0}],PARAMETER[\"scale_factor\",{k0}],PARAMETER[\"false_easting\",{false_easting}],PARAMETER[\"false_northing\",{false_northing}],UNIT[\"metre\",1],AUTHORITY[\"EPSG\",\"{}\"]]",
            crs.name,
            geographic(base),
            crs.epsg
        ),
    }
}

/// Map a WKT/ESRI `.prj` definition to a supported EPSG code.
pub fn epsg_from_wkt(wkt: &str) -> Option<u32> {
    let upper = wkt.to_ascii_uppercase();
    // An explicit top-level authority wins. The last AUTHORITY/ID in WKT1/WKT2
    // is the one attached to the outermost CRS.
    let mut explicit = None;
    for marker in ["AUTHORITY[\"EPSG\",", "ID[\"EPSG\","] {
        if let Some(index) = upper.rfind(marker) {
            let rest = &upper[index + marker.len()..];
            let digits: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            explicit = digits.parse().ok();
        }
    }
    if let Some(epsg) = explicit {
        return Some(epsg);
    }
    let name: String = upper
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let projected =
        upper.trim_start().starts_with("PROJCS") || upper.trim_start().starts_with("PROJCRS");
    if projected {
        for (zone, roman) in ROMAN.iter().enumerate() {
            let arabic = zone + 1;
            let plane = [
                format!("JAPAN_ZONE_{arabic}_"),
                format!("JAPAN_PLANE_RECTANGULAR_CS_{roman}_"),
                format!("JAPAN_PLANE_RECTANGULAR_CS{roman}_"),
            ];
            if plane.iter().any(|needle| name.contains(needle.as_str())) {
                if name.contains("2011") {
                    return Some(6669 + zone as u32);
                }
                if name.contains("2000") {
                    return Some(2443 + zone as u32);
                }
            }
        }
        if name.contains("WEB_MERCATOR") || name.contains("PSEUDO_MERCATOR") {
            return Some(3857);
        }
        if let Some(index) = name.find("UTM_ZONE_") {
            let rest = &name[index + "UTM_ZONE_".len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let hemisphere = rest.chars().nth(digits.len());
            if let (Ok(zone), Some(hemisphere)) = (digits.parse::<u32>(), hemisphere) {
                if (1..=60).contains(&zone) {
                    let north = hemisphere == 'N';
                    if name.contains("JGD_2011") || name.contains("JGD2011") {
                        return (51..=55).contains(&zone).then_some(6688 + zone - 51);
                    }
                    if name.contains("JGD_2000") || name.contains("JGD2000") {
                        return (51..=55).contains(&zone).then_some(3097 + zone - 51);
                    }
                    return Some(if north { 32600 + zone } else { 32700 + zone });
                }
            }
        }
        return None;
    }
    if name.contains("JGD_2011") || name.contains("JGD2011") {
        return Some(6668);
    }
    if name.contains("JGD_2000") || name.contains("JGD2000") {
        return Some(4612);
    }
    if name.contains("TOKYO") {
        return Some(4301);
    }
    if name.contains("WGS_1984") || name.contains("WGS_84") || name.contains("WGS84") {
        return Some(4326);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Distance, Geodesic, Point};

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn plane_rectangular_origin_maps_to_zero() {
        let zone7 = lookup("EPSG:6675").unwrap();
        let (x, y) = from_geographic(&zone7, 137.0 + 10.0 / 60.0, 36.0).unwrap();
        assert!(close(x, 0.0, 1e-6) && close(y, 0.0, 1e-6), "({x}, {y})");
    }

    #[test]
    fn utm_equator_on_central_meridian() {
        let utm31 = lookup("EPSG:32631").unwrap();
        let (x, y) = from_geographic(&utm31, 3.0, 0.0).unwrap();
        assert!(close(x, 500_000.0, 1e-6) && close(y, 0.0, 1e-6));
    }

    #[test]
    fn round_trip_is_millimetre_accurate() {
        let wgs84 = lookup("EPSG:4326").unwrap();
        for code in [
            "EPSG:6675",
            "EPSG:32654",
            "EPSG:3857",
            "EPSG:2449",
            "EPSG:6691",
        ] {
            let target = lookup(code).unwrap();
            let (x, y) = transform_coord(&wgs84, &target, 136.9066, 35.1815).unwrap();
            let (lon, lat) = transform_coord(&target, &wgs84, x, y).unwrap();
            assert!(
                close(lon, 136.9066, 1e-8) && close(lat, 35.1815, 1e-8),
                "{code}"
            );
        }
    }

    #[test]
    fn projected_distance_matches_geodesic_distance() {
        // Independent check: near the zone VII central meridian the projected
        // distance must equal the geodesic distance times the scale factor.
        let zone7 = lookup("EPSG:6675").unwrap();
        let a = (137.1667, 35.17);
        let b = (137.1667, 35.18);
        let pa = from_geographic(&zone7, a.0, a.1).unwrap();
        let pb = from_geographic(&zone7, b.0, b.1).unwrap();
        let projected = ((pa.0 - pb.0).powi(2) + (pa.1 - pb.1).powi(2)).sqrt();
        let geodesic = Geodesic.distance(Point::new(a.0, a.1), Point::new(b.0, b.1));
        let scale = projected / geodesic;
        assert!(close(scale, 0.9999, 2e-5), "scale={scale}");
    }

    #[test]
    fn nagoya_station_matches_published_zone_vii_coordinates() {
        // 名古屋駅付近 (136.8815E, 35.1709N) lies ~26 km west and ~92 km south
        // of the zone VII origin (137°10'E, 36°N).
        let zone7 = lookup("EPSG:6675").unwrap();
        let (x, y) = from_geographic(&zone7, 136.8815, 35.1709).unwrap();
        assert!((-27_000.0..-25_000.0).contains(&x), "x={x}");
        assert!((-93_000.0..-91_000.0).contains(&y), "y={y}");
    }

    #[test]
    fn prj_names_resolve_to_epsg() {
        assert_eq!(
            epsg_from_wkt(r#"PROJCS["JGD_2011_Japan_Zone_7",GEOGCS["GCS_JGD_2011"]]"#),
            Some(6675)
        );
        assert_eq!(
            epsg_from_wkt(
                r#"PROJCS["JGD2000 / Japan Plane Rectangular CS VII",GEOGCS["JGD2000"]]"#
            ),
            Some(2449)
        );
        assert_eq!(
            epsg_from_wkt(r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984"]]"#),
            Some(4326)
        );
        assert_eq!(
            epsg_from_wkt(r#"PROJCS["WGS_1984_UTM_Zone_54N",GEOGCS["GCS_WGS_1984"]]"#),
            Some(32654)
        );
        assert_eq!(epsg_from_wkt(r#"GEOGCS["GCS_Tokyo"]"#), Some(4301));
        assert!(lookup_epsg(4301).is_err(), "Tokyo datum must fail closed");
    }

    #[test]
    fn unknown_codes_fail_closed() {
        assert!(lookup("EPSG:27700").is_err());
        assert!(lookup("not a crs").is_err());
        assert_eq!(parse_epsg("urn:ogc:def:crs:EPSG::6675").unwrap(), 6675);
        assert_eq!(parse_epsg("urn:ogc:def:crs:OGC:1.3:CRS84").unwrap(), 4326);
    }
}
