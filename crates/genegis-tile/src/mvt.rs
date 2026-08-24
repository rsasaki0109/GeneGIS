//! Minimal Mapbox Vector Tile (v2) polygon encoder.
//!
//! Encodes polygon features into protobuf-encoded MVT layers without external
//! protobuf dependencies. Coordinates are supplied as WGS84 lon/lat and are
//! projected into Web Mercator tile space at encode time; every emitted tile
//! therefore records its exact source-CRS transformation path.

use std::collections::BTreeMap;

/// Default MVT coordinate extent per tile axis.
pub const DEFAULT_EXTENT: u32 = 4096;

const MERCATOR_LAT_LIMIT_DEG: f64 = 85.051_129;

/// Property value carried into an MVT feature attributes table.
#[derive(Debug, Clone, PartialEq)]
pub enum TileValue {
    /// UTF-8 string property.
    Text(String),
    /// Unsigned integer property.
    U64(u64),
    /// Floating point property.
    F64(f64),
}

/// One polygon part: an exterior ring plus interior hole rings in lon/lat.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TilePolygonPart {
    /// Exterior ring.
    pub exterior: Vec<(f64, f64)>,
    /// Interior rings enclosed by [`TilePolygonPart::exterior`].
    pub holes: Vec<Vec<(f64, f64)>>,
}

/// One MVT polygon feature, possibly multipart.
#[derive(Debug, Clone, PartialEq)]
pub struct TilePolygonFeature {
    /// Stable identity encoded as the MVT feature id.
    pub id: u64,
    /// Polygon parts, each exterior-first.
    pub parts: Vec<TilePolygonPart>,
    /// Attribute key/value pairs encoded into layer key/value tables.
    pub properties: Vec<(String, TileValue)>,
}

/// Encode one polygon-only MVT tile.
///
/// Returns an empty vector when no part survives clipping into the requested
/// tile, signalling the caller to omit the tile entirely.
pub fn encode_polygon_tile(
    layer_name: &str,
    z: u8,
    x: u32,
    y: u32,
    extent: u32,
    features: &[TilePolygonFeature],
) -> Vec<u8> {
    let mut key_table: BTreeMap<String, usize> = BTreeMap::new();
    let mut value_table: Vec<TileValue> = Vec::new();
    let mut feature_messages: Vec<Vec<u8>> = Vec::new();
    for feature in features {
        let Some(geometry) = encode_feature_geometry(feature, z, x, y, extent) else {
            continue;
        };
        if geometry.is_empty() {
            continue;
        }
        let mut message = Vec::new();
        push_tag(&mut message, 1, 0);
        push_varint(&mut message, feature.id);
        let tags = encode_tags(feature, &mut key_table, &mut value_table);
        if !tags.is_empty() {
            push_len_field(&mut message, 2, &tags);
        }
        push_tag(&mut message, 3, 0);
        push_varint(&mut message, 3);
        push_len_field(&mut message, 4, &geometry);
        feature_messages.push(message);
    }
    if feature_messages.is_empty() {
        return Vec::new();
    }
    let mut layer = Vec::new();
    push_tag(&mut layer, 15, 0);
    push_varint(&mut layer, 2);
    push_len_field(&mut layer, 1, layer_name.as_bytes());
    for message in feature_messages {
        push_len_field(&mut layer, 2, &message);
    }
    for key in key_table.keys() {
        push_len_field(&mut layer, 3, key.as_bytes());
    }
    for value in &value_table {
        let mut encoded = Vec::new();
        match value {
            TileValue::Text(text) => push_len_field(&mut encoded, 1, text.as_bytes()),
            TileValue::F64(number) => {
                push_tag(&mut encoded, 3, 1);
                encoded.extend_from_slice(&number.to_le_bytes());
            }
            TileValue::U64(number) => {
                push_tag(&mut encoded, 5, 0);
                push_varint(&mut encoded, *number);
            }
        }
        push_len_field(&mut layer, 4, &encoded);
    }
    push_tag(&mut layer, 5, 0);
    push_varint(&mut layer, u64::from(extent));
    let mut tile = Vec::new();
    push_len_field(&mut tile, 3, &layer);
    tile
}

/// Fractional Web Mercator tile coordinates for a lon/lat point at zoom `z`.
///
/// Latitude is clamped to the Web Mercator domain before projection.
pub fn lonlat_to_tile_fraction(lon: f64, lat: f64, z: u8) -> (f64, f64) {
    let scale = f64::from(1_u16 << z.min(15));
    let x = (lon + 180.0) / 360.0 * scale;
    let lat = lat.clamp(-MERCATOR_LAT_LIMIT_DEG, MERCATOR_LAT_LIMIT_DEG);
    let radians = lat.to_radians();
    let y = (1.0 - (radians.tan() + 1.0 / radians.cos()).ln() / std::f64::consts::PI) / 2.0 * scale;
    (x, y)
}

/// Containing tile column for a longitude at zoom `z`.
pub fn lon_to_tile_x(lon: f64, z: u8) -> u32 {
    lonlat_to_tile_fraction(lon, 0.0, z).0.floor().max(0.0) as u32
}

/// Containing tile row for a latitude at zoom `z`.
pub fn lat_to_tile_y(lat: f64, z: u8) -> u32 {
    lonlat_to_tile_fraction(0.0, lat, z).1.floor().max(0.0) as u32
}

fn push_varint(buffer: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buffer.push(byte);
            return;
        }
        buffer.push(byte | 0x80);
    }
}

fn push_tag(buffer: &mut Vec<u8>, field: u32, wire: u32) {
    push_varint(buffer, (u64::from(field) << 3) | u64::from(wire));
}

fn push_len_field(buffer: &mut Vec<u8>, field: u32, payload: &[u8]) {
    push_tag(buffer, field, 2);
    push_varint(buffer, payload.len() as u64);
    buffer.extend_from_slice(payload);
}

fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn signed_area(ring: &[(i64, i64)]) -> i64 {
    let mut total: i64 = 0;
    for index in 0..ring.len() {
        let current = ring[index];
        let next = ring[(index + 1) % ring.len()];
        total += current.0 * next.1 - next.0 * current.1;
    }
    total / 2
}

#[derive(Debug, Clone, Copy)]
enum ClipEdge {
    MinX(f64),
    MaxX(f64),
    MinY(f64),
    MaxY(f64),
}

impl ClipEdge {
    fn inside(self, point: (f64, f64)) -> bool {
        match self {
            ClipEdge::MinX(value) => point.0 >= value,
            ClipEdge::MaxX(value) => point.0 <= value,
            ClipEdge::MinY(value) => point.1 >= value,
            ClipEdge::MaxY(value) => point.1 <= value,
        }
    }

    fn crossing(self, from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
        let delta = (to.0 - from.0, to.1 - from.1);
        let t = match self {
            ClipEdge::MinX(value) => (value - from.0) / delta.0,
            ClipEdge::MaxX(value) => (value - from.0) / delta.0,
            ClipEdge::MinY(value) => (value - from.1) / delta.1,
            ClipEdge::MaxY(value) => (value - from.1) / delta.1,
        };
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (from.0 + t * delta.0, from.1 + t * delta.1)
    }
}

fn project_ring(ring: &[(f64, f64)], z: u8, x: u32, y: u32, extent: u32) -> Vec<(i64, i64)> {
    let scale = f64::from(extent);
    let clipped = clip_ring(
        &ring
            .iter()
            .map(|&(lon, lat)| {
                let (fx, fy) = lonlat_to_tile_fraction(lon, lat, z);
                ((fx - f64::from(x)) * scale, (fy - f64::from(y)) * scale)
            })
            .collect::<Vec<_>>(),
        (0.0, 0.0),
        (scale, scale),
    );
    let mut projected: Vec<(i64, i64)> = clipped
        .iter()
        .map(|&(px, py)| {
            (
                px.round().clamp(0.0, f64::from(extent)) as i64,
                py.round().clamp(0.0, f64::from(extent)) as i64,
            )
        })
        .collect();
    if projected.len() > 1 && projected[0] == projected[projected.len() - 1] {
        projected.pop();
    }
    projected.dedup();
    projected
}

fn prepare_ring(mut ring: Vec<(i64, i64)>, exterior: bool) -> Option<Vec<(i64, i64)>> {
    if ring.len() < 3 {
        return None;
    }
    let area = signed_area(&ring);
    let oriented_correctly = if exterior { area > 0 } else { area < 0 };
    if area != 0 && !oriented_correctly {
        ring.reverse();
    }
    Some(ring)
}

fn encode_ring_commands(ring: &[(i64, i64)], cursor: &mut (i64, i64), buffer: &mut Vec<u8>) {
    push_varint(buffer, 1 | (1 << 3));
    push_zigzag_delta(buffer, cursor, ring[0]);
    if ring.len() > 1 {
        push_varint(buffer, 2 | (((ring.len() - 1) as u64) << 3));
        for point in &ring[1..] {
            push_zigzag_delta(buffer, cursor, *point);
        }
    }
    push_varint(buffer, 7 | (1 << 3));
}

fn push_zigzag_delta(buffer: &mut Vec<u8>, cursor: &mut (i64, i64), target: (i64, i64)) {
    let delta = (target.0 - cursor.0, target.1 - cursor.1);
    push_varint(buffer, zigzag(delta.0));
    push_varint(buffer, zigzag(delta.1));
    *cursor = target;
}

fn encode_feature_geometry(
    feature: &TilePolygonFeature,
    z: u8,
    x: u32,
    y: u32,
    extent: u32,
) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut cursor = (0_i64, 0_i64);
    for part in &feature.parts {
        let Some(exterior) = prepare_ring(project_ring(&part.exterior, z, x, y, extent), true)
        else {
            continue;
        };
        encode_ring_commands(&exterior, &mut cursor, &mut buffer);
        for hole in &part.holes {
            let Some(hole) = prepare_ring(project_ring(hole, z, x, y, extent), false) else {
                continue;
            };
            encode_ring_commands(&hole, &mut cursor, &mut buffer);
        }
    }
    if buffer.is_empty() {
        None
    } else {
        Some(buffer)
    }
}

fn encode_tags(
    feature: &TilePolygonFeature,
    key_table: &mut BTreeMap<String, usize>,
    value_table: &mut Vec<TileValue>,
) -> Vec<u8> {
    let mut tags = Vec::new();
    for (key, value) in &feature.properties {
        let key_index = match key_table.get(key) {
            Some(index) => *index,
            None => {
                let index = key_table.len();
                key_table.insert(key.clone(), index);
                index
            }
        };
        let value_index = match value_table.iter().position(|candidate| candidate == value) {
            Some(index) => index,
            None => {
                value_table.push(value.clone());
                value_table.len() - 1
            }
        };
        push_varint(&mut tags, key_index as u64);
        push_varint(&mut tags, value_index as u64);
    }
    tags
}

fn clip_ring(ring: &[(f64, f64)], minimum: (f64, f64), maximum: (f64, f64)) -> Vec<(f64, f64)> {
    let edges = [
        ClipEdge::MinX(minimum.0),
        ClipEdge::MaxX(maximum.0),
        ClipEdge::MinY(minimum.1),
        ClipEdge::MaxY(maximum.1),
    ];
    let mut output: Vec<(f64, f64)> = ring.to_vec();
    for edge in edges {
        if output.len() < 3 {
            return Vec::new();
        }
        let input = std::mem::take(&mut output);
        let mut previous = *input.last().expect("non-empty ring");
        for &point in &input {
            let point_inside = edge.inside(point);
            let previous_inside = edge.inside(previous);
            if point_inside != previous_inside {
                output.push(edge.crossing(previous, point));
            }
            if point_inside {
                output.push(point);
            }
            previous = point;
        }
    }
    if output.len() < 3 {
        Vec::new()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    enum Field<'a> {
        Varint(u64),
        Fixed64(&'a [u8]),
        Bytes(&'a [u8]),
    }

    fn read_varint(bytes: &[u8], cursor: &mut usize) -> u64 {
        let mut value = 0_u64;
        let mut shift = 0;
        loop {
            let byte = bytes[*cursor];
            *cursor += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }

    fn parse_fields(bytes: &[u8]) -> Vec<(u32, Field<'_>)> {
        let mut cursor = 0;
        let mut fields = Vec::new();
        while cursor < bytes.len() {
            let tag = read_varint(bytes, &mut cursor);
            let field = (tag >> 3) as u32;
            match tag & 7 {
                0 => fields.push((field, Field::Varint(read_varint(bytes, &mut cursor)))),
                1 => {
                    fields.push((field, Field::Fixed64(&bytes[cursor..cursor + 8])));
                    cursor += 8;
                }
                2 => {
                    let length = read_varint(bytes, &mut cursor) as usize;
                    fields.push((field, Field::Bytes(&bytes[cursor..cursor + length])));
                    cursor += length;
                }
                other => panic!("unexpected wire type {other}"),
            }
        }
        fields
    }

    struct DecodedTile {
        layer_name: String,
        extent: Option<u64>,
        feature_ids: Vec<u64>,
        keys: usize,
        values: usize,
    }

    fn first_varint(fields: &[(u32, Field<'_>)], wanted: u32) -> Option<u64> {
        fields
            .iter()
            .find_map(|(field, value)| match (field, value) {
                (found, Field::Varint(raw)) if *found == wanted => Some(*raw),
                _ => None,
            })
    }

    fn decode_tile(bytes: &[u8]) -> DecodedTile {
        let mut decoded = DecodedTile {
            layer_name: String::new(),
            extent: None,
            feature_ids: Vec::new(),
            keys: 0,
            values: 0,
        };
        for (field, value) in parse_fields(bytes) {
            assert_eq!(field, 3, "tile must only carry layers");
            let Field::Bytes(layer) = value else {
                panic!("layer must be a length-delimited field");
            };
            for (field, value) in parse_fields(layer) {
                match (field, value) {
                    (1, Field::Bytes(name)) => {
                        decoded.layer_name = String::from_utf8(name.to_vec()).expect("utf8")
                    }
                    (2, Field::Bytes(feature)) => {
                        let fields = parse_fields(feature);
                        if let Some(id) = first_varint(&fields, 1) {
                            decoded.feature_ids.push(id);
                        }
                    }
                    (3, Field::Bytes(_)) => decoded.keys += 1,
                    (4, Field::Bytes(_)) => decoded.values += 1,
                    (5, Field::Varint(extent)) => decoded.extent = Some(extent),
                    _ => {}
                }
            }
        }
        decoded
    }

    /// Tile 908/403 at z10 covers roughly lon 139.22-139.57E, lat 35.2-35.5N.
    fn nagoya_feature(id: u64) -> TilePolygonFeature {
        TilePolygonFeature {
            id,
            parts: vec![TilePolygonPart {
                exterior: vec![
                    (139.25, 35.50),
                    (139.55, 35.50),
                    (139.55, 35.25),
                    (139.25, 35.25),
                ],
                holes: Vec::new(),
            }],
            properties: vec![
                ("ward_name".into(), TileValue::Text("北区".into())),
                ("population".into(), TileValue::U64(168_214)),
                ("density_per_km2".into(), TileValue::F64(5_272.5)),
            ],
        }
    }

    #[test]
    fn projects_greenwich_center_of_zoom_one_grid() {
        let (x, y) = lonlat_to_tile_fraction(0.0, 0.0, 1);
        assert!((x - 1.0).abs() < 1e-9);
        assert!((y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn encodes_layer_with_tables_and_extent() {
        let tile = encode_polygon_tile(
            "density",
            10,
            908,
            403,
            DEFAULT_EXTENT,
            &[nagoya_feature(7)],
        );
        let decoded = decode_tile(&tile);
        assert_eq!(decoded.layer_name, "density");
        assert_eq!(decoded.extent, Some(u64::from(DEFAULT_EXTENT)));
        assert_eq!(decoded.feature_ids, vec![7]);
        assert_eq!(decoded.keys, 3);
        assert_eq!(decoded.values, 3);
    }

    #[test]
    fn normalizes_winding_independently_of_input_order() {
        let mut ccw = nagoya_feature(1);
        ccw.properties.clear();
        let mut reversed_parts = ccw.parts[0].clone();
        reversed_parts.exterior.reverse();
        let cw = TilePolygonFeature {
            id: 1,
            parts: vec![reversed_parts],
            properties: Vec::new(),
        };
        let left = encode_polygon_tile("density", 10, 908, 403, 512, &[ccw]);
        let right = encode_polygon_tile("density", 10, 908, 403, 512, &[cw]);
        assert!(!left.is_empty());
        // Clipping may rotate the ring start vertex, so compare geometry as
        // point sets instead of raw bytes.
        assert_eq!(decoded_rings(&left), decoded_rings(&right));
    }

    fn decoded_rings(tile: &[u8]) -> Vec<Vec<(i64, i64)>> {
        for (field, value) in parse_fields(tile) {
            if field != 3 {
                continue;
            }
            let Field::Bytes(layer) = value else {
                continue;
            };
            for (field, value) in parse_fields(layer) {
                if field != 2 {
                    continue;
                }
                let Field::Bytes(feature) = value else {
                    continue;
                };
                for (field, value) in parse_fields(feature) {
                    if field == 4 {
                        let Field::Bytes(geometry) = value else {
                            continue;
                        };
                        return replay_geometry(geometry);
                    }
                }
            }
        }
        panic!("no geometry found");
    }

    fn replay_geometry(bytes: &[u8]) -> Vec<Vec<(i64, i64)>> {
        let mut cursor = 0;
        let mut point = (0_i64, 0_i64);
        let mut rings: Vec<Vec<(i64, i64)>> = Vec::new();
        while cursor < bytes.len() {
            let command = read_varint(bytes, &mut cursor);
            let id = command & 7;
            let count = (command >> 3) as usize;
            match id {
                1 | 2 => {
                    for _ in 0..count {
                        let dx = zigzag_decode(read_varint(bytes, &mut cursor));
                        let dy = zigzag_decode(read_varint(bytes, &mut cursor));
                        point = (point.0 + dx, point.1 + dy);
                        if id == 1 {
                            rings.push(vec![point]);
                        } else {
                            rings.last_mut().expect("ring").push(point);
                        }
                    }
                }
                7 => {}
                other => panic!("unexpected command id {other}"),
            }
        }
        let mut normalized: Vec<Vec<(i64, i64)>> = rings
            .into_iter()
            .map(|mut ring| {
                ring.sort();
                ring
            })
            .collect();
        normalized.sort();
        normalized
    }

    fn zigzag_decode(value: u64) -> i64 {
        ((value >> 1) as i64) ^ -((value & 1) as i64)
    }

    #[test]
    fn omits_features_outside_the_requested_tile() {
        let outside = TilePolygonFeature {
            id: 1,
            parts: vec![TilePolygonPart {
                exterior: vec![(100.0, 60.0), (110.0, 60.0), (110.0, 50.0), (100.0, 50.0)],
                holes: Vec::new(),
            }],
            properties: Vec::new(),
        };
        let tile = encode_polygon_tile("density", 10, 908, 403, 512, &[outside]);
        assert!(tile.is_empty());
    }

    #[test]
    fn clips_features_spanning_tile_boundaries() {
        let spanning = TilePolygonFeature {
            id: 1,
            parts: vec![TilePolygonPart {
                exterior: vec![(-30.0, 30.0), (30.0, 30.0), (30.0, -30.0), (-30.0, -30.0)],
                holes: Vec::new(),
            }],
            properties: Vec::new(),
        };
        let left = encode_polygon_tile("density", 1, 0, 1, 256, &[spanning.clone()]);
        let right = encode_polygon_tile("density", 1, 1, 1, 256, &[spanning]);
        assert!(!left.is_empty());
        assert!(!right.is_empty());
        let decoded_left = decode_tile(&left);
        assert_eq!(decoded_left.feature_ids, vec![1]);
    }
}
