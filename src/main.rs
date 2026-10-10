mod model;
mod pdf;
mod png;
mod sanitize;
mod svg;
mod walker;

use sanitize::sanitize_target;
use std::env;
use std::path::{Path, PathBuf};
use walker::{cat_metadata, cat_module, process_target, DateFilter, RunMode};

fn print_help() {
    println!("pnggrep - Search, inspect, and sanitize embedded figure metadata\n");
    println!("USAGE:");
    println!("    pnggrep <PATTERN> [PATHS...] [OPTIONS]   Search pattern in figures or directories");
    println!("    pnggrep -l [-n LINES] [PATHS...]         List metadata across figures or directories");
    println!("    pnggrep -p <PULSE> [PATHS...]            Unified fusion search (Shot, Shots, Pulse, Pulses)");
    println!("    pnggrep -M [PATHS...]                    List local modules bundled in figures");
    println!("    pnggrep --cat-module <NAME> <FILE>       Print source code of a bundled module");
    println!("    pnggrep --cat <KEY> <FILE>               Print raw value of a metadata key");
    println!("    pnggrep --sanitize [PATHS...]            Strip metadata and write <file>_sanitized.<ext>\n");
    println!("OPTIONS:");
    println!("    -l                          List metadata mode (no pattern required)");
    println!("    -n <LINES>                  Max lines to show per key in list mode [default: 3, 0=all]");
    println!("    -p, --pulse <PULSE>         Unified fusion search across Shot, Shots, Pulse, Pulses");
    println!("    -M, --modules               List bundled Python modules with names and line counts");
    println!("        --cat-module <NAME>     Dump source code of a specific local module (1 file only)");
    println!("        --after, --since <DATE> Match figures on or after date (YYYY, YYYY-MM, YYYY-MM-DD, ISO, 2d)");
    println!("        --before, --until <DATE> Match figures on or before date (YYYY, YYYY-MM, YYYY-MM-DD, ISO, 2d)");
    println!("    -k, --key <SUBSTRING>       Filter metadata keys matching substring (case-insensitive)");
    println!("    -K, --keys-only             Show only matching metadata key names, omitting lines");
    println!("    -F, --files-only            Print only filenames of matching figures (no details)");
    println!("        --files-with-matches    Alias for -F");
    println!("        --files-with-match      Alias for -F");
    println!("        --json                  Emit results as line-delimited JSON (ndjson)");
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
    let mut is_modules = false;
    let mut pulse_query: Option<String> = None;
    let mut cat_module_name: Option<String> = None;
    let mut after_arg: Option<String> = None;
    let mut before_arg: Option<String> = None;
    let mut max_lines = 3usize;
    let mut key_filter: Option<String> = None;
    let mut keys_only = false;
    let mut files_only = false;
    let mut is_json = false;
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
            "-M" | "--modules" | "--list-modules" => {
                is_modules = true;
            }
            "--sanitize" => {
                is_sanitize = true;
            }
            "--json" => {
                is_json = true;
            }
            "-p" | "--pulse" => {
                if i + 1 < args.len() {
                    i += 1;
                    pulse_query = Some(args[i].clone());
                } else {
                    eprintln!("Error: -p/--pulse requires a pulse or shot identifier.");
                    std::process::exit(1);
                }
            }
            "--cat-module" => {
                if i + 1 < args.len() {
                    i += 1;
                    cat_module_name = Some(args[i].clone());
                } else {
                    eprintln!("Error: --cat-module requires a module name: pnggrep --cat-module <NAME> <FILE>");
                    std::process::exit(1);
                }
            }
            "--after" | "--since" => {
                if i + 1 < args.len() {
                    i += 1;
                    after_arg = Some(args[i].clone());
                } else {
                    eprintln!("Error: {} requires a date or duration (e.g. 2026, 2026-10, 2026-10-01, 2d).", args[i]);
                    std::process::exit(1);
                }
            }
            "--before" | "--until" => {
                if i + 1 < args.len() {
                    i += 1;
                    before_arg = Some(args[i].clone());
                } else {
                    eprintln!("Error: {} requires a date or duration (e.g. 2026, 2026-10, 2026-10-01, 2d).", args[i]);
                    std::process::exit(1);
                }
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
            "-F" | "--files-only" | "--files-with-matches" | "--files-with-match" => {
                files_only = true;
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
    // Date Filter Parsing
    // -------------------------------------------------------------------------
    let after_epoch = if let Some(ref a) = after_arg {
        match model::parse_date_bound(a, false) {
            Ok(epoch) => Some(epoch),
            Err(e) => {
                eprintln!("Error parsing --after/--since '{}': {}", a, e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let before_epoch = if let Some(ref b) = before_arg {
        match model::parse_date_bound(b, true) {
            Ok(epoch) => Some(epoch),
            Err(e) => {
                eprintln!("Error parsing --before/--until '{}': {}", b, e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let date_filter = DateFilter {
        after: after_epoch,
        before: before_epoch,
    };

    // -------------------------------------------------------------------------
    // 1. --cat-module Mode (Strict single-file requirement)
    // -------------------------------------------------------------------------
    if let Some(mod_name) = cat_module_name {
        if positional.len() != 1 {
            eprintln!(
                "Error: --cat-module requires exactly one target file: pnggrep --cat-module <NAME> <FILE>\nReceived {} path argument(s).",
                positional.len()
            );
            std::process::exit(1);
        }
        cat_module(Path::new(&positional[0]), &mod_name);
        return;
    }

    // -------------------------------------------------------------------------
    // 2. --cat Mode (Strict single-file requirement)
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
    // 3. --sanitize Mode (Batch files/directories supported)
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
    // 4. -M / --modules Mode (List bundled modules across figures)
    // -------------------------------------------------------------------------
    if is_modules {
        let targets: Vec<PathBuf> = if positional.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            positional.into_iter().map(PathBuf::from).collect()
        };

        let mode = RunMode::ListModules { json: is_json };
        for target in &targets {
            process_target(target, &mode, &custom_excludes);
        }
        return;
    }

    // -------------------------------------------------------------------------
    // 5. -p / --pulse Mode (Unified fusion search across Shot/Pulse keys)
    // -------------------------------------------------------------------------
    if let Some(pulse) = pulse_query {
        let targets: Vec<PathBuf> = if positional.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            positional.into_iter().map(PathBuf::from).collect()
        };

        let mode = RunMode::Pulse {
            pulse,
            date_filter,
            files_only,
            json: is_json,
        };

        for target in &targets {
            process_target(target, &mode, &custom_excludes);
        }
        return;
    }

    // -------------------------------------------------------------------------
    // 6. -l (List) Mode (Multiple files/directories supported)
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
            date_filter,
            json: is_json,
        };

        for target in &targets {
            process_target(target, &mode, &custom_excludes);
        }
        return;
    }

    // -------------------------------------------------------------------------
    // 7. General Search Mode (Pattern + multiple files/directories supported)
    // -------------------------------------------------------------------------
    if positional.is_empty() {
        eprintln!("Error: Missing search pattern. Use -l to list all metadata, or -p for pulse search.");
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
        date_filter,
        keys_only,
        files_only,
        json: is_json,
    };

    for target in &targets {
        process_target(target, &mode, &custom_excludes);
    }
}