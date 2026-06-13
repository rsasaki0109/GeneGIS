use genegis_catalog::{alpha_catalog, Catalog, CatalogError, CatalogMatch};
use serde::{Deserialize, Serialize};

use crate::error::AiError;
use crate::intent::ParsedIntent;

/// Known MVP workflow identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowId {
    NagoyaDensity,
    RemoteCogDemo,
    LocalCogDemo,
    NagoyaGeoparquet,
}

impl WorkflowId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NagoyaDensity => "nagoya-density",
            Self::RemoteCogDemo => "remote-cog-demo",
            Self::LocalCogDemo => "local-cog-demo",
            Self::NagoyaGeoparquet => "nagoya-geoparquet",
        }
    }

    pub fn dataset_tags(self) -> &'static [&'static str] {
        match self {
            Self::NagoyaDensity => &["nagoya", "density"],
            Self::RemoteCogDemo => &["cog", "remote", "demo"],
            Self::LocalCogDemo => &["cog", "local", "demo"],
            Self::NagoyaGeoparquet => &["nagoya", "geoparquet", "demo"],
        }
    }
}

/// Binding between parsed intent and executable workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedWorkflow {
    pub workflow_id: WorkflowId,
    pub dataset_id: String,
    pub goal: String,
    pub confidence: f32,
    pub rationale: Vec<String>,
    pub ambiguities: Vec<String>,
}

pub fn resolve_workflow(intent: &ParsedIntent) -> Result<ResolvedWorkflow, AiError> {
    resolve_workflow_with_catalog(intent, &alpha_catalog())
}

pub fn resolve_workflow_with_catalog(
    intent: &ParsedIntent,
    catalog: &Catalog,
) -> Result<ResolvedWorkflow, AiError> {
    if intent.raw_prompt.is_empty() {
        return Err(AiError::EmptyPrompt);
    }

    let nagoya = intent.signals.place.as_deref() == Some("名古屋市");
    let density = intent.signals.metric.as_deref() == Some("population_density");
    let remote_cog = intent.signals.metric.as_deref() == Some("remote_cog");
    let local_cog = intent.signals.metric.as_deref() == Some("local_cog");
    let geoparquet = intent.signals.metric.as_deref() == Some("geoparquet");

    if nagoya && geoparquet {
        let mut resolved = ResolvedWorkflow {
            workflow_id: WorkflowId::NagoyaGeoparquet,
            dataset_id: String::new(),
            goal: intent.raw_prompt.clone(),
            confidence: intent.confidence,
            rationale: intent.signals.matched_tokens.clone(),
            ambiguities: vec![
                "GeoParquet read + feature-count verification only (Phase 9 alpha)".into(),
                "Expected 16 Nagoya wards in bundled fixture".into(),
            ],
        };
        bind_catalog_dataset(catalog, &mut resolved)?;
        return Ok(resolved);
    }

    if nagoya && density {
        let mut resolved = ResolvedWorkflow {
            workflow_id: WorkflowId::NagoyaDensity,
            dataset_id: String::new(),
            goal: intent.raw_prompt.clone(),
            confidence: intent.confidence,
            rationale: intent.signals.matched_tokens.clone(),
            ambiguities: default_ambiguities(),
        };
        bind_catalog_dataset(catalog, &mut resolved)?;
        return Ok(resolved);
    }

    if remote_cog {
        let mut resolved = ResolvedWorkflow {
            workflow_id: WorkflowId::RemoteCogDemo,
            dataset_id: String::new(),
            goal: intent.raw_prompt.clone(),
            confidence: intent.confidence,
            rationale: intent.signals.matched_tokens.clone(),
            ambiguities: vec![
                "Execution is metadata-only in Phase 4 alpha".into(),
                "Asset URI comes from catalog registry".into(),
            ],
        };
        bind_catalog_dataset(catalog, &mut resolved)?;
        return Ok(resolved);
    }

    if local_cog {
        let mut resolved = ResolvedWorkflow {
            workflow_id: WorkflowId::LocalCogDemo,
            dataset_id: String::new(),
            goal: intent.raw_prompt.clone(),
            confidence: intent.confidence,
            rationale: intent.signals.matched_tokens.clone(),
            ambiguities: vec![
                "Execution reads bundled smoke GeoTIFF fixture".into(),
                "Offline metadata verify only (no map export)".into(),
            ],
        };
        bind_catalog_dataset(catalog, &mut resolved)?;
        return Ok(resolved);
    }

    if nagoya && intent.signals.metric.is_none() {
        return Err(AiError::Ambiguous(
            "名古屋市は認識しましたが、指標が不明です（例: 人口密度）".into(),
        ));
    }

    if density && intent.signals.place.is_none() {
        return Err(AiError::Ambiguous(
            "人口密度は認識しましたが、対象地域が不明です（例: 名古屋市）".into(),
        ));
    }

    Err(AiError::Unresolved(format!(
        "未対応のプロンプトです: \"{}\"",
        intent.raw_prompt
    )))
}

pub fn bind_catalog_dataset(
    catalog: &Catalog,
    resolved: &mut ResolvedWorkflow,
) -> Result<CatalogMatch, AiError> {
    let matched = catalog
        .match_dataset(resolved.workflow_id.dataset_tags())
        .map_err(map_catalog_error)?;

    resolved.dataset_id = matched.dataset_id.clone();
    resolved
        .rationale
        .push(format!("catalog:{}", matched.dataset_id));
    Ok(matched)
}

fn map_catalog_error(err: CatalogError) -> AiError {
    match err {
        CatalogError::NotFound(id) => AiError::Unresolved(format!("catalog dataset not found: {id}")),
        CatalogError::NoMatch(tags) => {
            AiError::Unresolved(format!("no catalog dataset matches tags: {tags:?}"))
        }
        CatalogError::AmbiguousMatch(ids) => {
            AiError::Ambiguous(format!("multiple catalog datasets match: {ids:?}"))
        }
        CatalogError::Remote(msg) | CatalogError::InvalidStac(msg) => {
            AiError::Unresolved(format!("catalog error: {msg}"))
        }
    }
}

fn default_ambiguities() -> Vec<String> {
    vec![
        "行政区域粒度: ward（区） — MVPデモは16区".into(),
        "統計年: 2020年国勢調査".into(),
        "boundary: 国土数値情報 N03 行政区域 (JapanCityGeoJson)".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::ParsedIntent;
    use genegis_catalog::{LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID, NAGOYA_WARDS_GEOPARQUET_ID, REMOTE_COG_DEMO_ID};

    #[test]
    fn resolves_nagoya_density() {
        let intent = ParsedIntent::parse("名古屋市の人口密度を表示");
        let resolved = resolve_workflow(&intent).expect("resolve");
        assert_eq!(resolved.workflow_id, WorkflowId::NagoyaDensity);
        assert_eq!(resolved.dataset_id, NAGOYA_WARDS_DENSITY_ID);
        assert!(resolved
            .rationale
            .iter()
            .any(|token| token.starts_with("catalog:")));
    }

    #[test]
    fn resolves_remote_cog_demo() {
        let intent = ParsedIntent::parse("リモートCOGデモのメタデータを表示");
        let resolved = resolve_workflow(&intent).expect("resolve");
        assert_eq!(resolved.workflow_id, WorkflowId::RemoteCogDemo);
        assert_eq!(resolved.dataset_id, REMOTE_COG_DEMO_ID);
    }

    #[test]
    fn resolves_local_cog_demo() {
        let intent = ParsedIntent::parse("ローカルCOGデモのメタデータを表示");
        let resolved = resolve_workflow(&intent).expect("resolve");
        assert_eq!(resolved.workflow_id, WorkflowId::LocalCogDemo);
        assert_eq!(resolved.dataset_id, LOCAL_COG_DEMO_ID);
    }

    #[test]
    fn resolves_nagoya_geoparquet() {
        let intent = ParsedIntent::parse("名古屋 wards GeoParquet を検証");
        let resolved = resolve_workflow(&intent).expect("resolve");
        assert_eq!(resolved.workflow_id, WorkflowId::NagoyaGeoparquet);
        assert_eq!(resolved.dataset_id, NAGOYA_WARDS_GEOPARQUET_ID);
    }
}
