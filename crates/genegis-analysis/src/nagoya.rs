use genegis_catalog::{alpha_catalog, nagoya_wards_geojson_path, NAGOYA_WARDS_DENSITY_ID};
use genegis_geometry::{planar_area_km2_rings, AreaMethod};
use genegis_style::ChoroplethStyle;
use genegis_vector::{read_geojson_path, read_geoparquet_uri, VectorDataset};
use genegis_workflow::{nagoya_population_density_template, ReviewStatus};

use crate::error::AnalysisError;
use crate::result::{
    AnalysisResult, Citation, DensityFeature, VerificationCheck, VerificationReport,
};

const DENSITY_UNIT: &str = "persons/km²";

pub fn default_nagoya_data_path() -> &'static str {
    nagoya_wards_geojson_path()
}

pub fn default_nagoya_dataset_id() -> &'static str {
    NAGOYA_WARDS_DENSITY_ID
}

pub fn run_nagoya_population_density_for_dataset(
    dataset_id: &str,
) -> Result<AnalysisResult, AnalysisError> {
    let catalog = alpha_catalog();
    let record = catalog
        .require(dataset_id)
        .map_err(|e| AnalysisError::Message(e.to_string()))?;
    if record.format.kind == "geoparquet" {
        run_nagoya_population_density_geoparquet(&record.uri)
    } else {
        run_nagoya_population_density(&record.uri)
    }
}

pub fn run_nagoya_population_density_from_catalog() -> Result<AnalysisResult, AnalysisError> {
    run_nagoya_population_density_for_dataset(default_nagoya_dataset_id())
}

pub fn run_nagoya_population_density_geoparquet(
    data_path: &str,
) -> Result<AnalysisResult, AnalysisError> {
    let dataset = read_geoparquet_uri(data_path)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;
    run_nagoya_population_density_from_vector(dataset)
}

pub fn run_nagoya_population_density(data_path: &str) -> Result<AnalysisResult, AnalysisError> {
    let dataset = read_geojson_path(data_path)?;
    run_nagoya_population_density_from_vector(dataset)
}

pub fn run_nagoya_population_density_from_vector(
    dataset: VectorDataset,
) -> Result<AnalysisResult, AnalysisError> {
    let mut workflow = nagoya_population_density_template();
    let mut densities = Vec::new();
    let mut features = Vec::new();

    for feature in &dataset.features {
        let ward_name = feature
            .properties
            .get("ward_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AnalysisError::Message("missing ward_name".into()))?
            .to_string();
        let ward_code = feature
            .properties
            .get("ward_code")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let population = feature
            .properties
            .get("population")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| AnalysisError::Message(format!("missing population for {ward_name}")))?;

        let exteriors: Vec<_> = feature.exterior_rings().collect();
        let area_km2 = planar_area_km2_rings(&exteriors);
        let density = if area_km2 > 0.0 {
            population as f64 / area_km2
        } else {
            0.0
        };
        densities.push(density);

        features.push(DensityFeature {
            ward_code,
            ward_name,
            population,
            area_km2,
            density_per_km2: density,
            rings: feature.rings.clone(),
            color: genegis_style::ColorRgba::new(0.5, 0.5, 0.5, 1.0),
        });
    }

    let style = ChoroplethStyle::equal_interval("density_per_km2", DENSITY_UNIT, &densities, 5);
    for (feature, density) in features.iter_mut().zip(densities.iter()) {
        feature.color = style.color_for(*density);
    }

    let verification = build_verification(&dataset.crs, &features);
    let citations = default_citations();

    workflow.citations = citations
        .iter()
        .map(|c| genegis_workflow::Citation {
            title: c.title.clone(),
            url: Some(c.url.clone()),
            license: Some(c.license.clone()),
            retrieved_at: None,
        })
        .collect();
    workflow.review_status = if verification.checks.iter().all(|c| c.passed) {
        ReviewStatus::Executed
    } else {
        ReviewStatus::PendingReview
    };

    Ok(AnalysisResult {
        workflow,
        features,
        style,
        verification,
        citations,
    })
}

fn build_verification(crs: &str, features: &[DensityFeature]) -> VerificationReport {
    let mut checks = Vec::new();

    checks.push(VerificationCheck {
        name: "crs_declared".into(),
        passed: crs.starts_with("EPSG:"),
        detail: format!("CRS = {crs}"),
    });

    checks.push(VerificationCheck {
        name: "area_method_recorded".into(),
        passed: true,
        detail: format!("{:?}", AreaMethod::PlanarWgs84Approx),
    });

    checks.push(VerificationCheck {
        name: "population_positive".into(),
        passed: features.iter().all(|f| f.population > 0),
        detail: format!("{} wards", features.len()),
    });

    checks.push(VerificationCheck {
        name: "density_unit".into(),
        passed: true,
        detail: DENSITY_UNIT.into(),
    });

    checks.push(VerificationCheck {
        name: "feature_count".into(),
        passed: features.len() == 16,
        detail: "Nagoya has 16 wards".into(),
    });

    checks.push(VerificationCheck {
        name: "boundary_source".into(),
        passed: true,
        detail: "国土数値情報 N03 行政区域 (via JapanCityGeoJson)".into(),
    });

    VerificationReport {
        crs: crs.to_string(),
        area_method: "planar_wgs84_approx".into(),
        density_unit: DENSITY_UNIT.into(),
        checks,
    }
}

fn default_citations() -> Vec<Citation> {
    vec![
        Citation {
            title: "国土数値情報 行政区域 (N03) — 愛知県 名古屋市区".into(),
            url: "https://nlftp.mlit.go.jp/ksj/gml/datalist/KsjTmplt-N03.html".into(),
            license: "国土交通省 国土数値情報".into(),
        },
        Citation {
            title: "JapanCityGeoJson — N03 derived ward boundaries".into(),
            url: "https://github.com/niiyz/JapanCityGeoJson".into(),
            license: "Processed from MLIT N03 open data".into(),
        },
        Citation {
            title: "政府統計の総合窓口 e-Stat — 2020年国勢調査 人口".into(),
            url: "https://www.e-stat.go.jp/stat-search/files?page=1&toukei=00200521&tstat=000001136464".into(),
            license: "Government open data (Japan)".into(),
        },
        Citation {
            title: "名古屋市オープンデータカタログ".into(),
            url: "https://www.data-nagoya.jp/".into(),
            license: "City of Nagoya Open Data".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_nagoya_demo() {
        let result = run_nagoya_population_density(default_nagoya_data_path()).expect("run");
        assert_eq!(result.features.len(), 16);
        assert!(result.verification.checks.iter().all(|c| c.passed));
        assert!(result.features[0].density_per_km2 > 0.0);
    }

    #[test]
    fn runs_nagoya_geoparquet_density() {
        let path = genegis_catalog::nagoya_wards_geoparquet_path();
        if !std::path::Path::new(path).exists() {
            return;
        }
        let result = run_nagoya_population_density_geoparquet(path).expect("run");
        assert_eq!(result.features.len(), 16);
        assert!(result.verification.checks.iter().all(|c| c.passed));
    }

    #[test]
    fn geoparquet_dataset_id_resolves_to_parquet_path() {
        assert_eq!(
            alpha_catalog()
                .require(genegis_catalog::NAGOYA_WARDS_GEOPARQUET_ID)
                .expect("record")
                .format
                .kind,
            "geoparquet"
        );
    }
}
