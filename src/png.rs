use crate::model::MetadataEntry;
use std::io::{Read, Seek, SeekFrom};

pub const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
pub const MAX_METADATA_CHUNK_SIZE: u64 = 16 * 1024 * 1024; // 16 MB limit per chunk
pub const MAX_DECOMPRESSED_SIZE: usize = 32 * 1024 * 1024;  // 32 MB limit uncompressed

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
    fn test_png_ztxt_compressed_chunk() {
        let mut png = PNG_MAGIC.to_vec();
        let uncompressed_code = b"import matplotlib.pyplot as plt\ndef plot(): pass\n";
        let compressed = compress_data(uncompressed_code);

        let mut payload = b"SourceCode\0\0".to_vec();
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
        let fake_len = 32 * 1024 * 1024u32;
        png.extend_from_slice(&fake_len.to_be_bytes());
        png.extend_from_slice(b"tEXt");
        png.extend_from_slice(&create_png_chunk(b"IEND", &[]));

        let mut cursor = Cursor::new(png);
        let entries = extract_png_metadata(&mut cursor).unwrap();
        assert!(entries.is_empty());
    }
}