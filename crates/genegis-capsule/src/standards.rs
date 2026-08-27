//! Open-standard projections of one proof-carrying result capsule.

use super::{
    digest, read_json, verify_error, verify_nagoya_capsule, CapsuleError, CapsuleManifest,
    ExecutionReceipt, VerificationPolicy, POLICY_PATH, RECEIPT_PATH, WORKFLOW_PATH,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use genegis_workflow::GeoWorkflow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const PROV_PATH: &str = "prov.json";
const RO_CRATE_PATH: &str = "ro-crate-metadata.json";
const OPENLINEAGE_PATH: &str = "openlineage-complete.json";
const INTOTO_PATH: &str = "in-toto-statement.json";
const OGC_PROCESS_PATH: &str = "ogc-process-description.json";
const OGC_REQUEST_PATH: &str = "ogc-execute-request.json";
const GENEGIS_NS: &str = "https://genegis.org/ns#";

/// Result of writing and structurally validating the standards bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardExportReport {
    /// Export destination.
    pub output_directory: String,
    /// Content digests keyed by standard export filename.
    pub files: BTreeMap<String, String>,
    /// Structural/profile validation result for each export.
    pub validations: BTreeMap<String, bool>,
}

/// One DSSE signature entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DsseSignature {
    /// Caller-controlled public-key identifier.
    pub keyid: String,
    /// Base64-encoded Ed25519 signature over DSSE PAE bytes.
    pub sig: String,
}

/// DSSE envelope carrying an in-toto Statement v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DsseEnvelope {
    /// Signed payload media type.
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    /// Base64-encoded in-toto Statement JSON.
    pub payload: String,
    /// One or more signatures.
    pub signatures: Vec<DsseSignature>,
}

/// Derive the 32-byte Ed25519 public key corresponding to a signing key.
pub fn ed25519_public_key(signing_key: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(signing_key)
        .verifying_key()
        .to_bytes()
}

/// Export PROV-JSON, Workflow Run RO-Crate, OpenLineage, in-toto Statement,
/// and OGC API - Processes fixtures into a new or empty directory.
pub fn export_standard_bundle(
    capsule_root: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> Result<StandardExportReport, CapsuleError> {
    let capsule_root = capsule_root.as_ref();
    let output = output.as_ref();
    if output.exists()
        && fs::read_dir(output)
            .map_err(|source| super::io_error(output, source))?
            .next()
            .transpose()
            .map_err(|source| super::io_error(output, source))?
            .is_some()
    {
        return Err(verify_error(format!(
            "standards export destination is not empty: {}",
            output.display()
        )));
    }
    fs::create_dir_all(output).map_err(|source| super::io_error(output, source))?;
    verify_nagoya_capsule(capsule_root, None)?;
    let manifest: CapsuleManifest = read_json(&capsule_root.join("capsule.json"))?;
    let receipt: ExecutionReceipt = read_json(&capsule_root.join(RECEIPT_PATH))?;
    let workflow: GeoWorkflow = read_json(&capsule_root.join(WORKFLOW_PATH))?;

    // A Workflow Run RO-Crate is a package, not a detached metadata view.
    // Copy the already verified subjects so independent validators can check
    // entity availability and consumers can move the export on its own.
    for entry in &manifest.entries {
        super::validate_relative_path(&entry.path)?;
        let destination = output.join(&entry.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| super::io_error(parent, source))?;
        }
        fs::copy(capsule_root.join(&entry.path), &destination)
            .map_err(|source| super::io_error(&destination, source))?;
    }

    let documents = [
        (PROV_PATH, prov_json(&manifest, &receipt, &workflow)),
        (RO_CRATE_PATH, ro_crate_json(&manifest, &receipt, &workflow)),
        (
            OPENLINEAGE_PATH,
            openlineage_json(&manifest, &receipt, &workflow),
        ),
        (INTOTO_PATH, intoto_statement(&manifest, &receipt)),
        (OGC_PROCESS_PATH, ogc_process_description()),
        (
            OGC_REQUEST_PATH,
            ogc_execute_request(capsule_root, &capsule_root.join(POLICY_PATH)),
        ),
    ];
    let mut files = BTreeMap::new();
    for (name, document) in &documents {
        let bytes = serde_json::to_vec_pretty(document)?;
        fs::write(output.join(name), &bytes)
            .map_err(|source| super::io_error(&output.join(name), source))?;
        files.insert((*name).into(), digest(&bytes));
    }
    let mut validations = BTreeMap::new();
    validations.insert(
        "prov-json".into(),
        validate_prov_json(&documents[0].1, &manifest).is_ok(),
    );
    validations.insert(
        "workflow-run-ro-crate".into(),
        validate_ro_crate(&documents[1].1, &manifest).is_ok(),
    );
    validations.insert(
        "openlineage".into(),
        validate_openlineage(&documents[2].1, &manifest, &receipt).is_ok(),
    );
    validations.insert(
        "in-toto-statement-v1".into(),
        validate_intoto_statement(&documents[3].1, &manifest).is_ok(),
    );
    validations.insert(
        "ogc-api-processes-1.0".into(),
        validate_ogc_documents(&documents[4].1, &documents[5].1).is_ok(),
    );
    if validations.values().any(|valid| !valid) {
        return Err(verify_error(
            "one or more standards exports failed validation",
        ));
    }
    Ok(StandardExportReport {
        output_directory: output.display().to_string(),
        files,
        validations,
    })
}

/// Validate the required PROV-JSON entities and relations against a capsule.
pub fn validate_prov_json(
    document: &Value,
    manifest: &CapsuleManifest,
) -> Result<(), CapsuleError> {
    let object = document
        .as_object()
        .ok_or_else(|| verify_error("PROV-JSON document is not an object"))?;
    for key in [
        "prefix",
        "entity",
        "activity",
        "agent",
        "used",
        "wasGeneratedBy",
        "wasAssociatedWith",
    ] {
        if !object.get(key).is_some_and(Value::is_object) {
            return Err(verify_error(format!("PROV-JSON omitted {key}")));
        }
    }
    let entities = object["entity"].as_object().expect("validated object");
    for entry in &manifest.entries {
        let id = prov_subject_id(&entry.path);
        if entities
            .get(&id)
            .and_then(|entity| entity.get("genegis:sha256"))
            .and_then(Value::as_str)
            != Some(entry.sha256.as_str())
        {
            return Err(verify_error(format!(
                "PROV-JSON subject identity missing: {}",
                entry.path
            )));
        }
    }
    Ok(())
}

/// Validate base RO-Crate and Workflow Run profile requirements used here.
pub fn validate_ro_crate(document: &Value, manifest: &CapsuleManifest) -> Result<(), CapsuleError> {
    let context = document
        .get("@context")
        .and_then(Value::as_array)
        .ok_or_else(|| verify_error("RO-Crate @context is missing"))?;
    for required in [
        "https://w3id.org/ro/crate/1.1/context",
        "https://w3id.org/ro/terms/workflow-run/context",
    ] {
        if !context.iter().any(|value| value.as_str() == Some(required)) {
            return Err(verify_error(format!("RO-Crate context missing {required}")));
        }
    }
    let graph = document
        .get("@graph")
        .and_then(Value::as_array)
        .ok_or_else(|| verify_error("RO-Crate @graph is missing"))?;
    for required_id in ["ro-crate-metadata.json", "./", "#run", WORKFLOW_PATH] {
        if !graph
            .iter()
            .any(|entity| entity.get("@id").and_then(Value::as_str) == Some(required_id))
        {
            return Err(verify_error(format!(
                "RO-Crate entity missing {required_id}"
            )));
        }
    }
    let root = graph
        .iter()
        .find(|entity| entity.get("@id").and_then(Value::as_str) == Some("./"))
        .expect("validated root");
    if root.pointer("/mainEntity/@id").and_then(Value::as_str) != Some(WORKFLOW_PATH)
        || root.get("license").is_none()
        || root.get("datePublished").is_none()
        || root.get("description").is_none()
    {
        return Err(verify_error(
            "RO-Crate root omitted workflow, license, date, or description",
        ));
    }
    for entry in &manifest.entries {
        let entity = graph
            .iter()
            .find(|entity| entity.get("@id").and_then(Value::as_str) == Some(&entry.path))
            .ok_or_else(|| verify_error(format!("RO-Crate file missing {}", entry.path)))?;
        if entity.get("sha256").and_then(Value::as_str) != entry.sha256.strip_prefix("sha256:") {
            return Err(verify_error(format!(
                "RO-Crate digest mismatch for {}",
                entry.path
            )));
        }
    }
    Ok(())
}

/// Validate OpenLineage core run/job/dataset identity preservation.
pub fn validate_openlineage(
    document: &Value,
    manifest: &CapsuleManifest,
    receipt: &ExecutionReceipt,
) -> Result<(), CapsuleError> {
    if document.get("eventType").and_then(Value::as_str) != Some("COMPLETE")
        || document.pointer("/run/runId").and_then(Value::as_str)
            != Some(receipt.command_id.to_string().as_str())
        || document.get("schemaURL").and_then(Value::as_str)
            != Some("https://openlineage.io/spec/1-0-0/OpenLineage.json#/definitions/RunEvent")
    {
        return Err(verify_error("OpenLineage core run identity is invalid"));
    }
    let outputs = document
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| verify_error("OpenLineage outputs are missing"))?;
    let artifact_count = manifest
        .entries
        .iter()
        .filter(|entry| is_standard_output_role(&entry.role))
        .count();
    if outputs.len() != artifact_count {
        return Err(verify_error("OpenLineage output dataset count mismatch"));
    }
    let inputs = document
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or_else(|| verify_error("OpenLineage inputs are missing"))?;
    if inputs.len() != receipt.source_snapshots.len()
        || document
            .pointer("/run/facets/genegis_proof/resultDigest")
            .and_then(Value::as_str)
            != Some(receipt.result_digest.as_str())
        || document
            .pointer("/run/facets/genegis_proof/workflowDigest")
            .and_then(Value::as_str)
            != Some(receipt.workflow_digest.as_str())
    {
        return Err(verify_error(
            "OpenLineage source or proof identities were not preserved",
        ));
    }
    for output in outputs {
        let name = output
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| verify_error("OpenLineage output name is missing"))?;
        let expected = manifest
            .entries
            .iter()
            .find(|entry| is_standard_output_role(&entry.role) && entry.path == name)
            .map(|entry| entry.sha256.as_str());
        if output
            .pointer("/facets/genegis_artifact/sha256")
            .and_then(Value::as_str)
            != expected
        {
            return Err(verify_error(format!(
                "OpenLineage artifact identity mismatch: {name}"
            )));
        }
    }
    Ok(())
}

/// Create an Ed25519-signed DSSE envelope containing the capsule's in-toto
/// Statement. The 32-byte signing key remains caller-owned.
pub fn create_dsse_attestation(
    capsule_root: impl AsRef<Path>,
    signing_key: &[u8; 32],
    keyid: impl Into<String>,
) -> Result<DsseEnvelope, CapsuleError> {
    let manifest: CapsuleManifest = read_json(&capsule_root.as_ref().join("capsule.json"))?;
    let receipt: ExecutionReceipt = read_json(&capsule_root.as_ref().join(RECEIPT_PATH))?;
    let statement = intoto_statement(&manifest, &receipt);
    validate_intoto_statement(&statement, &manifest)?;
    let payload = serde_json::to_vec(&statement)?;
    let payload_type = "application/vnd.in-toto+json";
    let signature = SigningKey::from_bytes(signing_key).sign(&dsse_pae(payload_type, &payload));
    Ok(DsseEnvelope {
        payload_type: payload_type.into(),
        payload: STANDARD.encode(payload),
        signatures: vec![DsseSignature {
            keyid: keyid.into(),
            sig: STANDARD.encode(signature.to_bytes()),
        }],
    })
}

/// Verify a DSSE Ed25519 signature and all in-toto subject digests offline.
pub fn verify_dsse_attestation(
    capsule_root: impl AsRef<Path>,
    envelope: &DsseEnvelope,
    verifying_key: &[u8; 32],
) -> Result<Value, CapsuleError> {
    if envelope.payload_type != "application/vnd.in-toto+json" || envelope.signatures.is_empty() {
        return Err(verify_error("unsupported or unsigned DSSE envelope"));
    }
    let payload = STANDARD
        .decode(&envelope.payload)
        .map_err(|_| verify_error("invalid DSSE payload base64"))?;
    let key = VerifyingKey::from_bytes(verifying_key)
        .map_err(|_| verify_error("invalid Ed25519 verifying key"))?;
    let pae = dsse_pae(&envelope.payload_type, &payload);
    let mut verified = false;
    for candidate in &envelope.signatures {
        let bytes = STANDARD
            .decode(&candidate.sig)
            .map_err(|_| verify_error("invalid DSSE signature base64"))?;
        let signature = Signature::from_slice(&bytes)
            .map_err(|_| verify_error("invalid Ed25519 signature bytes"))?;
        verified |= key.verify(&pae, &signature).is_ok();
    }
    if !verified {
        return Err(verify_error("DSSE signature did not verify"));
    }
    let statement: Value = serde_json::from_slice(&payload)?;
    let manifest: CapsuleManifest = read_json(&capsule_root.as_ref().join("capsule.json"))?;
    validate_intoto_statement(&statement, &manifest)?;
    Ok(statement)
}

/// Execute the local OGC API - Processes verification fixture without HTTP.
pub fn execute_ogc_verify_request(request: &Value) -> Result<Value, CapsuleError> {
    validate_ogc_documents(&ogc_process_description(), request)?;
    let capsule = request
        .pointer("/inputs/capsule")
        .and_then(Value::as_str)
        .ok_or_else(|| verify_error("OGC execute request omitted inputs.capsule"))?;
    let policy_path = request.pointer("/inputs/policy").and_then(Value::as_str);
    let policy = policy_path
        .map(|path| read_json::<VerificationPolicy>(Path::new(path)))
        .transpose()?;
    let verification = verify_nagoya_capsule(capsule, policy.as_ref())?;
    Ok(json!({"verification": verification}))
}

fn prov_json(
    manifest: &CapsuleManifest,
    receipt: &ExecutionReceipt,
    workflow: &GeoWorkflow,
) -> Value {
    let mut entities = Map::new();
    for entry in &manifest.entries {
        entities.insert(
            prov_subject_id(&entry.path),
            json!({
                "prov:type": "genegis:CapsuleSubject",
                "prov:label": entry.path,
                "prov:location": entry.path,
                "genegis:role": entry.role,
                "genegis:mediaType": entry.media_type,
                "genegis:sha256": entry.sha256,
                "genegis:bytes": entry.bytes
            }),
        );
    }
    entities.insert(
        "genegis:result".into(),
        json!({
            "prov:type": "genegis:ProofCarryingSpatialResult",
            "genegis:resultDigest": manifest.subject_result_digest,
            "genegis:trustLevel": receipt.trust_assessment.as_ref().map(|trust| format!("{:?}", trust.level).to_lowercase())
        }),
    );
    let generated = manifest
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                format!("_:generation{index}"),
                json!({"prov:entity": prov_subject_id(&entry.path), "prov:activity": "genegis:run"}),
            )
        })
        .collect::<Map<_, _>>();
    let sources = receipt
        .source_snapshots
        .iter()
        .enumerate()
        .map(|(index, source)| {
            (
                format!("genegis:source{index}"),
                json!({"prov:type": "prov:Entity", "prov:location": source.uri, "genegis:sha256": source.checksum}),
            )
        })
        .collect::<Map<_, _>>();
    entities.extend(sources);
    let used = receipt
        .source_snapshots
        .iter()
        .enumerate()
        .map(|(index, _)| {
            (
                format!("_:usage{index}"),
                json!({"prov:activity": "genegis:run", "prov:entity": format!("genegis:source{index}")}),
            )
        })
        .collect::<Map<_, _>>();
    json!({
        "prefix": {"prov": "http://www.w3.org/ns/prov#", "xsd": "http://www.w3.org/2001/XMLSchema#", "genegis": GENEGIS_NS},
        "entity": entities,
        "activity": {"genegis:run": {"prov:type": "genegis:WorkflowRun", "prov:label": workflow.goal, "genegis:workflowDigest": receipt.workflow_digest.as_str()}},
        "agent": {"genegis:engine": {"prov:type": "prov:SoftwareAgent", "prov:label": receipt.engine.name, "genegis:version": receipt.engine.version}},
        "used": used,
        "wasGeneratedBy": generated,
        "wasAssociatedWith": {"_:association0": {"prov:activity": "genegis:run", "prov:agent": "genegis:engine"}},
        "wasDerivedFrom": {"_:derivation0": {"prov:generatedEntity": "genegis:result", "prov:usedEntity": "genegis:source0"}}
    })
}

fn ro_crate_json(
    manifest: &CapsuleManifest,
    receipt: &ExecutionReceipt,
    workflow: &GeoWorkflow,
) -> Value {
    let mut graph = vec![
        json!({
            "@id": "ro-crate-metadata.json",
            "@type": "CreativeWork",
            "about": {"@id": "./"},
            "conformsTo": {"@id": "https://w3id.org/ro/crate/1.1"}
        }),
        json!({
            "@id": "./",
            "@type": "Dataset",
            "name": "GeneGIS proof-carrying spatial analysis capsule",
            "conformsTo": [{"@id": "https://w3id.org/ro/wfrun/workflow/0.5"}],
            "mainEntity": {"@id": WORKFLOW_PATH},
            "description": "Portable proof-carrying spatial analysis with executable workflow, source identities, verification evidence, and result artifacts.",
            "license": "Apache-2.0 OR MIT; source dataset licenses are recorded on source entities",
            "datePublished": receipt.command_timestamp.to_rfc3339(),
            "hasPart": manifest.entries.iter().map(|entry| json!({"@id": entry.path})).collect::<Vec<_>>(),
            "mentions": {"@id": "#run"},
            "genegis:resultDigest": manifest.subject_result_digest
        }),
        json!({
            "@id": WORKFLOW_PATH,
            "@type": ["File", "SoftwareSourceCode", "ComputationalWorkflow"],
            "name": workflow.goal,
            "programmingLanguage": {"@id": "https://genegis.org/runtime/command-workflow-graph/0.1"},
            "input": receipt.source_snapshots.iter().enumerate().map(|(index, _)| json!({"@id": format!("#input-{index}")})).collect::<Vec<_>>(),
            "output": manifest.entries.iter().filter(|entry| is_standard_output_role(&entry.role)).enumerate().map(|(index, _)| json!({"@id": format!("#output-{index}")})).collect::<Vec<_>>(),
            "sha256": manifest.entries.iter().find(|entry| entry.path == WORKFLOW_PATH).and_then(|entry| entry.sha256.strip_prefix("sha256:"))
        }),
        json!({
            "@id": "#engine",
            "@type": "SoftwareApplication",
            "name": receipt.engine.name,
            "softwareVersion": receipt.engine.version
        }),
        json!({
            "@id": "#run",
            "@type": "CreateAction",
            "name": "GeneGIS verified workflow run",
            "instrument": {"@id": WORKFLOW_PATH},
            "agent": {"@id": "#engine"},
            "object": receipt.source_snapshots.iter().enumerate().map(|(index, _)| json!({"@id": format!("#source-{index}")})).collect::<Vec<_>>(),
            "result": manifest.entries.iter().filter(|entry| is_standard_output_role(&entry.role)).map(|entry| json!({"@id": entry.path})).collect::<Vec<_>>(),
            "startTime": receipt.command_timestamp.to_rfc3339(),
            "endTime": receipt.command_timestamp.to_rfc3339(),
            "actionStatus": {"@id": "http://schema.org/CompletedActionStatus"},
            "genegis:trustLevel": receipt.trust_assessment.as_ref().map(|trust| format!("{:?}", trust.level).to_lowercase()),
            "genegis:workflowDigest": receipt.workflow_digest.as_str(),
            "genegis:verificationGraphDigest": receipt.verification_graph_digest
        }),
        json!({
            "@id": "https://w3id.org/ro/wfrun/workflow/0.5",
            "@type": "CreativeWork",
            "name": "Workflow Run Crate profile 0.5"
        }),
        json!({
            "@id": "https://w3id.org/workflowhub/workflow-ro-crate/1.0",
            "@type": "CreativeWork",
            "name": "Workflow RO-Crate profile 1.0"
        }),
        json!({
            "@id": "https://genegis.org/runtime/command-workflow-graph/0.1",
            "@type": "ComputerLanguage",
            "name": "GeneGIS Command + Workflow Graph",
            "version": "0.1"
        }),
    ];
    for entry in &manifest.entries {
        graph.push(json!({
            "@id": entry.path,
            "@type": "File",
            "name": entry.path,
            "encodingFormat": entry.media_type,
            "contentSize": entry.bytes.to_string(),
            "sha256": entry.sha256.strip_prefix("sha256:")
        }));
    }
    for (index, source) in receipt.source_snapshots.iter().enumerate() {
        graph.push(json!({
            "@id": format!("#source-{index}"),
            "@type": "Dataset",
            "name": source.dataset_id,
            "url": source.uri,
            "version": source.source_version,
            "license": source.license,
            "sha256": source.checksum.as_deref().and_then(|digest| digest.strip_prefix("sha256:"))
        }));
        graph.push(json!({
            "@id": format!("#input-{index}"),
            "@type": "FormalParameter",
            "name": source.dataset_id.as_deref().unwrap_or("spatial source"),
            "additionalType": "Dataset",
            "workExample": {"@id": format!("#source-{index}")}
        }));
    }
    for (index, entry) in manifest
        .entries
        .iter()
        .filter(|entry| is_standard_output_role(&entry.role))
        .enumerate()
    {
        graph.push(json!({
            "@id": format!("#output-{index}"),
            "@type": "FormalParameter",
            "name": entry.role,
            "additionalType": "File",
            "workExample": {"@id": entry.path}
        }));
    }
    json!({
        "@context": [
            "https://w3id.org/ro/crate/1.1/context",
            "https://w3id.org/ro/terms/workflow-run/context",
            {"genegis": GENEGIS_NS}
        ],
        "@graph": graph
    })
}

fn openlineage_json(
    manifest: &CapsuleManifest,
    receipt: &ExecutionReceipt,
    workflow: &GeoWorkflow,
) -> Value {
    let inputs = receipt
        .source_snapshots
        .iter()
        .map(|source| {
            json!({
                "namespace": "genegis:source",
                "name": source.uri,
                "facets": {"genegis_source": custom_facet(json!({"checksum": source.checksum, "sourceVersion": source.source_version, "license": source.license}))}
            })
        })
        .collect::<Vec<_>>();
    let outputs = manifest
        .entries
        .iter()
        .filter(|entry| is_standard_output_role(&entry.role))
        .map(|entry| {
            json!({
                "namespace": "genegis:capsule",
                "name": entry.path,
                "facets": {"genegis_artifact": custom_facet(json!({"sha256": entry.sha256, "mediaType": entry.media_type, "bytes": entry.bytes, "role": entry.role}))}
            })
        })
        .collect::<Vec<_>>();
    json!({
        "eventType": "COMPLETE",
        "eventTime": receipt.command_timestamp.to_rfc3339(),
        "run": {"runId": receipt.command_id.to_string(), "facets": {"genegis_proof": custom_facet(json!({"resultDigest": receipt.result_digest, "workflowDigest": receipt.workflow_digest.as_str(), "verificationGraphDigest": receipt.verification_graph_digest, "trust": receipt.trust_assessment.as_ref().map(|trust| format!("{:?}", trust.level).to_lowercase())}))}},
        "job": {"namespace": "https://genegis.org/workflows", "name": workflow.goal, "facets": {}},
        "inputs": inputs,
        "outputs": outputs,
        "producer": "https://github.com/genegis/genegis",
        "schemaURL": "https://openlineage.io/spec/1-0-0/OpenLineage.json#/definitions/RunEvent"
    })
}

fn custom_facet(fields: Value) -> Value {
    let mut object = fields.as_object().cloned().unwrap_or_default();
    object.insert(
        "_producer".into(),
        json!("https://github.com/genegis/genegis"),
    );
    object.insert(
        "_schemaURL".into(),
        json!("https://genegis.org/schemas/openlineage/proof-carrying-spatial-analysis/0.1.json"),
    );
    Value::Object(object)
}

fn is_standard_output_role(role: &str) -> bool {
    role == "map-artifact"
        || role == "analysis-result"
        || super::operational_kind_for_role(role).is_some()
}

fn intoto_statement(manifest: &CapsuleManifest, receipt: &ExecutionReceipt) -> Value {
    let subjects = manifest
        .entries
        .iter()
        .filter(|entry| is_standard_output_role(&entry.role))
        .map(|entry| {
            json!({"name": entry.path, "digest": {"sha256": entry.sha256.strip_prefix("sha256:")}})
        })
        .collect::<Vec<_>>();
    json!({
        "_type": "https://in-toto.io/Statement/v1",
        "subject": subjects,
        "predicateType": "https://genegis.org/attestation/proof-carrying-spatial-analysis/v0.1",
        "predicate": {
            "policy": receipt.verification_policy.as_ref().map(|policy| policy.policy_id.clone()),
            "trust": receipt.trust_assessment.as_ref().map(|trust| format!("{:?}", trust.level).to_lowercase()),
            "resultDigest": receipt.result_digest,
            "workflowDigest": receipt.workflow_digest.as_str(),
            "verificationGraphDigest": receipt.verification_graph_digest,
            "integrityClaim": "Signature authenticates this statement and subject digests; it does not independently prove spatial truth."
        }
    })
}

fn validate_intoto_statement(
    statement: &Value,
    manifest: &CapsuleManifest,
) -> Result<(), CapsuleError> {
    if statement.get("_type").and_then(Value::as_str) != Some("https://in-toto.io/Statement/v1")
        || statement.get("predicateType").and_then(Value::as_str)
            != Some("https://genegis.org/attestation/proof-carrying-spatial-analysis/v0.1")
    {
        return Err(verify_error("in-toto Statement type is invalid"));
    }
    let subjects = statement
        .get("subject")
        .and_then(Value::as_array)
        .ok_or_else(|| verify_error("in-toto subjects are missing"))?;
    let expected_subjects = manifest
        .entries
        .iter()
        .filter(|entry| is_standard_output_role(&entry.role))
        .count();
    if subjects.len() != expected_subjects {
        return Err(verify_error("in-toto subject count mismatch"));
    }
    for subject in subjects {
        let name = subject
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| verify_error("in-toto subject name is missing"))?;
        let expected = manifest
            .entries
            .iter()
            .find(|entry| entry.path == name)
            .and_then(|entry| entry.sha256.strip_prefix("sha256:"));
        if subject.pointer("/digest/sha256").and_then(Value::as_str) != expected {
            return Err(verify_error(format!(
                "in-toto subject digest mismatch: {name}"
            )));
        }
    }
    if subjects.is_empty() {
        return Err(verify_error("in-toto Statement has no subjects"));
    }
    Ok(())
}

fn ogc_process_description() -> Value {
    json!({
        "id": "genegis-capsule-verify",
        "title": "Verify a GeneGIS proof-carrying spatial result capsule",
        "version": "0.1.0",
        "description": "Offline digest, policy, verification graph, and trust validation",
        "jobControlOptions": ["sync-execute"],
        "outputTransmission": ["value"],
        "inputs": {
            "capsule": {"title": "Capsule directory", "schema": {"type": "string"}, "minOccurs": 1, "maxOccurs": 1},
            "policy": {"title": "External verification policy", "schema": {"type": "string"}, "minOccurs": 1, "maxOccurs": 1}
        },
        "outputs": {
            "verification": {"title": "Capsule verification report", "schema": {"type": "object"}}
        },
        "links": [{"href": "http://localhost/processes/genegis-capsule-verify/execution", "rel": "http://www.opengis.net/def/rel/ogc/1.0/execute", "type": "application/json"}]
    })
}

fn ogc_execute_request(capsule: &Path, policy: &Path) -> Value {
    json!({
        "inputs": {"capsule": capsule.display().to_string(), "policy": policy.display().to_string()},
        "response": "document",
        "mode": "sync"
    })
}

fn validate_ogc_documents(description: &Value, request: &Value) -> Result<(), CapsuleError> {
    if description.get("id").and_then(Value::as_str) != Some("genegis-capsule-verify")
        || !description
            .get("jobControlOptions")
            .and_then(Value::as_array)
            .is_some_and(|options| options.iter().any(|value| value == "sync-execute"))
        || request
            .pointer("/inputs/capsule")
            .and_then(Value::as_str)
            .is_none()
        || request
            .pointer("/inputs/policy")
            .and_then(Value::as_str)
            .is_none()
        || request.get("mode").and_then(Value::as_str) != Some("sync")
    {
        return Err(verify_error("OGC API - Processes fixture is invalid"));
    }
    Ok(())
}

fn prov_subject_id(path: &str) -> String {
    format!(
        "genegis:subject/{}",
        path.replace('%', "%25").replace('/', "%2F")
    )
}

fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    format!(
        "DSSEv1 {} {} {} ",
        payload_type.len(),
        payload_type,
        payload.len()
    )
    .bytes()
    .chain(payload.iter().copied())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        seal_nagoya_capsule, seal_operational_capsule, OperationalEvidence, OperationalEvidenceKind,
    };
    use genegis_analysis::run_ask_pipeline;
    use genegis_contract::TrustLevel;
    use std::path::PathBuf;

    fn capsule() -> (tempfile::TempDir, PathBuf) {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("capsule");
        seal_nagoya_capsule(&result, &root).expect("capsule");
        (temporary, root)
    }

    fn operational_capsule() -> (tempfile::TempDir, PathBuf) {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("operational-capsule");
        let fixtures = [
            (
                "adapter",
                OperationalEvidenceKind::Adapter,
                json!({"manifest_digest": digest(b"manifest"), "operation_id": "typed.operation", "backend": {"family": "fixture"}, "parameters": {}, "output_digest": digest(b"output"), "elapsed_ns": 100}),
            ),
            (
                "edit",
                OperationalEvidenceKind::Edit,
                json!({"before_revision": 1, "after_revision": 2, "changed_features": ["feature-1"]}),
            ),
            (
                "io",
                OperationalEvidenceKind::Io,
                json!({"schema_version": "0.1.0", "format": "cog", "object_digest": digest(b"object"), "requests": [], "transferred_bytes": 0, "whole_object_fallback": false, "peak_rss_bytes": 1024}),
            ),
            (
                "trust-ux",
                OperationalEvidenceKind::TrustUx,
                json!({"schema_version": "0.1.0", "corpus_version": "phase-12-map-first-trust-v1", "corpus_digest": digest(b"corpus"), "admitted_human_reviewers": 3, "admitted_tasks": 36, "correctness": 1.0, "median_diagnosis_seconds": 30.0, "median_interactions_to_decisive_evidence": 2.0, "passed": true}),
            ),
        ];
        let evidence = fixtures
            .into_iter()
            .map(|(id, kind, payload)| {
                OperationalEvidence::new(id, kind, "standards-test", &payload)
                    .expect("operational evidence")
            })
            .collect::<Vec<_>>();
        seal_operational_capsule(&result, &root, &evidence).expect("operational capsule");
        (temporary, root)
    }

    #[test]
    fn exports_and_validates_all_standard_views() {
        let (temporary, root) = capsule();
        let output = temporary.path().join("standards");
        let report = export_standard_bundle(&root, &output).expect("standards export");
        assert_eq!(report.files.len(), 6);
        assert_eq!(report.validations.len(), 5);
        assert!(report.validations.values().all(|valid| *valid));

        let manifest: CapsuleManifest = read_json(&root.join("capsule.json")).unwrap();
        let receipt: ExecutionReceipt = read_json(&root.join(RECEIPT_PATH)).unwrap();
        let prov: Value = read_json(&output.join(PROV_PATH)).unwrap();
        let ro_crate: Value = read_json(&output.join(RO_CRATE_PATH)).unwrap();
        let lineage: Value = read_json(&output.join(OPENLINEAGE_PATH)).unwrap();
        validate_prov_json(&prov, &manifest).unwrap();
        validate_ro_crate(&ro_crate, &manifest).unwrap();
        validate_openlineage(&lineage, &manifest, &receipt).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&serde_json::to_vec(&lineage).unwrap()).unwrap(),
            lineage,
            "OpenLineage round-trip"
        );

        let request: Value = read_json(&output.join(OGC_REQUEST_PATH)).unwrap();
        let result = execute_ogc_verify_request(&request).expect("local OGC process execution");
        assert_eq!(result["verification"]["trust"]["level"], "verified");
    }

    #[test]
    fn operational_evidence_survives_all_standard_projections() {
        let (temporary, root) = operational_capsule();
        let output = temporary.path().join("operational-standards");
        export_standard_bundle(&root, &output).expect("standards export");

        let manifest: CapsuleManifest = read_json(&root.join("capsule.json")).unwrap();
        let operational_paths = manifest
            .entries
            .iter()
            .filter(|entry| super::super::operational_kind_for_role(&entry.role).is_some())
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(operational_paths.len(), 4);
        assert!(operational_paths
            .iter()
            .all(|path| output.join(path).is_file()));

        let ro_crate: Value = read_json(&output.join(RO_CRATE_PATH)).unwrap();
        let lineage: Value = read_json(&output.join(OPENLINEAGE_PATH)).unwrap();
        let statement: Value = read_json(&output.join(INTOTO_PATH)).unwrap();
        for path in operational_paths {
            assert!(ro_crate["@graph"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entity| entity["@id"] == path));
            assert!(lineage["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|output| output["name"] == path));
            assert!(statement["subject"]
                .as_array()
                .unwrap()
                .iter()
                .any(|subject| subject["name"] == path));
        }
    }

    #[test]
    fn dsse_attestation_verifies_subjects_and_rejects_tamper() {
        let (_temporary, root) = capsule();
        let secret = [7_u8; 32];
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let envelope =
            create_dsse_attestation(&root, &secret, "test-ed25519").expect("attestation");
        let statement =
            verify_dsse_attestation(&root, &envelope, &public).expect("offline signature verify");
        assert_eq!(statement["_type"], "https://in-toto.io/Statement/v1");

        let mut tampered = envelope.clone();
        tampered.payload.push('A');
        assert!(verify_dsse_attestation(&root, &tampered, &public).is_err());

        let wrong_public = SigningKey::from_bytes(&[8_u8; 32])
            .verifying_key()
            .to_bytes();
        assert!(verify_dsse_attestation(&root, &envelope, &wrong_public).is_err());

        let verification = verify_nagoya_capsule(&root, None).unwrap();
        assert_eq!(verification.trust.level, TrustLevel::Verified);
    }
}
