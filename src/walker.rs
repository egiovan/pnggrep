use crate::model::{resolve_key, MetadataEntry};
use crate::pdf::extract_pdf_metadata;
use crate::png::extract_png_metadata;
use crate::svg::extract_svg_metadata;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;


pub const DEFAULT_IGNORED_DIRS: &[&str] = &[
    "venv", ".venv", "env", ".env", "__pycache__", ".git", ".hg", ".svn",
    "target", "build", "dist", "node_modules", ".tox", ".pytest_cache",
    ".mypy_cache", ".cache",
];

/// Directories skipped silently without printing to stderr
pub const SILENT_IGNORED_DIRS: &[&str] = &[
    ".git", "__pycache__", ".hg", ".svn",
];

#[derive(Clone, Debug)]
pub enum RunMode {
    Search {
        pattern: String,
        key_filter: Option<String>,
    },
    List {
        max_lines: usize,
        key_filter: Option<String>,
    },
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
        "pdf" => extract_pdf_metadata(&mut reader),
        _ => Ok(Vec::new()),
    }
}

pub fn search_file(path: &Path, pattern_lower: &str, key_filter: Option<&str>) {
    if let Ok(entries) = extract_metadata(path) {
        for entry in entries {
            // Apply key substring filter if requested
            if let Some(kf) = key_filter {
                if !entry.key.to_lowercase().contains(kf) {
                    continue;
                }
            }

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

pub fn list_file(path: &Path, max_lines: usize, key_filter: Option<&str>) {
    if let Ok(entries) = extract_metadata(path) {
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

        // If no keys match the filter, do not display this file
        if filtered.is_empty() {
            return;
        }

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
                            RunMode::Search { pattern, key_filter } => {
                                search_file(&path, pattern, key_filter.as_deref())
                            }
                            RunMode::List { max_lines, key_filter } => {
                                list_file(&path, *max_lines, key_filter.as_deref())
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(is_ignored_dir("__pycache__"));
        assert!(is_ignored_dir(".git"));
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