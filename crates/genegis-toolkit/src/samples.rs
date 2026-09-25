//! Bundled Nagoya sample layers used by the Workbench, the MCP server, and
//! the planner evaluation.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::execute::{import_through_workflow, ImportReceipt};
use crate::import::ImportOptions;
use crate::layer::Layer;

/// Approximate locations of major stations (sample data, CC0).
pub const STATIONS_CSV: &str = "駅名,路線,lon,lat\n名古屋,JR・名鉄・近鉄・地下鉄,136.8815,35.1709\n栄,地下鉄東山線・名城線,136.9086,35.1681\n金山,JR・名鉄・地下鉄,136.9006,35.1430\n大曽根,JR・名鉄・地下鉄,136.9360,35.1910\n千種,JR・地下鉄,136.9312,35.1702\n今池,地下鉄,136.9340,35.1660\n本山,地下鉄,136.9637,35.1631\n八事,地下鉄,136.9650,35.1370\n";

/// Sample files: (key, file, display name, attribution, license).
pub const SAMPLE_FILES: [(&str, &str, &str, &str, &str); 4] = [
    (
        "wards",
        "nagoya-wards.geojson",
        "名古屋市 区界と人口（令和2年国勢調査）",
        "国土数値情報 N03 / 名古屋市 令和2年国勢調査",
        "政府標準利用規約 2.0",
    ),
    (
        "shelters",
        "nagoya-shelters.geojson",
        "避難所（サンプル）",
        "GeneGIS 合成サンプル",
        "CC0-1.0",
    ),
    (
        "pois",
        "nagoya-pois.geojson",
        "店舗・施設（サンプル）",
        "GeneGIS 合成サンプル",
        "CC0-1.0",
    ),
    (
        "flood",
        "nagoya-flood-zones.geojson",
        "浸水想定区域（サンプル）",
        "GeneGIS 合成サンプル（MLIT A31a 参考）",
        "CC0-1.0",
    ),
];

/// Locate `examples/nagoya-population-density/data` from `GENEGIS_SAMPLES_DIR`,
/// the working directory, or the source tree.
pub fn samples_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("GENEGIS_SAMPLES_DIR") {
        return PathBuf::from(dir);
    }
    let relative = Path::new("examples/nagoya-population-density/data");
    if relative.is_dir() {
        return relative.to_path_buf();
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/nagoya-population-density/data")
}

/// Import every sample layer through Command + Workflow Graph. Returns
/// `(key, layer, receipt)` with keys `wards`, `shelters`, `pois`, `flood`,
/// `stations`.
pub fn load_nagoya_samples(dir: &Path) -> Result<Vec<(&'static str, Layer, ImportReceipt)>> {
    let mut loaded = Vec::new();
    for (key, file, name, attribution, license) in SAMPLE_FILES {
        let bytes = std::fs::read(dir.join(file))?;
        let options = ImportOptions {
            name: Some(name.into()),
            attribution: Some(attribution.into()),
            license: Some(license.into()),
            ..Default::default()
        };
        let (layer, receipt) = import_through_workflow(file, &bytes, &options)?;
        loaded.push((key, layer, receipt));
    }
    let options = ImportOptions {
        name: Some("主要駅（サンプル）".into()),
        crs: Some("EPSG:4326".into()),
        attribution: Some("GeneGIS サンプル（概略位置）".into()),
        license: Some("CC0-1.0".into()),
        ..Default::default()
    };
    let (layer, receipt) =
        import_through_workflow("stations.csv", STATIONS_CSV.as_bytes(), &options)?;
    loaded.push(("stations", layer, receipt));
    Ok(loaded)
}
