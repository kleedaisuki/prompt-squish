//! `xmlsquish.toml` parsing and validation. / `xmlsquish.toml` 解析与校验。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    DependencySpec, MANIFEST_VERSION, Package, Profile, ProjectError, Target, ValidationIssue,
    Workspace, validate_package_name,
};

/// 完整的人工编写项目意图。 / Complete human-authored project intent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Manifest {
    /// Manifest schema version. / 清单架构版本。
    pub manifest_version: u32,
    /// Optional root workspace policy. / 可选的根工作区策略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    /// Optional package; virtual workspaces omit it. / 可选包；虚拟工作区省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<Package>,
    /// Named link targets. / 命名链接目标。
    #[serde(default, rename = "target", skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<String, Target>,
    /// Direct dependency aliases. / 直接依赖别名。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, DependencySpec>,
    /// Public export name to source path. / 公开 export 名称到源路径。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exports: BTreeMap<String, std::path::PathBuf>,
    /// Named build/format profiles. / 命名构建/格式化 profile。
    #[serde(
        default,
        rename = "profile",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub profiles: BTreeMap<String, Profile>,
}

impl Manifest {
    /// 解析并执行跨字段语义校验。 / Parses and performs cross-field semantic validation.
    pub fn parse(source: &str) -> Result<Self, ProjectError> {
        let value: Self = toml::from_str(source)?;
        value.validate()?;
        Ok(value)
    }

    /// 生成规范化 TOML；修改现有文档时应使用 [`crate::CandidateManifest`]。 / Emits normalized TOML; use [`crate::CandidateManifest`] when editing an existing document.
    pub fn to_toml(&self) -> Result<String, ProjectError> {
        self.validate()?;
        toml_edit::ser::to_string_pretty(self).map_err(ProjectError::from)
    }

    /// 验证清单不变式。 / Validates manifest invariants.
    pub fn validate(&self) -> Result<(), ProjectError> {
        let mut issues = Vec::new();
        if self.manifest_version != MANIFEST_VERSION {
            issues.push(ValidationIssue::new(
                "manifest-version",
                format!(
                    "unsupported version {}; expected {MANIFEST_VERSION}",
                    self.manifest_version
                ),
            ));
        }
        if self.package.is_none() && self.workspace.is_none() {
            issues.push(ValidationIssue::new(
                "$",
                "a manifest must define package or workspace",
            ));
        }
        if self.package.is_none()
            && (!self.targets.is_empty()
                || !self.dependencies.is_empty()
                || !self.exports.is_empty())
        {
            issues.push(ValidationIssue::new(
                "package",
                "targets, dependencies, and exports require a package",
            ));
        }
        if let Some(package) = &self.package {
            validate_name("package.name", &package.name, &mut issues);
            if package.source_root.as_os_str().is_empty() || package.source_root.is_absolute() {
                issues.push(ValidationIssue::new(
                    "package.source-root",
                    "source root must be a non-empty relative path",
                ));
            }
        }
        for (name, target) in &self.targets {
            validate_name(&format!("target.{name}"), name, &mut issues);
            if target.entry.as_os_str().is_empty() || target.entry.is_absolute() {
                issues.push(ValidationIssue::new(
                    format!("target.{name}.entry"),
                    "entry must be a non-empty relative path",
                ));
            }
            if target.backend.trim().is_empty() {
                issues.push(ValidationIssue::new(
                    format!("target.{name}.backend"),
                    "backend cannot be empty",
                ));
            }
            if let Some(output) = &target.output
                && (output.as_os_str().is_empty() || output.is_absolute())
            {
                issues.push(ValidationIssue::new(
                    format!("target.{name}.output"),
                    "output must be a non-empty relative path",
                ));
            }
            if target
                .output_path(name)
                .extension()
                .and_then(|value| value.to_str())
                != Some("prompt")
            {
                issues.push(ValidationIssue::new(
                    format!("target.{name}.output"),
                    "published target output must use the `.prompt` suffix",
                ));
            }
            validate_limits(
                &format!("target.{name}.limits"),
                &target.limits,
                &mut issues,
            );
        }
        let mut outputs = BTreeMap::<std::path::PathBuf, &str>::new();
        for (name, target) in &self.targets {
            let raw_output = target.output_path(name);
            let Some(output) = lexical_normalize(&raw_output) else {
                issues.push(ValidationIssue::new(
                    format!("target.{name}.output"),
                    "output must not escape the target directory",
                ));
                continue;
            };
            if let Some(other) = outputs.insert(output.clone(), name) {
                issues.push(ValidationIssue::new(
                    format!("target.{name}.output"),
                    format!(
                        "output `{}` collides with target `{other}`",
                        output.display()
                    ),
                ));
            }
        }
        for (alias, dependency) in &self.dependencies {
            validate_name(&format!("dependencies.{alias}"), alias, &mut issues);
            validate_dependency(&format!("dependencies.{alias}"), dependency, &mut issues);
        }
        for (name, path) in &self.exports {
            validate_name(&format!("exports.{name}"), name, &mut issues);
            if path.as_os_str().is_empty() || path.is_absolute() {
                issues.push(ValidationIssue::new(
                    format!("exports.{name}"),
                    "export source must be a non-empty relative path",
                ));
            }
        }
        if let Some(workspace) = &self.workspace {
            if workspace.target_dir.as_os_str().is_empty() || workspace.target_dir.is_absolute() {
                issues.push(ValidationIssue::new(
                    "workspace.target-dir",
                    "target directory must be a non-empty relative path",
                ));
            }
            let mut seen = BTreeSet::new();
            for member in &workspace.members {
                if member.is_absolute() || member.as_os_str().is_empty() {
                    issues.push(ValidationIssue::new(
                        "workspace.members",
                        "members must be non-empty relative paths",
                    ));
                }
                if !seen.insert(member) {
                    issues.push(ValidationIssue::new(
                        "workspace.members",
                        format!("duplicate member `{}`", member.display()),
                    ));
                }
            }
            for (alias, dependency) in &workspace.dependencies {
                validate_dependency(
                    &format!("workspace.dependencies.{alias}"),
                    dependency,
                    &mut issues,
                );
                if matches!(dependency, DependencySpec::Detail(detail) if detail.workspace) {
                    issues.push(ValidationIssue::new(
                        format!("workspace.dependencies.{alias}"),
                        "workspace dependencies cannot inherit themselves",
                    ));
                }
            }
        }
        validate_profiles(&self.profiles, &mut issues);
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ProjectError::Validation(issues))
        }
    }
}

fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn validate_name(path: &str, name: &str, issues: &mut Vec<ValidationIssue>) {
    if validate_package_name(name).is_err() {
        issues.push(ValidationIssue::new(
            path,
            "name must contain only ASCII letters, digits, '-' or '_'",
        ));
    }
}

fn validate_dependency(path: &str, spec: &DependencySpec, issues: &mut Vec<ValidationIssue>) {
    let DependencySpec::Detail(detail) = spec else {
        return;
    };
    let source_count = usize::from(detail.path.is_some())
        + usize::from(detail.git.is_some())
        + usize::from(detail.workspace)
        + usize::from(detail.version.is_some() || detail.registry.is_some());
    if source_count != 1 {
        issues.push(ValidationIssue::new(
            path,
            "dependency must select exactly one of registry/version, git, path, or workspace",
        ));
    }
    if detail.registry.is_some() && detail.version.is_none() {
        issues.push(ValidationIssue::new(
            format!("{path}.version"),
            "custom registry requires a version requirement",
        ));
    }
    let refs = [
        &detail.git_reference.branch,
        &detail.git_reference.tag,
        &detail.git_reference.rev,
    ]
    .into_iter()
    .filter(|v| v.is_some())
    .count();
    if refs > 1 {
        issues.push(ValidationIssue::new(
            path,
            "git branch, tag, and rev are mutually exclusive",
        ));
    }
    if detail.git.is_none() && refs != 0 {
        issues.push(ValidationIssue::new(path, "git selector requires `git`"));
    }
    if let Some(local) = &detail.path
        && (local.is_absolute() || local.as_os_str().is_empty())
    {
        issues.push(ValidationIssue::new(
            format!("{path}.path"),
            "path dependency must be a non-empty relative path",
        ));
    }
}

fn validate_limits(path: &str, limits: &crate::Limits, issues: &mut Vec<ValidationIssue>) {
    for (name, value) in [
        ("max-depth", limits.max_depth),
        ("max-expansions", limits.max_expansions),
        ("max-output-bytes", limits.max_output_bytes),
    ] {
        if value == Some(0) {
            issues.push(ValidationIssue::new(
                format!("{path}.{name}"),
                "limit must be greater than zero",
            ));
        }
    }
}

fn validate_profiles(profiles: &BTreeMap<String, Profile>, issues: &mut Vec<ValidationIssue>) {
    for (name, profile) in profiles {
        validate_name(&format!("profile.{name}"), name, issues);
        validate_limits(&format!("profile.{name}.limits"), &profile.limits, issues);
        let mut seen = BTreeSet::from([name.as_str()]);
        let mut cursor = profile.inherits.as_deref();
        while let Some(parent) = cursor {
            if !seen.insert(parent) {
                issues.push(ValidationIssue::new(
                    format!("profile.{name}.inherits"),
                    "profile inheritance cycle",
                ));
                break;
            }
            let Some(next) = profiles.get(parent) else {
                issues.push(ValidationIssue::new(
                    format!("profile.{name}.inherits"),
                    format!("unknown profile `{parent}`"),
                ));
                break;
            };
            cursor = next.inherits.as_deref();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_manifest() {
        let source = r#"
manifest-version = 1
[package]
name = "demo"
version = "1.2.3"
[workspace]
members = ["packages/common"]
[dependencies]
serde-prompts = "^1.0"
common = { path = "packages/common", features = ["chat"] }
remote = { git = "https://example.invalid/prompts.git", tag = "v2" }
[target.chat]
entry = "src/chat.xml"
backend = "squish"
output = "dist/chat.prompt"
args = { locale = "zh-CN" }
features = ["trace"]
[profile.release]
debug-info = true
optimization = "size"
"#;
        let manifest = Manifest::parse(source).unwrap();
        assert_eq!(manifest.targets["chat"].backend, "squish");
        assert_eq!(manifest.dependencies.len(), 3);
    }

    #[test]
    fn rejects_ambiguous_dependency_and_profile_cycle() {
        let source = r#"
manifest-version = 1
[package]
name = "demo"
version = "1.0.0"
[dependencies]
bad = { version = "1", path = "../bad" }
[profile.a]
inherits = "b"
[profile.b]
inherits = "a"
"#;
        let error = Manifest::parse(source).unwrap_err();
        let ProjectError::Validation(issues) = error else {
            panic!("wrong error")
        };
        assert!(issues.iter().any(|issue| issue.path == "dependencies.bad"));
        assert!(issues.iter().any(|issue| issue.message.contains("cycle")));
    }

    #[test]
    fn detects_lexically_equivalent_output_collision_and_escape() {
        let source = r#"
manifest-version = 1
[package]
name = "demo"
version = "1.0.0"
[target.first]
entry = "first.xml"
output = "x.prompt"
[target.second]
entry = "second.xml"
output = "sub/../x.prompt"
[target.escape]
entry = "escape.xml"
output = "../outside.prompt"
"#;
        let ProjectError::Validation(issues) = Manifest::parse(source).unwrap_err() else {
            panic!("wrong error")
        };
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("collides"))
        );
        assert!(issues.iter().any(|issue| issue.message.contains("escape")));
    }
}
