use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use toml_edit::{Document, Item, TableLike};
use url::Url;

use crate::{ConfigError, ConfigLayer, SourceLocation};

/// 用户配置目录，由宿主显式解析并传入。 / User configuration home explicitly resolved by the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigHome(PathBuf);

impl ConfigHome {
    /// 创建显式配置目录；本函数不读取环境或平台全局状态。 / Creates an explicit home without reading environment or platform globals.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
    /// 返回目录路径。 / Returns the directory path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// 终端颜色策略。 / Terminal color policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ColorPolicy {
    Auto,
    Always,
    Never,
}
/// 终端进度策略。 / Terminal progress policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressPolicy {
    Auto,
    Always,
    Never,
}
/// 操作消息编码。 / Operational message encoding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MessageFormat {
    Human,
    Short,
    Json,
}
/// 管理器输出详细度。 / Manager output verbosity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
    Trace,
}

/// 稳定注册表逻辑身份。 / Stable logical registry identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegistryId(String);
impl RegistryId {
    /// 返回规范 URI。 / Returns the canonical URI.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// 稀疏注册表索引地址。 / Sparse registry index locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryIndex(String);
impl RegistryIndex {
    /// 返回规范地址。 / Returns the canonical locator.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// 凭据查询作用域标识；它不是密钥。 / Credential lookup scope identifier; it is not a secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthScope(String);
impl AuthScope {
    /// 返回不透明作用域。 / Returns the opaque scope.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 一个已验证的注册表别名。 / A validated registry alias.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registry {
    /// 稳定逻辑身份。 / Stable logical identity.
    pub id: RegistryId,
    /// 稀疏索引地址。 / Sparse index locator.
    pub index: RegistryIndex,
    /// 注入式凭据端口使用的非秘密作用域。 / Non-secret scope used by the injected credential port.
    pub auth_scope: AuthScope,
}

/// 源缓存设置。 / Source cache settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceConfig {
    /// 源对象缓存根目录。 / Source-object cache root.
    pub cache_root: PathBuf,
}
/// 管理器持久状态设置。 / Manager persistent-state settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagerConfig {
    /// 管理器状态根目录。 / Manager state root.
    pub storage_root: PathBuf,
}
/// 构建调度设置。 / Build scheduling settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildConfig {
    /// 最大并发任务数；零表示由管理器选择。 / Maximum jobs; zero delegates selection to the manager.
    pub jobs: usize,
    /// 失败后是否继续独立任务。 / Whether independent jobs continue after a failure.
    pub keep_going: bool,
}
/// 终端呈现设置。 / Terminal presentation settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TermConfig {
    /// 颜色策略。 / Color policy.
    pub color: ColorPolicy,
    /// 进度策略。 / Progress policy.
    pub progress: ProgressPolicy,
    /// 消息格式。 / Message format.
    pub message_format: MessageFormat,
    /// 详细度。 / Verbosity.
    pub verbosity: Verbosity,
}
/// 完整、强类型的有效配置。 / Complete, strongly typed effective configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// 按别名排序的注册表。 / Registries sorted by alias.
    pub registries: BTreeMap<String, Registry>,
    /// 源设置。 / Source settings.
    pub source: SourceConfig,
    /// 管理器设置。 / Manager settings.
    pub manager: ManagerConfig,
    /// 构建设置。 / Build settings.
    pub build: BuildConfig,
    /// 终端设置。 / Terminal settings.
    pub term: TermConfig,
}

/// 一次值赋值的可解释记录。 / Explainable record of one value assignment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplainEntry {
    /// 提供赋值的层。 / Layer supplying the assignment.
    pub layer: ConfigLayer,
    /// 稳定的 TOML 风格值表示。 / Stable TOML-like value representation.
    pub value: String,
    /// 源范围（若存在）。 / Source span, when available.
    pub span: Option<std::ops::Range<usize>>,
}
/// 所有有效叶值的来源链。 / Provenance chains for every effective leaf value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Provenance(BTreeMap<String, Vec<ExplainEntry>>);
impl Provenance {
    /// 返回键从低到高优先级的赋值链。 / Returns a key's assignment chain from weakest to strongest.
    pub fn explain(&self, key: &str) -> Option<&[ExplainEntry]> {
        self.0.get(key).map(Vec::as_slice)
    }
    /// 迭代全部点分叶键。 / Iterates all dotted leaf keys.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[ExplainEntry])> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}
/// 有效配置及其完整来源。 / Effective configuration with complete provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveConfig {
    /// 强类型值。 / Typed values.
    pub config: Config,
    /// 来源索引。 / Provenance index.
    pub provenance: Provenance,
}
impl EffectiveConfig {
    /// 解释一个点分键。 / Explains one dotted key.
    pub fn explain(&self, key: &str) -> Option<&[ExplainEntry]> {
        self.provenance.explain(key)
    }
}

/// 确定性分层配置加载器。 / Deterministic layered configuration loader.
pub struct ConfigLoader {
    home: ConfigHome,
    workspace_root: Option<PathBuf>,
    cli_base: PathBuf,
    overrides: Vec<String>,
}
impl ConfigLoader {
    /// 创建加载器；宿主应在此之前解析 `XMLSQUISH_HOME` 或平台目录。 / Creates a loader; the host resolves `XMLSQUISH_HOME` or a platform directory first.
    pub fn new(home: ConfigHome) -> Self {
        let cli_base = home.0.clone();
        Self {
            home,
            workspace_root: None,
            cli_base,
            overrides: Vec::new(),
        }
    }
    /// 选择工作区根；仅加载其 `.xmlsquish/config.toml`。 / Selects a workspace root; only its `.xmlsquish/config.toml` is loaded.
    pub fn workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(root.into());
        self
    }
    /// 设置 CLI 相对路径的显式基准目录。 / Sets the explicit base for relative paths in CLI overrides.
    pub fn cli_base(mut self, base: impl Into<PathBuf>) -> Self {
        self.cli_base = base.into();
        self
    }
    /// 设置按给定顺序合并的 `KEY=VALUE` 覆盖项。 / Sets `KEY=VALUE` overrides merged in the supplied order.
    pub fn cli_overrides<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.overrides = values.into_iter().map(Into::into).collect();
        self
    }
    /// 加载已明确选择的层。不存在的文件视为空层。 / Loads explicitly selected layers; missing files are empty layers.
    pub fn load(self) -> Result<EffectiveConfig, ConfigError> {
        let mut state = State::defaults(&self.home);
        let user = self.home.0.join("config.toml");
        apply_file(&mut state, user.clone(), ConfigLayer::User(user))?;
        if let Some(root) = self.workspace_root {
            let path = root.join(".xmlsquish").join("config.toml");
            apply_file(&mut state, path.clone(), ConfigLayer::Workspace(path))?;
        }
        for (index, value) in self.overrides.iter().enumerate() {
            apply_override(&mut state, index, value, &self.cli_base)?;
        }
        state.finish()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct PartialConfig {
    #[serde(default)]
    registries: BTreeMap<String, RawRegistry>,
    source: Option<RawSource>,
    manager: Option<RawManager>,
    build: Option<RawBuild>,
    term: Option<RawTerm>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawRegistry {
    id: Option<String>,
    index: Option<String>,
    auth_scope: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawSource {
    cache_root: Option<PathBuf>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawManager {
    storage_root: Option<PathBuf>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawBuild {
    jobs: Option<usize>,
    keep_going: Option<bool>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawTerm {
    color: Option<ColorPolicy>,
    progress: Option<ProgressPolicy>,
    message_format: Option<MessageFormat>,
    verbosity: Option<Verbosity>,
}

#[derive(Default)]
struct RegistryState {
    id: Option<RegistryId>,
    index: Option<RegistryIndex>,
    auth_scope: Option<AuthScope>,
}
struct State {
    config: Config,
    provenance: Provenance,
    registry_locations: BTreeMap<String, SourceLocation>,
    registries: BTreeMap<String, RegistryState>,
}
impl State {
    fn defaults(home: &ConfigHome) -> Self {
        let mut provenance = Provenance::default();
        let base = home.0.clone();
        let config = Config {
            registries: BTreeMap::new(),
            source: SourceConfig {
                cache_root: base.join("cache").join("sources"),
            },
            manager: ManagerConfig {
                storage_root: base.join("state"),
            },
            build: BuildConfig {
                jobs: 0,
                keep_going: true,
            },
            term: TermConfig {
                color: ColorPolicy::Auto,
                progress: ProgressPolicy::Auto,
                message_format: MessageFormat::Human,
                verbosity: Verbosity::Normal,
            },
        };
        for (key, value) in [
            ("source.cache-root", display_path(&config.source.cache_root)),
            (
                "manager.storage-root",
                display_path(&config.manager.storage_root),
            ),
            ("build.jobs", "0".into()),
            ("build.keep-going", "true".into()),
            ("term.color", "auto".into()),
            ("term.progress", "auto".into()),
            ("term.message-format", "human".into()),
            ("term.verbosity", "normal".into()),
        ] {
            provenance.0.insert(
                key.into(),
                vec![ExplainEntry {
                    layer: ConfigLayer::Defaults,
                    value,
                    span: None,
                }],
            );
        }
        Self {
            config,
            provenance,
            registry_locations: BTreeMap::new(),
            registries: BTreeMap::new(),
        }
    }
    fn finish(mut self) -> Result<EffectiveConfig, ConfigError> {
        for (alias, raw) in std::mem::take(&mut self.registries) {
            let location = self
                .registry_locations
                .get(&alias)
                .cloned()
                .unwrap_or(SourceLocation {
                    layer: ConfigLayer::Defaults,
                    span: None,
                });
            let id = raw.id.ok_or_else(|| ConfigError::InvalidValue {
                key: format!("registries.{alias}.id"),
                location: location.clone(),
                message: "required for every registry alias".into(),
            })?;
            let index = raw.index.ok_or_else(|| ConfigError::InvalidValue {
                key: format!("registries.{alias}.index"),
                location: location.clone(),
                message: "required for every registry alias".into(),
            })?;
            let auth_scope = raw.auth_scope.unwrap_or_else(|| {
                let id_origin = self
                    .provenance
                    .0
                    .get(&format!("registries.{alias}.id"))
                    .and_then(|chain| chain.last())
                    .cloned()
                    .unwrap_or(ExplainEntry {
                        layer: location.layer.clone(),
                        value: id.0.clone(),
                        span: location.span.clone(),
                    });
                self.provenance
                    .0
                    .entry(format!("registries.{alias}.auth-scope"))
                    .or_default()
                    .push(id_origin);
                AuthScope(id.0.clone())
            });
            self.config.registries.insert(
                alias,
                Registry {
                    id,
                    index,
                    auth_scope,
                },
            );
        }
        let mut ids: BTreeMap<&str, (&str, &Registry)> = BTreeMap::new();
        let mut aliases: BTreeMap<String, &str> = BTreeMap::new();
        for (alias, registry) in &self.config.registries {
            let folded = alias.to_ascii_lowercase();
            if let Some(first) = aliases.insert(folded.clone(), alias) {
                return Err(collision(
                    first,
                    alias,
                    "alias",
                    &folded,
                    &self.registry_locations,
                ));
            }
            if let Some((first_alias, first_registry)) =
                ids.insert(registry.id.as_str(), (alias, registry))
            {
                if first_registry.index != registry.index {
                    return Err(collision(
                        first_alias,
                        alias,
                        "stable-id endpoint",
                        registry.id.as_str(),
                        &self.registry_locations,
                    ));
                }
                if first_registry.auth_scope != registry.auth_scope {
                    return Err(collision(
                        first_alias,
                        alias,
                        "stable-id auth scope",
                        registry.id.as_str(),
                        &self.registry_locations,
                    ));
                }
            }
        }
        Ok(EffectiveConfig {
            config: self.config,
            provenance: self.provenance,
        })
    }
}

fn collision(
    first: &str,
    second: &str,
    kind: &'static str,
    value: &str,
    locations: &BTreeMap<String, SourceLocation>,
) -> ConfigError {
    ConfigError::RegistryCollision {
        first: first.into(),
        second: second.into(),
        kind,
        value: value.into(),
        location: Box::new(locations.get(second).cloned().unwrap_or(SourceLocation {
            layer: ConfigLayer::Defaults,
            span: None,
        })),
    }
}

fn apply_file(state: &mut State, path: PathBuf, layer: ConfigLayer) -> Result<(), ConfigError> {
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ConfigError::Read {
                location: layer,
                source,
            });
        }
    };
    apply_text(state, &text, layer, path.parent().unwrap_or(Path::new(".")))
}
fn apply_override(
    state: &mut State,
    index: usize,
    raw: &str,
    base: &Path,
) -> Result<(), ConfigError> {
    let Some((key, value)) = raw.split_once('=') else {
        return Err(ConfigError::Parse {
            location: SourceLocation {
                layer: ConfigLayer::Cli { index },
                span: Some(0..raw.len()),
            },
            message: "expected KEY=VALUE".into(),
        });
    };
    if key.trim().is_empty() || value.trim().is_empty() {
        return Err(ConfigError::Parse {
            location: SourceLocation {
                layer: ConfigLayer::Cli { index },
                span: Some(0..raw.len()),
            },
            message: "expected non-empty KEY=VALUE".into(),
        });
    }
    // Parse the original argument verbatim so all spans index the argv value.
    apply_text(state, raw, ConfigLayer::Cli { index }, base)
}
fn apply_text(
    state: &mut State,
    text: &str,
    layer: ConfigLayer,
    base: &Path,
) -> Result<(), ConfigError> {
    let doc = Document::parse(text.to_owned()).map_err(|e| ConfigError::Parse {
        location: SourceLocation {
            layer: layer.clone(),
            span: e.span().or(Some(0..text.len())),
        },
        message: e.message().into(),
    })?;
    validate_keys(&doc, &layer)?;
    let partial: PartialConfig = toml::from_str(text).map_err(|e| ConfigError::Parse {
        location: SourceLocation {
            layer: layer.clone(),
            span: e.span().or(Some(0..text.len())),
        },
        message: e.message().into(),
    })?;
    merge(state, partial, layer, base, &doc)
}

fn validate_keys(doc: &Document<String>, layer: &ConfigLayer) -> Result<(), ConfigError> {
    const ROOT: &[&str] = &["registries", "source", "manager", "build", "term"];
    const SOURCE: &[&str] = &["cache-root"];
    const MANAGER: &[&str] = &["storage-root"];
    const BUILD: &[&str] = &["jobs", "keep-going"];
    const TERM: &[&str] = &["color", "progress", "message-format", "verbosity"];
    const REGISTRY: &[&str] = &["id", "index", "auth-scope"];
    for (key, item) in doc.iter() {
        if !ROOT.contains(&key) {
            return unknown(
                key,
                doc.as_table().key(key).and_then(toml_edit::Key::span),
                layer,
            );
        }
        let Some(table) = item.as_table_like() else {
            return invalid(key, item, layer, "expected a table");
        };
        if key == "registries" {
            for (alias, entry) in table.iter() {
                let Some(reg) = entry.as_table_like() else {
                    return invalid(
                        &format!("registries.{alias}"),
                        entry,
                        layer,
                        "expected a table",
                    );
                };
                check_table(reg, &format!("registries.{alias}"), REGISTRY, layer)?;
            }
        } else {
            let allowed = match key {
                "source" => SOURCE,
                "manager" => MANAGER,
                "build" => BUILD,
                _ => TERM,
            };
            check_table(table, key, allowed, layer)?;
        }
    }
    Ok(())
}
fn check_table(
    table: &dyn TableLike,
    prefix: &str,
    allowed: &[&str],
    layer: &ConfigLayer,
) -> Result<(), ConfigError> {
    for (key, _) in table.iter() {
        if !allowed.contains(&key) {
            let span = table
                .get_key_value(key)
                .and_then(|(parsed_key, _)| parsed_key.span());
            return unknown(&format!("{prefix}.{key}"), span, layer);
        }
    }
    Ok(())
}
fn unknown(
    key: &str,
    span: Option<std::ops::Range<usize>>,
    layer: &ConfigLayer,
) -> Result<(), ConfigError> {
    Err(ConfigError::UnknownKey {
        key: key.into(),
        location: SourceLocation {
            layer: layer.clone(),
            span,
        },
    })
}
fn invalid(key: &str, item: &Item, layer: &ConfigLayer, message: &str) -> Result<(), ConfigError> {
    Err(ConfigError::InvalidValue {
        key: key.into(),
        location: SourceLocation {
            layer: layer.clone(),
            span: item.span(),
        },
        message: message.into(),
    })
}

fn merge(
    state: &mut State,
    p: PartialConfig,
    layer: ConfigLayer,
    base: &Path,
    doc: &Document<String>,
) -> Result<(), ConfigError> {
    for (alias, raw) in p.registries {
        validate_alias(&alias, &layer, doc)?;
        let entry = state.registries.entry(alias.clone()).or_default();
        let mut values = Vec::new();
        if let Some(raw_id) = raw.id {
            let id = RegistryId(
                canonical_https(&raw_id, false)
                    .map_err(|m| value_error(&format!("registries.{alias}.id"), m, &layer, doc))?,
            );
            values.push(("id", id.0.clone()));
            entry.id = Some(id);
        }
        if let Some(raw_index) = raw.index {
            let index =
                RegistryIndex(canonical_index(&raw_index).map_err(|m| {
                    value_error(&format!("registries.{alias}.index"), m, &layer, doc)
                })?);
            values.push(("index", index.0.clone()));
            entry.index = Some(index);
        }
        if let Some(scope) = raw.auth_scope {
            if scope.trim().is_empty() {
                return Err(value_error(
                    &format!("registries.{alias}.auth-scope"),
                    "must not be empty",
                    &layer,
                    doc,
                ));
            }
            values.push(("auth-scope", scope.clone()));
            entry.auth_scope = Some(AuthScope(scope));
        }
        state.registry_locations.insert(
            alias.clone(),
            location_for(doc, &format!("registries.{alias}"), &layer),
        );
        for (field, value) in values {
            record(
                state,
                format!("registries.{alias}.{field}"),
                value,
                &layer,
                doc,
            );
        }
    }
    if let Some(v) = p.source.and_then(|v| v.cache_root) {
        state.config.source.cache_root = resolve(base, v);
        record(
            state,
            "source.cache-root".into(),
            display_path(&state.config.source.cache_root),
            &layer,
            doc,
        );
    }
    if let Some(v) = p.manager.and_then(|v| v.storage_root) {
        state.config.manager.storage_root = resolve(base, v);
        record(
            state,
            "manager.storage-root".into(),
            display_path(&state.config.manager.storage_root),
            &layer,
            doc,
        );
    }
    if let Some(build) = p.build {
        if let Some(v) = build.jobs {
            state.config.build.jobs = v;
            record(state, "build.jobs".into(), v.to_string(), &layer, doc);
        }
        if let Some(v) = build.keep_going {
            state.config.build.keep_going = v;
            record(state, "build.keep-going".into(), v.to_string(), &layer, doc);
        }
    }
    if let Some(term) = p.term {
        if let Some(v) = term.color {
            state.config.term.color = v;
            record(state, "term.color".into(), enum_text(v), &layer, doc);
        }
        if let Some(v) = term.progress {
            state.config.term.progress = v;
            record(state, "term.progress".into(), enum_text(v), &layer, doc);
        }
        if let Some(v) = term.message_format {
            state.config.term.message_format = v;
            record(
                state,
                "term.message-format".into(),
                enum_text(v),
                &layer,
                doc,
            );
        }
        if let Some(v) = term.verbosity {
            state.config.term.verbosity = v;
            record(state, "term.verbosity".into(), enum_text(v), &layer, doc);
        }
    }
    Ok(())
}
fn validate_alias(
    alias: &str,
    layer: &ConfigLayer,
    doc: &Document<String>,
) -> Result<(), ConfigError> {
    if alias.is_empty()
        || !alias
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(value_error(
            &format!("registries.{alias}"),
            "alias must contain only ASCII letters, digits, '-' or '_'",
            layer,
            doc,
        ));
    }
    Ok(())
}
fn canonical_https(raw: &str, trailing_slash: bool) -> Result<String, &'static str> {
    let url = Url::parse(raw).map_err(|_| "must be an absolute URI")?;
    if url.scheme() != "https" {
        return Err("must use https");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("must not contain user information, query, or fragment");
    }
    if trailing_slash && !url.path().ends_with('/') {
        return Err("must end in '/'");
    }
    Ok(url.to_string())
}
fn canonical_index(raw: &str) -> Result<String, &'static str> {
    let Some(inner) = raw.strip_prefix("sparse+") else {
        return Err("must use sparse+https");
    };
    let canonical = canonical_https(inner, true)?;
    Ok(format!("sparse+{canonical}"))
}
fn value_error(
    key: &str,
    message: impl Into<String>,
    layer: &ConfigLayer,
    doc: &Document<String>,
) -> ConfigError {
    ConfigError::InvalidValue {
        key: key.into(),
        location: location_for(doc, key, layer),
        message: message.into(),
    }
}
fn location_for(doc: &Document<String>, key: &str, layer: &ConfigLayer) -> SourceLocation {
    let mut item: &Item = doc.as_item();
    for part in key.split('.') {
        let Some(next) = item.get(part) else {
            break;
        };
        item = next;
    }
    SourceLocation {
        layer: layer.clone(),
        span: item.span(),
    }
}
fn record(
    state: &mut State,
    key: String,
    value: String,
    layer: &ConfigLayer,
    doc: &Document<String>,
) {
    let loc = location_for(doc, &key, layer);
    state
        .provenance
        .0
        .entry(key)
        .or_default()
        .push(ExplainEntry {
            layer: layer.clone(),
            value,
            span: loc.span,
        });
}
fn resolve(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}
fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn enum_text<T: std::fmt::Debug>(v: T) -> String {
    format!("{v:?}").to_ascii_lowercase()
}
