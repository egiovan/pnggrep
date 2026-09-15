mod model;
mod pdf;
mod png;
mod sanitize;
mod svg;
mod walker;

use sanitize::sanitize_target;
use std::env;
use std::path::{Path, PathBuf};
use walker::{cat_metadata, process_target, RunMode};

fn print_help() {
    println!("pnggrep - Search, inspect, and sanitize embedded figure metadata\n");
    println!("USAGE:");
    println!("    pnggrep <PATTERN> [PATHS...] [OPTIONS]   Search pattern in figures or directories");
    println!("    pnggrep -l [-n LINES] [PATHS...]         List metadata across figures or directories");
    println!("    pnggrep --cat <KEY> <FILE>               Print raw value of a metadata key");
    println!("    pnggrep --sanitize [PATHS...]            Strip metadata and write <file>_sanitized.<ext>\n");
    println!("OPTIONS:");
    println!("    -l                          List metadata mode (no pattern required)");
    println!("    -n <LINES>                  Max lines to show per key in list mode [default: 3, 0=all]");
    println!("    -k, --key <SUBSTRING>       Filter metadata keys matching substring (case-insensitive)");
    println!("    -K, --keys-only             Show only matching metadata key names, omitting lines");
    println!("    --sanitize                  Strip sensitive metadata and create _sanitized files");
    println!("    -s, --skip, --exclude <DIR> Skip specific directory during traversal (repeatable)");
    println!("    --cat, -cat <KEY>           Dump exact value without formatting (1 file only)");
    println!("    -h, --help                  Show help information");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_help();
        std::process::exit(1);
    }

    let mut is_list = false;
    let mut is_sanitize = false;
    let mut max_lines = 3usize;
    let mut key_filter: Option<String> = None;
    let mut keys_only = false;
    let mut cat_key: Option<String> = None;
    let mut custom_excludes = Vec::new();
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
            "--sanitize" => {
                is_sanitize = true;
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
            "-k" | "--key" => {
                if i + 1 < args.len() {
                    i += 1;
                    key_filter = Some(args[i].to_lowercase());
                } else {
                    eprintln!("Error: {} requires a key pattern.", args[i]);
                    std::process::exit(1);
                }
            }
            "-K" | "--keys-only" => {
                keys_only = true;
            }
            "-s" | "--skip" | "--exclude" => {
                if i + 1 < args.len() {
                    i += 1;
                    custom_excludes.push(args[i].clone());
                } else {
                    eprintln!("Error: {} requires a directory name or path.", args[i]);
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

    // -------------------------------------------------------------------------
    // 1. --cat Mode (Strict single-file requirement)
    // -------------------------------------------------------------------------
    if let Some(key) = cat_key {
        if positional.len() != 1 {
            eprintln!(
                "Error: --cat requires exactly one target file: pnggrep --cat <KEY> <FILE>\nReceived {} path argument(s).",
                positional.len()
            );
            std::process::exit(1);
        }
        cat_metadata(Path::new(&positional[0]), &key);
        return;
    }

    // -------------------------------------------------------------------------
    // 2. --sanitize Mode (Batch files/directories supported)
    // -------------------------------------------------------------------------
    if is_sanitize {
        let targets: Vec<PathBuf> = if positional.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            positional.into_iter().map(PathBuf::from).collect()
        };

        for target in &targets {
            sanitize_target(target, &custom_excludes);
        }
        return;
    }

    // -------------------------------------------------------------------------
    // 3. -l (List) Mode (Multiple files/directories supported)
    // -------------------------------------------------------------------------
    if is_list {
        let targets: Vec<PathBuf> = if positional.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            positional.into_iter().map(PathBuf::from).collect()
        };

        let mode = RunMode::List {
            max_lines,
            key_filter,
        };

        for target in &targets {
            process_target(target, &mode, &custom_excludes);
        }
        return;
    }

    // -------------------------------------------------------------------------
    // 4. Search Mode (Pattern + multiple files/directories supported)
    // -------------------------------------------------------------------------
    if positional.is_empty() {
        eprintln!("Error: Missing search pattern. Use -l to list all metadata.");
        std::process::exit(1);
    }

    let pattern = positional[0].to_lowercase();
    let targets: Vec<PathBuf> = if positional.len() > 1 {
        positional[1..].iter().map(PathBuf::from).collect()
    } else {
        vec![PathBuf::from(".")]
    };

    let mode = RunMode::Search {
        pattern,
        key_filter,
        keys_only,
    };

    for target in &targets {
        process_target(target, &mode, &custom_excludes);
    }
}