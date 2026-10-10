use crate::model::MetadataEntry;
use std::io::{Read, Seek, SeekFrom};

pub const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
pub const MAX_METADATA_CHUNK_SIZE: u64 = 16 * 1024 * 1024; // 16 MB limit per chunk
pub const MAX_DECOMPRESSED_SIZE: usize = 32 * 1024 * 1024;  // 32 MB limit uncompressed

// -----------------------------------------------------------------------------
// Unix / Linux / macOS: System zlib (Zero external dependencies)
// -----------------------------------------------------------------------------
#[cfg(not(windows))]
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

#[cfg(not(windows))]
pub fn decompress_zlib(compressed: &[u8]) -> Option<String> {
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
            if dest_len >= MAX_DECOMPRESSED_SIZE {
                return None; // Exceeds safety threshold
            }
            dest_len = dest_len.saturating_mul(2).min(MAX_DECOMPRESSED_SIZE);
            dest.resize(dest_len, 0);
        } else {
            return None;
        }
    }
}

// -----------------------------------------------------------------------------
// Windows: flate2 (Pure Rust miniz_oxide backend)
// -----------------------------------------------------------------------------
#[cfg(windows)]
pub fn decompress_zlib(compressed: &[u8]) -> Option<String> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    if compressed.is_empty() {
        return None;
    }

    let mut decoder = ZlibDecoder::new(compressed);
    let mut dest = Vec::new();
    if decoder
        .by_ref()
        .take(MAX_DECOMPRESSED_SIZE as u64)
        .read_to_end(&mut dest)
        .is_ok()
    {
        String::from_utf8(dest).ok()
    } else {
        None
    }
}

// -----------------------------------------------------------------------------
// Chunk Extraction with Lazy Decompression
// -----------------------------------------------------------------------------
#[allow(dead_code)]
pub fn extract_png_metadata<R: Read + Seek>(reader: &mut R) -> std::io::Result<Vec<MetadataEntry>> {
    extract_png_metadata_filtered(reader, None)
}

pub fn extract_png_metadata_filtered<R: Read + Seek>(
    reader: &mut R,
    key_filter: Option<&dyn Fn(&str) -> bool>,
) -> std::io::Result<Vec<MetadataEntry>> {
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

        if length > MAX_METADATA_CHUNK_SIZE {
            reader.seek(SeekFrom::Current(length as i64 + 4))?;
            continue;
        }

        match &type_buf {
            b"IEND" => break,

            b"tEXt" => {
                let probe_len = (length as usize).min(80);
                let mut probe = vec![0u8; probe_len];
                reader.read_exact(&mut probe)?;

                if let Some(pos) = probe.iter().position(|&b| b == 0) {
                    let key = String::from_utf8_lossy(&probe[..pos]).to_string();

                    // Lazy check: skip reading remaining uncompressed text if not needed
                    if let Some(pred) = key_filter {
                        if !pred(&key) {
                            let rem = length - (probe_len as u64);
                            reader.seek(SeekFrom::Current(rem as i64 + 4))?;
                            continue;
                        }
                    }

                    reader.seek(SeekFrom::Current(-(probe_len as i64)))?;
                    let mut data = vec![0u8; length as usize];
                    reader.read_exact(&mut data)?;
                    let val = String::from_utf8_lossy(&data[pos + 1..]).to_string();
                    entries.push(MetadataEntry { key, value: val });
                } else {
                    let rem = length - (probe_len as u64);
                    reader.seek(SeekFrom::Current(rem as i64))?;
                }
                reader.seek(SeekFrom::Current(4))?; // CRC
            }

            b"zTXt" => {
                let probe_len = (length as usize).min(80);
                let mut probe = vec![0u8; probe_len];
                reader.read_exact(&mut probe)?;

                if let Some(pos) = probe.iter().position(|&b| b == 0) {
                    let key = String::from_utf8_lossy(&probe[..pos]).to_string();

                    // Lazy check: skip compressed stream without decompressing
                    if let Some(pred) = key_filter {
                        if !pred(&key) {
                            let rem = length - (probe_len as u64);
                            reader.seek(SeekFrom::Current(rem as i64 + 4))?;
                            continue;
                        }
                    }

                    reader.seek(SeekFrom::Current(-(probe_len as i64)))?;
                    let mut data = vec![0u8; length as usize];
                    reader.read_exact(&mut data)?;
                    if pos + 2 <= data.len() {
                        if let Some(val) = decompress_zlib(&data[pos + 2..]) {
                            entries.push(MetadataEntry { key, value: val });
                        }
                    }
                } else {
                    let rem = length - (probe_len as u64);
                    reader.seek(SeekFrom::Current(rem as i64))?;
                }
                reader.seek(SeekFrom::Current(4))?; // CRC
            }

            b"iTXt" => {
                let probe_len = (length as usize).min(80);
                let mut probe = vec![0u8; probe_len];
                reader.read_exact(&mut probe)?;

                if let Some(pos) = probe.iter().position(|&b| b == 0) {
                    let key = String::from_utf8_lossy(&probe[..pos]).to_string();

                    // Lazy check: skip chunk if key is not needed
                    if let Some(pred) = key_filter {
                        if !pred(&key) {
                            let rem = length - (probe_len as u64);
                            reader.seek(SeekFrom::Current(rem as i64 + 4))?;
                            continue;
                        }
                    }

                    reader.seek(SeekFrom::Current(-(probe_len as i64)))?;
                    let mut data = vec![0u8; length as usize];
                    reader.read_exact(&mut data)?;
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
                } else {
                    let rem = length - (probe_len as u64);
                    reader.seek(SeekFrom::Current(rem as i64))?;
                }
                reader.seek(SeekFrom::Current(4))?; // CRC
            }

            _ => {
                reader.seek(SeekFrom::Current(length as i64 + 4))?;
            }
        }
    }

    Ok(entries)
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn create_png_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(chunk_type);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        chunk
    }

    #[cfg(not(windows))]
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
        assert_eq!(ret, 0, "zlib compression failed");
        dest.truncate(dest_len);
        dest
    }

    #[cfg(windows)]
    fn compress_data(data: &[u8]) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_png_text_chunk() {
        let mut png = PNG_MAGIC.to_vec();
        let mut payload = b"Author\0".to_vec();
        payload.extend_from_slice(b"Scientific Researcher");
        png.extend_from_slice(&create_png_chunk(b"tEXt", &payload));
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let entries = extract_png_metadata(&mut cursor).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "Author");
        assert_eq!(entries[0].value, "Scientific Researcher");
    }

    #[test]
    fn test_png_lazy_decompression_skips_unwanted() {
        let mut png = PNG_MAGIC.to_vec();
        let compressed = compress_data(b"huge_source_code_line_here");
        let mut payload = b"SourceCode\0\0".to_vec();
        payload.extend_from_slice(&compressed);
        png.extend_from_slice(&create_png_chunk(b"zTXt", &payload));

        let mut shot_payload = b"Shot\0".to_vec();
        shot_payload.extend_from_slice(b"92185");
        png.extend_from_slice(&create_png_chunk(b"tEXt", &shot_payload));
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let filter = |k: &str| k == "Shot";
        let entries = extract_png_metadata_filtered(&mut cursor, Some(&filter)).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "Shot");
        assert_eq!(entries[0].value, "92185");
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
        let fake_len = 32 * 1024 * 1024u32;
        png.extend_from_slice(&fake_len.to_be_bytes());
        png.extend_from_slice(b"tEXt");
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let entries = extract_png_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty());
    }
}