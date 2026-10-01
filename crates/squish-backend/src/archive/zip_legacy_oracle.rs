//! Frozen pre-repair ZIP codec used only as an independent canonicality oracle.
//! Do not optimize this module: its whole-archive rebuild is intentional.

use super::{ArchiveError, ArchiveLimits, fail, validate_archive_path};
use crate::archive::ArchiveEntry;
use std::collections::BTreeSet;
fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn word(bytes: &[u8], offset: usize) -> Result<u16, ArchiveError> {
    let data = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| fail("truncated ZIP"))?;
    Ok(u16::from_le_bytes([data[0], data[1]]))
}
fn dword(bytes: &[u8], offset: usize) -> Result<u32, ArchiveError> {
    let data = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| fail("truncated ZIP"))?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}
/// Writes classic stored ZIP with sorted names, CRC32, UTF-8, 1980-01-01 and mode 0644.
///
/// Input order is irrelevant. ZIP64, compression, directory entries, extra fields and
/// comments are deliberately absent, avoiding platform and compressor variability.
pub fn write_reproducible_zip(
    entries: &[ArchiveEntry],
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ArchiveError> {
    if entries.len() > limits.max_entries || entries.len() > u16::MAX as usize {
        return Err(fail("ZIP entry limit exceeded"));
    }
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut names = BTreeSet::new();
    let mut content = 0u64;
    let mut size = 22u64;
    for entry in &sorted {
        validate_archive_path(&entry.path)?;
        if !names.insert(entry.path.to_lowercase()) {
            return Err(fail("duplicate ZIP member"));
        }
        content = content
            .checked_add(entry.bytes.len() as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        size = size
            .checked_add(76 + 2 * entry.path.len() as u64 + entry.bytes.len() as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        if entry.bytes.len() > u32::MAX as usize {
            return Err(fail("ZIP64 is unsupported"));
        }
    }
    for name in &names {
        for (offset, _) in name.match_indices('/') {
            if names.contains(&name[..offset]) {
                return Err(fail("ZIP file/directory collision"));
            }
        }
    }
    if content > limits.max_content_bytes
        || size > limits.max_archive_bytes
        || size > u32::MAX as u64
    {
        return Err(fail("ZIP byte limit exceeded"));
    }
    let mut out = Vec::with_capacity(size as usize);
    let mut central = Vec::new();
    for entry in sorted {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(&entry.bytes);
        put32(&mut out, 0x04034b50);
        for n in [20, 0x800, 0, 0, 33] {
            put16(&mut out, n);
        }
        put32(&mut out, crc);
        put32(&mut out, entry.bytes.len() as u32);
        put32(&mut out, entry.bytes.len() as u32);
        put16(&mut out, entry.path.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(entry.path.as_bytes());
        out.extend_from_slice(&entry.bytes);
        put32(&mut central, 0x02014b50);
        for n in [0x314, 20, 0x800, 0, 0, 33] {
            put16(&mut central, n);
        }
        put32(&mut central, crc);
        put32(&mut central, entry.bytes.len() as u32);
        put32(&mut central, entry.bytes.len() as u32);
        for n in [entry.path.len() as u16, 0, 0, 0, 0] {
            put16(&mut central, n);
        }
        put32(&mut central, 0o100644 << 16);
        put32(&mut central, offset);
        central.extend_from_slice(entry.path.as_bytes());
    }
    let offset = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, 0x06054b50);
    for n in [0, 0, entries.len() as u16, entries.len() as u16] {
        put16(&mut out, n);
    }
    put32(&mut out, central_len);
    put32(&mut out, offset);
    put16(&mut out, 0);
    Ok(out)
}
/// Reads only canonical stored ZIP; bounds and CRC are checked before accepting it.
///
/// Re-encoding is an intentional strict validation of both ZIP indexes, timestamps,
/// flags, modes, ordering and absence of hidden/trailing data. General third-party
/// ZIPs must first be normalized by a trusted producer.
pub fn read_reproducible_zip(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    if bytes.len() as u64 > limits.max_archive_bytes || bytes.len() > u32::MAX as usize {
        return Err(fail("ZIP byte limit exceeded"));
    }
    let mut entries = Vec::new();
    let mut offset = 0usize;
    let mut total = 0u64;
    while bytes.get(offset..offset + 4) == Some(&0x04034b50u32.to_le_bytes()) {
        if entries.len() >= limits.max_entries || entries.len() >= u16::MAX as usize {
            return Err(fail("ZIP entry limit exceeded"));
        }
        if word(bytes, offset + 8)? != 0 || word(bytes, offset + 6)? != 0x800 {
            return Err(fail("unsupported ZIP encoding"));
        }
        let size = dword(bytes, offset + 22)? as usize;
        if dword(bytes, offset + 18)? as usize != size || word(bytes, offset + 28)? != 0 {
            return Err(fail("unsupported ZIP size or extra fields"));
        }
        total = total
            .checked_add(size as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        if total > limits.max_content_bytes {
            return Err(fail("ZIP content limit exceeded"));
        }
        let start = offset + 30;
        let end = start
            .checked_add(word(bytes, offset + 26)? as usize)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        let next = end
            .checked_add(size)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        let path = std::str::from_utf8(
            bytes
                .get(start..end)
                .ok_or_else(|| fail("truncated ZIP name"))?,
        )
        .map_err(|_| fail("invalid ZIP UTF-8"))?;
        validate_archive_path(path)?;
        let data = bytes
            .get(end..next)
            .ok_or_else(|| fail("truncated ZIP file"))?;
        if crc32fast::hash(data) != dword(bytes, offset + 14)? {
            return Err(fail("ZIP checksum mismatch"));
        }
        entries.push(ArchiveEntry {
            path: path.into(),
            bytes: data.to_vec(),
        });
        offset = next;
    }
    if write_reproducible_zip(&entries, limits)? != bytes {
        return Err(fail("noncanonical or damaged ZIP directory"));
    }
    Ok(entries)
}
