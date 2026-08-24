//! Generate the UC-3 NDVI time-series fixtures (two Sentinel-2-like epochs).
//!
//! Writes four single-band u8 COGs over the Nagoya bbox:
//!   examples/nagoya-population-density/data/nagoya-sentinel-{red,nir}-2025-{04,10}.tif
//!
//! The scene is a pure function of this binary: an urban-core NDVI gradient
//! plus deterministic texture; epoch B adds a deforestation ellipse east of
//! the core and a coastal park gain. DN values are reflectance fractions ×
//! 255 on a fixed total (red+nir = 230) so NDVI inverts exactly.
//!
//! Usage: cargo run -p genegis-raster --example write_ndvi_fixture

use geotiff_writer::{CogBuilder, GeoTiffBuilder};
use ndarray::Array2;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 72;
const LON0: f64 = 136.79;
const LAT0: f64 = 35.03;
const LON1: f64 = 137.07;
const LAT1: f64 = 35.27;
const BAND_TOTAL: f64 = 230.0;

#[derive(Clone, Copy, PartialEq)]
enum Epoch {
    April,
    October,
}

fn main() {
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nagoya-population-density/data");
    std::fs::create_dir_all(&data_dir).expect("data dir");

    for (epoch, tag) in [(Epoch::April, "2025-04"), (Epoch::October, "2025-10")] {
        let mut red = Array2::<u8>::zeros((HEIGHT as usize, WIDTH as usize));
        let mut nir = Array2::<u8>::zeros((HEIGHT as usize, WIDTH as usize));
        for row in 0..HEIGHT as usize {
            for col in 0..WIDTH as usize {
                let lon = LON0 + (col as f64 + 0.5) * (LON1 - LON0) / WIDTH as f64;
                let lat = LAT1 - (row as f64 + 0.5) * (LAT1 - LAT0) / HEIGHT as f64;
                let ndvi = ndvi_at(lon, lat, epoch);
                let r = ((1.0 - ndvi) / 2.0 * BAND_TOTAL).round().clamp(0.0, 255.0);
                let n = (BAND_TOTAL - r).clamp(0.0, 255.0);
                red[[row, col]] = r as u8;
                nir[[row, col]] = n as u8;
            }
        }
        write_cog(
            &data_dir.join(format!("nagoya-sentinel-red-{tag}.tif")),
            red,
        );
        write_cog(
            &data_dir.join(format!("nagoya-sentinel-nir-{tag}.tif")),
            nir,
        );
        println!("wrote epoch {tag}");
    }
}

fn write_cog(path: &std::path::Path, data: Array2<u8>) {
    let dx = (LON1 - LON0) / WIDTH as f64;
    let dy = (LAT1 - LAT0) / HEIGHT as f64;
    let builder = GeoTiffBuilder::new(WIDTH, HEIGHT)
        .epsg(4326)
        .pixel_scale(dx, dy)
        .origin(LON0, LAT1);
    CogBuilder::new(builder)
        .no_overviews()
        .write_2d(path, data.view())
        .expect("write NDVI COG");
}

/// Urban-core NDVI gradient with deterministic texture and per-epoch edits.
fn ndvi_at(lon: f64, lat: f64, epoch: Epoch) -> f64 {
    let dlon = lon - 136.905;
    let dlat = lat - 35.170;
    let urban = (-((dlon * dlon + dlat * dlat) / (2.0 * 0.020_f64 * 0.020))).exp();
    let mut ndvi = 0.78 - urban * 0.60;

    // Checkerboard texture (~pixel-block scale) keeps zonal means non-flat.
    let parity = (((lon * 400.0).floor() as i64) + ((lat * 400.0).floor() as i64)).rem_euclid(2);
    ndvi += if parity == 0 { 0.02 } else { -0.02 };

    if epoch == Epoch::October {
        // Deforestation ellipse east of the urban core.
        let ex = (lon - 136.995) / 0.030;
        let ey = (lat - 35.130) / 0.020;
        let inside_loss = ex * ex + ey * ey < 1.0;
        if inside_loss {
            ndvi -= 0.24;
        }
        // Coastal park gain near 港区.
        let px = (lon - 136.855) / 0.026;
        let py = (lat - 35.075) / 0.019;
        let inside_gain = px * px + py * py < 1.0;
        if inside_gain {
            ndvi += 0.14;
        }
    }

    ndvi.clamp(-0.05, 0.90)
}
