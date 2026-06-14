/// Extract a catalog/STAC asset URL or repo-relative path from a natural-language prompt.
pub fn extract_catalog_url(prompt: &str) -> Option<String> {
    for token in prompt.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '「' | '」' | ',' | '.' | ';' | ':' | ')' | '(' | ']' | '['
            )
        });
        if cleaned.starts_with("http://")
            || cleaned.starts_with("https://")
            || cleaned.starts_with("file://")
            || cleaned.starts_with("examples/")
        {
            return Some(cleaned.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_repo_relative_path() {
        let url = extract_catalog_url("外部STAC examples/stac/sample-collection.json を取得");
        assert_eq!(url.as_deref(), Some("examples/stac/sample-collection.json"));
    }

    #[test]
    fn extracts_https_url() {
        let url = extract_catalog_url("fetch https://example.com/collection.json");
        assert_eq!(url.as_deref(), Some("https://example.com/collection.json"));
    }
}
