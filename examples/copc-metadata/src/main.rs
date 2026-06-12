//! COPC metadata smoke example — read local fixture or HTTP(S) URI.

use genegis_pointcloud::read_copc_uri;
use std::env;
use std::process;

fn default_fixture_uri() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/genegis-pointcloud/testdata/lone-star.copc.laz"
    )
}

fn main() {
    let uri = env::args()
        .nth(1)
        .unwrap_or_else(|| default_fixture_uri().to_string());

    match read_copc_uri(&uri) {
        Ok(info) => {
            println!("{}", serde_json::to_string_pretty(&info.summary_json()).expect("json"));
            println!();
            println!("COPC metadata read OK");
            println!("  points: {}", info.point_count);
            println!("  hierarchy entries: {}", info.hierarchy_entries);
            println!("  read_mode: {}", info.read_mode.as_deref().unwrap_or("unknown"));
        }
        Err(err) => {
            eprintln!("COPC read failed: {err}");
            eprintln!();
            eprintln!("Usage: cargo run -p copc-metadata [PATH|URL]");
            eprintln!("  default: PDAL lone-star fixture (local)");
            process::exit(1);
        }
    }
}
