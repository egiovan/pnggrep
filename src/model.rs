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

/// Checks whether a metadata key represents a timestamp or creation date
pub fn is_date_key(key: &str) -> bool {
    let leaf = key.rsplit('/').next().unwrap_or(key);
    let norm = normalize_key(leaf);
    norm == "timestamp" || norm == "date" || norm == "creationdate"
}

pub fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

pub fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Howard Hinnant algorithm: converts civil date into days since 1970-01-01
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u32;
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

pub fn civil_to_epoch_seconds(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> u64 {
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return 0;
    }
    (days as u64) * 86400 + (hour as u64) * 3600 + (minute as u64) * 60 + (second as u64)
}

/// Parses relative durations like 30m, 12h, 2d, 3w, 2 days
pub fn parse_relative_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let idx = s.find(|c: char| !c.is_ascii_digit())?;
    if idx == 0 {
        return None;
    }
    let num: u64 = s[..idx].trim().parse().ok()?;
    let unit = s[idx..].trim().to_lowercase();
    match unit.as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(num),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(num * 60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(num * 3600),
        "d" | "day" | "days" => Some(num * 86400),
        "w" | "wk" | "wks" | "week" | "weeks" => Some(num * 7 * 86400),
        _ => None,
    }
}

/// Parses ISO-8601 strings, PDF CreationDate format, or date-only values into Unix epoch seconds
pub fn parse_iso_or_pdf_datetime(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // PDF Format: D:YYYYMMDDHHmmSS...
    if let Some(rest) = trimmed.strip_prefix("D:") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 8 {
            let year: i64 = digits[0..4].parse().ok()?;
            let month: u32 = digits[4..6].parse().ok()?;
            let day: u32 = digits[6..8].parse().ok()?;
            let hour: u32 = if digits.len() >= 10 { digits[8..10].parse().unwrap_or(0) } else { 0 };
            let min: u32 = if digits.len() >= 12 { digits[10..12].parse().unwrap_or(0) } else { 0 };
            let sec: u32 = if digits.len() >= 14 { digits[12..14].parse().unwrap_or(0) } else { 0 };
            return Some(civil_to_epoch_seconds(year, month, day, hour, min, sec));
        }
    }

    // ISO Format: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS or YYYY-MM-DD HH:MM:SS
    let parts: Vec<&str> = trimmed.split(|c| c == 'T' || c == ' ').collect();
    let date_part = parts[0];
    let date_sub: Vec<&str> = date_part.split('-').collect();
    if date_sub.len() != 3 {
        return None;
    }
    let year: i64 = date_sub[0].parse().ok()?;
    let month: u32 = date_sub[1].parse().ok()?;
    let day: u32 = date_sub[2].parse().ok()?;

    let mut hour = 0;
    let mut min = 0;
    let mut sec = 0;

    if parts.len() > 1 && !parts[1].is_empty() {
        let time_str = parts[1];
        let clean_time = time_str
            .split(|c| c == '.' || c == 'Z' || c == '+' || c == '-')
            .next()
            .unwrap_or("");
        let time_sub: Vec<&str> = clean_time.split(':').collect();
        if !time_sub.is_empty() {
            hour = time_sub[0].parse().unwrap_or(0);
        }
        if time_sub.len() > 1 {
            min = time_sub[1].parse().unwrap_or(0);
        }
        if time_sub.len() > 2 {
            sec = time_sub[2].parse().unwrap_or(0);
        }
    }

    Some(civil_to_epoch_seconds(year, month, day, hour, min, sec))
}

/// Parses CLI boundary arguments into Unix epoch seconds.
/// Supports partial dates (YYYY, YYYY-MM), full calendar dates (YYYY-MM-DD),
/// ISO timestamps, and relative durations (30m, 2h, 3d, 2w).
pub fn parse_date_bound(s: &str, is_end_of_day: bool) -> Result<u64, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("Empty date string".to_string());
    }

    // 1. Relative duration (30m, 2h, 3d, 2w)
    if let Some(dur_secs) = parse_relative_duration(trimmed) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return Ok(now.saturating_sub(dur_secs));
    }

    // 2. Calendar dates (YYYY, YYYY-MM, YYYY-MM-DD)
    let parts: Vec<&str> = trimmed.split(|c| c == 'T' || c == ' ').collect();
    let date_sub: Vec<&str> = parts[0].split('-').collect();

    // 2a. Year only: YYYY (e.g. "2020")
    if date_sub.len() == 1 && date_sub[0].len() == 4 && date_sub[0].chars().all(|c| c.is_ascii_digit()) {
        let year: i64 = date_sub[0].parse().map_err(|_| format!("Invalid year in '{}'", s))?;
        let (month, day, hour, min, sec) = if is_end_of_day {
            (12, 31, 23, 59, 59)
        } else {
            (1, 1, 0, 0, 0)
        };
        return Ok(civil_to_epoch_seconds(year, month, day, hour, min, sec));
    }

    // 2b. Year and month: YYYY-MM (e.g. "2020-02" or "2020-2")
    if date_sub.len() == 2 && parts.len() == 1 {
        let year: i64 = date_sub[0].parse().map_err(|_| format!("Invalid year in '{}'", s))?;
        let month: u32 = date_sub[1].parse().map_err(|_| format!("Invalid month in '{}'", s))?;
        if month < 1 || month > 12 {
            return Err(format!("Month out of range in '{}'", s));
        }
        let (day, hour, min, sec) = if is_end_of_day {
            (days_in_month(year, month), 23, 59, 59)
        } else {
            (1, 0, 0, 0)
        };
        return Ok(civil_to_epoch_seconds(year, month, day, hour, min, sec));
    }

    // 2c. Full date: YYYY-MM-DD
    if date_sub.len() == 3 {
        let year: i64 = date_sub[0].parse().map_err(|_| format!("Invalid year in '{}'", s))?;
        let month: u32 = date_sub[1].parse().map_err(|_| format!("Invalid month in '{}'", s))?;
        let day: u32 = date_sub[2].parse().map_err(|_| format!("Invalid day in '{}'", s))?;

        if month < 1 || month > 12 || day < 1 || day > days_in_month(year, month) {
            return Err(format!("Date out of range in '{}'", s));
        }

        if parts.len() == 1 {
            let (hour, min, sec) = if is_end_of_day {
                (23, 59, 59)
            } else {
                (0, 0, 0)
            };
            return Ok(civil_to_epoch_seconds(year, month, day, hour, min, sec));
        }
    }

    // 3. Fallback to general datetime parser (with explicit time)
    if let Some(epoch) = parse_iso_or_pdf_datetime(trimmed) {
        return Ok(epoch);
    }

    Err(format!(
        "Unsupported date format '{}'. Expected YYYY, YYYY-MM, YYYY-MM-DD, ISO timestamp, or relative like 12h, 2d, 1w",
        s
    ))
}

/// Extracts creation timestamp in epoch seconds from figure metadata entries
pub fn extract_entry_timestamp(entries: &[MetadataEntry]) -> Option<u64> {
    // Priority 1: timestamp
    for entry in entries {
        let leaf = entry.key.rsplit('/').next().unwrap_or(&entry.key);
        if normalize_key(leaf) == "timestamp" {
            if let Some(epoch) = parse_iso_or_pdf_datetime(&entry.value) {
                return Some(epoch);
            }
        }
    }
    // Priority 2: date
    for entry in entries {
        let leaf = entry.key.rsplit('/').next().unwrap_or(&entry.key);
        if normalize_key(leaf) == "date" {
            if let Some(epoch) = parse_iso_or_pdf_datetime(&entry.value) {
                return Some(epoch);
            }
        }
    }
    // Priority 3: creationdate
    for entry in entries {
        let leaf = entry.key.rsplit('/').next().unwrap_or(&entry.key);
        if normalize_key(leaf) == "creationdate" {
            if let Some(epoch) = parse_iso_or_pdf_datetime(&entry.value) {
                return Some(epoch);
            }
        }
    }
    None
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

    #[test]
    fn test_epoch_conversion_and_iso_parsing() {
        // 1970-01-01 00:00:00 UTC = 0
        assert_eq!(civil_to_epoch_seconds(1970, 1, 1, 0, 0, 0), 0);
        // 2026-10-10 00:00:00 UTC = 1791590400
        assert_eq!(civil_to_epoch_seconds(2026, 10, 10, 0, 0, 0), 1791590400);

        let iso = "2026-10-10T11:37:59.821361";
        let parsed = parse_iso_or_pdf_datetime(iso).unwrap();
        assert_eq!(parsed, 1791590400 + 11 * 3600 + 37 * 60 + 59);

        let pdf_date = "D:20261010113802+02'00'";
        let parsed_pdf = parse_iso_or_pdf_datetime(pdf_date).unwrap();
        assert_eq!(parsed_pdf, 1791590400 + 11 * 3600 + 38 * 60 + 2);
    }

    #[test]
    fn test_parse_date_bounds() {
        // Partial year boundaries
        let start_year = parse_date_bound("2020", false).unwrap();
        assert_eq!(start_year, civil_to_epoch_seconds(2020, 1, 1, 0, 0, 0));

        let end_year = parse_date_bound("2020", true).unwrap();
        assert_eq!(end_year, civil_to_epoch_seconds(2020, 12, 31, 23, 59, 59));

        // Partial month boundaries
        let start_month = parse_date_bound("2020-02", false).unwrap();
        assert_eq!(start_month, civil_to_epoch_seconds(2020, 2, 1, 0, 0, 0));

        // Leap year vs common year end of February
        let end_month_leap = parse_date_bound("2020-02", true).unwrap();
        assert_eq!(end_month_leap, civil_to_epoch_seconds(2020, 2, 29, 23, 59, 59));

        let end_month_common = parse_date_bound("2021-02", true).unwrap();
        assert_eq!(end_month_common, civil_to_epoch_seconds(2021, 2, 28, 23, 59, 59));

        let end_month_30 = parse_date_bound("2020-04", true).unwrap();
        assert_eq!(end_month_30, civil_to_epoch_seconds(2020, 4, 30, 23, 59, 59));

        // Full day boundaries
        let start = parse_date_bound("2026-10-10", false).unwrap();
        assert_eq!(start, 1791590400);

        let end = parse_date_bound("2026-10-10", true).unwrap();
        assert_eq!(end, 1791590400 + 86399);

        // Relative durations
        assert!(parse_relative_duration("2d").is_some());
        assert_eq!(parse_relative_duration("2d").unwrap(), 2 * 86400);
        assert_eq!(parse_relative_duration("12h").unwrap(), 12 * 3600);
    }
}