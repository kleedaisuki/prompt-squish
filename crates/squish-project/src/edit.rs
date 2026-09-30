//! 保留注释的有类型候选清单编辑。 / Comment-preserving typed candidate-manifest edits.
//!
//! 初始文本同时形成保留格式的文档与已验证的语义快照；单次编辑从快照读取旧依赖，
//! 并对修改后的文本重新验证。 / Initial text produces both a lossless document and a
//! validated semantic snapshot. One edit reads prior dependencies from that snapshot and
//! validates the changed text again before returning it for resolution or publication.

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::{DependencyDetail, DependencySpec, Manifest, ProjectError, SkillSpec};

/// A skill-intent change, independent of XML dependency edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillChange {
    /// New skill name and source.
    Added { name: String, spec: SkillSpec },
    /// Explicit replacement of a prior source.
    Replaced {
        name: String,
        before: SkillSpec,
        after: SkillSpec,
    },
    /// Removed skill name and source.
    Removed { name: String, spec: SkillSpec },
}

/// Comment-preserving candidate skill edit; no filesystem or network effects.
#[derive(Clone, Debug)]
pub struct SkillEditPlan {
    /// Semantic change.
    pub change: SkillChange,
    /// Exact original manifest text.
    pub before: String,
    /// Validated candidate manifest text.
    pub after: String,
}

impl SkillEditPlan {
    /// Machine-readable changed lines, preserving duplicate-line multiplicity.
    pub fn changed_lines(&self) -> Vec<(char, String)> {
        let before: Vec<_> = self.before.lines().collect();
        let after: Vec<_> = self.after.lines().collect();
        line_excess(&before, &after, '-')
            .into_iter()
            .chain(line_excess(&after, &before, '+'))
            .collect()
    }
}

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
    /// 已验证意图供单次编辑使用；候选文本仍须重新验证。 / Validated intent for one edit; candidate text still needs validation.
    manifest: Manifest,
}

impl CandidateManifest {
    /// 解析可编辑文档并首先验证当前意图。 / Parses an editable document and validates current intent first.
    pub fn parse(source: &str) -> Result<Self, ProjectError> {
        let manifest = Manifest::parse(source)?;
        Ok(Self {
            original: source.to_owned(),
            document: source.parse()?,
            manifest,
        })
    }

    /// 添加依赖；仅在 `replace` 明确时覆盖。 / Adds a dependency, overwriting only when `replace` is explicit.
    pub fn add(
        mut self,
        alias: &str,
        spec: DependencySpec,
        replace: bool,
    ) -> Result<EditPlan, ProjectError> {
        let prior = self.manifest.dependencies.get(alias).cloned();
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
        let spec = self
            .manifest
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

    /// Add or explicitly replace a skill while retaining unrelated TOML formatting.
    pub fn add_skill(
        mut self,
        name: &str,
        spec: SkillSpec,
        replace: bool,
    ) -> Result<SkillEditPlan, ProjectError> {
        let prior = self.manifest.skills.get(name).cloned();
        if prior.is_some() && !replace {
            return Err(ProjectError::DuplicateSkill(name.into()));
        }
        let skills = named_table(&mut self.document, "skills");
        skills.insert(name, skill_item(&spec));
        let after = self.document.to_string();
        Manifest::parse(&after)?;
        let change = match prior {
            Some(before) => SkillChange::Replaced {
                name: name.into(),
                before,
                after: spec,
            },
            None => SkillChange::Added {
                name: name.into(),
                spec,
            },
        };
        Ok(SkillEditPlan {
            change,
            before: self.original,
            after,
        })
    }

    /// Remove a skill while preserving unrelated TOML formatting and comments.
    pub fn remove_skill(mut self, name: &str) -> Result<SkillEditPlan, ProjectError> {
        let spec = self
            .manifest
            .skills
            .get(name)
            .cloned()
            .ok_or_else(|| ProjectError::MissingSkill(name.into()))?;
        let table = self
            .document
            .get_mut("skills")
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| ProjectError::MissingSkill(name.into()))?;
        table.remove(name);
        let after = self.document.to_string();
        Manifest::parse(&after)?;
        Ok(SkillEditPlan {
            change: SkillChange::Removed {
                name: name.into(),
                spec,
            },
            before: self.original,
            after,
        })
    }
}

fn dependency_table(document: &mut DocumentMut) -> &mut dyn TableLike {
    named_table(document, "dependencies")
}

fn named_table<'a>(document: &'a mut DocumentMut, name: &str) -> &'a mut dyn TableLike {
    if !document.contains_key(name) {
        document.insert(name, Item::Table(Table::new()));
    }
    document[name]
        .as_table_like_mut()
        .expect("validated manifest guarantees a table")
}

fn skill_item(spec: &SkillSpec) -> Item {
    let mut table = InlineTable::new();
    if let Some(path) = &spec.path {
        table.insert("path", Value::from(path.to_string_lossy().as_ref()));
    }
    if let Some(git) = &spec.git {
        table.insert("git", Value::from(git.as_str()));
    }
    if let Some(branch) = &spec.git_reference.branch {
        table.insert("branch", Value::from(branch.as_str()));
    }
    if let Some(tag) = &spec.git_reference.tag {
        table.insert("tag", Value::from(tag.as_str()));
    }
    if let Some(rev) = &spec.git_reference.rev {
        table.insert("rev", Value::from(rev.as_str()));
    }
    if let Some(subdir) = &spec.subdir {
        table.insert("subdir", Value::from(subdir.to_string_lossy().as_ref()));
    }
    Item::Value(Value::InlineTable(table))
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
    if let Some(value) = &detail.sopack {
        table.insert("sopack", Value::from(value.to_string_lossy().as_ref()));
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
    fn sopack_add_and_remove_preserve_manifest_intent() {
        let add = CandidateManifest::parse(BASE)
            .unwrap()
            .add(
                "compiled",
                DependencySpec::sopack("vendor/compiled.sopack"),
                false,
            )
            .unwrap();
        assert!(add.after.contains("sopack = \"vendor/compiled.sopack\""));
        let manifest = Manifest::parse(&add.after).unwrap();
        assert_eq!(
            manifest.dependencies["compiled"],
            DependencySpec::sopack("vendor/compiled.sopack")
        );
        let remove = CandidateManifest::parse(&add.after)
            .unwrap()
            .remove("compiled")
            .unwrap();
        assert!(!remove.after.contains("compiled ="));
        assert!(remove.after.contains("# project comment"));
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
    fn replacement_uses_original_semantic_dependency_and_validates_candidate() {
        let replacement = DependencySpec::Version(VersionReq::parse("2").unwrap());
        let edit = CandidateManifest::parse(BASE)
            .unwrap()
            .add("old", replacement.clone(), true)
            .unwrap();
        assert!(matches!(
            edit.change,
            DependencyChange::Replaced { alias, before: DependencySpec::Version(_), after }
                if alias == "old" && after == replacement
        ));
        assert!(edit.after.contains("old = \"^2\""));

        let invalid = CandidateManifest::parse(BASE)
            .unwrap()
            .add("bad.alias", replacement, false);
        assert!(matches!(invalid, Err(ProjectError::Validation(_))));
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

    #[test]
    fn skill_edits_preserve_xml_dependencies_and_comments() {
        let source =
            format!("{BASE}\n[skills]\nold-skill = {{ path = \"skills/old\" }} # keep me\n");
        let add = CandidateManifest::parse(&source)
            .unwrap()
            .add_skill("review", SkillSpec::path("skills/review"), false)
            .unwrap();
        assert!(add.after.contains("old = \"1\" # keep me"));
        assert!(
            add.after
                .contains("old-skill = { path = \"skills/old\" } # keep me")
        );
        assert!(add.after.contains("review = { path = \"skills/review\" }"));
        assert!(matches!(
            CandidateManifest::parse(&add.after).unwrap().add_skill(
                "review",
                SkillSpec::path("other"),
                false
            ),
            Err(ProjectError::DuplicateSkill(_))
        ));
        let remove = CandidateManifest::parse(&add.after)
            .unwrap()
            .remove_skill("review")
            .unwrap();
        assert!(!remove.after.contains("review ="));
        assert!(remove.after.contains("# keep me"));
        assert!(matches!(
            CandidateManifest::parse(&remove.after)
                .unwrap()
                .remove_skill("review"),
            Err(ProjectError::MissingSkill(_))
        ));
    }
}
