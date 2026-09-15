use std::fs;
use std::path::{Component, Path, PathBuf};

use squish_resolver::{FilesystemPort, LocalPackage, LocalRequest, SourceUnavailable};

/// 将 path dependency 限制在给定工作区根下的文件系统端口。 / Filesystem port confining path dependencies to a workspace root.
#[derive(Clone, Debug)]
pub struct FilesystemHost {
    root: PathBuf,
}

impl FilesystemHost {
    /// 规范化工作区根。 / Canonicalizes the workspace root.
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }
}

impl FilesystemPort for FilesystemHost {
    fn load(&self, request: LocalRequest<'_>) -> Result<LocalPackage, SourceUnavailable> {
        let from = Path::new(request.from_manifest)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let relative = normalize(from.join(request.path)).map_err(|detail| SourceUnavailable {
            identity: request.path.display().to_string(),
            detail,
        })?;
        let manifest_path = self
            .root
            .join(&relative)
            .join(squish_project::MANIFEST_FILE_NAME);
        let canonical = fs::canonicalize(&manifest_path).map_err(|e| SourceUnavailable {
            identity: relative.display().to_string(),
            detail: e.to_string(),
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(SourceUnavailable {
                identity: relative.display().to_string(),
                detail: "path dependency escapes workspace root".into(),
            });
        }
        let source = fs::read_to_string(&canonical).map_err(|e| SourceUnavailable {
            identity: relative.display().to_string(),
            detail: e.to_string(),
        })?;
        let manifest = squish_project::Manifest::parse(&source).map_err(|e| SourceUnavailable {
            identity: relative.display().to_string(),
            detail: e.to_string(),
        })?;
        Ok(LocalPackage {
            manifest_path: relative
                .join(squish_project::MANIFEST_FILE_NAME)
                .to_string_lossy()
                .replace('\\', "/"),
            manifest,
        })
    }
}

fn normalize(path: PathBuf) -> Result<PathBuf, String> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(v) => result.push(v),
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return Err("path dependency escapes workspace root".into());
                }
            }
            _ => return Err("path dependency must be workspace-relative".into()),
        }
    }
    Ok(result)
}
