#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

pub fn normalize_key(k: &str) -> String {
    k.chars()
        .filter(|c| *c != '_' && *c != '-' && *c != '/')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn resolve_key<'a>(
    entries: &'a [MetadataEntry],
    query: &str,
) -> Result<&'a MetadataEntry, String> {
    let norm_query = normalize_key(query);

    // 1. Exact full-path match (case-insensitive)
    if let Some(entry) = entries.iter().find(|e| e.key.eq_ignore_ascii_case(query)) {
        return Ok(entry);
    }

    // 2. Exact leaf-name match (after '/')
    let leaf_matches: Vec<&MetadataEntry> = entries
        .iter()
        .filter(|e| {
            let leaf = e.key.rsplit('/').next().unwrap_or(&e.key);
            leaf.eq_ignore_ascii_case(query)
        })
        .collect();

    if leaf_matches.len() == 1 {
        return Ok(leaf_matches[0]);
    } else if leaf_matches.len() > 1 {
        let keys: Vec<String> = leaf_matches.iter().map(|e| format!("  - {}", e.key)).collect();
        return Err(format!(
            "Ambiguous key '{}'. Matches multiple entries:\n{}",
            query,
            keys.join("\n")
        ));
    }

    // 3. Normalized match (ignores case, '-', and '_')
    let norm_leaf_matches: Vec<&MetadataEntry> = entries
        .iter()
        .filter(|e| {
            let leaf = e.key.rsplit('/').next().unwrap_or(&e.key);
            normalize_key(leaf) == norm_query || normalize_key(&e.key) == norm_query
        })
        .collect();

    if norm_leaf_matches.len() == 1 {
        return Ok(norm_leaf_matches[0]);
    } else if norm_leaf_matches.len() > 1 {
        let keys: Vec<String> = norm_leaf_matches.iter().map(|e| format!("  - {}", e.key)).collect();
        return Err(format!(
            "Ambiguous key '{}'. Matches multiple entries:\n{}",
            query,
            keys.join("\n")
        ));
    }

    let available: Vec<String> = entries.iter().map(|e| format!("  - {}", e.key)).collect();
    Err(format!(
        "Key '{}' not found. Available keys:\n{}",
        query,
        available.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_key_exact_and_leaf() {
        let entries = vec![
            MetadataEntry {
                key: "Description/source_code".to_string(),
                value: "x = 42".to_string(),
            },
            MetadataEntry {
                key: "GitCommit".to_string(),
                value: "1a2b3c".to_string(),
            },
        ];

        assert_eq!(resolve_key(&entries, "Description/source_code").unwrap().value, "x = 42");
        assert_eq!(resolve_key(&entries, "source_code").unwrap().value, "x = 42");
        assert_eq!(resolve_key(&entries, "source-code").unwrap().value, "x = 42");
        assert_eq!(resolve_key(&entries, "SourceCode").unwrap().value, "x = 42");
    }

    #[test]
    fn test_resolve_key_ambiguity() {
        let entries = vec![
            MetadataEntry {
                key: "SectionA/Status".to_string(),
                value: "OK".to_string(),
            },
            MetadataEntry {
                key: "SectionB/Status".to_string(),
                value: "ERROR".to_string(),
            },
        ];

        let res = resolve_key(&entries, "Status");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Ambiguous key"));

        assert_eq!(resolve_key(&entries, "SectionA/Status").unwrap().value, "OK");
    }

    #[test]
    fn test_resolve_key_not_found() {
        let entries = vec![MetadataEntry {
            key: "Author".to_string(),
            value: "User".to_string(),
        }];

        let res = resolve_key(&entries, "NonExistentKey");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Available keys"));
    }
}