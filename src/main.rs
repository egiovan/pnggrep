use std::env;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    "venv",
    ".venv",
    "env",
    ".env",
    "__pycache__",
    ".git",
    ".hg",
    ".svn",
    "target",
    "build",
    "dist",
    "node_modules",
    ".tox",
    ".pytest_cache",
    ".mypy_cache",
    ".cache",
];

#[link(name = "z")]
extern "C" {
    fn uncompress(
        dest: *mut u8,
        dest_len: *mut usize,
        source: *const u8,
        source_len: usize,
    ) -> i32;
}

fn decompress_zlib(compressed: &[u8]) -> Option<String> {
    let mut dest_len = compressed.len().max(1024) * 4;
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
            dest_len *= 2;
            dest.resize(dest_len, 0);
        } else {
            return None;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
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
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut reader = BufReader::new(file);

    if let Ok(entries) = extract_png_metadata(&mut reader) {
        for entry in entries {
            if entry.value.to_lowercase().contains(pattern_lower)
                || entry.key.to_lowercase().contains(pattern_lower)
            {
                println!("\x1b[35m{}\x1b[0m [\x1b[36m{}\x1b[0m]", path.display(), entry.key);
                for (line_idx, line) in entry.value.lines().enumerate() {
                    if line.to_lowercase().contains(pattern_lower) {
                        println!("  \x1b[32m{:4}:\x1b[0m {}", line_idx + 1, line.trim_end());
                    }
                }
            }
        }
    }
}

fn list_file(path: &Path, max_lines: usize) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut reader = BufReader::new(file);

    if let Ok(entries) = extract_png_metadata(&mut reader) {
        if entries.is_empty() {
            return;
        }

        println!("\x1b[1;35m{}\x1b[0m", path.display());

        for entry in entries {
            let lines: Vec<&str> = entry.value.lines().collect();

            if lines.is_empty() {
                println!("  \x1b[36m{}\x1b[0m: (empty)", entry.key);
            } else if lines.len() == 1 {
                println!("  \x1b[36m{}\x1b[0m: {}", entry.key, lines[0]);
            } else {
                println!("  \x1b[36m{}\x1b[0m:", entry.key);
                let limit = if max_lines == 0 { lines.len() } else { max_lines.min(lines.len()) };

                for line in &lines[..limit] {
                    println!("    \x1b[90m|\x1b[0m {}", line.trim_end());
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
    let file = match File::open(file_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error opening {}: {}", file_path.display(), e);
            std::process::exit(1);
        }
    };
    let mut reader = BufReader::new(file);

    if let Ok(entries) = extract_png_metadata(&mut reader) {
        for entry in entries {
            if entry.key.eq_ignore_ascii_case(target_key) {
                print!("{}", entry.value);
                if !entry.value.ends_with('\n') {
                    println!();
                }
                return;
            }
        }
    }
    eprintln!("Key '{}' not found in {}", target_key, file_path.display());
    std::process::exit(1);
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
                    if ext.eq_ignore_ascii_case("png") {
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
    println!("pnggrep - Search and list embedded PNG metadata\n");
    println!("USAGE:");
    println!("    pnggrep <PATTERN> [PATH]            Search for pattern inside metadata");
    println!("    pnggrep -l [-n LINES] [PATH]        List all metadata keys and values");
    println!("    pnggrep --cat <KEY> <FILE>          Print raw value of a metadata key\n");
    println!("OPTIONS:");
    println!("    -l                  List metadata mode (no search pattern required)");
    println!("    -n <LINES>          Max lines to show per key in list mode [default: 3, 0=all]");
    println!("    --cat <KEY> <FILE>  Dump exact value without formatting (useful for scripts)");
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
                }
            }
            "--cat" => {
                if i + 1 < args.len() {
                    i += 1;
                    cat_key = Some(args[i].clone());
                }
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
        let mode = RunMode::Search { pattern };
        if target_dir.is_file() {
            if let RunMode::Search { ref pattern } = mode {
                search_file(&target_dir, pattern);
            }
        } else {
            visit_dirs(&target_dir, &mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn create_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(chunk_type);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        chunk
    }

    #[test]
    fn test_extract_text_chunk() {
        let mut png_bytes = PNG_MAGIC.to_vec();
        let mut text_payload = b"Comment\0".to_vec();
        text_payload.extend_from_slice(b"def compute():\n    x = 1\n    return x\n");
        png_bytes.extend_from_slice(&create_chunk(b"tEXt", &text_payload));
        png_bytes.extend_from_slice(&create_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png_bytes);
        let entries = extract_png_metadata(&mut cursor).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "Comment");
        assert!(entries[0].value.contains("def compute():"));
    }

    #[test]
    fn test_ignored_directories() {
        assert!(is_ignored_dir("venv"));
        assert!(is_ignored_dir(".venv"));
        assert!(is_ignored_dir("__pycache__"));
        assert!(!is_ignored_dir("figures"));
    }
}
