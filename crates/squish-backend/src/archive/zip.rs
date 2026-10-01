//! Borrowed canonical ZIP transport: validate indexes without rebuilding payloads.

use super::{ArchiveError, ArchiveLimits, fail, validate_archive_path};
use std::collections::BTreeSet;

/// A regular ZIP member borrowed from producer storage or a validated archive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveEntryRef<'a> {
    /// Portable relative UTF-8 file name.
    pub path: &'a str,
    /// Exact uncompressed bytes, never copied by the borrowed reader.
    pub bytes: &'a [u8],
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn slice(bytes: &[u8], start: usize, len: usize) -> Result<&[u8], ArchiveError> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| fail("ZIP size overflow"))?;
    bytes.get(start..end).ok_or_else(|| fail("truncated ZIP"))
}
fn word(bytes: &[u8], start: usize) -> Result<u16, ArchiveError> {
    let value = slice(bytes, start, 2)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}
fn dword(bytes: &[u8], start: usize) -> Result<u32, ArchiveError> {
    let value = slice(bytes, start, 4)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

/// Registers one safe portable name, rejecting case-insensitive duplicates.
fn register_name(names: &mut BTreeSet<String>, path: &str) -> Result<(), ArchiveError> {
    validate_archive_path(path)?;
    if !names.insert(path.to_lowercase()) {
        return Err(fail("duplicate ZIP member"));
    }
    Ok(())
}

/// Rejects file/directory collisions after all portable names are registered.
fn validate_prefixes(names: &BTreeSet<String>) -> Result<(), ArchiveError> {
    for name in names {
        for (offset, _) in name.match_indices('/') {
            if names.contains(&name[..offset]) {
                return Err(fail("ZIP file/directory collision"));
            }
        }
    }
    Ok(())
}

/// Writes sorted STORED members with fixed UTF-8, 1980 timestamp and mode 0644.
///
/// Borrows member data and emits the central directory directly into the single
/// reserved output allocation. Content CRC is computed once and reused in both
/// indexes. Input order is immaterial; ZIP64 and optional ZIP fields are absent.
pub fn write_reproducible_zip_ref(
    entries: &[ArchiveEntryRef<'_>],
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ArchiveError> {
    if entries.len() > limits.max_entries || entries.len() > u16::MAX as usize {
        return Err(fail("ZIP entry limit exceeded"));
    }
    let mut names = BTreeSet::new();
    for entry in entries {
        register_name(&mut names, entry.path)?;
    }
    validate_prefixes(&names)?;
    let mut content = 0u64;
    let mut size = 22u64;
    for entry in entries {
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
    if content > limits.max_content_bytes
        || size > limits.max_archive_bytes
        || size > u32::MAX as u64
    {
        return Err(fail("ZIP byte limit exceeded"));
    }
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| a.path.cmp(b.path));
    let mut records = Vec::with_capacity(sorted.len());
    let mut out = Vec::with_capacity(size as usize);
    for entry in &sorted {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(entry.bytes);
        records.push((offset, crc));
        put32(&mut out, 0x04034b50);
        for value in [20, 0x800, 0, 0, 33] {
            put16(&mut out, value);
        }
        put32(&mut out, crc);
        put32(&mut out, entry.bytes.len() as u32);
        put32(&mut out, entry.bytes.len() as u32);
        put16(&mut out, entry.path.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(entry.path.as_bytes());
        out.extend_from_slice(entry.bytes);
    }
    let central_start = out.len() as u32;
    for (entry, (offset, crc)) in sorted.iter().zip(records) {
        put32(&mut out, 0x02014b50);
        for value in [0x314, 20, 0x800, 0, 0, 33] {
            put16(&mut out, value);
        }
        put32(&mut out, crc);
        put32(&mut out, entry.bytes.len() as u32);
        put32(&mut out, entry.bytes.len() as u32);
        for value in [entry.path.len() as u16, 0, 0, 0, 0] {
            put16(&mut out, value);
        }
        put32(&mut out, 0o100644 << 16);
        put32(&mut out, offset);
        out.extend_from_slice(entry.path.as_bytes());
    }
    let central_len = out.len() as u32 - central_start;
    put32(&mut out, 0x06054b50);
    for value in [0, 0, entries.len() as u16, entries.len() as u16] {
        put16(&mut out, value);
    }
    put32(&mut out, central_len);
    put32(&mut out, central_start);
    put16(&mut out, 0);
    Ok(out)
}

/// Reads strictly canonical ZIP without copying bodies or reconstructing a ZIP.
///
/// Every local, central and end-record field is checked against the canonical
/// writer contract. Each body is CRC-checked once. Returned slices borrow the
/// original archive, which must remain alive; general third-party ZIPs are not
/// accepted. Bounds apply before content access or owned payload allocation.
pub fn read_reproducible_zip_ref(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<Vec<ArchiveEntryRef<'_>>, ArchiveError> {
    if bytes.len() as u64 > limits.max_archive_bytes || bytes.len() > u32::MAX as usize {
        return Err(fail("ZIP byte limit exceeded"));
    }
    let mut names = BTreeSet::new();
    let mut entries: Vec<ArchiveEntryRef<'_>> = Vec::new();
    let mut records = Vec::new();
    let mut offset = 0usize;
    let mut total = 0u64;
    while slice(bytes, offset, 4)? == 0x04034b50u32.to_le_bytes() {
        if entries.len() >= limits.max_entries || entries.len() >= u16::MAX as usize {
            return Err(fail("ZIP entry limit exceeded"));
        }
        let header = slice(bytes, offset, 30)?;
        if header[4..14] != [20, 0, 0, 8, 0, 0, 0, 0, 33, 0] || word(header, 28)? != 0 {
            return Err(fail("noncanonical ZIP local header"));
        }
        let crc = dword(header, 14)?;
        let size = dword(header, 22)? as usize;
        if dword(header, 18)? as usize != size {
            return Err(fail("unsupported ZIP size"));
        }
        total = total
            .checked_add(size as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        if total > limits.max_content_bytes {
            return Err(fail("ZIP content limit exceeded"));
        }
        let name_start = offset + 30;
        let name_len = word(header, 26)? as usize;
        let path = std::str::from_utf8(slice(bytes, name_start, name_len)?)
            .map_err(|_| fail("invalid ZIP UTF-8"))?;
        register_name(&mut names, path)?;
        if entries.last().is_some_and(|previous| previous.path >= path) {
            return Err(fail("noncanonical ZIP member order"));
        }
        let data_start = name_start + name_len;
        let data = slice(bytes, data_start, size)?;
        if crc32fast::hash(data) != crc {
            return Err(fail("ZIP checksum mismatch"));
        }
        records.push((offset as u32, crc));
        entries.push(ArchiveEntryRef { path, bytes: data });
        offset = data_start + size;
    }
    validate_prefixes(&names)?;
    let central_start = offset;
    for (entry, (local_offset, crc)) in entries.iter().zip(records) {
        let header = slice(bytes, offset, 46)?;
        if dword(header, 0)? != 0x02014b50
            || header[4..16] != [20, 3, 20, 0, 0, 8, 0, 0, 0, 0, 33, 0]
            || dword(header, 16)? != crc
            || dword(header, 20)? as usize != entry.bytes.len()
            || dword(header, 24)? as usize != entry.bytes.len()
            || word(header, 28)? as usize != entry.path.len()
            || header[30..38] != [0; 8]
            || dword(header, 38)? != 0o100644 << 16
            || dword(header, 42)? != local_offset
            || slice(bytes, offset + 46, entry.path.len())? != entry.path.as_bytes()
        {
            return Err(fail("noncanonical or damaged ZIP directory"));
        }
        offset += 46 + entry.path.len();
    }
    let end = slice(bytes, offset, 22)?;
    if dword(end, 0)? != 0x06054b50
        || end[4..8] != [0; 4]
        || word(end, 8)? as usize != entries.len()
        || word(end, 10)? as usize != entries.len()
        || dword(end, 12)? as usize != offset - central_start
        || dword(end, 16)? as usize != central_start
        || word(end, 20)? != 0
        || offset + 22 != bytes.len()
    {
        return Err(fail("noncanonical ZIP end record"));
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "zip_legacy_oracle.rs"]
mod zip_legacy_oracle;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        write_reproducible_zip_ref(
            &[
                ArchiveEntryRef {
                    path: "z/二.bin",
                    bytes: &[0, 255, 1, 128],
                },
                ArchiveEntryRef {
                    path: "a.txt",
                    bytes: b"hello",
                },
            ],
            ArchiveLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn borrowed_roundtrip_and_empty_archive() {
        let bytes = fixture();
        let entries = read_reproducible_zip_ref(&bytes, ArchiveLimits::default()).unwrap();
        assert_eq!(entries[0].path, "a.txt");
        assert_eq!(entries[1].bytes, &[0, 255, 1, 128]);
        assert!(std::ptr::eq(
            entries[0].bytes.as_ptr(),
            bytes[35..].as_ptr()
        ));
        assert_eq!(
            write_reproducible_zip_ref(&entries, ArchiveLimits::default()).unwrap(),
            bytes
        );
        let empty = write_reproducible_zip_ref(&[], ArchiveLimits::default()).unwrap();
        assert_eq!(empty.len(), 22);
        assert!(
            read_reproducible_zip_ref(&empty, ArchiveLimits::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn every_single_byte_corruption_and_truncation_is_rejected() {
        let original = fixture();
        for offset in 0..original.len() {
            for bit in 0..8 {
                let mut changed = original.clone();
                changed[offset] ^= 1 << bit;
                let result = read_reproducible_zip_ref(&changed, ArchiveLimits::default());
                assert_eq!(
                    result.is_ok(),
                    zip_legacy_oracle::read_reproducible_zip(&changed, ArchiveLimits::default())
                        .is_ok()
                );
                assert!(result.is_err(), "mutation at {offset}, bit {bit}");
            }
            assert!(
                read_reproducible_zip_ref(&original[..offset], ArchiveLimits::default()).is_err(),
                "truncation at {offset}"
            );
        }
        let mut trailing = original;
        trailing.push(0);
        assert!(read_reproducible_zip_ref(&trailing, ArchiveLimits::default()).is_err());
    }

    #[test]
    fn writer_matches_frozen_codec_and_valid_payload_changes_remain_valid() {
        let bytes = fixture();
        let borrowed = read_reproducible_zip_ref(&bytes, ArchiveLimits::default()).unwrap();
        let owned: Vec<_> = borrowed
            .iter()
            .map(|entry| crate::archive::ArchiveEntry {
                path: entry.path.into(),
                bytes: entry.bytes.to_vec(),
            })
            .collect();
        assert_eq!(
            bytes,
            zip_legacy_oracle::write_reproducible_zip(&owned, ArchiveLimits::default()).unwrap()
        );
        let mut changed = bytes;
        changed[35] ^= 1;
        let crc = crc32fast::hash(&changed[35..40]);
        changed[14..18].copy_from_slice(&crc.to_le_bytes());
        // The first central record follows both complete local records.
        let central = changed
            .windows(4)
            .position(|v| v == 0x02014b50u32.to_le_bytes())
            .unwrap();
        changed[central + 16..central + 20].copy_from_slice(&crc.to_le_bytes());
        assert!(read_reproducible_zip_ref(&changed, ArchiveLimits::default()).is_ok());
        assert!(
            zip_legacy_oracle::read_reproducible_zip(&changed, ArchiveLimits::default()).is_ok()
        );
    }
    #[test]
    fn writer_limits_and_portable_collisions() {
        for names in [
            ["A", "a"],
            ["A", "a/file"],
            ["../a", "b"],
            ["CON.txt", "b"],
            ["a\\b", "b"],
        ] {
            let entries = names.map(|path| ArchiveEntryRef { path, bytes: b"x" });
            assert!(write_reproducible_zip_ref(&entries, ArchiveLimits::default()).is_err());
        }
        let bytes = fixture();
        for limits in [
            ArchiveLimits {
                max_entries: 1,
                ..ArchiveLimits::default()
            },
            ArchiveLimits {
                max_content_bytes: 8,
                ..ArchiveLimits::default()
            },
            ArchiveLimits {
                max_archive_bytes: (bytes.len() - 1) as u64,
                ..ArchiveLimits::default()
            },
        ] {
            assert!(read_reproducible_zip_ref(&bytes, limits).is_err());
            let entries = [
                ArchiveEntryRef {
                    path: "a.txt",
                    bytes: b"hello",
                },
                ArchiveEntryRef {
                    path: "z/二.bin",
                    bytes: &[0, 255, 1, 128],
                },
            ];
            assert!(write_reproducible_zip_ref(&entries, limits).is_err());
        }
    }
}
