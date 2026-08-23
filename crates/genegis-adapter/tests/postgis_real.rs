use genegis_adapter::{
    postgis_manifest, CapabilityPolicy, PostgisAdapter, PostgisError, PostgisOperation,
    POSTGIS_IMAGE_DIGEST,
};
use postgres::{IsolationLevel, NoTls};
use serde_json::json;
use std::process::Command;
use std::thread;
use std::time::Duration;

struct PostgisContainer {
    name: String,
    port: u16,
}

impl PostgisContainer {
    fn start() -> Self {
        let name = format!("genegis-postgis-real-{}", std::process::id());
        let image = format!("postgis/postgis@{POSTGIS_IMAGE_DIGEST}");
        let output = Command::new("docker")
            .args([
                "run",
                "--rm",
                "-d",
                "--name",
                &name,
                "-p",
                "127.0.0.1::5432",
                "-e",
                "POSTGRES_HOST_AUTH_METHOD=trust",
                &image,
            ])
            .output()
            .expect("start Docker PostGIS fixture");
        assert!(
            output.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut initialized = false;
        for _ in 0..120 {
            let logs = Command::new("docker")
                .args(["logs", &name])
                .output()
                .expect("read PostGIS fixture logs");
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&logs.stdout),
                String::from_utf8_lossy(&logs.stderr)
            );
            if combined.contains("PostgreSQL init process complete; ready for start up.") {
                initialized = true;
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }
        assert!(
            initialized,
            "PostGIS fixture initialization did not finish in 30 seconds"
        );

        let output = Command::new("docker")
            .args(["port", &name, "5432/tcp"])
            .output()
            .expect("resolve mapped PostGIS port");
        assert!(output.status.success(), "docker port failed");
        let endpoint = String::from_utf8(output.stdout).expect("UTF-8 docker port output");
        let port = endpoint
            .trim()
            .rsplit_once(':')
            .expect("host:port")
            .1
            .parse()
            .expect("numeric port");
        let connection =
            format!("host=127.0.0.1 port={port} user=postgres dbname=postgres connect_timeout=2");
        let mut ready = false;
        for _ in 0..120 {
            if let Ok(mut client) = postgres::Client::connect(&connection, NoTls) {
                if client
                    .query_one("SELECT postgis_full_version()", &[])
                    .is_ok()
                {
                    ready = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(250));
        }
        assert!(
            ready,
            "PostGIS TCP fixture did not become ready in 30 seconds"
        );
        Self { name, port }
    }

    fn connection_string(&self) -> String {
        format!(
            "host=127.0.0.1 port={} user=postgres dbname=postgres connect_timeout=5",
            self.port
        )
    }
}

impl Drop for PostgisContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["stop", "--timeout", "1", &self.name])
            .status();
    }
}

#[test]
#[ignore = "requires Docker and the pinned real PostGIS image"]
fn real_postgis_five_positive_and_five_negative_cases() {
    let fixture = PostgisContainer::start();
    let connection = fixture.connection_string();
    let adapter = PostgisAdapter::default();

    let positives = [
        PostgisOperation::ProjectedArea {
            wkt: "POLYGON((0 0,1000 0,1000 1000,0 1000,0 0))".into(),
            srid: 6675,
        },
        PostgisOperation::TransformPoint {
            x: 136.9066,
            y: 35.1815,
            source_srid: 4326,
            target_srid: 6675,
        },
        PostgisOperation::Intersects {
            left_wkt: "POLYGON((0 0,2 0,2 2,0 2,0 0))".into(),
            right_wkt: "POLYGON((1 1,3 1,3 3,1 3,1 1))".into(),
            srid: 6675,
        },
        PostgisOperation::MultipartPartCount {
            wkt: "MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)),((2 2,3 2,3 3,2 3,2 2)))".into(),
            srid: 6675,
        },
        PostgisOperation::OrderNullableValues {
            values: vec![Some(3), None, Some(1), Some(2)],
        },
    ];
    let receipts = positives
        .iter()
        .map(|operation| {
            adapter
                .execute(&connection, operation)
                .expect("typed PostGIS operation")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        receipts[0].output,
        json!({ "area_square_metres": 1_000_000.0 })
    );
    assert!(receipts[1].output["x"].as_f64().unwrap().is_finite());
    assert!(receipts[1].output["y"].as_f64().unwrap().is_finite());
    assert_eq!(receipts[2].output, json!({ "intersects": true }));
    assert_eq!(receipts[3].output, json!({ "part_count": 2 }));
    assert_eq!(receipts[4].output, json!({ "values": [1, 2, 3, null] }));
    assert!(receipts.iter().all(|receipt| {
        receipt.transaction_read_only
            && receipt.isolation_level == "repeatable_read"
            && receipt.query_plan_digest.starts_with("sha256:")
            && receipt.output_digest.starts_with("sha256:")
            && receipt.runtime_postgresql_version.starts_with("18.6")
            && receipt
                .runtime_postgis_full_version
                .contains("POSTGIS=\"3.6.4")
    }));

    let malformed_wkt = adapter.execute(
        &connection,
        &PostgisOperation::ProjectedArea {
            wkt: "NOT A GEOMETRY".into(),
            srid: 6675,
        },
    );
    assert!(matches!(malformed_wkt, Err(PostgisError::Database(_))));

    let unknown_crs = adapter.execute(
        &connection,
        &PostgisOperation::TransformPoint {
            x: 136.9,
            y: 35.1,
            source_srid: 999_999,
            target_srid: 6675,
        },
    );
    assert!(matches!(unknown_crs, Err(PostgisError::Contract(_))));

    let mut denied_policy = CapabilityPolicy::read_only_database("different-adapter");
    denied_policy.accepted_adapters.clear();
    let denied = PostgisAdapter::new(postgis_manifest(), denied_policy).execute(
        &connection,
        &PostgisOperation::ProjectedArea {
            wkt: "POLYGON((0 0,1 0,1 1,0 1,0 0))".into(),
            srid: 6675,
        },
    );
    assert!(matches!(denied, Err(PostgisError::Admission(_))));

    let mut drifted_manifest = postgis_manifest();
    drifted_manifest.backend.engine_version = "18.5".into();
    let drifted_policy = CapabilityPolicy::read_only_database(&drifted_manifest.adapter_id);
    let drifted = PostgisAdapter::new(drifted_manifest, drifted_policy).execute(
        &connection,
        &PostgisOperation::ProjectedArea {
            wkt: "POLYGON((0 0,1 0,1 1,0 1,0 0))".into(),
            srid: 6675,
        },
    );
    assert!(matches!(drifted, Err(PostgisError::RuntimeIdentity(_))));

    let mut client = postgres::Client::connect(&connection, NoTls).expect("connect for denial");
    let mut read_only = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .expect("start read-only transaction");
    assert!(read_only
        .batch_execute("CREATE TABLE forbidden_write(id integer)")
        .is_err());

    let evidence = json!({
        "schema_version": "0.1.0",
        "image_digest": POSTGIS_IMAGE_DIGEST,
        "positive_cases": 5,
        "negative_cases": 5,
        "false_accepts": 0,
        "receipts": receipts
    });
    println!("{}", serde_json::to_string(&evidence).unwrap());
}
