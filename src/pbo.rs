//! Strict PBO parsing and Swifty partitioning.
//!
//! SwiftyPboFile parts are:
//! 1) $$HEADER$$  (bytes [0..header_len))
//! 2) each entry payload in header order (skip the first "dummy" entry)
//! 3) $$END$$ (remaining bytes)
//!
//! Each part checksum is MD5(part bytes).
//! File checksum is MD5(concat(part_md5_hex_upper ...)).

use crate::checksum::{part_md5_from_reader, part_md5_zeroes, validate_part_coverage, SwiftyError};
use crate::model::SrfPart;
use crate::path::ensure_ascii_path;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

#[derive(Debug, PartialEq, Eq)]
enum PboEntryType {
    Vers,
    Cprs,
    Enco,
    None,
}

#[derive(Debug)]
struct PboEntry {
    filename: String,
    #[allow(dead_code)]
    entry_type: PboEntryType,
    data_size: u32,
}

#[derive(Debug)]
struct PboMeta {
    header_len: u64,
    #[allow(dead_code)]
    extensions: HashMap<String, String>,
    entries: Vec<PboEntry>,
}

fn pbo_read_cstring<R: BufRead>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        r.read_exact(&mut b)?;
        if b[0] == 0 {
            break;
        }
        out.push(b[0]);
        if out.len() > 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cstring too long",
            ));
        }
    }
    Ok(out)
}

fn pbo_read_cstring_string<R: BufRead>(r: &mut R) -> io::Result<String> {
    let v = pbo_read_cstring(r)?;
    Ok(String::from_utf8_lossy(&v).into_owned())
}

fn pbo_read_u32_le<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn pbo_map_type_strict(t: u32) -> io::Result<PboEntryType> {
    Ok(match t {
        0x5665_7273 => PboEntryType::Vers, // "Vers"
        0x4370_7273 => PboEntryType::Cprs, // "Cprs"
        0x456e_6372 => PboEntryType::Enco, // "Enco"
        0x0000_0000 => PboEntryType::None,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown pbo entry type: 0x{t:08X}"),
            ));
        }
    })
}

fn pbo_read_extensions<R: BufRead>(r: &mut R) -> io::Result<HashMap<String, String>> {
    let mut m = HashMap::new();
    loop {
        let key = pbo_read_cstring_string(r)?;
        if key.is_empty() {
            break;
        }
        let val = pbo_read_cstring_string(r)?;
        m.insert(key, val);
    }
    Ok(m)
}

fn pbo_read_meta<R: BufRead + Seek>(r: &mut R) -> io::Result<PboMeta> {
    r.seek(SeekFrom::Start(0))?;

    let mut extensions = HashMap::new();
    let mut entries = Vec::new();

    loop {
        let filename = pbo_read_cstring_string(r)?;
        let t_raw = pbo_read_u32_le(r)?;
        let entry_type = pbo_map_type_strict(t_raw)?;

        let _original_size = pbo_read_u32_le(r)?;
        let _offset = pbo_read_u32_le(r)?;
        let _timestamp = pbo_read_u32_le(r)?;
        let data_size = pbo_read_u32_le(r)?;

        if entry_type == PboEntryType::None && filename.is_empty() {
            break;
        }

        if entry_type == PboEntryType::Vers {
            extensions = pbo_read_extensions(r)?;
        }

        entries.push(PboEntry {
            filename,
            entry_type,
            data_size,
        });
    }

    let header_len = r.stream_position()?;
    Ok(PboMeta {
        header_len,
        extensions,
        entries,
    })
}

fn pbo_partition_named_from_meta(
    meta: &PboMeta,
    file_len: u64,
    file_path: &str,
) -> Result<Vec<(String, u64, u64)>, SwiftyError> {
    if meta.header_len > file_len {
        return Err(SwiftyError::InvalidPbo {
            file: file_path.to_string(),
            reason: "header_len exceeds file length".to_string(),
        });
    }

    if meta.entries.is_empty() {
        return Err(SwiftyError::InvalidPbo {
            file: file_path.to_string(),
            reason: "missing pbo entries".to_string(),
        });
    }
    if meta.entries[0].data_size != 0 {
        return Err(SwiftyError::InvalidPbo {
            file: file_path.to_string(),
            reason: "first entry has non-zero size".to_string(),
        });
    }

    let mut parts = Vec::with_capacity(meta.entries.len() + 2);
    parts.push(("$$HEADER$$".to_string(), 0, meta.header_len));

    let mut offset = meta.header_len;
    for entry in meta.entries.iter().skip(1) {
        let size = entry.data_size as u64;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| SwiftyError::InvalidPbo {
                file: file_path.to_string(),
                reason: "entry offset overflow".to_string(),
            })?;
        if end > file_len {
            return Err(SwiftyError::InvalidPbo {
                file: file_path.to_string(),
                reason: "entry exceeds file length".to_string(),
            });
        }
        parts.push((entry.filename.clone(), offset, size));
        offset = end;
    }

    let tail_len = file_len
        .checked_sub(offset)
        .ok_or_else(|| SwiftyError::InvalidPbo {
            file: file_path.to_string(),
            reason: "offset exceeds file length".to_string(),
        })?;
    parts.push(("$$END$$".to_string(), offset, tail_len));

    Ok(parts)
}

/// Compute SwiftyPboFile parts for a `.pbo`, including MD5 for each part.
pub fn swifty_pbo_parts_from_reader<R: BufRead + Seek>(
    file_path: &str,
    reader: &mut R,
    file_len: u64,
    buf: &mut [u8],
) -> Result<Vec<SrfPart>, SwiftyError> {
    ensure_ascii_path("pbo file path", file_path)?;

    let meta = pbo_read_meta(reader).map_err(|e| SwiftyError::InvalidPbo {
        file: file_path.to_string(),
        reason: e.to_string(),
    })?;
    let plan = pbo_partition_named_from_meta(&meta, file_len, file_path)?;

    reader.seek(SeekFrom::Start(0))?;
    let mut out = Vec::with_capacity(plan.len());
    for (name, start, len) in plan {
        reader.seek(SeekFrom::Start(start))?;
        let md5 = part_md5_from_reader(reader, len, buf)?;
        out.push(SrfPart {
            path: name,
            start,
            length: len,
            checksum: md5,
        });
    }

    validate_part_coverage(file_path, file_len, &out)?;
    Ok(out)
}

/// Compute SwiftyPboFile parts for a `.pbo`, using zero-md5 for each part.
pub fn swifty_pbo_parts_zero_md5_from_reader<R: BufRead + Seek>(
    file_path: &str,
    reader: &mut R,
    file_len: u64,
) -> Result<Vec<SrfPart>, SwiftyError> {
    ensure_ascii_path("pbo file path", file_path)?;

    let meta = pbo_read_meta(reader).map_err(|e| SwiftyError::InvalidPbo {
        file: file_path.to_string(),
        reason: e.to_string(),
    })?;
    let plan = pbo_partition_named_from_meta(&meta, file_len, file_path)?;

    let mut out = Vec::with_capacity(plan.len());
    for (name, start, len) in plan {
        let md5 = part_md5_zeroes(len);
        out.push(SrfPart {
            path: name,
            start,
            length: len,
            checksum: md5,
        });
    }

    validate_part_coverage(file_path, file_len, &out)?;
    Ok(out)
}
