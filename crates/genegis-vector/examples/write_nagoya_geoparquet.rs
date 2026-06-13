//! Write the bundled Nagoya wards GeoParquet fixture used by Phase 9 alpha workflows.

use std::path::PathBuf;

use genegis_vector::geojson::read_geojson_path;
use genegis_catalog::nagoya_wards_geojson_path;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use geo_types::{Coord, Geometry, LineString, Polygon};
use geoarrow_array::builder::GeometryBuilder;
use geoarrow_array::GeoArrowArray;
use geoarrow_schema::GeometryType;
use geoparquet::writer::{GeoParquetRecordBatchEncoder, GeoParquetWriterOptions};
use parquet::arrow::ArrowWriter;

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nagoya-population-density/data/nagoya-wards.parquet");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create data dir");
    }

    let bytes = write_nagoya_geoparquet_bytes();
    std::fs::write(&out, bytes).expect("write parquet");
    println!("wrote {}", out.display());
}

fn write_nagoya_geoparquet_bytes() -> Vec<u8> {
    use std::sync::Arc;

    let dataset = read_geojson_path(nagoya_wards_geojson_path()).expect("geojson");
    let mut ward_names = Vec::new();
    let mut ward_codes = Vec::new();
    let mut populations = Vec::new();

    for feature in &dataset.features {
        ward_names.push(
            feature
                .properties
                .get("ward_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        );
        ward_codes.push(
            feature
                .properties
                .get("ward_code")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        );
        populations.push(
            feature
                .properties
                .get("population")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as i64,
        );
    }

    let mut geom_builder = GeometryBuilder::new(GeometryType::default());
    for feature in &dataset.features {
        let geom = rings_to_geometry(&feature.rings);
        geom_builder
            .push_geometry(Some(&geom))
            .expect("push geometry");
    }
    let geom_array = geom_builder.finish();

    let geometry_field = GeometryType::default().to_field("geometry", false);
    let schema = Schema::new(vec![
        Field::new("ward_name", DataType::Utf8, false),
        Field::new("ward_code", DataType::Utf8, false),
        Field::new("population", DataType::Int64, false),
        geometry_field,
    ]);

    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(StringArray::from(ward_names)) as _,
            Arc::new(StringArray::from(ward_codes)) as _,
            Arc::new(Int64Array::from(populations)) as _,
            geom_array.into_array_ref(),
        ],
    )
    .expect("record batch");

    let mut buffer = Vec::new();
    let options = GeoParquetWriterOptions::default();
    let mut encoder = GeoParquetRecordBatchEncoder::try_new(&schema, &options).expect("encoder");
    let mut writer =
        ArrowWriter::try_new(&mut buffer, encoder.target_schema(), None).expect("writer");
    let encoded = encoder.encode_record_batch(&batch).expect("encode");
    writer.write(&encoded).expect("write");
    writer
        .append_key_value_metadata(encoder.into_keyvalue().expect("metadata"));
    writer.close().expect("close");
    buffer
}

fn rings_to_geometry(rings: &[genegis_geometry::PolygonRing]) -> Geometry {
    let polygons: Vec<Polygon<f64>> = rings
        .iter()
        .map(|ring| {
            let coords: Vec<Coord<f64>> = ring
                .exterior()
                .iter()
                .map(|(x, y)| Coord { x: *x, y: *y })
                .collect();
            Polygon::new(LineString::from(coords), vec![])
        })
        .collect();
    if polygons.len() == 1 {
        Geometry::Polygon(polygons.into_iter().next().expect("polygon"))
    } else {
        Geometry::MultiPolygon(polygons.into())
    }
}
