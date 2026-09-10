//! UTF-8 input envelopes and atomic artifact persistence.
//! UTF-8 输入封装与原子产物持久化。
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tempfile::NamedTempFile;

/// UTF-8 byte order mark, excluded from text measurements.
/// UTF-8 字节序标记，不计入文本统计。
pub(super) const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Reads UTF-8 text and records its optional encoding envelope.
/// 读取 UTF-8 文本并记录可选编码标记；无效编码返回错误。
pub(super) fn read_xml(path: &Path) -> Result<(String, bool), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let (bytes, bom) = match bytes.strip_prefix(UTF8_BOM) {
        Some(text) => (text, true),
        None => (bytes.as_slice(), false),
    };
    let text = std::str::from_utf8(bytes).map_err(|error| format!("not valid UTF-8: {error}"))?;
    Ok((text.to_owned(), bom))
}

/// Replaces an artifact only after its sibling temporary file is synchronized.
/// 同目录临时文件同步成功后才替换产物；失败时由临时文件负责清理。
pub(super) fn atomic_write(path: &Path, has_bom: bool, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    if has_bom {
        temporary.write_all(UTF8_BOM)?;
    }
    temporary.write_all(contents.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
