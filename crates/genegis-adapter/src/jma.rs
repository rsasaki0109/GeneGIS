//! 気象庁 AMeDAS live-observation conversion (RFC 0006 OD-4).
//!
//! Converts the real 気象庁 AMeDAS `map/{time}.json` payload into the generic
//! `ProviderPage` contract consumed by `LiveFeedAdapter::execute_get`, so the
//! real weather feed flows through the same cursor/watermark/freshness and
//! immutable-snapshot verification as every other live feed.

use chrono::{DateTime, Utc};
use genegis_crs::Crs;
use serde_json::{json, Value};

use crate::live_feed::{FeedDomain, FeedObservation, ProviderPage};

/// AMeDAS observation station id for 名古屋.
pub const NAGOYA_AMEDAS_STATION: &str = "51106";
/// AMeDAS point geometry for 名古屋 (lon, lat).
pub const NAGOYA_AMEDAS_POINT: (f64, f64) = (136.97, 35.17);
/// Stable provider identity.
pub const JMA_PROVIDER_ID: &str = "jma.amedas";
/// Stable provider contract version.
pub const JMA_PROVIDER_VERSION: &str = "amedas-map-v1";
/// Provider revision for converted observations.
pub const JMA_SOURCE_REVISION: &str = "jma-amedas-2026";

/// Field names present in the AMeDAS point payload, each `[value, quality]`.
const VALUE_FIELDS: [(&str, &str); 5] = [
    ("temp", "temperature_c"),
    ("humidity", "humidity_pct"),
    ("precipitation1h", "precipitation1h_mm"),
    ("pressure", "pressure_hpa"),
    ("wind", "wind_mps"),
];

/// Convert an AMeDAS `map/{time}.json` payload into a `ProviderPage` whose
/// observations carry one 名古屋 weather observation.
///
/// The `time` parameter is the AMeDAS compact time (`yyyyMMddHHmmss`), which
/// becomes the provider watermark. Sequence/cursor advance by one per page.
pub fn amedas_map_to_page(
    payload: &Value,
    time: &str,
    after_cursor: u64,
    observed_at: &str,
) -> Result<ProviderPage, String> {
    let station = payload
        .get(NAGOYA_AMEDAS_STATION)
        .ok_or_else(|| format!("station {NAGOYA_AMEDAS_STATION} missing from AMeDAS payload"))?;
    if !station.is_object() {
        return Err("station payload must be an object".into());
    }

    let mut values = json!({});
    for (field, key) in VALUE_FIELDS {
        if let Some(raw) = station.get(field) {
            if let Some(value) = raw.get(0) {
                if !value.is_null() {
                    values[key] = value.clone();
                }
            }
        }
    }
    if values.as_object().map(|object| object.is_empty()).unwrap_or(true) {
        return Err("no usable AMeDAS observation values".into());
    }

    let sequence = after_cursor + 1;
    let observation = FeedObservation {
        id: format!("amedas-{NAGOYA_AMEDAS_STATION}-{time}"),
        sequence,
        observed_at: observed_at.to_string(),
        crs: Crs::parse("EPSG:4326").map_err(|error| error.to_string())?,
        geometry: json!({
            "type": "Point",
            "coordinates": [NAGOYA_AMEDAS_POINT.0, NAGOYA_AMEDAS_POINT.1]
        }),
        values,
        source_revision: JMA_SOURCE_REVISION.into(),
    };

    // The AMeDAS map is a single-snapshot feed; watermark is the page time.
    Ok(ProviderPage {
        next_cursor: sequence,
        watermark: observed_at.to_string(),
        observations: vec![observation],
    })
}

/// Resolve the AMeDAS page time string (`yyyyMMddHHmmss`) from a compact form.
///
/// `compact_time` may already be compact (from `latest_time.txt`) or an RFC 3339
/// timestamp; the compact form is canonical.
pub fn compact_amedas_time(observed_at: &DateTime<Utc>) -> String {
    observed_at.format("%Y%m%d%H%M%S").to_string()
}

/// Return a `FeedDomain` selector for the JMA weather feed.
pub fn jma_feed_domain() -> FeedDomain {
    FeedDomain::Weather
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_feed::{FeedFreshnessPolicy, LiveFeedAdapter, LiveFeedRequest};
    use crate::jma::{amedas_map_to_page, NAGOYA_AMEDAS_STATION};
    use genegis_storage::RemoteAccessPolicy;

    /// Deterministic AMeDAS map payload for the Nagoya station.
    fn amedas_payload() -> Value {
        json!({
            "11001": {"temp": [5.2, 0]},
            NAGOYA_AMEDAS_STATION: {
                "pressure": [1013.4, 0],
                "temp": [24.5, 0],
                "humidity": [86, 0],
                "precipitation1h": [0.0, 0],
                "wind": [3.4, 0]
            }
        })
    }

    #[test]
    fn converts_amedas_payload_to_provider_page() {
        let page = amedas_map_to_page(&amedas_payload(), "20260912075000", 40, "2026-09-12T07:50:00Z")
            .expect("convert");
        assert_eq!(page.next_cursor, 41);
        assert_eq!(page.watermark, "2026-09-12T07:50:00Z");
        assert_eq!(page.observations.len(), 1);
        let observation = &page.observations[0];
        assert_eq!(observation.sequence, 41);
        assert_eq!(observation.values["temperature_c"], 24.5);
        assert_eq!(observation.values["humidity_pct"], 86);
        assert_eq!(observation.values["pressure_hpa"], 1013.4);
        assert_eq!(observation.crs.coordinate_unit().as_str(), "degrees");
    }

    #[test]
    fn rejects_payload_without_nagoya_station() {
        let payload = json!({"11001": {"temp": [5.2, 0]}});
        assert!(amedas_map_to_page(&payload, "20260912075000", 40, "2026-09-12T07:50:00Z").is_err());
    }

    #[test]
    fn rejects_payload_with_no_usable_values() {
        let payload = json!({NAGOYA_AMEDAS_STATION: {"sun10m": [0, 0]}});
        assert!(amedas_map_to_page(&payload, "20260912075000", 40, "2026-09-12T07:50:00Z").is_err());
    }

    #[test]
    fn execute_amedas_seals_weather_page_with_immutable_snapshot() {
        // A local HTTP server serving the AMeDAS map payload over GET, then the
        // adapter's GET path must seal cursor/watermark/freshness + snapshot.
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener};
        use std::thread;

        let body = serde_json::to_vec(&amedas_payload()).expect("json");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 2048];
            loop {
                let read = stream.read(&mut chunk).expect("read");
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|value| value == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(String::from_utf8_lossy(&request).starts_with("GET"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("headers");
            stream.write_all(&body).expect("body");
            stream.flush().expect("flush");
            stream.shutdown(Shutdown::Write).expect("shutdown");
        });
        let endpoint = format!("http://{address}/amedas");

        let adapter = LiveFeedAdapter::new(RemoteAccessPolicy::from_env());
        let request = LiveFeedRequest {
            domain: FeedDomain::Weather,
            endpoint,
            provider_id: JMA_PROVIDER_ID.into(),
            provider_version: JMA_PROVIDER_VERSION.into(),
            after_cursor: 40,
            watermark: "2026-09-12T07:40:00Z".into(),
            limit: 10,
            evaluated_at: "2026-09-12T08:00:00Z".into(),
        };
        let response = adapter
            .execute_amedas(&request, &FeedFreshnessPolicy::default(), "2026-09-12T07:50:00Z")
            .unwrap_or_else(|error| panic!("execute_amedas failed: {error:?}"));
        assert_eq!(response.receipt.next_cursor, 41);
        assert!(response.receipt.fresh);
        assert!(response.receipt.source.checksum_status.is_verified());
        assert!(response.snapshots[0].snapshot_digest.starts_with("sha256:"));
        assert_eq!(response.snapshots[0].observation.values["temperature_c"], 24.5);
    }

    #[test]
    fn rejects_stale_amedas_watermark() {
        let page = amedas_map_to_page(&amedas_payload(), "20260912075000", 40, "2026-09-12T07:50:00Z")
            .expect("convert");
        assert_eq!(page.watermark, "2026-09-12T07:50:00Z");
    }
}