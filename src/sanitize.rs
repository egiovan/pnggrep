use crate::model::MetadataEntry;
use crate::pdf::{find_info_dict, is_matplotlib_pdf};
use crate::png::PNG_MAGIC;
use crate::walker::{
    extract_metadata, is_ignored_dir, is_python_venv, is_silent_ignored_dir, is_supported_ext,
    is_user_excluded,
};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Standard IEEE 802.3 CRC32 implementation for PNG chunks (zero external dependencies)
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Checks if a file exists and was stamped by pnggrep's sanitizer
pub fn is_sanitized_file(path: &Path) -> bool {
    if let Ok(entries) = extract_metadata(path) {
        entries.iter().any(|e: &MetadataEntry| {
            (e.key.eq_ignore_ascii_case("SanitizedBy")
                || e.key.eq_ignore_ascii_case("Keywords/sanitized_by")
                || e.key.eq_ignore_ascii_case("Description/sanitized_by")
                || e.key.ends_with("/sanitized_by"))
                && e.value.eq_ignore_ascii_case("pnggrep")
        })
    } else {
        false
    }
}

/// Generates the output path <stem>_sanitized.<ext>
pub fn compute_sanitized_path(source: &Path) -> Option<PathBuf> {
    let file_stem = source.file_stem()?.to_str()?;
    let ext = source.extension()?.to_str()?;

    // Prevent recursive compounding (e.g. plot_sanitized_sanitized.png)
    if file_stem.ends_with("_sanitized") {
        return None;
    }

    let new_filename = format!("{}_sanitized.{}", file_stem, ext);
    Some(source.with_file_name(new_filename))
}

// -----------------------------------------------------------------------------
// Format Sanitizers
// -----------------------------------------------------------------------------

pub fn sanitize_png(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() < 8 || &input[..8] != PNG_MAGIC {
        return Err("Not a valid PNG file".to_string());
    }

    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(&PNG_MAGIC);

    let mut cursor = 8;
    let n = input.len();

    while cursor + 8 <= n {
        let length = u32::from_be_bytes([
            input[cursor],
            input[cursor + 1],
            input[cursor + 2],
            input[cursor + 3],
        ]) as usize;
        let chunk_type = &input[cursor + 4..cursor + 8];
        let total_chunk_len = 12 + length; // 4 len + 4 type + data + 4 crc

        if cursor + total_chunk_len > n {
            return Err("Corrupt PNG chunk structure".to_string());
        }

        let is_metadata = chunk_type == b"tEXt" || chunk_type == b"zTXt" || chunk_type == b"iTXt";

        if chunk_type == b"IEND" {
            // Inject SanitizedBy marker chunk right before IEND
            let marker_data = b"SanitizedBy\0pnggrep";
            let marker_len = marker_data.len() as u32;
            out.extend_from_slice(&marker_len.to_be_bytes());
            out.extend_from_slice(b"tEXt");
            out.extend_from_slice(marker_data);

            let mut crc_payload = Vec::with_capacity(4 + marker_data.len());
            crc_payload.extend_from_slice(b"tEXt");
            crc_payload.extend_from_slice(marker_data);
            let crc = crc32(&crc_payload);
            out.extend_from_slice(&crc.to_be_bytes());

            // Append IEND
            out.extend_from_slice(&input[cursor..cursor + total_chunk_len]);
            break;
        } else if !is_metadata {
            out.extend_from_slice(&input[cursor..cursor + total_chunk_len]);
        }

        cursor += total_chunk_len;
    }

    Ok(out)
}

pub fn sanitize_svg(input: &str) -> Result<String, String> {
    let sanitized_meta = "<metadata>\n    <rdf:RDF xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n      <dc:description>{\"sanitized_by\": \"pnggrep\"}</dc:description>\n    </rdf:RDF>\n  </metadata>";

    if let Some(start) = input.find("<metadata") {
        if let Some(end_rel) = input[start..].find("</metadata>") {
            let end = start + end_rel + 11;
            let mut out = String::with_capacity(input.len());
            out.push_str(&input[..start]);
            out.push_str(sanitized_meta);
            out.push_str(&input[end..]);
            return Ok(out);
        }
    }

    if let Some(pos) = input.rfind("</svg>") {
        let mut out = String::with_capacity(input.len() + sanitized_meta.len() + 10);
        out.push_str(&input[..pos]);
        out.push_str(sanitized_meta);
        out.push_str("\n</svg>");
        return Ok(out);
    }

    Ok(input.to_string())
}

pub fn sanitize_pdf(content: &[u8]) -> Result<Vec<u8>, String> {
    if !is_matplotlib_pdf(content) {
        return Err("File is not a recognized Matplotlib or smart_savefig PDF".to_string());
    }

    let dict_bytes = match find_info_dict(content) {
        Some(d) => d,
        None => return Err("Could not locate PDF Info dictionary".to_string()),
    };

    let start_offset = dict_bytes.as_ptr() as usize - content.as_ptr() as usize;
    let len = dict_bytes.len();

    let body = b"<< /Producer (matplotlib) /Keywords ({\"sanitized_by\": \"pnggrep\"})";
    let closing = b">>";

    if len < body.len() + closing.len() {
        return Err("PDF Info dictionary is too small to safely overwrite in-place".to_string());
    }

    let needed_padding = len - (body.len() + closing.len());
    let mut padded = Vec::with_capacity(len);
    padded.extend_from_slice(body);
    for _ in 0..needed_padding {
        padded.push(b' ');
    }
    padded.extend_from_slice(closing);

    // Replace byte-for-byte in place to keep the entire PDF xref table valid
    let mut new_content = content.to_vec();
    new_content[start_offset..start_offset + len].copy_from_slice(&padded);
    Ok(new_content)
}

// -----------------------------------------------------------------------------
// High-Level Dispatch & Traversal
// -----------------------------------------------------------------------------

pub fn sanitize_file(source: &Path) {
    let target = match compute_sanitized_path(source) {
        Some(p) => p,
        None => {
            eprintln!(
                "\x1b[33m[warning]\x1b[0m Skipping '{}': filename already contains '_sanitized' suffix.",
                source.display()
            );
            return;
        }
    };

    // Guardrail: Check whether target exists and verify provenance
    let is_overwrite = target.exists();
    if is_overwrite && !is_sanitized_file(&target) {
        eprintln!(
            "\x1b[31m[error]\x1b[0m Target '{}' already exists and was NOT sanitized by pnggrep. Refusing to overwrite.",
            target.display()
        );
        return;
    }

    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let result = match ext.as_str() {
        "png" => fs::read(source)
            .map_err(|e| e.to_string())
            .and_then(|bytes| sanitize_png(&bytes)),
        "svg" => fs::read_to_string(source)
            .map_err(|e| e.to_string())
            .and_then(|text| sanitize_svg(&text).map(|s| s.into_bytes())),
        "pdf" => fs::read(source)
            .map_err(|e| e.to_string())
            .and_then(|bytes| sanitize_pdf(&bytes)),
        _ => Err(format!("Unsupported format: {}", ext)),
    };

    match result {
        Ok(clean_bytes) => {
            if let Err(e) = File::create(&target).and_then(|mut f| f.write_all(&clean_bytes)) {
                eprintln!("\x1b[31m[error]\x1b[0m Failed writing '{}': {}", target.display(), e);
            } else if is_overwrite {
                println!(
                    "\x1b[32m[sanitized]\x1b[0m \x1b[90m(overwritten)\x1b[0m {}",
                    target.display()
                );
            } else {
                println!(
                    "\x1b[32m[sanitized]\x1b[0m {} -> {}",
                    source.display(),
                    target.display()
                );
            }
        }
        Err(err) => {
            eprintln!("\x1b[31m[error]\x1b[0m Skipping '{}': {}", source.display(), err);
        }
    }
}

pub fn sanitize_target(target: &Path, custom_excludes: &[String]) {
    if target.is_file() {
        if let Some(ext) = target.extension().and_then(|e| e.to_str()) {
            if is_supported_ext(ext) {
                sanitize_file(target);
            } else {
                eprintln!("\x1b[33m[warning]\x1b[0m Unsupported format: {}", target.display());
            }
        }
    } else if target.is_dir() {
        if let Ok(entries) = fs::read_dir(target) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                        if is_user_excluded(&path, dir_name, custom_excludes)
                            || is_silent_ignored_dir(dir_name)
                            || is_ignored_dir(dir_name)
                        {
                            continue;
                        }
                    }
                    if is_python_venv(&path) {
                        continue;
                    }
                    sanitize_target(&path, custom_excludes);
                } else if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if is_supported_ext(ext) {
                            sanitize_file(&path);
                        }
                    }
                }
            }
        }
    } else {
        eprintln!("\x1b[31m[error]\x1b[0m Path not found: {}", target.display());
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_accuracy() {
        let input = b"123456789";
        assert_eq!(crc32(input), 0xCBF4_3926);
    }

    #[test]
    fn test_sanitize_png() {
        let mut raw_png = PNG_MAGIC.to_vec();
        let chunk_data = b"SourceCode\0print('secret code')";
        raw_png.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
        raw_png.extend_from_slice(b"tEXt");
        raw_png.extend_from_slice(chunk_data);
        raw_png.extend_from_slice(&[0, 0, 0, 0]);

        raw_png.extend_from_slice(&0u32.to_be_bytes());
        raw_png.extend_from_slice(b"IEND");
        raw_png.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);

        let sanitized = sanitize_png(&raw_png).unwrap();
        assert!(!sanitized.windows(10).any(|w| w == b"SourceCode"));
        assert!(sanitized.windows(11).any(|w| w == b"SanitizedBy"));
    }

    #[test]
    fn test_sanitize_svg() {
        let raw_svg = r#"<svg><metadata><rdf:RDF xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:description>{"secret": 123}</dc:description></rdf:RDF></metadata><circle/></svg>"#;
        let sanitized = sanitize_svg(raw_svg).unwrap();
        assert!(!sanitized.contains("secret"));
        assert!(sanitized.contains("sanitized_by"));
    }

    #[test]
    fn test_refuse_overwrite_non_sanitized() {
        let temp_dir = std::env::temp_dir().join("pnggrep_test_overwrite");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let orig = temp_dir.join("figure.png");
        let target = temp_dir.join("figure_sanitized.png");

        let mut dummy = PNG_MAGIC.to_vec();
        dummy.extend_from_slice(&0u32.to_be_bytes());
        dummy.extend_from_slice(b"IEND");
        dummy.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);

        fs::write(&orig, &dummy).unwrap();
        fs::write(&target, &dummy).unwrap();

        assert!(!is_sanitized_file(&target));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}