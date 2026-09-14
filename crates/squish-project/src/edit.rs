//! 保留注释的有类型候选清单编辑。 / Comment-preserving typed candidate-manifest edits.

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::{DependencyDetail, DependencySpec, Manifest, ProjectError};

/// 依赖意图变更。 / A dependency-intent change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyChange {
    /// Added an alias. / 添加别名。
    Added { alias: String, spec: DependencySpec },
    /// Replaced an existing alias explicitly. / 显式替换已有别名。
    Replaced {
        alias: String,
        before: DependencySpec,
        after: DependencySpec,
    },
    /// Removed an alias. / 删除别名。
    Removed { alias: String, spec: DependencySpec },
}

/// 无副作用的候选编辑结果，亦是 `--dry-run` 的数据源。 / Side-effect-free candidate edit and `--dry-run` data source.
#[derive(Clone, Debug)]
pub struct EditPlan {
    /// Semantic edit. / 语义编辑。
    pub change: DependencyChange,
    /// Exact original bytes/text. / 精确原始文本。
    pub before: String,
    /// Comment-preserving candidate text. / 保留注释的候选文本。
    pub after: String,
}

impl EditPlan {
    /// 返回适合机器消费的逐行变更记录，不伪装成有位置信息的 unified diff。 / Returns machine-friendly changed lines without pretending to be a positioned unified diff.
    pub fn changed_lines(&self) -> Vec<(char, String)> {
        let before: Vec<_> = self.before.lines().collect();
        let after: Vec<_> = self.after.lines().collect();
        line_excess(&before, &after, '-')
            .into_iter()
            .chain(line_excess(&after, &before, '+'))
            .collect()
    }
}

fn line_excess(lines: &[&str], counterparts: &[&str], marker: char) -> Vec<(char, String)> {
    let mut available = std::collections::BTreeMap::<&str, usize>::new();
    for line in counterparts {
        *available.entry(line).or_default() += 1;
    }
    let mut excess = Vec::new();
    for line in lines {
        let count = available.entry(line).or_default();
        if *count == 0 {
            excess.push((marker, (*line).to_owned()));
        } else {
            *count -= 1;
        }
    }
    excess
}

/// 内存中的可编辑 TOML 候选文档。 / In-memory editable TOML candidate document.
#[derive(Clone, Debug)]
pub struct CandidateManifest {
    original: String,
    document: DocumentMut,
}

impl CandidateManifest {
    /// 解析可编辑文档并首先验证当前意图。 / Parses an editable document and validates current intent first.
    pub fn parse(source: &str) -> Result<Self, ProjectError> {
        Manifest::parse(source)?;
        Ok(Self {
            original: source.to_owned(),
            document: source.parse()?,
        })
    }

    /// 添加依赖；仅在 `replace` 明确时覆盖。 / Adds a dependency, overwriting only when `replace` is explicit.
    pub fn add(
        mut self,
        alias: &str,
        spec: DependencySpec,
        replace: bool,
    ) -> Result<EditPlan, ProjectError> {
        let manifest = Manifest::parse(&self.document.to_string())?;
        let prior = manifest.dependencies.get(alias).cloned();
        if prior.is_some() && !replace {
            return Err(ProjectError::DuplicateDependency(alias.into()));
        }
        let dependencies = dependency_table(&mut self.document);
        dependencies.insert(alias, spec_item(&spec));
        let after = self.document.to_string();
        Manifest::parse(&after)?;
        let change = match prior {
            Some(before) => DependencyChange::Replaced {
                alias: alias.into(),
                before,
                after: spec,
            },
            None => DependencyChange::Added {
                alias: alias.into(),
                spec,
            },
        };
        Ok(EditPlan {
            change,
            before: self.original,
            after,
        })
    }

    /// 删除依赖并保留其他格式与注释。 / Removes a dependency while preserving unrelated formatting and comments.
    pub fn remove(mut self, alias: &str) -> Result<EditPlan, ProjectError> {
        let manifest = Manifest::parse(&self.document.to_string())?;
        let spec = manifest
            .dependencies
            .get(alias)
            .cloned()
            .ok_or_else(|| ProjectError::MissingDependency(alias.into()))?;
        let Some(table) = self
            .document
            .get_mut("dependencies")
            .and_then(Item::as_table_like_mut)
        else {
            return Err(ProjectError::MissingDependency(alias.into()));
        };
        table.remove(alias);
        let after = self.document.to_string();
        Manifest::parse(&after)?;
        Ok(EditPlan {
            change: DependencyChange::Removed {
                alias: alias.into(),
                spec,
            },
            before: self.original,
            after,
        })
    }
}

fn dependency_table(document: &mut DocumentMut) -> &mut dyn TableLike {
    if !document.contains_key("dependencies") {
        document.insert("dependencies", Item::Table(Table::new()));
    }
    document["dependencies"]
        .as_table_like_mut()
        .expect("validated manifest guarantees dependency table")
}

fn spec_item(spec: &DependencySpec) -> Item {
    match spec {
        DependencySpec::Version(requirement) => Item::Value(Value::from(requirement.to_string())),
        DependencySpec::Detail(detail) => Item::Value(Value::InlineTable(detail_table(detail))),
    }
}

fn detail_table(detail: &DependencyDetail) -> InlineTable {
    let mut table = InlineTable::new();
    if let Some(value) = &detail.version {
        table.insert("version", Value::from(value.to_string()));
    }
    if let Some(value) = &detail.registry {
        table.insert("registry", Value::from(value.as_str()));
    }
    if let Some(value) = &detail.git {
        table.insert("git", Value::from(value.as_str()));
    }
    if let Some(value) = &detail.git_reference.branch {
        table.insert("branch", Value::from(value.as_str()));
    }
    if let Some(value) = &detail.git_reference.tag {
        table.insert("tag", Value::from(value.as_str()));
    }
    if let Some(value) = &detail.git_reference.rev {
        table.insert("rev", Value::from(value.as_str()));
    }
    if let Some(value) = &detail.path {
        table.insert("path", Value::from(value.to_string_lossy().as_ref()));
    }
    if detail.workspace {
        table.insert("workspace", Value::from(true));
    }
    if let Some(value) = &detail.package {
        table.insert("package", Value::from(value.as_str()));
    }
    if detail.optional {
        table.insert("optional", Value::from(true));
    }
    if !detail.default_features {
        table.insert("default-features", Value::from(false));
    }
    if !detail.features.is_empty() {
        let mut values = Array::new();
        for feature in &detail.features {
            values.push(feature.as_str());
        }
        table.insert("features", Value::Array(values));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::VersionReq;

    const BASE: &str = "# project comment\nmanifest-version = 1\n[package]\nname = \"demo\" # name comment\nversion = \"1.0.0\"\n\n[dependencies]\nold = \"1\" # keep me\n";

    #[test]
    fn add_and_remove_preserve_comments_and_validate_candidate() {
        let spec = DependencySpec::Version(VersionReq::parse("^2").unwrap());
        let add = CandidateManifest::parse(BASE)
            .unwrap()
            .add("new", spec, false)
            .unwrap();
        assert!(add.after.contains("# project comment"));
        assert!(add.after.contains("old = \"1\" # keep me"));
        assert!(add.after.contains("new = \"^2\""));
        let remove = CandidateManifest::parse(&add.after)
            .unwrap()
            .remove("new")
            .unwrap();
        assert!(!remove.after.contains("new ="));
        assert!(remove.after.contains("# name comment"));
    }

    #[test]
    fn duplicate_requires_explicit_replace() {
        let spec = DependencySpec::Version(VersionReq::STAR);
        assert!(matches!(
            CandidateManifest::parse(BASE)
                .unwrap()
                .add("old", spec, false),
            Err(ProjectError::DuplicateDependency(_))
        ));
    }

    #[test]
    fn edits_inline_dependency_table_without_panicking() {
        let source = "manifest-version = 1\ndependencies = { old = \"1\" }\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n";
        let added = CandidateManifest::parse(source)
            .unwrap()
            .add(
                "new",
                DependencySpec::Version(VersionReq::parse("2").unwrap()),
                false,
            )
            .unwrap();
        assert!(added.after.contains("new = \"^2\""));
        CandidateManifest::parse(&added.after)
            .unwrap()
            .remove("old")
            .unwrap();
    }

    #[test]
    fn changed_lines_preserves_duplicate_line_multiplicity() {
        let plan = EditPlan {
            change: DependencyChange::Removed {
                alias: "old".into(),
                spec: DependencySpec::Version(VersionReq::parse("1").unwrap()),
            },
            before: "version = \"1\"\nkeep = true\nversion = \"1\"\n".into(),
            after: "version = \"1\"\nkeep = true\n".into(),
        };
        assert_eq!(plan.changed_lines(), vec![('-', "version = \"1\"".into())]);
    }
}
