use genegis_adapter::{
    grass_manifest, CapabilityPolicy, GrassAdapter, GrassError, GrassOperation, GRASS_IMAGE_DIGEST,
};
use serde_json::json;

#[test]
#[ignore = "requires Docker and the pinned real GRASS image"]
fn real_grass_six_positive_and_five_negative_cases() {
    let adapter = GrassAdapter::default();
    let positives = [
        GrassOperation::RegionGrid {
            north: 1000.0,
            south: 0.0,
            east: 1000.0,
            west: 0.0,
            resolution: 10.0,
            srid: 6675,
        },
        GrassOperation::ProjectionInfo { srid: 6675 },
        GrassOperation::TransformPoint {
            longitude: 136.9066,
            latitude: 35.1815,
            source_srid: 4326,
            target_srid: 6675,
        },
        GrassOperation::GeodesicDistance {
            start_longitude: 136.9,
            start_latitude: 35.1,
            end_longitude: 136.91,
            end_latitude: 35.11,
        },
        GrassOperation::ConstantRasterStats {
            rows: 10,
            cols: 10,
            value: 7,
        },
        GrassOperation::SeededPointCount {
            points: 5,
            seed: 42,
        },
    ];
    let receipts = positives
        .iter()
        .map(|operation| adapter.execute(operation).expect("typed GRASS operation"))
        .collect::<Vec<_>>();

    assert_eq!(receipts[0].output["rows"], 100);
    assert_eq!(receipts[0].output["cols"], 100);
    assert_eq!(receipts[0].output["cells"], 10_000);
    assert_eq!(receipts[1].output["srid"], "EPSG:6675");
    assert!((receipts[2].output["x"].as_f64().unwrap() + 23_686.126_611_95).abs() < 1e-8);
    assert!((receipts[2].output["y"].as_f64().unwrap() + 90_773.698_643_82).abs() < 1e-8);
    assert!((receipts[3].output["length"].as_f64().unwrap() - 1_435.982_679).abs() < 1e-6);
    assert_eq!(receipts[4].output["cells"], 100);
    assert_eq!(receipts[4].output["mean"], 7);
    assert_eq!(receipts[4].output["sum"], 700);
    assert_eq!(receipts[5].output["points"], 5);
    assert_eq!(receipts[5].output["primitives"], 5);
    assert!(receipts.iter().all(|receipt| {
        receipt.output_digest.starts_with("sha256:")
            && receipt.sandbox.network == "none"
            && receipt.sandbox.read_only_root
            && receipt.sandbox.user == "65534:65534"
            && receipt.sandbox.all_capabilities_dropped
            && receipt.sandbox.no_new_privileges
            && receipt.backend.build_digest == GRASS_IMAGE_DIGEST
    }));

    let wrong_crs = adapter.execute(&GrassOperation::TransformPoint {
        longitude: 136.9,
        latitude: 35.1,
        source_srid: 3857,
        target_srid: 6675,
    });
    assert!(matches!(wrong_crs, Err(GrassError::Contract(_))));

    let invalid_coordinate = adapter.execute(&GrassOperation::GeodesicDistance {
        start_longitude: 181.0,
        start_latitude: 35.1,
        end_longitude: 136.91,
        end_latitude: 35.11,
    });
    assert!(matches!(invalid_coordinate, Err(GrassError::Contract(_))));

    let resource_exhaustion = adapter.execute(&GrassOperation::ConstantRasterStats {
        rows: 10_000,
        cols: 10_000,
        value: 7,
    });
    assert!(matches!(resource_exhaustion, Err(GrassError::Contract(_))));

    let mut denied_policy = CapabilityPolicy::sandboxed_process("different-adapter");
    denied_policy.accepted_adapters.clear();
    let denied = GrassAdapter::new(grass_manifest(), denied_policy)
        .execute(&GrassOperation::ProjectionInfo { srid: 6675 });
    assert!(matches!(denied, Err(GrassError::Admission(_))));

    let mut drifted_manifest = grass_manifest();
    drifted_manifest.backend.build_digest =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    let drifted_policy = CapabilityPolicy::sandboxed_process(&drifted_manifest.adapter_id);
    let drifted = GrassAdapter::new(drifted_manifest, drifted_policy)
        .execute(&GrassOperation::ProjectionInfo { srid: 6675 });
    assert!(matches!(drifted, Err(GrassError::RuntimeIdentity(_))));

    let evidence = json!({
        "schema_version": "0.1.0",
        "image_digest": GRASS_IMAGE_DIGEST,
        "positive_cases": 6,
        "negative_cases": 5,
        "false_accepts": 0,
        "receipts": receipts
    });
    println!("{}", serde_json::to_string(&evidence).unwrap());
}
