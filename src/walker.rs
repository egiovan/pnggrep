use crate::model::{
    find_local_modules, is_fusion_key, matches_fusion_pulse, normalize_key, resolve_key,
    MetadataEntry,
};
use crate::pdf::extract_pdf_metadata;
use crate::png::extract_png_metadata_filtered;
use crate::svg::extract_svg_metadata;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

pub const DEFAULT_IGNORED_DIRS: &[&str] = &[
    "venv", ".venv", "env", ".env", "target", "build", "dist",
    "node_modules", ".tox", ".pytest_cache", ".mypy_cache",
];

/// Directories skipped silently without printing notices to stderr
pub const SILENT_IGNORED_DIRS: &[&str] = &[
    // Version control and bytecode caches
    ".git", "__pycache__", ".hg", ".svn",
    // Language toolchains & package stores
    ".julia", ".juliaup",
    ".rustup", ".cargo",
    // Remote editor servers & settings
    ".vscode-server", ".vscode",
    // Python user caches & notebook environments
    ".conda", ".cache", ".local", ".ipython", ".jupyter",
    // JavaScript runtimes & package managers
    ".npm", ".nvm", ".yarn",
    // Container runtimes & HPC package managers
    ".singularity", ".apptainer", ".docker", ".spack",
];

#[derive(Clone, Debug)]
pub enum RunMode {
    Search {
        pattern: String,
        key_filter: Option<String>,
        keys_only: bool,
        files_only: bool,
        json: bool,
    },
    List {
        max_lines: usize,
        key_filter: Option<String>,
        json: bool,
    },
    Pulse {
        pulse: String,
        files_only: bool,
        json: bool,
    },
    ListModules {
        json: bool,
    },
}

pub fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

pub fn sanitize_for_terminal(input: &str) -> String {
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

pub fn is_silent_ignored_dir(dir_name: &str) -> bool {
    let clean_name = dir_name.trim_end_matches('/');
    SILENT_IGNORED_DIRS
        .iter()
        .any(|&ignored| ignored.eq_ignore_ascii_case(clean_name))
}

pub fn is_ignored_dir(dir_name: &str) -> bool {
    let clean_name = dir_name.trim_end_matches('/');
    DEFAULT_IGNORED_DIRS
        .iter()
        .any(|&ignored| ignored.eq_ignore_ascii_case(clean_name))
}

pub fn is_user_excluded(path: &Path, dir_name: &str, custom_excludes: &[String]) -> bool {
    for exc in custom_excludes {
        let clean_exc = exc.trim_end_matches('/');
        if dir_name.eq_ignore_ascii_case(clean_exc) {
            return true;
        }
        if path == Path::new(clean_exc) || path.ends_with(clean_exc) {
            return true;
        }
    }
    false
}

pub fn is_python_venv(path: &Path) -> bool {
    if path.join("pyvenv.cfg").is_file() {
        return true;
    }
    if path.join("conda-meta").is_dir() {
        return true;
    }
    if (path.join("bin").join("activate").is_file() && path.join("bin").join("python").is_file())
        || path.join("Scripts").join("activate.bat").is_file()
    {
        return true;
    }
    false
}

pub fn is_supported_ext(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("png")
        || ext.eq_ignore_ascii_case("svg")
        || ext.eq_ignore_ascii_case("pdf")
}

pub fn extract_metadata(path: &Path) -> std::io::Result<Vec<MetadataEntry>> {
    extract_metadata_filtered(path, None)
}

pub fn extract_metadata_filtered(
    path: &Path,
    chunk_filter: Option<&dyn Fn(&str) -> bool>,
) -> std::io::Result<Vec<MetadataEntry>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    match ext.as_str() {
        "png" => extract_png_metadata_filtered(&mut reader, chunk_filter),
        "svg" => extract_svg_metadata(&mut reader),
        "pdf" => extract_pdf_metadata(&mut reader),
        _ => Ok(Vec::new()),
    }
}

pub fn search_pulse_file(path: &Path, pulse: &str, files_only: bool, json: bool) {
    let filter = |k: &str| is_fusion_key(k);
    if let Ok(entries) = extract_metadata_filtered(path, Some(&filter)) {
        let matched: Vec<&MetadataEntry> = entries
            .iter()
            .filter(|e| matches_fusion_pulse(&e.key, &e.value, pulse))
            .collect();

        if matched.is_empty() {
            return;
        }

        if files_only {
            println!("{}", path.display());
            return;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if json {
            let mut json_str = format!(
                "{{\"path\":\"{}\",\"format\":\"{}\",\"matches\":[",
                escape_json(&path.display().to_string()),
                escape_json(&ext)
            );
            for (i, entry) in matched.iter().enumerate() {
                if i > 0 {
                    json_str.push(',');
                }
                json_str.push_str(&format!(
                    "{{\"key\":\"{}\",\"value\":\"{}\"}}",
                    escape_json(&entry.key),
                    escape_json(&entry.value)
                ));
            }
            json_str.push_str("]}");
            println!("{}", json_str);
            return;
        }

        println!("\x1b[1;35m{}\x1b[0m", path.display());
        for entry in matched {
            let clean_key = sanitize_for_terminal(&entry.key);
            let clean_val = sanitize_for_terminal(&entry.value);
            println!("  \x1b[36m{}\x1b[0m: \x1b[32m{}\x1b[0m", clean_key, clean_val);
        }
        println!();
    }
}

pub fn list_modules_file(path: &Path, json: bool) {
    let filter = |k: &str| {
        let norm = normalize_key(k);
        norm.ends_with("localmodules")
    };
    if let Ok(entries) = extract_metadata_filtered(path, Some(&filter)) {
        let modules = find_local_modules(&entries);
        if modules.is_empty() {
            return;
        }

        if json {
            let mut json_str = format!(
                "{{\"path\":\"{}\",\"modules\":[",
                escape_json(&path.display().to_string())
            );
            for (i, m) in modules.iter().enumerate() {
                if i > 0 {
                    json_str.push(',');
                }
                json_str.push_str(&format!(
                    "{{\"name\":\"{}\",\"relpath\":\"{}\",\"lines\":{}}}",
                    escape_json(&m.name),
                    escape_json(&m.relpath),
                    m.source_code.lines().count()
                ));
            }
            json_str.push_str("]}");
            println!("{}", json_str);
        } else {
            println!("\x1b[1;35m{}\x1b[0m", path.display());
            for m in modules {
                let line_count = m.source_code.lines().count();
                if m.relpath.is_empty() {
                    println!("  \x1b[36m{}\x1b[0m \x1b[90m({} lines)\x1b[0m", m.name, line_count);
                } else {
                    println!(
                        "  \x1b[36m{}\x1b[0m \x1b[90m[{}]\x1b[0m \x1b[90m({} lines)\x1b[0m",
                        m.name, m.relpath, line_count
                    );
                }
            }
            println!();
        }
    }
}

pub fn cat_module(file_path: &Path, module_name: &str) {
    let filter = |k: &str| {
        let norm = normalize_key(k);
        norm.ends_with("localmodules")
    };
    let entries = match extract_metadata_filtered(file_path, Some(&filter)) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("Error reading {}: {}", file_path.display(), err);
            std::process::exit(1);
        }
    };

    let modules = find_local_modules(&entries);
    if modules.is_empty() {
        eprintln!("No local modules found in {}", file_path.display());
        std::process::exit(1);
    }

    if let Some(m) = modules.iter().find(|m| m.name.eq_ignore_ascii_case(module_name)) {
        print!("{}", m.source_code);
        if !m.source_code.ends_with('\n') {
            println!();
        }
    } else {
        let available: Vec<String> = modules
            .iter()
            .map(|m| format!("  - {} ({})", m.name, m.relpath))
            .collect();
        eprintln!(
            "Module '{}' not found in {}. Available modules:\n{}",
            module_name,
            file_path.display(),
            available.join("\n")
        );
        std::process::exit(1);
    }
}

pub fn search_file(
    path: &Path,
    pattern_lower: &str,
    key_filter: Option<&str>,
    keys_only: bool,
    files_only: bool,
    json: bool,
) {
    let chunk_filter = key_filter.map(|kf| {
        let kf_string = kf.to_lowercase();
        move |k: &str| k.to_lowercase().contains(&kf_string)
    });

    let filter_ref = chunk_filter.as_ref().map(|f| f as &dyn Fn(&str) -> bool);

    if let Ok(entries) = extract_metadata_filtered(path, filter_ref) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let is_sanitized = entries.iter().any(|e| {
            (e.key.eq_ignore_ascii_case("SanitizedBy")
                || e.key.eq_ignore_ascii_case("Keywords/sanitized_by")
                || e.key.eq_ignore_ascii_case("Description/sanitized_by")
                || e.key.ends_with("/sanitized_by"))
                && e.value.eq_ignore_ascii_case("pnggrep")
        });

        if json {
            let mut matched_entries: Vec<(String, Vec<(usize, String)>)> = Vec::new();

            for entry in entries {
                if let Some(kf) = key_filter {
                    if !entry.key.to_lowercase().contains(kf) {
                        continue;
                    }
                }

                let matches_key = entry.key.to_lowercase().contains(pattern_lower);
                let matches_val = entry.value.to_lowercase().contains(pattern_lower);

                if matches_key || matches_val {
                    if files_only {
                        println!(
                            "{{\"path\":\"{}\",\"format\":\"{}\",\"is_sanitized\":{}}}",
                            escape_json(&path.display().to_string()),
                            escape_json(&ext),
                            is_sanitized
                        );
                        return;
                    }

                    let mut matched_lines = Vec::new();
                    if !keys_only {
                        for (line_idx, line) in entry.value.lines().enumerate() {
                            if line.to_lowercase().contains(pattern_lower) {
                                matched_lines.push((line_idx + 1, line.to_string()));
                            }
                        }
                    }
                    matched_entries.push((entry.key, matched_lines));
                }
            }

            if !matched_entries.is_empty() {
                let mut json_str = String::new();
                json_str.push_str(&format!(
                    "{{\"path\":\"{}\",\"format\":\"{}\",\"is_sanitized\":{},\"matches\":[",
                    escape_json(&path.display().to_string()),
                    escape_json(&ext),
                    is_sanitized
                ));

                for (i, (key, lines)) in matched_entries.iter().enumerate() {
                    if i > 0 {
                        json_str.push(',');
                    }
                    json_str.push_str(&format!("{{\"key\":\"{}\",\"lines\":[", escape_json(key)));
                    for (j, (line_no, line_content)) in lines.iter().enumerate() {
                        if j > 0 {
                            json_str.push(',');
                        }
                        json_str.push_str(&format!(
                            "{{\"line_number\":{},\"content\":\"{}\"}}",
                            line_no,
                            escape_json(line_content)
                        ));
                    }
                    json_str.push_str("]}");
                }
                json_str.push_str("]}");
                println!("{}", json_str);
            }
            return;
        }

        // Terminal Mode
        for entry in entries {
            if let Some(kf) = key_filter {
                if !entry.key.to_lowercase().contains(kf) {
                    continue;
                }
            }

            if entry.value.to_lowercase().contains(pattern_lower)
                || entry.key.to_lowercase().contains(pattern_lower)
            {
                if files_only {
                    println!("{}", path.display());
                    return;
                }

                let clean_key = sanitize_for_terminal(&entry.key);
                println!("\x1b[35m{}\x1b[0m [\x1b[36m{}\x1b[0m]", path.display(), clean_key);

                if !keys_only {
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
}

pub fn list_file(path: &Path, max_lines: usize, key_filter: Option<&str>, json: bool) {
    let chunk_filter = key_filter.map(|kf| {
        let kf_string = kf.to_lowercase();
        move |k: &str| k.to_lowercase().contains(&kf_string)
    });

    let filter_ref = chunk_filter.as_ref().map(|f| f as &dyn Fn(&str) -> bool);

    if let Ok(entries) = extract_metadata_filtered(path, filter_ref) {
        let filtered: Vec<MetadataEntry> = entries
            .into_iter()
            .filter(|e| {
                if let Some(kf) = key_filter {
                    e.key.to_lowercase().contains(kf)
                } else {
                    true
                }
            })
            .collect();

        if filtered.is_empty() {
            return;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let is_sanitized = filtered.iter().any(|e| {
            (e.key.eq_ignore_ascii_case("SanitizedBy")
                || e.key.eq_ignore_ascii_case("Keywords/sanitized_by")
                || e.key.eq_ignore_ascii_case("Description/sanitized_by")
                || e.key.ends_with("/sanitized_by"))
                && e.value.eq_ignore_ascii_case("pnggrep")
        });

        if json {
            let mut json_str = String::new();
            json_str.push_str(&format!(
                "{{\"path\":\"{}\",\"format\":\"{}\",\"is_sanitized\":{},\"entries\":[",
                escape_json(&path.display().to_string()),
                escape_json(&ext),
                is_sanitized
            ));

            for (i, entry) in filtered.iter().enumerate() {
                if i > 0 {
                    json_str.push(',');
                }
                json_str.push_str(&format!(
                    "{{\"key\":\"{}\",\"value\":\"{}\"}}",
                    escape_json(&entry.key),
                    escape_json(&entry.value)
                ));
            }
            json_str.push_str("]}");
            println!("{}", json_str);
            return;
        }

        // Terminal Mode
        println!("\x1b[1;35m{}\x1b[0m", path.display());

        for entry in filtered {
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

pub fn cat_metadata(file_path: &Path, target_key: &str) {
    let norm_target = normalize_key(target_key);
    let filter = move |k: &str| {
        let norm = normalize_key(k);
        norm == norm_target || norm.ends_with(&norm_target)
    };

    let entries = match extract_metadata_filtered(file_path, Some(&filter)) {
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

pub fn visit_dirs(dir: &Path, mode: &RunMode, custom_excludes: &[String]) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    // 1. User-defined exclusions (always notify)
                    if is_user_excluded(&path, dir_name, custom_excludes) {
                        eprintln!(
                            "\x1b[33m[info]\x1b[0m Skipping excluded directory: \x1b[90m{}\x1b[0m",
                            path.display()
                        );
                        continue;
                    }

                    // 2. Silent skips (internal VCS and bytecode caches)
                    if is_silent_ignored_dir(dir_name) {
                        continue;
                    }

                    // 3. Default ignored directories (notify)
                    if is_ignored_dir(dir_name) {
                        eprintln!(
                            "\x1b[33m[info]\x1b[0m Skipping default ignored directory: \x1b[90m{}\x1b[0m",
                            path.display()
                        );
                        continue;
                    }
                }

                // 4. Dynamic Python virtual environment check (notify)
                if is_python_venv(&path) {
                    eprintln!(
                        "\x1b[33m[info]\x1b[0m Skipping virtual environment: \x1b[90m{}\x1b[0m",
                        path.display()
                    );
                    continue;
                }
                visit_dirs(&path, mode, custom_excludes);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if is_supported_ext(ext) {
                        match mode {
                            RunMode::Search {
                                pattern,
                                key_filter,
                                keys_only,
                                files_only,
                                json,
                            } => search_file(
                                &path,
                                pattern,
                                key_filter.as_deref(),
                                *keys_only,
                                *files_only,
                                *json,
                            ),
                            RunMode::List {
                                max_lines,
                                key_filter,
                                json,
                            } => list_file(&path, *max_lines, key_filter.as_deref(), *json),
                            RunMode::Pulse {
                                pulse,
                                files_only,
                                json,
                            } => search_pulse_file(&path, pulse, *files_only, *json),
                            RunMode::ListModules { json } => list_modules_file(&path, *json),
                        }
                    }
                }
            }
        }
    }
}

/// Dispatches a target path (file or directory) according to the active mode
pub fn process_target(target: &Path, mode: &RunMode, custom_excludes: &[String]) {
    if target.is_file() {
        if let Some(ext) = target.extension().and_then(|e| e.to_str()) {
            if is_supported_ext(ext) {
                match mode {
                    RunMode::Search {
                        pattern,
                        key_filter,
                        keys_only,
                        files_only,
                        json,
                    } => {
                        search_file(
                            target,
                            pattern,
                            key_filter.as_deref(),
                            *keys_only,
                            *files_only,
                            *json,
                        );
                    }
                    RunMode::List {
                        max_lines,
                        key_filter,
                        json,
                    } => {
                        list_file(target, *max_lines, key_filter.as_deref(), *json);
                    }
                    RunMode::Pulse {
                        pulse,
                        files_only,
                        json,
                    } => {
                        search_pulse_file(target, pulse, *files_only, *json);
                    }
                    RunMode::ListModules { json } => {
                        list_modules_file(target, *json);
                    }
                }
            } else {
                eprintln!(
                    "\x1b[33m[warning]\x1b[0m Unsupported image format: {}",
                    target.display()
                );
            }
        }
    } else if target.is_dir() {
        visit_dirs(target, mode, custom_excludes);
    } else {
        eprintln!(
            "\x1b[31m[error]\x1b[0m Path not found: {}",
            target.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_json() {
        let input = "quote: \", backslash: \\, tab: \t, newline: \n";
        let escaped = escape_json(input);
        assert_eq!(escaped, "quote: \\\", backslash: \\\\, tab: \\t, newline: \\n");
    }

    #[test]
    fn test_sanitize_terminal_escapes() {
        let malicious_str = "\x1b[31mMalicious\x1b[0m\x07Text\nLine 2\tTabbed";
        let sanitized = sanitize_for_terminal(malicious_str);

        assert!(!sanitized.contains('\x1b'));
        assert!(!sanitized.contains('\x07'));
        assert_eq!(sanitized, "MaliciousText\nLine 2\tTabbed");
    }

    #[test]
    fn test_silent_ignored_directories() {
        assert!(is_silent_ignored_dir(".git"));
        assert!(is_silent_ignored_dir(".git/"));
        assert!(is_silent_ignored_dir("__pycache__"));
        assert!(is_silent_ignored_dir(".hg"));
        assert!(is_silent_ignored_dir(".svn"));
        assert!(is_silent_ignored_dir(".juliaup"));
        assert!(is_silent_ignored_dir(".rustup"));
        assert!(is_silent_ignored_dir(".vscode-server"));
        assert!(is_silent_ignored_dir(".cargo"));
        assert!(is_silent_ignored_dir(".conda"));

        assert!(!is_silent_ignored_dir("venv"));
        assert!(!is_silent_ignored_dir("target"));
        assert!(!is_silent_ignored_dir("build"));
    }

    #[test]
    fn test_ignored_directories() {
        assert!(is_ignored_dir("venv"));
        assert!(is_ignored_dir("venv/"));
        assert!(is_ignored_dir(".venv"));
        assert!(is_ignored_dir("VENV"));
        assert!(is_ignored_dir("target"));

        assert!(!is_ignored_dir("figures"));
        assert!(!is_ignored_dir("my_venv"));
        assert!(!is_ignored_dir("venv_output"));
    }

    #[test]
    fn test_user_excluded_directory() {
        let excludes = vec!["scratch_dir".to_string(), "archive/old_runs".to_string()];
        assert!(is_user_excluded(Path::new("runs/scratch_dir"), "scratch_dir", &excludes));
        assert!(is_user_excluded(Path::new("archive/old_runs"), "old_runs", &excludes));
        assert!(!is_user_excluded(Path::new("runs/active_run"), "active_run", &excludes));
    }

    #[test]
    fn test_dynamic_venv_detection() {
        let temp_base = std::env::temp_dir().join("pnggrep_venv_test");
        let _ = fs::remove_dir_all(&temp_base);
        fs::create_dir_all(&temp_base).unwrap();

        let custom_venv = temp_base.join("custom_env_folder");
        fs::create_dir_all(&custom_venv).unwrap();
        fs::File::create(custom_venv.join("pyvenv.cfg")).unwrap();
        assert!(is_python_venv(&custom_venv));

        let conda_env = temp_base.join("my_conda_env");
        fs::create_dir_all(conda_env.join("conda-meta")).unwrap();
        assert!(is_python_venv(&conda_env));

        let normal_dir = temp_base.join("regular_plot_folder");
        fs::create_dir_all(&normal_dir).unwrap();
        assert!(!is_python_venv(&normal_dir));

        let _ = fs::remove_dir_all(&temp_base);
    }
}