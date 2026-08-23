use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use genegis_catalog::{AssetRequirements, FederatedCatalog, StacEndpoint, StacSearchRequest};
use genegis_core::{Command, CommandEnvelope, CommandOrigin};
use genegis_vector::{read_geoparquet_uri_with_options_and_policy, GeoParquetReadOptions};
use genegis_workflow::federated_asset_execution_template;

#[test]
fn search_bind_range_execute_and_verify_retains_one_audit_chain() {
    let parquet = fs::read("../../examples/nagoya-population-density/data/nagoya-wards.parquet")
        .expect("bundled GeoParquet");
    let endpoint = spawn_federated_fixture(parquet.clone());
    let mirror_endpoint = spawn_federated_fixture(parquet);
    let mut catalog = FederatedCatalog::new();
    catalog.register(StacEndpoint::new("nagoya-primary", &endpoint));
    catalog.register(StacEndpoint::new("nagoya-mirror", &mirror_endpoint));
    let request = StacSearchRequest {
        bbox: Some([136.79, 35.03, 137.07, 35.27]),
        limit: Some(10),
        ..StacSearchRequest::default()
    };

    let remote_policy = genegis_storage::RemoteAccessPolicy {
        allowed_hosts: vec![],
        allow_loopback: true,
        max_response_bytes: 8 * 1024 * 1024,
        timeout_ms: 15_000,
        max_redirects: 0,
    };
    let search = catalog.search_with_policy(&request, &remote_policy);
    assert_eq!(
        search.successful_endpoints(),
        2,
        "endpoint outcomes: {:?}",
        search.endpoints
    );
    assert_eq!(search.items.len(), 1, "search result: {search:?}");
    let binding = search
        .compare_and_bind(&AssetRequirements {
            bbox: request.bbox,
            ..AssetRequirements::default()
        })
        .expect("verified binding");
    let command = CommandEnvelope::new(
        CommandOrigin::System,
        Command::BindStacAsset {
            stac_item_key: binding.selected.stac_item_key.clone(),
            asset_key: binding.selected.asset_key.clone(),
            source_endpoints: binding.selected.source_endpoints.clone(),
            href: binding.selected.href.clone(),
            media_type: binding.selected.media_type.clone(),
            crs: binding.crs.clone(),
            units: binding.units.clone(),
            license: binding.license.clone(),
        },
    );
    let endpoint_ids = vec!["nagoya-primary".into(), "nagoya-mirror".into()];
    let workflow = federated_asset_execution_template(
        &endpoint_ids,
        &binding.selected.stac_item_key,
        &binding.selected.asset_key,
        &binding.selected.href,
    );
    let execution = read_geoparquet_uri_with_options_and_policy(
        &binding.selected.href,
        GeoParquetReadOptions {
            row_groups: Some(vec![0]),
        },
        remote_policy,
    )
    .expect("HTTP range execution");

    assert!(matches!(command.command, Command::BindStacAsset { .. }));
    assert!(workflow
        .steps
        .iter()
        .any(|step| step.operation == "BindStacAsset"));
    assert!(workflow
        .steps
        .iter()
        .any(|step| step.operation == "EnforceRemoteAccessPolicy"));
    assert_eq!(binding.selected.source_endpoints.len(), 2);
    assert!(binding
        .selected
        .verifications
        .iter()
        .all(|check| check.passed));
    assert_eq!(execution.read_mode, "http_range");
    assert!(execution.range_requests > 0);
    assert!(execution.bytes_fetched > 0);
    assert_eq!(execution.dataset.crs, binding.crs);
    assert_eq!(execution.source_uri, binding.selected.href);
    assert!(!execution.schema_fields.is_empty());
}

fn spawn_federated_fixture(parquet: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let base_url = format!("http://{address}");
    let item_collection = serde_json::to_vec(&serde_json::json!({
        "type": "FeatureCollection",
        "features": [{
            "stac_version": "1.0.0",
            "type": "Feature",
            "id": "nagoya-wards",
            "collection": "nagoya-open-data",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [136.792, 35.034], [137.061, 35.034], [137.061, 35.26],
                    [136.792, 35.26], [136.792, 35.034]
                ]]
            },
            "bbox": [136.792, 35.034, 137.061, 35.26],
            "properties": {
                "proj:code": "EPSG:4326",
                "genegis:units": "degrees",
                "license": "CC-BY-4.0"
            },
            "assets": {
                "geoparquet": {
                    "href": format!("{base_url}/nagoya-wards.parquet"),
                    "type": "application/vnd.apache.parquet",
                    "roles": ["data"],
                    "title": "Nagoya wards"
                }
            },
            "links": []
        }],
        "links": []
    }))
    .expect("item collection");

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve_fixture_request(stream, &item_collection, &parquet);
        }
    });
    base_url
}

fn serve_fixture_request(mut stream: TcpStream, item_collection: &[u8], parquet: &[u8]) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = stream.read(&mut buffer).expect("request read");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
        else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = header_value(&headers, "Content-Length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if request.len() >= header_end + content_length {
            break;
        }
    }
    let request_text = String::from_utf8_lossy(&request);
    let first_line = request_text.lines().next().unwrap_or_default();
    if first_line.starts_with("POST /search ") {
        write_response(
            &mut stream,
            "200 OK",
            "application/geo+json",
            &[],
            item_collection,
        );
        return;
    }
    if first_line.starts_with("HEAD /nagoya-wards.parquet ") {
        write_response(
            &mut stream,
            "200 OK",
            "application/vnd.apache.parquet",
            &[("Accept-Ranges", "bytes".into())],
            &vec![0; parquet.len()],
        );
        return;
    }
    if first_line.starts_with("GET /nagoya-wards.parquet ") {
        if let Some(range) = header_value(&request_text, "Range") {
            let (start, end) = range
                .trim_start_matches("bytes=")
                .split_once('-')
                .and_then(|(start, end)| Some((start.parse().ok()?, end.parse().ok()?)))
                .expect("valid range");
            let body = &parquet[start..=end];
            write_response(
                &mut stream,
                "206 Partial Content",
                "application/vnd.apache.parquet",
                &[
                    ("Accept-Ranges", "bytes".into()),
                    (
                        "Content-Range",
                        format!("bytes {start}-{end}/{}", parquet.len()),
                    ),
                ],
                body,
            );
        } else {
            write_response(
                &mut stream,
                "200 OK",
                "application/vnd.apache.parquet",
                &[("Accept-Ranges", "bytes".into())],
                parquet,
            );
        }
        return;
    }
    write_response(
        &mut stream,
        "404 Not Found",
        "text/plain",
        &[],
        b"not found",
    );
}

fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        candidate.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    headers: &[(&str, String)],
    body: &[u8],
) {
    let extra = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("headers");
    if !body.is_empty() {
        stream.write_all(body).expect("body");
    }
    stream.flush().expect("flush");
}
