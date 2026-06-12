use crate::catalog::Catalog;
use crate::dataset::DatasetRecord;
use crate::error::CatalogError;

/// Result of matching a catalog entry to planner intent tags.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogMatch {
    pub dataset_id: String,
    pub score: f32,
    pub matched_tags: Vec<String>,
}

impl Catalog {
    /// Find the best catalog entry whose tags contain all `required_tags`.
    pub fn match_dataset(&self, required_tags: &[&str]) -> Result<CatalogMatch, CatalogError> {
        if required_tags.is_empty() {
            return Err(CatalogError::NoMatch(Vec::new()));
        }

        let mut matches: Vec<CatalogMatch> = self
            .list()
            .into_iter()
            .filter_map(|record| score_record(record, required_tags))
            .collect();

        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.dataset_id.cmp(&b.dataset_id))
        });

        match matches.as_slice() {
            [] => Err(CatalogError::NoMatch(
                required_tags.iter().map(|tag| (*tag).to_string()).collect(),
            )),
            [single] => Ok(single.clone()),
            [best, second, ..] if best.score > second.score => Ok(best.clone()),
            multiple => Err(CatalogError::AmbiguousMatch(
                multiple.iter().map(|m| m.dataset_id.clone()).collect(),
            )),
        }
    }
}

fn score_record(record: &DatasetRecord, required_tags: &[&str]) -> Option<CatalogMatch> {
    let matched_tags: Vec<String> = required_tags
        .iter()
        .filter(|tag| record.tags.iter().any(|candidate| candidate == *tag))
        .map(|tag| (*tag).to_string())
        .collect();

    if matched_tags.len() != required_tags.len() {
        return None;
    }

    Some(CatalogMatch {
        dataset_id: record.id.clone(),
        score: matched_tags.len() as f32 / required_tags.len() as f32,
        matched_tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{alpha_catalog, NAGOYA_WARDS_DENSITY_ID, REMOTE_COG_DEMO_ID};

    #[test]
    fn matches_nagoya_density_tags() {
        let catalog = alpha_catalog();
        let matched = catalog
            .match_dataset(&["nagoya", "density"])
            .expect("match");
        assert_eq!(matched.dataset_id, NAGOYA_WARDS_DENSITY_ID);
        assert_eq!(matched.score, 1.0);
    }

    #[test]
    fn matches_remote_cog_demo_tags() {
        let catalog = alpha_catalog();
        let matched = catalog
            .match_dataset(&["cog", "remote", "demo"])
            .expect("match");
        assert_eq!(matched.dataset_id, REMOTE_COG_DEMO_ID);
    }

    #[test]
    fn rejects_unknown_tag_set() {
        let catalog = alpha_catalog();
        let err = catalog.match_dataset(&["tokyo", "density"]).unwrap_err();
        assert!(matches!(err, CatalogError::NoMatch(_)));
    }
}
