//! Acquisition of local skill directories as bounded, portable logical trees.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::{FetchError, Limits, LogicalFile, LogicalTree};

/// Reads the complete local skill directory, rejecting aliases and non-regular entries.
///
/// The selected directory must be relative to the declaring project; `..` may
/// select a sibling directory. Each entered component is checked without following
/// symlinks, and the same cross-platform path and byte
/// limits used for remote source trees apply to local skills.
pub fn read_local_skill_tree(
    project_root: &Path,
    selected: &Path,
    limits: &Limits,
) -> Result<LogicalTree, FetchError> {
    let mut directory = fs::canonicalize(project_root)?;
    for component in selected.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                directory.push(name);
                let meta = fs::symlink_metadata(&directory)?;
                if is_alias(&meta) || !meta.is_dir() {
                    return Err(FetchError::Unsupported(format!(
                        "skill source component is not a plain directory: {}",
                        directory.display()
                    )));
                }
            }
            Component::ParentDir => {
                if !directory.pop() {
                    return Err(FetchError::Path(
                        "skill source escapes filesystem root".into(),
                    ));
                }
            }
            _ => return Err(FetchError::Path("skill source must be relative".into())),
        }
    }
    if !directory.is_dir() {
        return Err(FetchError::Path("skill source is not a directory".into()));
    }
    let root = fs::canonicalize(&directory)?;
    let mut files = Vec::new();
    let mut pending = vec![(directory, PathBuf::new())];
    let mut total = 0u64;
    let mut directories = 0usize;
    while let Some((physical, relative)) = pending.pop() {
        directories += 1;
        if directories > limits.max_files.saturating_add(1) {
            return Err(FetchError::Integrity(
                "skill tree exceeds directory limit".into(),
            ));
        }
        for entry in fs::read_dir(&physical)? {
            let entry = entry?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)?;
            if is_alias(&meta) {
                return Err(FetchError::Unsupported(format!(
                    "skill tree contains a symlink: {}",
                    path.display()
                )));
            }
            let child = relative.join(entry.file_name());
            if meta.is_dir() {
                if !fs::canonicalize(&path)?.starts_with(&root) {
                    return Err(FetchError::Path("skill tree escapes project root".into()));
                }
                pending.push((path, child));
                continue;
            }
            if !meta.is_file() {
                return Err(FetchError::Unsupported(format!(
                    "skill tree contains a non-regular file: {}",
                    path.display()
                )));
            }
            if files.len() >= limits.max_files || meta.len() > limits.max_file_bytes {
                return Err(FetchError::Integrity(
                    "skill tree exceeds file limits".into(),
                ));
            }
            total = total
                .checked_add(meta.len())
                .ok_or_else(|| FetchError::Integrity("skill tree length overflow".into()))?;
            if total > limits.max_expanded_bytes {
                return Err(FetchError::Integrity(
                    "skill tree exceeds byte limit".into(),
                ));
            }
            let relative = child
                .to_str()
                .ok_or_else(|| FetchError::Path("skill tree contains a non-UTF-8 path".into()))?
                .replace('\\', "/");
            if !fs::canonicalize(&path)?.starts_with(&root) {
                return Err(FetchError::Path(
                    "skill file escapes selected source".into(),
                ));
            }
            let mut bytes = Vec::new();
            File::open(&path)?
                .take(limits.max_file_bytes + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limits.max_file_bytes {
                return Err(FetchError::Integrity("skill file grew beyond limit".into()));
            }
            files.push(LogicalFile {
                path: relative,
                bytes,
            });
        }
    }
    LogicalTree::build(files, limits)
}

/// Rejects symbolic links and Windows reparse points, including directory junctions.
fn is_alias(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        fs::create_dir_all(&scratch).unwrap();
        tempfile::Builder::new()
            .prefix("squish-fetch-skill-")
            .tempdir_in(scratch)
            .unwrap()
    }

    #[test]
    fn reads_whole_skill_and_rejects_escape() {
        let temporary = fixture();
        let root = temporary.path();
        fs::create_dir_all(root.join("skills/example/scripts")).unwrap();
        fs::write(root.join("skills/example/SKILL.md"), b"skill").unwrap();
        fs::write(root.join("skills/example/scripts/run.sh"), b"echo hi").unwrap();
        let tree =
            read_local_skill_tree(root, Path::new("skills/example"), &Limits::default()).unwrap();
        assert_eq!(tree.files.len(), 2);
        assert!(tree.files.iter().any(|file| file.path == "scripts/run.sh"));
        assert!(read_local_skill_tree(root, Path::new("../outside"), &Limits::default()).is_err());
    }

    #[test]
    fn relative_parent_may_select_explicit_sibling_skill() {
        let temporary = fixture();
        let project = temporary.path().join("project");
        let sibling = temporary.path().join("shared/skill");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("SKILL.md"), b"skill").unwrap();
        let tree =
            read_local_skill_tree(&project, Path::new("../shared/skill"), &Limits::default())
                .unwrap();
        assert_eq!(tree.files[0].path, "SKILL.md");
    }

    #[test]
    fn enforces_file_size_before_copying_local_skill() {
        let temporary = fixture();
        fs::create_dir_all(temporary.path().join("skill")).unwrap();
        fs::write(temporary.path().join("skill/SKILL.md"), b"12345").unwrap();
        let limits = Limits {
            max_file_bytes: 4,
            ..Limits::default()
        };
        assert!(matches!(
            read_local_skill_tree(temporary.path(), Path::new("skill"), &limits),
            Err(FetchError::Integrity(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_inside_skill() {
        let temporary = fixture();
        let root = temporary.path();
        fs::create_dir_all(root.join("skill")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("skill/secret")).unwrap();
        assert!(matches!(
            read_local_skill_tree(root, Path::new("skill"), &Limits::default()),
            Err(FetchError::Unsupported(_))
        ));
    }
}
