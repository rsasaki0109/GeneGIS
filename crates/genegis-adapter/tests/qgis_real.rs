use genegis_adapter::{
    qgis_manifest, CapabilityPolicy, QgisAdapter, QgisError, QgisOperation, QgisRepairMethod,
    QGIS_IMAGE_DIGEST,
};
use serde_json::json;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

struct ArtifactDirectory {
    path: PathBuf,
}

impl ArtifactDirectory {
    fn create(label: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("qgis-real-{label}-{}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale QGIS test directory");
        }
        fs::create_dir_all(&path).expect("create QGIS test directory");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o777))
            .expect("make QGIS output writable by sandbox UID");
        Self {
            path: path.canonicalize().expect("canonical QGIS test directory"),
        }
    }
}

impl Drop for ArtifactDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
#[ignore = "requires Docker and the pinned real QGIS image"]
fn real_qgis_five_positive_and_five_negative_cases() {
    let artifacts = ArtifactDirectory::create("positive");
    let adapter = QgisAdapter::default();

    let grid = adapter
        .execute(
            &QgisOperation::CreateGrid {
                min_x: 0.0,
                max_x: 1_000.0,
                min_y: 0.0,
                max_y: 1_000.0,
                horizontal_spacing: 100.0,
                vertical_spacing: 100.0,
                srid: 6675,
            },
            &artifacts.path,
        )
        .expect("QGIS create grid");
    let grid_path = artifacts.path.join("grid.geojson");

    let buffer = adapter
        .execute(
            &QgisOperation::Buffer {
                input: grid_path.clone(),
                distance: 10.0,
                segments: 8,
            },
            &artifacts.path,
        )
        .expect("QGIS buffer");
    let reproject = adapter
        .execute(
            &QgisOperation::Reproject {
                input: grid_path.clone(),
                target_srid: 4326,
            },
            &artifacts.path,
        )
        .expect("QGIS reproject");
    let centroids = adapter
        .execute(
            &QgisOperation::Centroids {
                input: grid_path,
                all_parts: false,
            },
            &artifacts.path,
        )
        .expect("QGIS centroids");
    let fixed = adapter
        .execute(
            &QgisOperation::FixGeometries {
                input: artifacts.path.join("buffer.geojson"),
                method: QgisRepairMethod::Structure,
            },
            &artifacts.path,
        )
        .expect("QGIS fix geometries");
    let receipts = vec![grid, buffer, reproject, centroids, fixed];

    assert_eq!(receipts[0].output.feature_count, 100);
    assert_eq!(receipts[0].output.geometry_types, ["Polygon"]);
    assert_eq!(receipts[1].output.feature_count, 100);
    assert_eq!(receipts[1].output.geometry_types, ["MultiPolygon"]);
    assert_eq!(receipts[2].output.feature_count, 100);
    assert_eq!(receipts[2].output.geometry_types, ["Polygon"]);
    let reprojected: serde_json::Value = serde_json::from_slice(
        &fs::read(artifacts.path.join("reproject.geojson")).expect("read reprojected artifact"),
    )
    .expect("parse reprojected artifact");
    let first_coordinate = reprojected
        .pointer("/features/0/geometry/coordinates/0/0")
        .and_then(serde_json::Value::as_array)
        .expect("first reprojected coordinate");
    let longitude = first_coordinate[0].as_f64().expect("longitude");
    let latitude = first_coordinate[1].as_f64().expect("latitude");
    // Fixed EPSG operation oracle, independently reproduced by pinned PostGIS:
    // ST_Transform(ST_SetSRID(ST_Point(0, 1000), 6675), 4326).
    assert!((longitude - 137.166_666_666_667).abs() < 1e-12);
    assert!((latitude - 36.009_013_232_373_47).abs() < 1e-12);
    assert_eq!(receipts[3].output.feature_count, 100);
    assert_eq!(receipts[3].output.geometry_types, ["Point"]);
    assert_eq!(receipts[4].output.feature_count, 100);
    assert_eq!(receipts[4].output.geometry_types, ["MultiPolygon"]);
    assert!(receipts.iter().all(|receipt| {
        receipt.output_digest.starts_with("sha256:")
            && receipt.process_report_digest.starts_with("sha256:")
            && receipt.backend.build_digest == QGIS_IMAGE_DIGEST
            && receipt.sandbox.network == "none"
            && receipt.sandbox.read_only_root
            && receipt.sandbox.user == "65534:65534"
            && receipt.sandbox.all_capabilities_dropped
            && receipt.sandbox.no_new_privileges
            && !receipt
                .parameters
                .to_string()
                .contains(&artifacts.path.to_string_lossy()[..])
    }));

    let invalid_budget = adapter.execute(
        &QgisOperation::CreateGrid {
            min_x: 0.0,
            max_x: 1_000_000.0,
            min_y: 0.0,
            max_y: 1_000_000.0,
            horizontal_spacing: 1.0,
            vertical_spacing: 1.0,
            srid: 6675,
        },
        &artifacts.path,
    );
    assert!(matches!(invalid_budget, Err(QgisError::Contract(_))));

    let denied_artifacts = ArtifactDirectory::create("denied");
    let mut denied_policy = CapabilityPolicy::sandboxed_file_process("different-adapter");
    denied_policy.accepted_adapters.clear();
    let denied = QgisAdapter::new(qgis_manifest(), denied_policy).execute(
        &QgisOperation::CreateGrid {
            min_x: 0.0,
            max_x: 100.0,
            min_y: 0.0,
            max_y: 100.0,
            horizontal_spacing: 10.0,
            vertical_spacing: 10.0,
            srid: 6675,
        },
        &denied_artifacts.path,
    );
    assert!(matches!(denied, Err(QgisError::Admission(_))));

    let mut drifted_manifest = qgis_manifest();
    drifted_manifest.backend.build_digest =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    let drifted_policy = CapabilityPolicy::sandboxed_file_process(&drifted_manifest.adapter_id);
    let drifted = QgisAdapter::new(drifted_manifest, drifted_policy).execute(
        &QgisOperation::CreateGrid {
            min_x: 0.0,
            max_x: 100.0,
            min_y: 0.0,
            max_y: 100.0,
            horizontal_spacing: 10.0,
            vertical_spacing: 10.0,
            srid: 6675,
        },
        &denied_artifacts.path,
    );
    assert!(matches!(drifted, Err(QgisError::RuntimeIdentity(_))));

    let wrong_crs_artifacts = ArtifactDirectory::create("wrong-crs");
    let wrong_crs = adapter.execute(
        &QgisOperation::Buffer {
            input: artifacts.path.join("reproject.geojson"),
            distance: 10.0,
            segments: 8,
        },
        &wrong_crs_artifacts.path,
    );
    assert!(matches!(wrong_crs, Err(QgisError::FileBoundary(_))));

    let overwrite = adapter.execute(
        &QgisOperation::Centroids {
            input: artifacts.path.join("grid.geojson"),
            all_parts: false,
        },
        &artifacts.path,
    );
    assert!(matches!(overwrite, Err(QgisError::FileBoundary(_))));

    let evidence = json!({
        "schema_version": "0.1.0",
        "image_digest": QGIS_IMAGE_DIGEST,
        "positive_cases": 5,
        "negative_cases": 5,
        "false_accepts": 0,
        "receipts": receipts
    });
    println!("{}", serde_json::to_string(&evidence).unwrap());
}
