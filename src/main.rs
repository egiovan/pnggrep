use std::env;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

#[link(name = "z")]
extern "C" {
    fn uncompress(
        dest: *mut u8,
        dest_len: *mut usize,
        source: *const u8,
        source_len: usize,
    ) -> i32;
}

/// Decomprime dati zlib usando libz.so di sistema (Z_OK = 0, Z_BUF_ERROR = -5)
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
            // Buffer insufficiente: raddoppia e riprova
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
                reader.seek(SeekFrom::Current(4))?; // Skip CRC
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
                reader.seek(SeekFrom::Current(4))?; // Skip CRC
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
                reader.seek(SeekFrom::Current(4))?; // Skip CRC
            }

            _ => {
                reader.seek(SeekFrom::Current(length as i64 + 4))?;
            }
        }
    }

    Ok(entries)
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


fn visit_dirs(dir: &Path, pattern_lower: &str) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, pattern_lower);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("png") {
                        search_file(&path, pattern_lower);
                    }
                }
            }
        }
    }
}
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: pnggrep <PATTERN> [PATH]");
        std::process::exit(1);
    }

    let pattern = &args[1];
    let target_dir = if args.len() >= 3 {
        PathBuf::from(&args[2])
    } else {
        PathBuf::from(".")
    };

    visit_dirs(&target_dir, &pattern.to_lowercase());
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
        chunk.extend_from_slice(&[0, 0, 0, 0]); // Dummy CRC
        chunk
    }

    #[test]
    fn test_extract_text_chunk() {
        let mut png_bytes = PNG_MAGIC.to_vec();
        let mut text_payload = b"Comment\0".to_vec();
        text_payload.extend_from_slice(b"def compute_flux(): pass");
        png_bytes.extend_from_slice(&create_chunk(b"tEXt", &text_payload));
        png_bytes.extend_from_slice(&create_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png_bytes);
        let entries = extract_png_metadata(&mut cursor).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "Comment");
        assert_eq!(entries[0].value, "def compute_flux(): pass");
    }

    #[test]
    fn test_non_png_file() {
        let dummy = b"NOT_A_PNG_FILE".to_vec();
        let mut cursor = Cursor::new(dummy);
        let entries = extract_png_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty());
    }
}
