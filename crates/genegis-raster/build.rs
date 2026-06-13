use std::path::PathBuf;

use geotiff_writer::{CogBuilder, GeoTiffBuilder};
use ndarray::Array2;

fn main() {
    let path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest"))
        .join("fixtures/smoke-demo.tif");

    if !path.is_file() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("fixtures dir");
        }
        write_smoke_cog(&path);
    }

    println!("cargo:rerun-if-changed=build.rs");
}

fn write_smoke_cog(path: &std::path::Path) {
    let width = 64u32;
    let height = 48u32;
    let mut data = Array2::<u8>::zeros((height as usize, width as usize));
    for y in 0..height as usize {
        for x in 0..width as usize {
            data[[y, x]] = ((x + y) % 256) as u8;
        }
    }

    let builder = GeoTiffBuilder::new(width, height)
        .epsg(4326)
        .pixel_scale(0.01, 0.01)
        .origin(-180.0, 90.0);

    CogBuilder::new(builder)
        .no_overviews()
        .write_2d(path, data.view())
        .expect("write smoke-demo.tif fixture");
}
