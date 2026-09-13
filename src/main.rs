mod model;
mod png;
mod svg;
mod walker;

use std::env;
use std::path::{Path, PathBuf};
use walker::{cat_metadata, list_file, search_file, visit_dirs, RunMode};

fn print_help() {
    println!("pnggrep - Search and list embedded PNG and SVG metadata\n");
    println!("USAGE:");
    println!("    pnggrep <PATTERN> [PATH] [OPTIONS]  Search pattern in PNG/SVG metadata");
    println!("    pnggrep -l [-n LINES] [PATH]        List all metadata keys and values");
    println!("    pnggrep --cat <KEY> <FILE>          Print raw value of a metadata key\n");
    println!("OPTIONS:");
    println!("    -l                          List metadata mode (no pattern required)");
    println!("    -n <LINES>                  Max lines to show per key in list mode [default: 3, 0=all]");
    println!("    -s, --skip, --exclude <DIR> Skip specific directory during traversal (repeatable)");
    println!("    --cat, -cat <KEY>           Dump exact value without formatting");
    println!("    -h, --help                  Show help information");
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
            "-n" => {
                if i + 1 < args.len() {
                    i += 1;
                    max_lines = args[i].parse().unwrap_or(3);
                } else {
                    eprintln!("Error: -n requires a number of lines.");
                    std::process::exit(1);
                }
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
            visit_dirs(&target_dir, &mode, &custom_excludes);
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
            visit_dirs(&target_dir, &mode, &custom_excludes);
        }
    }
}