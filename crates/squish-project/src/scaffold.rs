//! 确定性的项目脚手架领域。 / Deterministic project-scaffold domain.
//!
//! This module describes bytes and logical relative paths only. Filesystem collision checks,
//! workspace membership, Git initialization, staging, recovery, and publication belong to the
//! manager adapters. Keeping generation pure makes retries byte-for-byte deterministic.

use std::{collections::BTreeMap, path::Path};

use semver::Version;

use crate::{
    MANIFEST_FILE_NAME, MANIFEST_VERSION, Manifest, NewPackageName, Package, ProjectError, Target,
};

/// 新项目的规范入口源码。 / Canonical starter source for a new project.
pub const STARTER_SOURCE: &[u8] = br#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns">
  <Prompt>
    <System>You are a helpful assistant.</System>
  </Prompt>
</xs:entry>
"#;

const STARTER_SOURCE_PATH: &str = "src/prompt.xml";
const GITIGNORE_PATH: &str = ".gitignore";
const GITIGNORE: &[u8] = b"/target/\n";
const TARGET_NAME: &str = "prompt";

/// 对脚手架可见文件生效的版本控制策略。 / VCS policy effective for scaffold-visible files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaffoldVcs {
    /// Git is effective, whether newly initialized or inherited. / Git 生效，无论是新建还是继承。
    Git,
    /// VCS integration is disabled. / 禁用版本控制集成。
    None,
}

/// 纯粹的新项目生成输入。 / Pure new-project generation input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewProjectSpec {
    package: NewPackageName,
    vcs: ScaffoldVcs,
}

impl NewProjectSpec {
    /// 从已验证包名和 manager 已解析的 VCS 策略创建规范。 / Creates a spec from a validated name and manager-resolved VCS policy.
    #[must_use]
    pub const fn new(package: NewPackageName, vcs: ScaffoldVcs) -> Self {
        Self { package, vcs }
    }

    /// 返回稳定包身份。 / Returns the stable package identity.
    #[must_use]
    pub const fn package(&self) -> &NewPackageName {
        &self.package
    }

    /// 返回有效版本控制策略。 / Returns the effective VCS policy.
    #[must_use]
    pub const fn vcs(&self) -> ScaffoldVcs {
        self.vcs
    }
}

/// 一个脚手架拥有的规范文件。 / One canonical file owned by a scaffold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaffoldFile {
    path: &'static str,
    contents: Vec<u8>,
}

impl ScaffoldFile {
    fn new(path: &'static str, contents: impl Into<Vec<u8>>) -> Self {
        Self {
            path,
            contents: contents.into(),
        }
    }

    /// 返回使用 `/` 的规范项目相对路径。 / Returns the canonical `/`-separated project-relative path.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// 返回要原样写入的字节。 / Returns bytes to write verbatim.
    #[must_use]
    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    /// 将规范相对路径连接到目标根。 / Joins the canonical relative path to a destination root.
    #[must_use]
    pub fn destination(&self, root: &Path) -> std::path::PathBuf {
        self.path
            .split('/')
            .fold(root.to_path_buf(), |path, segment| path.join(segment))
    }
}

/// 经过自校验且可原子发布的规范项目文件集。 / Self-validated canonical project file set ready for atomic publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectScaffold {
    manifest: Manifest,
    files: Vec<ScaffoldFile>,
}

impl ProjectScaffold {
    /// 生成一个完整、确定且不包含锁文件的项目文件集。 / Generates a complete deterministic project file set without a lockfile.
    ///
    /// The manifest is built through [`Manifest`] and its production serializer, then reparsed
    /// before any bytes are exposed. Files are returned in lexical path order so structured
    /// results and publication plans share one canonical ordering.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError`] if the typed manifest violates an invariant, cannot be serialized,
    /// or does not round-trip through the production manifest parser.
    pub fn generate(spec: &NewProjectSpec) -> Result<Self, ProjectError> {
        let manifest = canonical_manifest(spec.package());
        let manifest_source = manifest.to_toml()?;
        let reparsed = Manifest::parse(&manifest_source)?;
        if manifest != reparsed {
            return Err(ProjectError::ScaffoldInvariant(
                "manifest serializer round-trip changed typed semantics".into(),
            ));
        }

        let mut files = vec![
            ScaffoldFile::new(MANIFEST_FILE_NAME, manifest_source.into_bytes()),
            ScaffoldFile::new(STARTER_SOURCE_PATH, STARTER_SOURCE),
        ];
        if spec.vcs() == ScaffoldVcs::Git {
            files.push(ScaffoldFile::new(GITIGNORE_PATH, GITIGNORE));
        }
        files.sort_unstable_by_key(ScaffoldFile::path);

        Ok(Self { manifest, files })
    }

    /// 返回生成清单的有类型语义。 / Returns the generated manifest's typed semantics.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// 返回按路径词法排序的完整公开文件集。 / Returns the complete public file set in lexical path order.
    #[must_use]
    pub fn files(&self) -> &[ScaffoldFile] {
        &self.files
    }
}

fn canonical_manifest(package: &NewPackageName) -> Manifest {
    let mut targets = BTreeMap::new();
    targets.insert(
        TARGET_NAME.to_owned(),
        Target {
            entry: STARTER_SOURCE_PATH.into(),
            backend: "squish".into(),
            output: None,
            args: BTreeMap::new(),
            limits: crate::Limits::default(),
            features: Default::default(),
        },
    );
    Manifest {
        manifest_version: MANIFEST_VERSION,
        workspace: None,
        package: Some(Package {
            name: package.as_str().to_owned(),
            version: Version::new(0, 1, 0),
            dialect: "xmlsquish/1".into(),
            source_root: "src".into(),
        }),
        targets,
        dependencies: BTreeMap::new(),
        exports: BTreeMap::new(),
        profiles: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_format::{StyleEdition, format};
    use squish_ir::PackageInstanceId;
    use squish_source::{
        LogicalPath, PackageId, SnapshotBuilder, SourceId, SourceLocator, SourceProvider,
    };
    use squish_xml_front::{FrontendSourceContext, compile};
    use std::io;

    struct MemorySource;

    impl SourceProvider for MemorySource {
        fn read(&self, _: &SourceLocator) -> io::Result<Vec<u8>> {
            Ok(STARTER_SOURCE.to_vec())
        }
    }

    const GOLDEN_MANIFEST: &str = r#"manifest-version = 1

[package]
name = "hello"
version = "0.1.0"
dialect = "xmlsquish/1"
source-root = "src"

[target.prompt]
entry = "src/prompt.xml"
backend = "squish"
"#;

    fn spec(vcs: ScaffoldVcs) -> NewProjectSpec {
        NewProjectSpec::new(NewPackageName::new("hello").unwrap(), vcs)
    }

    #[test]
    fn git_scaffold_matches_golden_tree_and_bytes() {
        let scaffold = ProjectScaffold::generate(&spec(ScaffoldVcs::Git)).unwrap();
        assert_eq!(
            scaffold
                .files()
                .iter()
                .map(ScaffoldFile::path)
                .collect::<Vec<_>>(),
            [".gitignore", "src/prompt.xml", "xmlsquish.toml"]
        );
        assert_eq!(scaffold.files()[0].contents(), b"/target/\n");
        assert_eq!(scaffold.files()[1].contents(), STARTER_SOURCE);
        assert_eq!(scaffold.files()[2].contents(), GOLDEN_MANIFEST.as_bytes());
        assert!(
            scaffold
                .files()
                .iter()
                .all(|file| !file.contents().starts_with(b"\xef\xbb\xbf")
                    && !file.contents().contains(&b'\r'))
        );
    }

    #[test]
    fn vcs_none_omits_git_policy_and_lockfile() {
        let scaffold = ProjectScaffold::generate(&spec(ScaffoldVcs::None)).unwrap();
        assert_eq!(
            scaffold
                .files()
                .iter()
                .map(ScaffoldFile::path)
                .collect::<Vec<_>>(),
            ["src/prompt.xml", "xmlsquish.toml"]
        );
        assert!(
            scaffold
                .files()
                .iter()
                .all(|file| file.path() != crate::LOCK_FILE_NAME)
        );
    }

    #[test]
    fn generation_is_deterministic_and_manifest_reparses() {
        let first = ProjectScaffold::generate(&spec(ScaffoldVcs::Git)).unwrap();
        let second = ProjectScaffold::generate(&spec(ScaffoldVcs::Git)).unwrap();
        assert_eq!(first, second);
        let source = std::str::from_utf8(first.files()[2].contents()).unwrap();
        assert_eq!(Manifest::parse(source).unwrap(), *first.manifest());
    }

    #[test]
    fn starter_source_is_formatter_canonical_and_semantically_valid() {
        let plan = format(STARTER_SOURCE, StyleEdition::V1).unwrap();
        assert!(!plan.is_changed());

        let package = PackageId::new("hello").unwrap();
        let source_id = SourceId::new(package, LogicalPath::new("src/prompt.xml").unwrap());
        let mut sources = SnapshotBuilder::new(MemorySource);
        let source = sources
            .load(source_id, SourceLocator::file("unused"))
            .unwrap();
        let context = FrontendSourceContext::new(PackageInstanceId {
            source_kind: 1,
            canonical_source: "path:hello".into(),
            package_name: "hello".into(),
            exact_revision: "manifest:scaffold-test".into(),
        });
        compile(&source, &context).unwrap();
    }
}
