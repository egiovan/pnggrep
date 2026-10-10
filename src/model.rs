#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalModule {
    pub name: String,
    pub relpath: String,
    pub source_code: String,
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

/// Parses the line-delimited stream format:
/// %%% <name> % <relpath>
/// <source_code...>
pub fn parse_local_modules(raw: &str) -> Vec<LocalModule> {
    let mut modules = Vec::new();
    let mut current_name = String::new();
    let mut current_relpath = String::new();
    let mut current_lines = Vec::new();
    let mut in_module = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("%%% ") {
            if in_module && !current_name.is_empty() {
                modules.push(LocalModule {
                    name: current_name.clone(),
                    relpath: current_relpath.clone(),
                    source_code: current_lines.join("\n"),
                });
                current_lines.clear();
            }
            in_module = true;
            let header = trimmed[4..].trim();
            if let Some((name_part, path_part)) = header.split_once('%') {
                current_name = name_part.trim().to_string();
                current_relpath = path_part.trim().to_string();
            } else {
                current_name = header.to_string();
                current_relpath = String::new();
            }
        } else if in_module {
            current_lines.push(line);
        }
    }

    if in_module && !current_name.is_empty() {
        modules.push(LocalModule {
            name: current_name,
            relpath: current_relpath,
            source_code: current_lines.join("\n"),
        });
    }

    modules
}

/// Extracts bundled local modules from metadata entries across PNG, SVG, or PDF
pub fn find_local_modules(entries: &[MetadataEntry]) -> Vec<LocalModule> {
    for entry in entries {
        let leaf = entry.key.rsplit('/').next().unwrap_or(&entry.key);
        let norm = normalize_key(leaf);
        if norm == "localmodules" {
            let mods = parse_local_modules(&entry.value);
            if !mods.is_empty() {
                return mods;
            }
        }
    }
    Vec::new()
}

/// Checks whether a metadata key represents a fusion shot or pulse identifier
pub fn is_fusion_key(key: &str) -> bool {
    let leaf = key.rsplit('/').next().unwrap_or(key);
    let norm = normalize_key(leaf);
    norm == "shot" || norm == "shots" || norm == "pulse" || norm == "pulses"
}

/// Matches exact shot/pulse numbers across scalar strings, JSON lists, and lines
pub fn matches_fusion_pulse(entry_key: &str, entry_value: &str, pulse_query: &str) -> bool {
    if !is_fusion_key(entry_key) {
        return false;
    }
    let query_clean = pulse_query.trim().to_lowercase();
    if query_clean.is_empty() {
        return false;
    }

    // 1. Direct line check
    for line in entry_value.lines() {
        let clean = line
            .trim()
            .trim_matches(|c| c == '[' || c == ']' || c == '"' || c == '\'' || c == ',' || c == ' ');
        if clean.eq_ignore_ascii_case(&query_clean) {
            return true;
        }
    }

    // 2. Tokenize on comma, bracket, quote, whitespace boundaries
    for token in entry_value.split(|c: char| {
        c == ',' || c == '[' || c == ']' || c == '"' || c == '\'' || c.is_whitespace()
    }) {
        let clean = token.trim();
        if !clean.is_empty() && clean.eq_ignore_ascii_case(&query_clean) {
            return true;
        }
    }

    false
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

    #[test]
    fn test_parse_local_modules() {
        let raw = "%%% mod_a % mod_a.py\ndef a(): pass\n\n%%% mod_b % sub/mod_b.py\ndef b(): pass\n";
        let modules = parse_local_modules(raw);
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].name, "mod_a");
        assert_eq!(modules[0].relpath, "mod_a.py");
        assert_eq!(modules[0].source_code, "def a(): pass\n");
        assert_eq!(modules[1].name, "mod_b");
        assert_eq!(modules[1].relpath, "sub/mod_b.py");
    }

    #[test]
    fn test_fusion_matching() {
        assert!(matches_fusion_pulse("Shot", "92185", "92185"));
        assert!(!matches_fusion_pulse("Shot", "92185", "9218"));
        assert!(matches_fusion_pulse("Description/shots", "[92185, 92186]", "92185"));
        assert!(matches_fusion_pulse("Keywords/pulses", "92185\n92186", "92186"));
        assert!(!matches_fusion_pulse("SourceCode", "shot = 92185", "92185"));
    }
}