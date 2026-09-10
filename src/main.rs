use std::env;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

// Limiti di sicurezza contro DoS e Memory Exhaustion
const MAX_METADATA_CHUNK_SIZE: u64 = 16 * 1024 * 1024;    // Max 16 MB per singolo chunk PNG
const MAX_DECOMPRESSED_SIZE: usize = 32 * 1024 * 1024;    // Max 32 MB dopo decompressione zlib
const MAX_SVG_READ_SIZE: u64 = 32 * 1024 * 1024;          // Max 32 MB analizzati per file SVG
const MAX_JSON_RECURSION_DEPTH: usize = 8;                 // Max 8 livelli di annidamento JSON

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    "venv", ".venv", "env", ".env", "__pycache__", ".git", ".hg", ".svn",
    "target", "build", "dist", "node_modules", ".tox", ".pytest_cache",
    ".mypy_cache", ".cache",
];


#[link(name = "z")]
extern "C" {
    fn uncompress(
        dest: *mut u8,
        dest_len: *mut usize,
        source: *const u8,
        source_len: usize,
    ) -> i32;

    #[cfg(test)]
    fn compress(
        dest: *mut u8,
        dest_len: *mut usize,
        source: *const u8,
        source_len: usize,
    ) -> i32;
}

fn decompress_zlib(compressed: &[u8]) -> Option<String> {
    if compressed.is_empty() {
        return None;
    }

    let mut dest_len = compressed.len().max(1024).saturating_mul(4);
    if dest_len > MAX_DECOMPRESSED_SIZE {
        dest_len = MAX_DECOMPRESSED_SIZE;
    }
    let mut dest = vec![0u8; dest_len];

    loop {
        let mut cur_len = dest_len;
        let ret = unsafe {
            uncompress(
                dest.as_mut_ptr(),
                &mut cur_len,
                compressed.as_ptr(),
                compressed.len(),
            )
        };

        if ret == 0 {
            dest.truncate(cur_len);
            return String::from_utf8(dest).ok();
        } else if ret == -5 {
            // Buffer insufficiente: raddoppia se sotto la soglia di sicurezza
            if dest_len >= MAX_DECOMPRESSED_SIZE {
                return None; // Possibile zlib bomb: interruzione di sicurezza
            }
            dest_len = dest_len.saturating_mul(2).min(MAX_DECOMPRESSED_SIZE);
            dest.resize(dest_len, 0);
        } else {
            return None;
        }
    }
}

/// Rimuove sequenze di escape ANSI e caratteri di controllo non stampabili
fn sanitize_for_terminal(input: &str) -> String {
    let mut clean = String::with_capacity(input.len());
    let mut in_escape = false;

    for c in input.chars() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if c == '\t' || c == '\n' || c == '\r' || !c.is_control() {
            clean.push(c);
        }
    }
    clean
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

pub fn extract_png_metadata<R: Read + Seek>(reader: &mut R) -> std::io::Result<Vec<MetadataEntry>> {
    let mut magic = [0u8; 8];
    if reader.read_exact(&mut magic).is_err() || magic != PNG_MAGIC {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();

    loop {
        let mut len_buf = [0u8; 4];
        if reader.read_exact(&mut len_buf).is_err() {
            break;
        }
        let length = u32::from_be_bytes(len_buf) as u64;

        let mut type_buf = [0u8; 4];
        if reader.read_exact(&mut type_buf).is_err() {
            break;
        }

        // Controllo di sicurezza: ignora chunk con lunghezze anomale
        if length > MAX_METADATA_CHUNK_SIZE {
            reader.seek(SeekFrom::Current(length as i64 + 4))?;
            continue;
        }

        match &type_buf {
            b"IEND" => break,

            b"tEXt" => {
                let mut data = vec![0u8; length as usize];
                reader.read_exact(&mut data)?;
                if let Some(pos) = data.iter().position(|&b| b == 0) {
                    let key = String::from_utf8_lossy(&data[..pos]).to_string();
                    let val = String::from_utf8_lossy(&data[pos + 1..]).to_string();
                    entries.push(MetadataEntry { key, value: val });
                }
                reader.seek(SeekFrom::Current(4))?;
            }

            b"zTXt" => {
                let mut data = vec![0u8; length as usize];
                reader.read_exact(&mut data)?;
                if let Some(pos) = data.iter().position(|&b| b == 0) {
                    let key = String::from_utf8_lossy(&data[..pos]).to_string();
                    if pos + 2 <= data.len() {
                        if let Some(val) = decompress_zlib(&data[pos + 2..]) {
                            entries.push(MetadataEntry { key, value: val });
                        }
                    }
                }
                reader.seek(SeekFrom::Current(4))?;
            }

            b"iTXt" => {
                let mut data = vec![0u8; length as usize];
                reader.read_exact(&mut data)?;
                if let Some(pos) = data.iter().position(|&b| b == 0) {
                    let key = String::from_utf8_lossy(&data[..pos]).to_string();
                    let rest = &data[pos + 1..];
                    if rest.len() >= 2 {
                        let comp_flag = rest[0];
                        let mut slice = &rest[2..];
                        if let Some(p1) = slice.iter().position(|&b| b == 0) {
                            slice = &slice[p1 + 1..];
                            if let Some(p2) = slice.iter().position(|&b| b == 0) {
                                let payload = &slice[p2 + 1..];
                                if comp_flag == 1 {
                                    if let Some(val) = decompress_zlib(payload) {
                                        entries.push(MetadataEntry { key, value: val });
                                    }
                                } else {
                                    let val = String::from_utf8_lossy(payload).to_string();
                                    entries.push(MetadataEntry { key, value: val });
                                }
                            }
                        }
                    }
                }
                reader.seek(SeekFrom::Current(4))?;
            }

            _ => {
                reader.seek(SeekFrom::Current(length as i64 + 4))?;
            }
        }
    }

    Ok(entries)
}

fn xml_unescape(input: &str) -> String {
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

pub fn extract_svg_metadata<R: Read>(reader: &mut R) -> std::io::Result<Vec<MetadataEntry>> {
    let mut content = String::new();
    reader.take(MAX_SVG_READ_SIZE).read_to_string(&mut content)?;

    let search_area = if let Some(start) = content.find("<metadata") {
        if let Some(end) = content[start..].find("</metadata>") {
            &content[start..start + end + 11]
        } else {
            &content[..]
        }
    } else {
        &content[..]
    };

    let mut entries = Vec::new();
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

            let has_child_tags = inner_raw.contains('<') && !inner_raw.contains("<![CDATA[");
            if !has_child_tags {
                let clean_name = raw_tag_name.split(':').last().unwrap_or(raw_tag_name);
                let key_name = capitalize(clean_name);
                let unescaped_text = xml_unescape(inner_raw.trim());

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

    Ok(entries)
}

pub fn extract_metadata(path: &Path) -> std::io::Result<Vec<MetadataEntry>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    match ext.as_str() {
        "png" => extract_png_metadata(&mut reader),
        "svg" => extract_svg_metadata(&mut reader),
        _ => Ok(Vec::new()),
    }
}

fn normalize_key(k: &str) -> String {
    k.chars()
        .filter(|c| *c != '_' && *c != '-' && *c != '/')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn resolve_key<'a>(entries: &'a [MetadataEntry], query: &str) -> Result<&'a MetadataEntry, String> {
    let norm_query = normalize_key(query);

    if let Some(entry) = entries.iter().find(|e| e.key.eq_ignore_ascii_case(query)) {
        return Ok(entry);
    }

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

pub fn is_ignored_dir(dir_name: &str) -> bool {
    let clean_name = dir_name.trim_end_matches('/');
    DEFAULT_IGNORED_DIRS
        .iter()
        .any(|&ignored| ignored.eq_ignore_ascii_case(clean_name))
}

#[derive(Clone, Debug)]
enum RunMode {
    Search { pattern: String },
    List { max_lines: usize },
}

fn search_file(path: &Path, pattern_lower: &str) {
    if let Ok(entries) = extract_metadata(path) {
        for entry in entries {
            if entry.value.to_lowercase().contains(pattern_lower)
                || entry.key.to_lowercase().contains(pattern_lower)
            {
                let clean_key = sanitize_for_terminal(&entry.key);
                println!("\x1b[35m{}\x1b[0m [\x1b[36m{}\x1b[0m]", path.display(), clean_key);
                for (line_idx, line) in entry.value.lines().enumerate() {
                    if line.to_lowercase().contains(pattern_lower) {
                        let clean_line = sanitize_for_terminal(line.trim_end());
                        println!("  \x1b[32m{:4}:\x1b[0m {}", line_idx + 1, clean_line);
                    }
                }
            }
        }
    }
}

fn list_file(path: &Path, max_lines: usize) {
    if let Ok(entries) = extract_metadata(path) {
        if entries.is_empty() {
            return;
        }

        println!("\x1b[1;35m{}\x1b[0m", path.display());

        for entry in entries {
            let clean_key = sanitize_for_terminal(&entry.key);
            let lines: Vec<&str> = entry.value.lines().collect();

            if lines.is_empty() {
                println!("  \x1b[36m{}\x1b[0m: (empty)", clean_key);
            } else if lines.len() == 1 {
                let clean_val = sanitize_for_terminal(lines[0]);
                println!("  \x1b[36m{}\x1b[0m: {}", clean_key, clean_val);
            } else {
                println!("  \x1b[36m{}\x1b[0m:", clean_key);
                let limit = if max_lines == 0 {
                    lines.len()
                } else {
                    max_lines.min(lines.len())
                };

                for line in &lines[..limit] {
                    let clean_val = sanitize_for_terminal(line.trim_end());
                    println!("    \x1b[90m|\x1b[0m {}", clean_val);
                }

                if lines.len() > limit {
                    let remaining = lines.len() - limit;
                    println!("    \x1b[33m... [{} more lines omitted]\x1b[0m", remaining);
                }
            }
        }
        println!();
    }
}

fn cat_metadata(file_path: &Path, target_key: &str) {
    let entries = match extract_metadata(file_path) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("Error reading {}: {}", file_path.display(), err);
            std::process::exit(1);
        }
    };

    if entries.is_empty() {
        eprintln!("No metadata found in {}", file_path.display());
        std::process::exit(1);
    }

    match resolve_key(&entries, target_key) {
        Ok(entry) => {
            print!("{}", entry.value);
            if !entry.value.ends_with('\n') {
                println!();
            }
        }
        Err(err_msg) => {
            eprintln!("Error in {}: {}", file_path.display(), err_msg);
            std::process::exit(1);
        }
    }
}

fn is_supported_ext(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("svg")
}

fn visit_dirs(dir: &Path, mode: &RunMode) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    if is_ignored_dir(dir_name) {
                        continue;
                    }
                }
                visit_dirs(&path, mode);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if is_supported_ext(ext) {
                        match mode {
                            RunMode::Search { pattern } => search_file(&path, pattern),
                            RunMode::List { max_lines } => list_file(&path, *max_lines),
                        }
                    }
                }
            }
        }
    }
}

fn print_help() {
    println!("pnggrep - Search and list embedded PNG and SVG metadata\n");
    println!("USAGE:");
    println!("    pnggrep <PATTERN> [PATH]            Search pattern in PNG/SVG metadata");
    println!("    pnggrep -l [-n LINES] [PATH]        List all metadata keys and values");
    println!("    pnggrep --cat <KEY> <FILE>          Print raw value of a metadata key\n");
    println!("OPTIONS:");
    println!("    -l                  List metadata mode (no pattern required)");
    println!("    -n <LINES>          Max lines to show per key in list mode [default: 3, 0=all]");
    println!("    --cat, -cat <KEY>   Dump exact value without formatting");
    println!("    -h, --help          Show help information");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_help();
        std::process::exit(1);
    }

    let mut is_list = false;
    let mut max_lines = 3usize;
    let mut cat_key: Option<String> = None;
    let mut positional = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-l" => {
                is_list = true;
            }
            "-n" => {
                if i + 1 < args.len() {
                    i += 1;
                    max_lines = args[i].parse().unwrap_or(3);
                } else {
                    eprintln!("Error: -n requires a number of lines.");
                    std::process::exit(1);
                }
            }
            "-cat" | "--cat" => {
                if i + 1 < args.len() {
                    i += 1;
                    cat_key = Some(args[i].clone());
                } else {
                    eprintln!("Error: --cat requires a metadata key: pnggrep --cat <KEY> <FILE>");
                    std::process::exit(1);
                }
            }
            opt if opt.starts_with('-') && opt != "-" => {
                eprintln!("Error: Unknown option '{}'. Run 'pnggrep --help' for usage.", opt);
                std::process::exit(1);
            }
            other => {
                positional.push(other.to_string());
            }
        }
        i += 1;
    }

    if let Some(key) = cat_key {
        if positional.is_empty() {
            eprintln!("Error: --cat requires a file path: pnggrep --cat <KEY> <FILE>");
            std::process::exit(1);
        }
        cat_metadata(Path::new(&positional[0]), &key);
        return;
    }

    if is_list {
        let target_dir = if !positional.is_empty() {
            PathBuf::from(&positional[0])
        } else {
            PathBuf::from(".")
        };
        let mode = RunMode::List { max_lines };
        if target_dir.is_file() {
            list_file(&target_dir, max_lines);
        } else {
            visit_dirs(&target_dir, &mode);
        }
    } else {
        if positional.is_empty() {
            eprintln!("Error: Missing search pattern. Use -l to list all metadata.");
            std::process::exit(1);
        }
        let pattern = positional[0].to_lowercase();
        let target_dir = if positional.len() >= 2 {
            PathBuf::from(&positional[1])
        } else {
            PathBuf::from(".")
        };

        if target_dir.is_file() {
            search_file(&target_dir, &pattern);
        } else {
            let mode = RunMode::Search { pattern };
            visit_dirs(&target_dir, &mode);
        }
    }
}


// -----------------------------------------------------------------------------
// Test Unitari
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // Helper per creare chunk PNG sintetici validi: [length(4B)][type(4B)][data][CRC(4B)]
    fn create_png_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(chunk_type);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]); // Dummy CRC (skippato da seek)
        chunk
    }

    // Helper per comprimere con zlib nativo nei test
    fn compress_data(data: &[u8]) -> Vec<u8> {
        let mut dest_len = data.len().saturating_add(64);
        let mut dest = vec![0u8; dest_len];
        let ret = unsafe {
            compress(
                dest.as_mut_ptr(),
                &mut dest_len,
                data.as_ptr(),
                data.len(),
            )
        };
        assert_eq!(ret, 0, "Compressione zlib fallita");
        dest.truncate(dest_len);
        dest
    }

    // --- Test PNG ---

    #[test]
    fn test_png_text_chunk() {
        let mut png = PNG_MAGIC.to_vec();
        let mut payload = b"Author\0".to_vec();
        payload.extend_from_slice(b"Edmondo Giovannozzi");
        png.extend_from_slice(&create_png_chunk(b"tEXt", &payload));
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let entries = extract_png_metadata(&mut cursor).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "Author");
        assert_eq!(entries[0].value, "Edmondo Giovannozzi");
    }

    #[test]
    fn test_png_ztxt_compressed_chunk() {
        let mut png = PNG_MAGIC.to_vec();
        let uncompressed_code = b"import matplotlib.pyplot as plt\ndef plot(): pass\n";
        let compressed = compress_data(uncompressed_code);

        let mut payload = b"SourceCode\0\0".to_vec(); // Key + null + method 0 (deflate)
        payload.extend_from_slice(&compressed);

        png.extend_from_slice(&create_png_chunk(b"zTXt", &payload));
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let entries = extract_png_metadata(&mut cursor).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "SourceCode");
        assert_eq!(
            entries[0].value,
            "import matplotlib.pyplot as plt\ndef plot(): pass\n"
        );
    }

    #[test]
    fn test_png_invalid_header_rejection() {
        let not_a_png = b"GIF89a\x01\x00\x01\x00...not a png";
        let mut cursor = Cursor::new(not_a_png.to_vec());
        let entries = extract_png_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_png_oversized_chunk_protection() {
        let mut png = PNG_MAGIC.to_vec();
        // Chunk fittizio con lunghezza dichiarata superiore a 16MB (32MB)
        let fake_len = 32 * 1024 * 1024u32;
        png.extend_from_slice(&fake_len.to_be_bytes());
        png.extend_from_slice(b"tEXt");
        // Non alloca payload ma chiude subito: il parser deve skippare senza andare in OOM
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let entries = extract_png_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty());
    }

    // --- Test SVG & XML/JSON ---

    #[test]
    fn test_svg_metadata_with_json_in_description() {
        let svg_data = r#"<?xml version="1.0" encoding="utf-8" standalone="no"?>
<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
  <metadata>
    <rdf:RDF xmlns:dc="http://purl.org/dc/elements/1.1/">
      <dc:title>equilibrio.svg</dc:title>
      <dc:date>2026-09-10T10:00:00</dc:date>
      <dc:description>{
        &quot;directory&quot;: &quot;/home/plasma/sim&quot;,
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

        assert_eq!(resolve_key(&entries, "Title").unwrap().value, "equilibrio.svg");
        assert_eq!(resolve_key(&entries, "Description/filename").unwrap().value, "run.py");
        assert_eq!(
            resolve_key(&entries, "source_code").unwrap().value,
            "import numpy as np\ndef solve(): pass\n"
        );
        // Normalizzazione automatica: camelCase -> snake_case
        assert_eq!(
            resolve_key(&entries, "SourceCode").unwrap().value,
            "import numpy as np\ndef solve(): pass\n"
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
    fn test_xml_entity_unescaping() {
        let raw = "&quot;Hello &amp; &lt;World&gt;&apos; &#38; &#x41;";
        let decoded = xml_unescape(raw);
        assert_eq!(decoded, "\"Hello & <World>' & A");
    }

    #[test]
    fn test_json_recursion_depth_limit() {
        // Genera un JSON annidato oltre il limite MAX_JSON_RECURSION_DEPTH (8)
        let deep_json = "{\"a\":{\"b\":{\"c\":{\"d\":{\"e\":{\"f\":{\"g\":{\"h\":{\"i\":{\"j\":\"deep\"}}}}}}}}}}";
        let parsed = parse_json_object(deep_json, 0);
        // Deve rifiutare l'albero per prevenire stack overflow
        assert!(parsed.is_none());
    }

    // --- Test Risoluzione Chiavi (--cat) ---

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

        // Match su percorso completo
        assert_eq!(resolve_key(&entries, "Description/source_code").unwrap().value, "x = 42");
        // Match solo sul nodo foglia
        assert_eq!(resolve_key(&entries, "source_code").unwrap().value, "x = 42");
        // Match normalizzato (ignora '-' o '_')
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

        // Match ambiguo deve restituire errore
        let res = resolve_key(&entries, "Status");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Ambiguous key"));

        // Specificando il percorso completo deve risolvere univocamente
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

    // --- Test Utility & Sicurezza Terminale ---

    #[test]
    fn test_sanitize_terminal_escapes() {
        // Escape ANSI colore rosso (\x1b[31m), reset (\x1b[0m) e carattere di controllo Bell (\x07)
        let malicious_str = "\x1b[31mMalicious\x1b[0m\x07Text\nLine 2\tTabbed";
        let sanitized = sanitize_for_terminal(malicious_str);

        assert!(!sanitized.contains('\x1b'));
        assert!(!sanitized.contains('\x07'));
        assert_eq!(sanitized, "MaliciousText\nLine 2\tTabbed");
    }

    #[test]
    fn test_ignored_directories() {
        assert!(is_ignored_dir("venv"));
        assert!(is_ignored_dir("venv/"));
        assert!(is_ignored_dir(".venv"));
        assert!(is_ignored_dir("VENV"));
        assert!(is_ignored_dir("__pycache__"));
        assert!(is_ignored_dir(".git"));
        assert!(is_ignored_dir("target"));

        assert!(!is_ignored_dir("figures"));
        assert!(!is_ignored_dir("my_venv"));
        assert!(!is_ignored_dir("venv_output"));
    }
}



