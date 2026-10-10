use crate::model::MetadataEntry;
use std::io::Read;

pub const MAX_SVG_READ_SIZE: u64 = 32 * 1024 * 1024;
pub const MAX_JSON_RECURSION_DEPTH: usize = 8;

pub fn xml_unescape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '&' {
            let mut entity = String::new();
            let mut closed = false;
            while let Some(&next_c) = chars.peek() {
                if next_c == ';' {
                    chars.next();
                    closed = true;
                    break;
                }
                if next_c == '&' || next_c == ' ' || entity.len() > 10 {
                    break;
                }
                entity.push(chars.next().unwrap());
            }

            if closed {
                match entity.as_str() {
                    "quot" => out.push('"'),
                    "amp" => out.push('&'),
                    "apos" => out.push('\''),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    s if s.starts_with("#x") || s.starts_with("#X") => {
                        if let Ok(code) = u32::from_str_radix(&s[2..], 16) {
                            if let Some(ch) = std::char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                    }
                    s if s.starts_with('#') => {
                        if let Ok(code) = s[1..].parse::<u32>() {
                            if let Some(ch) = std::char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                    }
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                        out.push(';');
                    }
                }
            } else {
                out.push('&');
                out.push_str(&entity);
            }
        } else {
            out.push(c);
        }
    }
    out
}

enum JsonArrayValue {
    Primitive(String),
    Object(Vec<(String, String)>),
}

fn parse_json_array(chars: &[char], depth: usize) -> Option<Vec<JsonArrayValue>> {
    let n = chars.len();
    if n < 2 || chars[0] != '[' || chars[n - 1] != ']' {
        return None;
    }

    let mut items = Vec::new();
    let mut idx = 1;

    while idx < n - 1 {
        while idx < n - 1 && (chars[idx].is_whitespace() || chars[idx] == ',') {
            idx += 1;
        }
        if idx >= n - 1 {
            break;
        }

        if chars[idx] == '"' {
            idx += 1;
            let mut val = String::new();
            while idx < n && chars[idx] != '"' {
                if chars[idx] == '\\' && idx + 1 < n {
                    idx += 1;
                    match chars[idx] {
                        '"' => val.push('"'),
                        '\\' => val.push('\\'),
                        '/' => val.push('/'),
                        'n' => val.push('\n'),
                        'r' => val.push('\r'),
                        't' => val.push('\t'),
                        'u' if idx + 4 < n => {
                            let hex_str: String = chars[idx + 1..=idx + 4].iter().collect();
                            if let Ok(code) = u32::from_str_radix(&hex_str, 16) {
                                if let Some(ch) = std::char::from_u32(code) {
                                    val.push(ch);
                                }
                            }
                            idx += 4;
                        }
                        _ => val.push(chars[idx]),
                    }
                } else {
                    val.push(chars[idx]);
                }
                idx += 1;
            }
            if idx < n {
                idx += 1;
            }
            items.push(JsonArrayValue::Primitive(val));
        } else if chars[idx] == '{' {
            let start_obj = idx;
            let mut obj_depth = 0;
            let mut in_str = false;
            while idx < n {
                if chars[idx] == '"' && (idx == 0 || chars[idx - 1] != '\\') {
                    in_str = !in_str;
                } else if !in_str {
                    if chars[idx] == '{' {
                        obj_depth += 1;
                    } else if chars[idx] == '}' {
                        obj_depth -= 1;
                        if obj_depth == 0 {
                            idx += 1;
                            break;
                        }
                    }
                }
                idx += 1;
            }
            let obj_str: String = chars[start_obj..idx].iter().collect();
            let sub_entries = parse_json_object(&obj_str, depth + 1)?;
            items.push(JsonArrayValue::Object(sub_entries));
        } else if chars[idx] == '[' {
            let start_sub = idx;
            let mut sub_depth = 0;
            let mut in_str = false;
            while idx < n {
                if chars[idx] == '"' && (idx == 0 || chars[idx - 1] != '\\') {
                    in_str = !in_str;
                } else if !in_str {
                    if chars[idx] == '[' {
                        sub_depth += 1;
                    } else if chars[idx] == ']' {
                        sub_depth -= 1;
                        if sub_depth == 0 {
                            idx += 1;
                            break;
                        }
                    }
                }
                idx += 1;
            }
            let sub_str: String = chars[start_sub..idx].iter().collect();
            items.push(JsonArrayValue::Primitive(sub_str));
        } else {
            let start_val = idx;
            while idx < n - 1 && chars[idx] != ',' && chars[idx] != ']' && !chars[idx].is_whitespace() {
                idx += 1;
            }
            let val_str: String = chars[start_val..idx].iter().collect();
            items.push(JsonArrayValue::Primitive(val_str));
        }
    }

    Some(items)
}

pub fn parse_json_object(input: &str, depth: usize) -> Option<Vec<(String, String)>> {
    if depth > MAX_JSON_RECURSION_DEPTH {
        return None;
    }

    let trimmed = input.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }

    let mut results = Vec::new();
    let chars: Vec<char> = trimmed.chars().collect();
    let n = chars.len();
    let mut i = 1;

    while i < n {
        while i < n && (chars[i].is_whitespace() || chars[i] == ',') {
            i += 1;
        }
        if i >= n || chars[i] == '}' {
            break;
        }

        if chars[i] != '"' {
            return None;
        }
        i += 1;
        let mut key = String::new();
        while i < n && chars[i] != '"' {
            if chars[i] == '\\' && i + 1 < n {
                i += 1;
                match chars[i] {
                    '"' => key.push('"'),
                    '\\' => key.push('\\'),
                    '/' => key.push('/'),
                    'n' => key.push('\n'),
                    'r' => key.push('\r'),
                    't' => key.push('\t'),
                    _ => key.push(chars[i]),
                }
            } else {
                key.push(chars[i]);
            }
            i += 1;
        }
        if i >= n {
            return None;
        }
        i += 1;

        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n || chars[i] != ':' {
            return None;
        }
        i += 1;
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            return None;
        }

        if chars[i] == '"' {
            i += 1;
            let mut val = String::new();
            while i < n && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                    match chars[i] {
                        '"' => val.push('"'),
                        '\\' => val.push('\\'),
                        '/' => val.push('/'),
                        'n' => val.push('\n'),
                        'r' => val.push('\r'),
                        't' => val.push('\t'),
                        'u' if i + 4 < n => {
                            let hex_str: String = chars[i + 1..=i + 4].iter().collect();
                            if let Ok(code) = u32::from_str_radix(&hex_str, 16) {
                                if let Some(ch) = std::char::from_u32(code) {
                                    val.push(ch);
                                }
                            }
                            i += 4;
                        }
                        _ => val.push(chars[i]),
                    }
                } else {
                    val.push(chars[i]);
                }
                i += 1;
            }
            if i < n {
                i += 1;
            }
            results.push((key, val));
        } else if chars[i] == '{' {
            let start_obj = i;
            let mut obj_depth = 0;
            let mut in_str = false;
            while i < n {
                if chars[i] == '"' && (i == 0 || chars[i - 1] != '\\') {
                    in_str = !in_str;
                } else if !in_str {
                    if chars[i] == '{' {
                        obj_depth += 1;
                    } else if chars[i] == '}' {
                        obj_depth -= 1;
                        if obj_depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                }
                i += 1;
            }
            let obj_str: String = chars[start_obj..i].iter().collect();
            let sub_entries = parse_json_object(&obj_str, depth + 1)?;
            for (sub_k, sub_v) in sub_entries {
                results.push((format!("{}/{}", key, sub_k), sub_v));
            }
        } else if chars[i] == '[' {
            let start_arr = i;
            let mut arr_depth = 0;
            let mut in_str = false;
            while i < n {
                if chars[i] == '"' && (i == 0 || chars[i - 1] != '\\') {
                    in_str = !in_str;
                } else if !in_str {
                    if chars[i] == '[' {
                        arr_depth += 1;
                    } else if chars[i] == ']' {
                        arr_depth -= 1;
                        if arr_depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                }
                i += 1;
            }
            let arr_chars = &chars[start_arr..i];
            if let Some(items) = parse_json_array(arr_chars, depth + 1) {
                let has_objects = items.iter().any(|it| matches!(it, JsonArrayValue::Object(_)));
                if has_objects {
                    for (idx, it) in items.into_iter().enumerate() {
                        match it {
                            JsonArrayValue::Object(entries) => {
                                for (sub_k, sub_v) in entries {
                                    results.push((format!("{}/{}/{}", key, idx, sub_k), sub_v));
                                }
                            }
                            JsonArrayValue::Primitive(p) => {
                                results.push((format!("{}/{}", key, idx), p));
                            }
                        }
                    }
                } else {
                    let lines: Vec<String> = items
                        .into_iter()
                        .map(|it| match it {
                            JsonArrayValue::Primitive(p) => p,
                            _ => String::new(),
                        })
                        .collect();
                    results.push((key, lines.join("\n")));
                }
            } else {
                let arr_str: String = arr_chars.iter().collect();
                results.push((key, arr_str));
            }
        } else {
            let start_val = i;
            while i < n && chars[i] != ',' && chars[i] != '}' && !chars[i].is_whitespace() {
                i += 1;
            }
            let val_str: String = chars[start_val..i].iter().collect();
            results.push((key, val_str));
        }
    }

    Some(results)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Checks whether an inner XML slice contains child elements (excluding CDATA blocks and comments)
fn has_child_elements(inner: &str) -> bool {
    let mut cursor = 0;
    while let Some(pos) = inner[cursor..].find('<') {
        let abs = cursor + pos;
        let rest = &inner[abs..];
        if rest.starts_with("<![CDATA[") {
            if let Some(end) = rest.find("]]>") {
                cursor = abs + end + 3;
            } else {
                break;
            }
        } else if rest.starts_with("<!--") {
            if let Some(end) = rest.find("-->") {
                cursor = abs + end + 3;
            } else {
                break;
            }
        } else if rest.starts_with("</") {
            cursor = abs + 2;
        } else {
            return true;
        }
    }
    false
}

/// Strips wrapping CDATA markup if present
fn unwrap_cdata(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
        &trimmed[9..trimmed.len() - 3]
    } else {
        trimmed
    }
}

/// Parses the inner contents of a <metadata> block into key-value entries
fn parse_metadata_block(search_area: &str, entries: &mut Vec<MetadataEntry>) {
    let mut cursor = 0;

    while let Some(tag_start) = search_area[cursor..].find('<') {
        let abs_start = cursor + tag_start;
        cursor = abs_start + 1;

        if search_area[abs_start..].starts_with("<!--")
            || search_area[abs_start..].starts_with("<?")
            || search_area[abs_start..].starts_with("<!")
            || search_area[abs_start..].starts_with("</")
        {
            continue;
        }

        let tag_open_end = match search_area[abs_start..].find('>') {
            Some(pos) => abs_start + pos,
            None => break,
        };

        if search_area[tag_open_end - 1..=tag_open_end].starts_with('/') {
            cursor = tag_open_end + 1;
            continue;
        }

        let tag_header = &search_area[abs_start + 1..tag_open_end];
        let raw_tag_name = tag_header
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/');

        if raw_tag_name.is_empty() {
            continue;
        }

        let closing_tag = format!("</{}>", raw_tag_name);
        if let Some(close_pos) = search_area[tag_open_end + 1..].find(&closing_tag) {
            let abs_close = tag_open_end + 1 + close_pos;
            let inner_raw = &search_area[tag_open_end + 1..abs_close];

            // Only extract leaf elements (tags that do not contain child elements)
            if !has_child_elements(inner_raw) {
                let clean_name = raw_tag_name.split(':').last().unwrap_or(raw_tag_name);
                let key_name = capitalize(clean_name);
                let text_content = unwrap_cdata(inner_raw);
                let unescaped_text = xml_unescape(text_content.trim());

                if unescaped_text.starts_with('{') && unescaped_text.ends_with('}') {
                    if let Some(json_entries) = parse_json_object(&unescaped_text, 0) {
                        for (jk, jv) in json_entries {
                            entries.push(MetadataEntry {
                                key: format!("{}/{}", key_name, jk),
                                value: jv,
                            });
                        }
                    } else {
                        entries.push(MetadataEntry {
                            key: key_name,
                            value: unescaped_text,
                        });
                    }
                } else if !unescaped_text.is_empty() {
                    entries.push(MetadataEntry {
                        key: key_name,
                        value: unescaped_text,
                    });
                }
            }
        }
    }
}

pub fn extract_svg_metadata<R: Read>(reader: &mut R) -> std::io::Result<Vec<MetadataEntry>> {
    let mut content = String::new();
    reader.take(MAX_SVG_READ_SIZE).read_to_string(&mut content)?;

    let mut entries = Vec::new();
    let mut search_pos = 0;

    // Strict metadata scoping: only parse inside <metadata>...</metadata> blocks
    while let Some(start_offset) = content[search_pos..].find("<metadata") {
        let abs_start = search_pos + start_offset;
        let close_tag = "</metadata>";
        if let Some(end_offset) = content[abs_start..].find(close_tag) {
            let abs_end = abs_start + end_offset + close_tag.len();
            let block = &content[abs_start..abs_end];
            parse_metadata_block(block, &mut entries);
            search_pos = abs_end;
        } else {
            break;
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::resolve_key;
    use std::io::Cursor;

    #[test]
    fn test_svg_metadata_with_json_in_description() {
        let svg_data = r#"<?xml version="1.0" encoding="utf-8" standalone="no"?>
<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
  <metadata>
    <rdf:RDF xmlns:dc="http://purl.org/dc/elements/1.1/">
      <dc:title>equilibrium.svg</dc:title>
      <dc:date>2026-09-10T10:00:00</dc:date>
      <dc:description>{
        &quot;directory&quot;: &quot;/home/researcher/sim&quot;,
        &quot;filename&quot;: &quot;run.py&quot;,
        &quot;source_code&quot;: &quot;import numpy as np\ndef solve(): pass\n&quot;,
        &quot;git_commit&quot;: &quot;abcdef1&quot;
      }</dc:description>
    </rdf:RDF>
  </metadata>
  <rect width="100" height="100" />
</svg>"#;

        let mut cursor = Cursor::new(svg_data.as_bytes());
        let entries = extract_svg_metadata(&mut cursor).unwrap();

        assert_eq!(resolve_key(&entries, "Title").unwrap().value, "equilibrium.svg");
        assert_eq!(resolve_key(&entries, "Description/filename").unwrap().value, "run.py");
        assert_eq!(
            resolve_key(&entries, "source_code").unwrap().value,
            "import numpy as np\ndef solve(): pass\n"
        );
        assert_eq!(
            resolve_key(&entries, "SourceCode").unwrap().value,
            "import numpy as np\ndef solve(): pass\n"
        );
    }

    #[test]
    fn test_svg_metadata_with_json_arrays() {
        let svg_data = r#"<svg xmlns="http://www.w3.org/2000/svg">
  <metadata>
    <dc:description>{
      &quot;directory&quot;: &quot;/sim&quot;,
      &quot;argv&quot;: [&quot;run.py&quot;, &quot;--shot&quot;, &quot;92185&quot;],
      &quot;semantics&quot;: {
        &quot;labels&quot;: [&quot;rho&quot;, &quot;shear&quot;]
      }
    }</dc:description>
  </metadata>
</svg>"#;
        let mut cursor = Cursor::new(svg_data.as_bytes());
        let entries = extract_svg_metadata(&mut cursor).unwrap();

        assert_eq!(resolve_key(&entries, "Description/directory").unwrap().value, "/sim");
        assert_eq!(
            resolve_key(&entries, "Description/argv").unwrap().value,
            "run.py\n--shot\n92185"
        );
        assert_eq!(
            resolve_key(&entries, "Description/semantics/labels").unwrap().value,
            "rho\nshear"
        );
    }

    #[test]
    fn test_svg_without_metadata() {
        let svg_simple = r#"<svg width="50" height="50"><circle cx="25" cy="25" r="20"/></svg>"#;
        let mut cursor = Cursor::new(svg_simple.as_bytes());
        let entries = extract_svg_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_svg_flamegraph_ignored() {
        let flamegraph_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="800">
<style>.func_g:hover { stroke:black; }</style>
<script><![CDATA[ function init() { alert('zoom'); } ]]></script>
<text>Flame Graph</text>
<g class="func_g"><title>kernel_func (150 samples)</title><rect width="10" height="10"/></g>
<g class="func_g"><title>mean_n_ (400 samples)</title><rect width="20" height="10"/></g>
</svg>"#;
        let mut cursor = Cursor::new(flamegraph_svg.as_bytes());
        let entries = extract_svg_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty(), "FlameGraph elements must not be treated as metadata");
    }

    #[test]
    fn test_xml_entity_unescaping() {
        let raw = "&quot;Hello &amp; &lt;World&gt;&apos; &#38; &#x41;";
        let decoded = xml_unescape(raw);
        assert_eq!(decoded, "\"Hello & <World>' & A");
    }

    #[test]
    fn test_json_recursion_depth_limit() {
        let deep_json = "{\"a\":{\"b\":{\"c\":{\"d\":{\"e\":{\"f\":{\"g\":{\"h\":{\"i\":{\"j\":\"deep\"}}}}}}}}}}";
        let parsed = parse_json_object(deep_json, 0);
        assert!(parsed.is_none());
    }
}