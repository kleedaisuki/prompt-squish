use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fs2::FileExt;
use reqwest::blocking::Client;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    Access, ArchiveDigest, ContentDigest, FetchError, HostContext, ManifestDigest,
    MaterializedPackage, Materializer, Sha256Digest, SourceEvent,
};

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// 单次、禁止自动 redirect 的 HTTP 请求。 / One HTTP request for which automatic redirects are forbidden.
#[derive(Clone)]
pub struct HttpRequest {
    /// 完整请求 URL。 / Complete request URL.
    pub url: Url,
    /// 请求头；authorization 仅在内存中存在。 / Request headers; authorization exists only in memory.
    pub headers: BTreeMap<String, String>,
    /// 本次响应允许读取的最大字节数。 / Maximum response bytes permitted for this request.
    pub max_bytes: u64,
}

/// Transport 返回的有限 HTTP 响应。 / Bounded HTTP response returned by a transport.
#[derive(Clone)]
pub struct HttpResponse {
    /// HTTP 状态码。 / HTTP status code.
    pub status: u16,
    /// 小写响应头名到值。 / Lowercase response-header names to values.
    pub headers: BTreeMap<String, String>,
    /// 不超过请求限制的响应体。 / Response body no larger than the request limit.
    pub body: Vec<u8>,
}

/// 可注入 HTTP transport；redirect 与 credential scope 始终由 fetch layer 处理。 / Injectable HTTP transport; the fetch layer always owns redirects and credential scope.
pub trait HttpTransport: Send + Sync {
    /// 执行一次请求，不得自动跟随 redirect。 / Executes one request and must not follow redirects automatically.
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError>;
}

/// 基于 reqwest/rustls 的安全默认 transport。 / Safe default transport backed by reqwest/rustls.
pub struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
    /// 构造禁用自动 redirect 的默认 client。 / Builds the default client with automatic redirects disabled.
    pub fn new() -> Result<Self, FetchError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(http_error)?;
        Ok(Self { client })
    }
}

impl HttpTransport for ReqwestTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError> {
        let mut builder = self.client.get(request.url);
        for (name, value) in request.headers {
            builder = builder.header(&name, value);
        }
        let response = builder.send().map_err(http_error)?;
        if response
            .content_length()
            .is_some_and(|n| n > request.max_bytes)
        {
            return Err(FetchError::Integrity(
                "HTTP response exceeds acquisition limit".into(),
            ));
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_ascii_lowercase(), v.to_owned()))
            })
            .collect();
        let mut body = Vec::new();
        response
            .take(request.max_bytes + 1)
            .read_to_end(&mut body)?;
        if body.len() as u64 > request.max_bytes {
            return Err(FetchError::Integrity(
                "HTTP response exceeds acquisition limit".into(),
            ));
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// 已验证且不可观察的 HTTP Authorization 字段值。 / Validated, opaque HTTP
/// Authorization field value.
#[derive(Clone, Eq, PartialEq)]
pub struct AuthorizationValue(String);

impl AuthorizationValue {
    /// 验证精确字段值；不会修剪或添加认证 scheme。 / Validates an exact field value;
    /// it neither trims it nor adds an authentication scheme.
    pub fn new(value: String) -> Result<Self, CredentialError> {
        if value.is_empty() || reqwest::header::HeaderValue::from_str(&value).is_err() {
            return Err(CredentialError::Invalid(
                "Authorization environment value is empty or is not a valid HTTP header value"
                    .into(),
            ));
        }
        Ok(Self(value))
    }

    /// 仅供 HTTP 请求构造器读取字段内容。 / Exposes the field only for HTTP request
    /// construction.
    #[must_use]
    pub(crate) fn expose_for_http(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AuthorizationValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationValue(<redacted>)")
    }
}

impl fmt::Display for AuthorizationValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// 凭据提供器的非秘密配置错误。 / Non-secret credential-provider configuration error.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// 环境或提供器配置无效；消息不得包含 credential value。 / Environment or
    /// provider configuration is invalid; the message must not contain credential values.
    #[error("{0}")]
    Invalid(String),
}

/// Registry credential provider; returned values are never persisted or observed. / Registry credential provider；返回值绝不持久化或进入事件。
pub trait CredentialPort: Send + Sync {
    /// 为稳定 registry、配置作用域和当前 origin 查询 credential。 / Looks up a
    /// credential for the stable registry, configured scope, and current origin.
    fn authorization(
        &self,
        registry_id: &str,
        auth_scope: &str,
        origin: &str,
    ) -> Result<Option<AuthorizationValue>, CredentialError>;
}
/// 不提供认证信息。 / Supplies no credentials.
#[derive(Default)]
pub struct NoCredentials;
impl CredentialPort for NoCredentials {
    fn authorization(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<AuthorizationValue>, CredentialError> {
        Ok(None)
    }
}

/// 一个 sparse registry 的稳定身份和可变端点。 / Stable identity and mutable endpoint of one sparse registry.
#[derive(Clone, Debug)]
pub struct RegistryConfig {
    pub id: String,
    pub index: String,
    /// 非秘密 credential namespace；别名不得参与查询。 / Non-secret credential
    /// namespace; aliases never participate in lookup.
    pub auth_scope: String,
}

#[derive(Clone, Debug)]
pub struct RegistryArchive {
    pub format: String,
    pub size: u64,
    pub digest: ArchiveDigest,
    pub content_digest: ContentDigest,
}

/// 解析器投影以及精确归档身份。 / Resolver projection plus exact archive identity.
#[derive(Clone, Debug)]
pub struct SparseCandidate {
    pub version: Version,
    pub yanked: bool,
    pub manifest: squish_project::Manifest,
    pub archive: RegistryArchive,
    pub manifest_digest: ManifestDigest,
}

#[derive(Deserialize)]
struct WireConfig {
    v: u32,
    #[serde(rename = "registry-id")]
    registry_id: String,
    dl: String,
    #[serde(default, rename = "auth-required")]
    auth_required: bool,
}
#[derive(Deserialize)]
struct Row {
    v: u32,
    name: String,
    vers: Version,
    package: RowPackage,
    deps: Vec<RowDependency>,
    archive: RowArchive,
    #[serde(rename = "manifest-sha256")]
    manifest_sha256: String,
    yanked: bool,
}
#[derive(Deserialize)]
struct RowPackage {
    dialect: String,
    #[serde(rename = "source-root")]
    source_root: PathBuf,
}
#[derive(Deserialize)]
struct RowArchive {
    format: String,
    size: u64,
    sha256: String,
    #[serde(rename = "content-sha256")]
    content_sha256: String,
}
#[derive(Deserialize)]
struct RowDependency {
    alias: String,
    package: String,
    req: VersionReq,
    #[serde(default, rename = "registry-id")]
    registry_id: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default, rename = "default-features")]
    default_features: bool,
    #[serde(default)]
    features: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Cached {
    etag: Option<String>,
    last_modified: Option<String>,
    body_sha256: String,
}

struct FetchedResponse {
    body: Vec<u8>,
    headers: BTreeMap<String, String>,
    not_modified: bool,
}

/// 支持 conditional cache 与严格 offline 的 sparse HTTP registry。 / Sparse HTTP registry with conditional caching and strict offline operation.
pub struct SparseRegistry<C = NoCredentials> {
    context: HostContext,
    config: RegistryConfig,
    credentials: C,
    transport: Box<dyn HttpTransport>,
}

impl SparseRegistry<NoCredentials> {
    /// 创建无认证 registry。 / Creates a registry without authentication.
    pub fn new(context: HostContext, config: RegistryConfig) -> Result<Self, FetchError> {
        Self::with_credentials(context, config, NoCredentials)
    }
}

impl<C: CredentialPort> SparseRegistry<C> {
    /// 创建带独立 credential port 的 registry。 / Creates a registry with a separate credential port.
    pub fn with_credentials(
        context: HostContext,
        config: RegistryConfig,
        credentials: C,
    ) -> Result<Self, FetchError> {
        Self::with_dependencies(
            context,
            config,
            credentials,
            Box::new(ReqwestTransport::new()?),
        )
    }

    /// 使用显式 transport 与 credential port 的 production constructor。 / Production constructor with explicit transport and credential port.
    pub fn with_dependencies(
        context: HostContext,
        config: RegistryConfig,
        credentials: C,
        transport: Box<dyn HttpTransport>,
    ) -> Result<Self, FetchError> {
        validate_registry_config(&config)?;
        Ok(Self {
            context,
            config,
            credentials,
            transport,
        })
    }

    /// 获取并完整验证某包的 JSONL 投影。 / Fetches and fully validates one package JSONL projection.
    pub fn candidates(
        &self,
        package: &str,
        access: Access,
    ) -> Result<Vec<SparseCandidate>, FetchError> {
        validate_package_name(package)?;
        let cfg_bytes = self.resource(
            "config.json",
            "application/vnd.xmlsquish.registry-config+json; version=1",
            access,
            false,
            |body| validate_wire_config(body, &self.config.id).map(|_| ()),
        )?;
        let cfg: WireConfig = serde_json::from_slice(&cfg_bytes)?;
        if cfg.v != 1 {
            return Err(FetchError::Metadata(format!(
                "unsupported registry config version {}",
                cfg.v
            )));
        }
        if cfg.registry_id != self.config.id {
            return Err(FetchError::Config("registry identity mismatch".into()));
        }
        validate_download_template(&cfg.dl)?;
        let shard = shard_path(package);
        let body = self.resource(
            &shard,
            "application/vnd.xmlsquish.package-index+jsonl; version=1",
            access,
            cfg.auth_required,
            |body| validate_metadata_body(body, package, &self.config.id, &self.context.limits),
        )?;
        if body.len() as u64 > self.context.limits.max_metadata_bytes {
            return Err(FetchError::Metadata(
                "metadata exceeds configured limit".into(),
            ));
        }
        let mut result = Vec::new();
        let mut versions = BTreeSet::new();
        let mut supported = false;
        for line in body.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            if line.len() > self.context.limits.max_metadata_line {
                return Err(FetchError::Metadata("metadata line exceeds limit".into()));
            }
            let generic: serde_json::Value = serde_json::from_slice(line)?;
            if generic.get("v").and_then(|v| v.as_u64()) != Some(1) {
                continue;
            }
            supported = true;
            let row: Row = serde_json::from_value(generic)?;
            debug_assert_eq!(row.v, 1, "wire version was checked before deserialization");
            if !row.name.eq_ignore_ascii_case(package)
                || row.vers.build != semver::BuildMetadata::EMPTY
            {
                return Err(FetchError::Metadata(
                    "row package/version identity invalid".into(),
                ));
            }
            if !versions.insert(row.vers.clone()) {
                return Err(FetchError::Metadata("duplicate package version".into()));
            }
            if row.archive.format != "xspkg-tar-gzip/1" {
                return Err(FetchError::Metadata("unsupported archive format".into()));
            }
            let dependencies = row
                .deps
                .into_iter()
                .map(|d| {
                    let detail = squish_project::DependencyDetail {
                        version: Some(d.req),
                        registry: d.registry_id.or_else(|| Some(self.config.id.clone())),
                        package: Some(d.package),
                        optional: d.optional,
                        default_features: d.default_features,
                        features: d.features,
                        ..Default::default()
                    };
                    (
                        d.alias,
                        squish_project::DependencySpec::Detail(Box::new(detail)),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let manifest = squish_project::Manifest {
                manifest_version: 1,
                workspace: None,
                package: Some(squish_project::Package {
                    name: row.name,
                    version: row.vers.clone(),
                    dialect: row.package.dialect,
                    source_root: row.package.source_root,
                }),
                targets: BTreeMap::new(),
                dependencies,
                exports: BTreeMap::new(),
                profiles: BTreeMap::new(),
            };
            manifest.validate()?;
            result.push(SparseCandidate {
                version: row.vers,
                yanked: row.yanked,
                manifest,
                archive: RegistryArchive {
                    format: row.archive.format,
                    size: row.archive.size,
                    digest: ArchiveDigest(Sha256Digest::parse(row.archive.sha256)?),
                    content_digest: ContentDigest(Sha256Digest::parse(row.archive.content_sha256)?),
                },
                manifest_digest: ManifestDigest(Sha256Digest::parse(row.manifest_sha256)?),
            });
        }
        if !supported && !body.is_empty() {
            return Err(FetchError::Metadata(
                "all package metadata rows use unsupported versions".into(),
            ));
        }
        result.sort_by(|a, b| a.version.cmp(&b.version));
        Ok(result)
    }

    /// 下载并校验精确归档字节，先写 staging 后发布到 CAS。 / Downloads and verifies exact archive bytes, staging before CAS publication.
    pub fn archive(
        &self,
        package: &str,
        candidate: &SparseCandidate,
        access: Access,
    ) -> Result<Vec<u8>, FetchError> {
        let path = self.blob_path(&candidate.archive.digest.0.0);
        if let Ok(bytes) = read_bounded(&path, self.context.limits.max_archive_bytes)
            && bytes.len() as u64 == candidate.archive.size
            && hex::encode(Sha256::digest(&bytes)) == candidate.archive.digest.0.0
        {
            return Ok(bytes);
        }
        if access == Access::LocalOnly {
            return Err(FetchError::OfflineMiss(format!(
                "registry archive {}",
                candidate.archive.digest.0.0
            )));
        }
        let cfg: WireConfig = serde_json::from_slice(&self.resource(
            "config.json",
            "application/vnd.xmlsquish.registry-config+json; version=1",
            access,
            false,
            |body| validate_wire_config(body, &self.config.id).map(|_| ()),
        )?)?;
        let url = render_download(
            &cfg.dl,
            package,
            &candidate.version,
            &candidate.archive.digest.0.0,
        )?;
        let bytes = self
            .get_url(
                &url,
                "archive",
                cfg.auth_required,
                None,
                candidate
                    .archive
                    .size
                    .min(self.context.limits.max_archive_bytes),
            )?
            .body;
        if bytes.len() as u64 != candidate.archive.size
            || hex::encode(Sha256::digest(&bytes)) != candidate.archive.digest.0.0
        {
            return Err(FetchError::Integrity(
                "downloaded archive identity mismatch".into(),
            ));
        }
        fs::create_dir_all(path.parent().expect("blob path parent"))?;
        atomic_write(&path, &bytes)?;
        Ok(bytes)
    }

    /// 获取、验证投影并通过统一 materializer 发布精确包。 / Acquires, verifies the projection, and publishes an exact package through the unified materializer.
    pub fn materialize(
        &self,
        package: &str,
        candidate: &SparseCandidate,
        access: Access,
        materializer: &Materializer,
    ) -> Result<MaterializedPackage, FetchError> {
        let archive = self.archive(package, candidate, access)?;
        let tree = materializer.tree_from_xspkg(
            &archive,
            package,
            &candidate.version,
            &candidate.archive.digest,
            candidate.archive.size,
            &candidate.archive.content_digest,
        )?;
        let manifest_file = tree
            .files
            .iter()
            .find(|f| f.path == "xmlsquish.toml")
            .ok_or_else(|| FetchError::Integrity("root manifest missing".into()))?;
        let digest = hex::encode(Sha256::digest(&manifest_file.bytes));
        if digest != candidate.manifest_digest.0.0 {
            return Err(FetchError::Integrity(
                "manifest digest disagrees with registry row".into(),
            ));
        }
        let actual = squish_project::Manifest::parse(
            std::str::from_utf8(&manifest_file.bytes)
                .map_err(|_| FetchError::Integrity("manifest is not UTF-8".into()))?,
        )?;
        if actual.package != candidate.manifest.package
            || canonical_dependencies(&actual.dependencies, &self.config.id)?
                != canonical_dependencies(&candidate.manifest.dependencies, &self.config.id)?
        {
            return Err(FetchError::Integrity(
                "manifest projection disagrees with registry row".into(),
            ));
        }
        let result = materializer.materialize(
            &format!(
                "registry:{}:{package}@{}",
                self.config.id, candidate.version
            ),
            &tree,
        )?;
        let mapping = self
            .context
            .cache
            .join("v1/registry-materialized")
            .join(&candidate.archive.digest.0.0);
        if let Some(parent) = mapping.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&mapping, candidate.archive.content_digest.0.0.as_bytes())?;
        Ok(result)
    }

    fn resource<F>(
        &self,
        resource: &str,
        accept: &str,
        access: Access,
        authenticated: bool,
        validate: F,
    ) -> Result<Vec<u8>, FetchError>
    where
        F: Fn(&[u8]) -> Result<(), FetchError>,
    {
        let key = hex::encode(Sha256::digest(format!("{}\0{resource}", self.config.id)));
        let dir = self
            .context
            .cache
            .join("v1/sparse")
            .join(hex::encode(Sha256::digest(&self.config.id)))
            .join("bodies");
        let meta_path = dir.join(format!("{key}.json"));
        let cached: Option<Cached> = fs::read(&meta_path)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok());
        let old = cached
            .as_ref()
            .and_then(|c| {
                read_bounded(
                    &dir.join(format!("{}.body", c.body_sha256)),
                    self.context.limits.max_metadata_bytes,
                )
                .ok()
            })
            .filter(|body| {
                cached
                    .as_ref()
                    .is_some_and(|c| hex::encode(Sha256::digest(body)) == c.body_sha256)
            });
        if access == Access::LocalOnly {
            let body = old
                .ok_or_else(|| FetchError::OfflineMiss(format!("registry metadata {resource}")))?;
            validate(&body)?;
            return Ok(body);
        }
        let base = self
            .config
            .index
            .strip_prefix("sparse+")
            .expect("validated sparse endpoint");
        let url = Url::parse(base)
            .map_err(|e| FetchError::Config(e.to_string()))?
            .join(resource)
            .map_err(|e| FetchError::Config(e.to_string()))?;
        if url.scheme() == "file" {
            let path = url
                .to_file_path()
                .map_err(|_| FetchError::Config("invalid file registry URL".into()))?;
            let body = read_bounded(&path, self.context.limits.max_metadata_bytes)?;
            validate(&body)?;
            self.store_body(&dir, &meta_path, &body, None, None)?;
            return Ok(body);
        }
        let validator = old.as_ref().and(cached.as_ref()).and_then(|c| {
            c.etag
                .as_ref()
                .map(|v| ("if-none-match", v.as_str()))
                .or_else(|| {
                    c.last_modified
                        .as_ref()
                        .map(|v| ("if-modified-since", v.as_str()))
                })
        });
        let fetched = self.get_url_with_accept(
            &url,
            resource,
            authenticated,
            validator,
            accept,
            self.context.limits.max_metadata_bytes,
        )?;
        if fetched.not_modified {
            let body = old.ok_or_else(|| {
                FetchError::Metadata("server returned 304 without a cached body".into())
            })?;
            validate(&body)?;
            return Ok(body);
        }
        let body = fetched.body;
        let headers = fetched.headers;
        validate(&body)?;
        self.store_body(
            &dir,
            &meta_path,
            &body,
            headers.get("etag").map(String::as_str),
            headers.get("last-modified").map(String::as_str),
        )?;
        Ok(body)
    }

    fn get_url(
        &self,
        url: &Url,
        purpose: &str,
        auth: bool,
        validator: Option<(&str, &str)>,
        max_bytes: u64,
    ) -> Result<FetchedResponse, FetchError> {
        self.get_url_with_accept(
            url,
            purpose,
            auth,
            validator,
            "application/vnd.xmlsquish.package+gzip; version=1",
            max_bytes,
        )
    }
    fn get_url_with_accept(
        &self,
        url: &Url,
        purpose: &str,
        auth: bool,
        validator: Option<(&str, &str)>,
        accept: &str,
        max_bytes: u64,
    ) -> Result<FetchedResponse, FetchError> {
        let mut current_url = url.clone();
        let mut origin = redacted_origin(&current_url);
        let mut authorization = auth
            .then(|| {
                self.credentials.authorization(
                    &self.config.id,
                    &self.config.auth_scope,
                    &request_origin(&current_url),
                )
            })
            .transpose()?
            .flatten();
        if auth && authorization.is_none() {
            return Err(authentication_unavailable(&self.config, &current_url));
        }
        let mut bootstrap_attempted = auth;
        'redirects: for redirects in 0..=8 {
            for attempt in 1..=3u8 {
                let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
                self.context
                    .observer
                    .emit(SourceEvent::NetworkRequestStarted {
                        request_id: id,
                        origin: origin.clone(),
                        purpose: purpose.into(),
                    });
                let mut headers = BTreeMap::from([("accept".into(), accept.into())]);
                if let Some((name, value)) = validator {
                    headers.insert(name.into(), value.into());
                }
                if let Some(value) = &authorization {
                    headers.insert("authorization".into(), value.expose_for_http().to_owned());
                }
                let mut response = self.transport.execute(HttpRequest {
                    url: current_url.clone(),
                    headers,
                    max_bytes,
                })?;
                response.headers = response
                    .headers
                    .into_iter()
                    .map(|(name, value)| (name.to_ascii_lowercase(), value))
                    .collect();
                let status = response.status;
                if status == 304 {
                    self.context
                        .observer
                        .emit(SourceEvent::NetworkRequestFinished {
                            request_id: id,
                            status: "304".into(),
                            received_bytes: 0,
                            validator_used: validator.is_some(),
                        });
                    return Ok(FetchedResponse {
                        body: Vec::new(),
                        headers: response.headers,
                        not_modified: true,
                    });
                }
                if matches!(status, 301 | 302 | 303 | 307 | 308) {
                    if redirects == 8 {
                        return Err(FetchError::Http("redirect limit exceeded".into()));
                    }
                    let location = response
                        .headers
                        .get("location")
                        .ok_or_else(|| FetchError::Http("redirect has no Location".into()))?;
                    let next = current_url
                        .join(location)
                        .map_err(|_| FetchError::Http("invalid redirect Location".into()))?;
                    if next.scheme() != "https" {
                        return Err(FetchError::Http("redirect must preserve HTTPS".into()));
                    }
                    let next_origin = redacted_origin(&next);
                    authorization = if bootstrap_attempted {
                        self.credentials.authorization(
                            &self.config.id,
                            &self.config.auth_scope,
                            &request_origin(&next),
                        )?
                    } else {
                        None
                    };
                    if bootstrap_attempted && authorization.is_none() {
                        return Err(authentication_unavailable(&self.config, &next));
                    }
                    current_url = next;
                    origin = next_origin;
                    continue 'redirects;
                }
                if status == 401 && !bootstrap_attempted {
                    bootstrap_attempted = true;
                    authorization = self.credentials.authorization(
                        &self.config.id,
                        &self.config.auth_scope,
                        &request_origin(&current_url),
                    )?;
                    if authorization.is_some() {
                        self.context
                            .observer
                            .emit(SourceEvent::SourceRetryScheduled {
                                request_id: id,
                                reason: "authentication-bootstrap".into(),
                                attempt,
                                delay_ms: 0,
                            });
                        continue;
                    }
                    return Err(authentication_unavailable(&self.config, &current_url));
                }
                if matches!(status, 401 | 403) && authorization.is_some() {
                    return Err(FetchError::AuthenticationRejected(format!(
                        "registry `{}` at `{}` rejected its scoped credential",
                        self.config.id,
                        request_origin(&current_url)
                    )));
                }
                let retryable = status == 408 || status == 429 || status >= 500;
                if retryable && attempt < 3 {
                    let delay_ms = u64::from(attempt) * 20 + id % 17;
                    self.context
                        .observer
                        .emit(SourceEvent::SourceRetryScheduled {
                            request_id: id,
                            reason: format!("http-{}xx", status / 100),
                            attempt,
                            delay_ms,
                        });
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    continue;
                }
                if !(200..300).contains(&status) {
                    return Err(FetchError::Metadata(format!(
                        "HTTP status class {}",
                        status / 100
                    )));
                }
                if response.body.len() as u64 > max_bytes {
                    return Err(FetchError::Integrity(
                        "HTTP response exceeds acquisition limit".into(),
                    ));
                }
                self.context
                    .observer
                    .emit(SourceEvent::NetworkRequestFinished {
                        request_id: id,
                        status: format!("{}xx", status / 100),
                        received_bytes: response.body.len() as u64,
                        validator_used: validator.is_some(),
                    });
                return Ok(FetchedResponse {
                    body: response.body,
                    headers: response.headers,
                    not_modified: false,
                });
            }
        }
        Err(FetchError::Metadata("HTTP retry budget exhausted".into()))
    }

    fn store_body(
        &self,
        dir: &Path,
        meta: &Path,
        body: &[u8],
        etag: Option<&str>,
        modified: Option<&str>,
    ) -> Result<(), FetchError> {
        fs::create_dir_all(dir)?;
        let body_sha256 = hex::encode(Sha256::digest(body));
        atomic_write(&dir.join(format!("{body_sha256}.body")), body)?;
        atomic_write(
            meta,
            &serde_json::to_vec(&Cached {
                etag: etag.map(str::to_owned),
                last_modified: if etag.is_none() {
                    modified.map(str::to_owned)
                } else {
                    None
                },
                body_sha256,
            })?,
        )
    }
    fn blob_path(&self, digest: &str) -> PathBuf {
        self.context
            .cache
            .join("v1/blobs/sha256")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(digest)
    }
}

impl<C: CredentialPort> squish_resolver::RegistryPort for SparseRegistry<C> {
    fn candidates(
        &self,
        _: &str,
        package: &str,
        access: squish_resolver::Access,
    ) -> Result<Vec<squish_resolver::RegistryCandidate>, squish_resolver::SourceUnavailable> {
        let access = if access == squish_resolver::Access::Online {
            Access::Online
        } else {
            Access::LocalOnly
        };
        SparseRegistry::candidates(self, package, access)
            .map(|rows| {
                rows.into_iter()
                    .filter(|r| !r.yanked)
                    .map(|r| squish_resolver::RegistryCandidate {
                        version: r.version,
                        checksum: format!("sha256:{}", r.archive.digest.0.0),
                        manifest: r.manifest,
                    })
                    .collect()
            })
            .map_err(|e| squish_resolver::SourceUnavailable {
                identity: self.config.id.clone(),
                detail: e.to_string(),
            })
    }
    fn contains(&self, _: &str, _: &str, _: &Version, checksum: &str) -> bool {
        let Some(archive) = checksum.strip_prefix("sha256:") else {
            return false;
        };
        let Ok(content) = fs::read_to_string(
            self.context
                .cache
                .join("v1/registry-materialized")
                .join(archive),
        ) else {
            return false;
        };
        let Ok(digest) = Sha256Digest::parse(content) else {
            return false;
        };
        Materializer::new(self.context.clone()).contains_complete(&ContentDigest(digest))
    }
}

/// Cargo-compatible lowercase shard path. / Cargo 兼容的小写分片路径。
pub fn shard_path(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.len() {
        1 => format!("1/{n}"),
        2 => format!("2/{n}"),
        3 => format!("3/{}/{n}", &n[..1]),
        _ => format!("{}/{}/{n}", &n[..2], &n[2..4]),
    }
}
fn validate_package_name(name: &str) -> Result<(), FetchError> {
    if !(1..=64).contains(&name.len())
        || !name.as_bytes()[0].is_ascii_alphabetic()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(FetchError::Config("invalid package name".into()));
    }
    Ok(())
}
fn validate_registry_config(c: &RegistryConfig) -> Result<(), FetchError> {
    if c.auth_scope.is_empty() {
        return Err(FetchError::Config(
            "registry auth scope must not be empty".into(),
        ));
    }
    let id = Url::parse(&c.id).map_err(|e| FetchError::Config(e.to_string()))?;
    if id.scheme() != "https"
        || id.username() != ""
        || id.password().is_some()
        || id.query().is_some()
        || id.fragment().is_some()
    {
        return Err(FetchError::Config(
            "registry id must be a credential-free absolute HTTPS URI".into(),
        ));
    }
    let valid_index = c.index.ends_with('/')
        && (c.index.starts_with("sparse+https://")
            || c.index.starts_with("sparse+file://")
            || (cfg!(test) && c.index.starts_with("sparse+http://")));
    if !valid_index {
        return Err(FetchError::Config(
            "index must be sparse+https (or sparse+file for local tests) and end in /".into(),
        ));
    }
    let u = Url::parse(c.index.strip_prefix("sparse+").unwrap())
        .map_err(|e| FetchError::Config(e.to_string()))?;
    if u.username() != "" || u.password().is_some() || u.query().is_some() || u.fragment().is_some()
    {
        return Err(FetchError::Config(
            "index endpoint contains forbidden URL components".into(),
        ));
    }
    Ok(())
}
fn validate_download_template(s: &str) -> Result<(), FetchError> {
    let probe = s
        .replace("{package}", "p")
        .replace("{version}", "1.0.0")
        .replace("{prefix}", "p")
        .replace("{lowerprefix}", "p")
        .replace("{archive-sha256}", &"a".repeat(64));
    let u = Url::parse(&probe).map_err(|e| FetchError::Config(e.to_string()))?;
    if u.scheme() != "https"
        || u.username() != ""
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(FetchError::Config("invalid download template".into()));
    }
    Ok(())
}

fn validate_wire_config(bytes: &[u8], registry_id: &str) -> Result<WireConfig, FetchError> {
    let cfg: WireConfig = serde_json::from_slice(bytes)?;
    if cfg.v != 1 {
        return Err(FetchError::Metadata(format!(
            "unsupported registry config version {}",
            cfg.v
        )));
    }
    if cfg.registry_id != registry_id {
        return Err(FetchError::Config("registry identity mismatch".into()));
    }
    validate_download_template(&cfg.dl)?;
    Ok(cfg)
}

fn validate_metadata_body(
    bytes: &[u8],
    package: &str,
    registry_id: &str,
    limits: &crate::Limits,
) -> Result<(), FetchError> {
    if bytes.len() as u64 > limits.max_metadata_bytes {
        return Err(FetchError::Metadata(
            "metadata exceeds configured limit".into(),
        ));
    }
    let mut versions = BTreeSet::new();
    let mut supported = false;
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        if line.len() > limits.max_metadata_line {
            return Err(FetchError::Metadata("metadata line exceeds limit".into()));
        }
        let value: serde_json::Value = serde_json::from_slice(line)?;
        if value.get("v").and_then(|v| v.as_u64()) != Some(1) {
            continue;
        }
        supported = true;
        let row: Row = serde_json::from_value(value)?;
        if !row.name.eq_ignore_ascii_case(package)
            || row.vers.build != semver::BuildMetadata::EMPTY
            || !versions.insert(row.vers.clone())
        {
            return Err(FetchError::Metadata(
                "row package/version identity invalid or duplicated".into(),
            ));
        }
        if row.archive.format != "xspkg-tar-gzip/1" {
            return Err(FetchError::Metadata("unsupported archive format".into()));
        }
        Sha256Digest::parse(row.archive.sha256)?;
        Sha256Digest::parse(row.archive.content_sha256)?;
        Sha256Digest::parse(row.manifest_sha256)?;
        let dependencies = row
            .deps
            .into_iter()
            .map(|d| {
                (
                    d.alias,
                    squish_project::DependencySpec::Detail(Box::new(
                        squish_project::DependencyDetail {
                            version: Some(d.req),
                            registry: d.registry_id.or_else(|| Some(registry_id.to_owned())),
                            package: Some(d.package),
                            optional: d.optional,
                            default_features: d.default_features,
                            features: d.features,
                            ..Default::default()
                        },
                    )),
                )
            })
            .collect();
        squish_project::Manifest {
            manifest_version: 1,
            workspace: None,
            package: Some(squish_project::Package {
                name: row.name,
                version: row.vers,
                dialect: row.package.dialect,
                source_root: row.package.source_root,
            }),
            targets: BTreeMap::new(),
            dependencies,
            exports: BTreeMap::new(),
            profiles: BTreeMap::new(),
        }
        .validate()?;
    }
    if !supported && !bytes.is_empty() {
        return Err(FetchError::Metadata(
            "all package metadata rows use unsupported versions".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct CanonicalDependency {
    alias: String,
    package: String,
    requirement: String,
    registry: String,
    optional: bool,
    default_features: bool,
    features: Vec<String>,
}

fn canonical_dependencies(
    dependencies: &BTreeMap<String, squish_project::DependencySpec>,
    registry_id: &str,
) -> Result<Vec<CanonicalDependency>, FetchError> {
    dependencies
        .iter()
        .map(|(alias, spec)| {
            let (package, requirement, registry, optional, defaults, features) = match spec {
                squish_project::DependencySpec::Version(req) => (
                    alias.clone(),
                    req.to_string(),
                    registry_id.to_owned(),
                    false,
                    true,
                    Vec::new(),
                ),
                squish_project::DependencySpec::Detail(detail) => {
                    if detail.git.is_some() || detail.path.is_some() || detail.workspace {
                        return Err(FetchError::Integrity(
                            "published registry manifest contains a non-registry dependency".into(),
                        ));
                    }
                    let req = detail.version.as_ref().ok_or_else(|| {
                        FetchError::Integrity(
                            "registry dependency has no version requirement".into(),
                        )
                    })?;
                    (
                        detail.package.clone().unwrap_or_else(|| alias.clone()),
                        req.to_string(),
                        detail
                            .registry
                            .clone()
                            .unwrap_or_else(|| registry_id.to_owned()),
                        detail.optional,
                        detail.default_features,
                        detail.features.iter().cloned().collect(),
                    )
                }
            };
            Ok(CanonicalDependency {
                alias: alias.clone(),
                package,
                requirement,
                registry,
                optional,
                default_features: defaults,
                features,
            })
        })
        .collect()
}
fn render_download(
    template: &str,
    package: &str,
    version: &Version,
    digest: &str,
) -> Result<Url, FetchError> {
    let shard = shard_path(package);
    let prefix = shard.rsplit_once('/').map(|v| v.0).unwrap_or("");
    let mut s = template.to_owned();
    let replacements = [
        ("{package}", package.to_owned()),
        ("{version}", version.to_string()),
        ("{prefix}", prefix.to_owned()),
        ("{lowerprefix}", prefix.to_ascii_lowercase()),
        ("{archive-sha256}", digest.to_owned()),
    ];
    let had = replacements.iter().any(|(m, _)| s.contains(m));
    for (m, v) in replacements {
        s = s.replace(m, &percent_path(&v));
    }
    if !had {
        s.push_str(&format!(
            "/{}/{}/download",
            percent_path(package),
            percent_path(&version.to_string())
        ));
    }
    Url::parse(&s).map_err(|e| FetchError::Config(e.to_string()))
}
fn percent_path(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
fn redacted_origin(u: &Url) -> String {
    format!(
        "{}://{}{}{}",
        u.scheme(),
        u.host_str().unwrap_or("invalid"),
        u.port().map(|p| format!(":{p}")).unwrap_or_default(),
        u.path()
    )
}

fn request_origin(u: &Url) -> String {
    format!(
        "{}://{}{}",
        u.scheme(),
        u.host_str().unwrap_or("invalid"),
        u.port().map(|p| format!(":{p}")).unwrap_or_default()
    )
}

fn authentication_unavailable(config: &RegistryConfig, url: &Url) -> FetchError {
    FetchError::AuthenticationUnavailable(format!(
        "registry `{}` requires scope `{}` at `{}`",
        config.id,
        config.auth_scope,
        request_origin(url)
    ))
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FetchError> {
    let parent = path.parent().ok_or_else(|| {
        FetchError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "cache path has no parent",
        ))
    })?;
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("update.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut temporary, bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| FetchError::Io(e.error))?;
    Ok(())
}

fn http_error(error: reqwest::Error) -> FetchError {
    let class = error
        .status()
        .map(|s| format!("status class {}xx", s.as_u16() / 100))
        .unwrap_or_else(|| "transport failure".into());
    FetchError::Http(class)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, FetchError> {
    if fs::metadata(path)?.len() > limit {
        return Err(FetchError::Integrity(
            "cached object exceeds acquisition limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(FetchError::Integrity(
            "cached object exceeds acquisition limit".into(),
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::Mutex;

    struct FakeHttp {
        calls: Mutex<Vec<HttpRequest>>,
    }
    impl HttpTransport for FakeHttp {
        fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError> {
            self.calls.lock().unwrap().push(request);
            Ok(HttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: b"fake".to_vec(),
            })
        }
    }
    struct RedirectHttp {
        calls: std::sync::Arc<Mutex<Vec<HttpRequest>>>,
    }
    struct BootstrapRedirectHttp {
        calls: std::sync::Arc<Mutex<Vec<HttpRequest>>>,
    }
    impl HttpTransport for BootstrapRedirectHttp {
        fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(request);
            match calls.len() {
                1 => Ok(HttpResponse {
                    status: 401,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                }),
                2 => Ok(HttpResponse {
                    status: 302,
                    headers: BTreeMap::from([("location".into(), "https://b.example/file".into())]),
                    body: Vec::new(),
                }),
                _ => Ok(HttpResponse {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: b"done".to_vec(),
                }),
            }
        }
    }
    impl HttpTransport for RedirectHttp {
        fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(request);
            if calls.len() == 1 {
                Ok(HttpResponse {
                    status: 302,
                    headers: BTreeMap::from([("location".into(), "https://b.example/file".into())]),
                    body: Vec::new(),
                })
            } else {
                Ok(HttpResponse {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: b"done".to_vec(),
                })
            }
        }
    }
    struct ScopedCredentials;
    impl CredentialPort for ScopedCredentials {
        fn authorization(
            &self,
            registry_id: &str,
            auth_scope: &str,
            origin: &str,
        ) -> Result<Option<AuthorizationValue>, CredentialError> {
            assert_eq!(registry_id, "https://registry.example/v1");
            assert_eq!(auth_scope, "test-scope");
            Ok(Some(
                AuthorizationValue::new(format!("token-for-{origin}")).unwrap(),
            ))
        }
    }
    #[test]
    fn shard_vectors() {
        assert_eq!(shard_path("A"), "1/a");
        assert_eq!(shard_path("Ab"), "2/ab");
        assert_eq!(shard_path("AbC"), "3/a/abc");
        assert_eq!(shard_path("Common-Prompts"), "co/mm/common-prompts");
    }

    #[test]
    fn invalid_replacement_preserves_last_validated_metadata() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-registry-{}", std::process::id()));
        let index = root.join("index");
        fs::create_dir_all(index.join("de/mo")).unwrap();
        fs::write(index.join("config.json"), r#"{"v":1,"registry-id":"https://registry.example/v1","dl":"https://download.example/{package}/{version}/{archive-sha256}.xspkg"}"#).unwrap();
        let row = r#"{"v":1,"name":"demo","vers":"1.0.0","package":{"dialect":"xmlsquish/1","source-root":"src"},"deps":[],"archive":{"format":"xspkg-tar-gzip/1","size":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","content-sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"manifest-sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","yanked":false}"#;
        fs::write(index.join("de/mo/demo"), row).unwrap();
        let registry = SparseRegistry::new(
            HostContext::new(root.join("cache")).unwrap(),
            RegistryConfig {
                id: "https://registry.example/v1".into(),
                auth_scope: "test".into(),
                index: format!(
                    "sparse+{}",
                    Url::from_directory_path(fs::canonicalize(&index).unwrap()).unwrap()
                ),
            },
        )
        .unwrap();
        assert_eq!(
            registry.candidates("demo", Access::Online).unwrap().len(),
            1
        );
        fs::write(index.join("de/mo/demo"), b"{broken").unwrap();
        assert!(registry.candidates("demo", Access::Online).is_err());
        assert_eq!(
            registry
                .candidates("demo", Access::LocalOnly)
                .unwrap()
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dependency_projection_normalizes_shorthand_and_detail_forms() {
        let req: VersionReq = "^1.2".parse().unwrap();
        let shorthand = BTreeMap::from([(
            "dep".into(),
            squish_project::DependencySpec::Version(req.clone()),
        )]);
        let detailed = BTreeMap::from([(
            "dep".into(),
            squish_project::DependencySpec::Detail(Box::new(squish_project::DependencyDetail {
                version: Some(req),
                package: Some("dep".into()),
                registry: Some("https://registry.example/v1".into()),
                default_features: true,
                ..Default::default()
            })),
        )]);
        assert_eq!(
            canonical_dependencies(&shorthand, "https://registry.example/v1").unwrap(),
            canonical_dependencies(&detailed, "https://registry.example/v1").unwrap()
        );
    }

    #[test]
    fn corrupt_cached_body_forces_unconditional_http_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut conditional = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                conditional.push(
                    String::from_utf8_lossy(&request)
                        .to_ascii_lowercase()
                        .contains("if-none-match:"),
                );
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nETag: \"v1\"\r\nConnection: close\r\n\r\ngood").unwrap();
            }
            conditional
        });
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-http-{}", std::process::id()));
        let registry = SparseRegistry::new(
            HostContext::new(root.clone()).unwrap(),
            RegistryConfig {
                id: "https://registry.example/v1".into(),
                auth_scope: "test".into(),
                index: format!("sparse+http://{address}/"),
            },
        )
        .unwrap();
        assert_eq!(
            registry
                .resource("body", "text/plain", Access::Online, false, |_| Ok(()))
                .unwrap(),
            b"good"
        );
        let bodies = root
            .join("v1/sparse")
            .join(hex::encode(Sha256::digest("https://registry.example/v1")))
            .join("bodies");
        let body = fs::read_dir(&bodies)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "body"))
            .unwrap();
        fs::write(body, b"evil").unwrap();
        assert_eq!(
            registry
                .resource("body", "text/plain", Access::Online, false, |_| Ok(()))
                .unwrap(),
            b"good"
        );
        assert_eq!(server.join().unwrap(), vec![false, false]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn http_content_length_limit_is_checked_before_body_allocation() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-http-limit-{}", std::process::id()));
        let registry = SparseRegistry::new(
            HostContext::new(root.clone()).unwrap(),
            RegistryConfig {
                id: "https://registry.example/v1".into(),
                auth_scope: "test".into(),
                index: format!("sparse+http://{address}/"),
            },
        )
        .unwrap();
        let url = Url::parse(&format!("http://{address}/archive")).unwrap();
        assert!(
            matches!(registry.get_url(&url, "archive", false, None, 16), Err(FetchError::Integrity(message)) if message.contains("exceeds"))
        );
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_sidecar_writers_publish_one_complete_value() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-atomic-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("mapping");
        let writers = [vec![b'a'; 8192], vec![b'b'; 16384]]
            .into_iter()
            .map(|bytes| {
                let path = path.clone();
                std::thread::spawn(move || atomic_write(&path, &bytes).unwrap())
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().unwrap();
        }
        let result = fs::read(&path).unwrap();
        assert!(result == vec![b'a'; 8192] || result == vec![b'b'; 16384]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_http_transport_has_no_hidden_network_client() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-fake-http-{}", std::process::id()));
        let registry = SparseRegistry::with_dependencies(
            HostContext::new(root.clone()).unwrap(),
            RegistryConfig {
                id: "https://registry.example/v1".into(),
                auth_scope: "test".into(),
                index: "sparse+https://unreachable.invalid/".into(),
            },
            NoCredentials,
            Box::new(FakeHttp {
                calls: Mutex::new(Vec::new()),
            }),
        )
        .unwrap();
        assert_eq!(
            registry
                .resource("body", "text/plain", Access::Online, false, |body| {
                    if body == b"fake" {
                        Ok(())
                    } else {
                        Err(FetchError::Metadata("wrong fake".into()))
                    }
                })
                .unwrap(),
            b"fake"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn redirect_credentials_are_rescoped_by_fetch_layer() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-redirect-http-{}", std::process::id()));
        let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
        let registry = SparseRegistry::with_dependencies(
            HostContext::new(root.clone()).unwrap(),
            RegistryConfig {
                id: "https://registry.example/v1".into(),
                auth_scope: "test-scope".into(),
                index: "sparse+https://a.example/".into(),
            },
            ScopedCredentials,
            Box::new(RedirectHttp {
                calls: calls.clone(),
            }),
        )
        .unwrap();
        let result = registry
            .get_url(
                &Url::parse("https://a.example/start").unwrap(),
                "test",
                true,
                None,
                16,
            )
            .unwrap()
            .body;
        assert_eq!(result, b"done");
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0].headers.get("authorization").unwrap(),
            "token-for-https://a.example"
        );
        assert_eq!(
            calls[1].headers.get("authorization").unwrap(),
            "token-for-https://b.example"
        );
        drop(calls);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bootstrap_then_redirect_relooks_up_the_new_origin() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-bootstrap-redirect-{}", std::process::id()));
        let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
        let registry = SparseRegistry::with_dependencies(
            HostContext::new(root.clone()).unwrap(),
            RegistryConfig {
                id: "https://registry.example/v1".into(),
                auth_scope: "test-scope".into(),
                index: "sparse+https://a.example/".into(),
            },
            ScopedCredentials,
            Box::new(BootstrapRedirectHttp {
                calls: calls.clone(),
            }),
        )
        .unwrap();
        assert_eq!(
            registry
                .get_url(
                    &Url::parse("https://a.example/start").unwrap(),
                    "test",
                    false,
                    None,
                    16,
                )
                .unwrap()
                .body,
            b"done"
        );
        let calls = calls.lock().unwrap();
        assert!(!calls[0].headers.contains_key("authorization"));
        assert_eq!(
            calls[1].headers.get("authorization").unwrap(),
            "token-for-https://a.example"
        );
        assert_eq!(
            calls[2].headers.get("authorization").unwrap(),
            "token-for-https://b.example"
        );
        drop(calls);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authorization_values_are_validated_and_always_redacted() {
        let secret = AuthorizationValue::new("Bearer distinctive-secret".into()).unwrap();
        assert_eq!(secret.expose_for_http(), "Bearer distinctive-secret");
        assert_eq!(format!("{secret}"), "<redacted>");
        assert_eq!(format!("{secret:?}"), "AuthorizationValue(<redacted>)");
        assert!(AuthorizationValue::new(String::new()).is_err());
        let error = AuthorizationValue::new("Bearer secret\nInjected: yes".into()).unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }

    struct FixedStatus(u16);
    impl HttpTransport for FixedStatus {
        fn execute(&self, _: HttpRequest) -> Result<HttpResponse, FetchError> {
            Ok(HttpResponse {
                status: self.0,
                headers: BTreeMap::new(),
                body: Vec::new(),
            })
        }
    }

    #[test]
    fn missing_and_rejected_authentication_are_distinct() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-auth-errors-{}", std::process::id()));
        let config = RegistryConfig {
            id: "https://registry.example/v1".into(),
            auth_scope: "test-scope".into(),
            index: "sparse+https://a.example/".into(),
        };
        let missing = SparseRegistry::with_dependencies(
            HostContext::new(root.join("missing")).unwrap(),
            config.clone(),
            NoCredentials,
            Box::new(FixedStatus(401)),
        )
        .unwrap()
        .get_url(
            &Url::parse("https://a.example/private").unwrap(),
            "test",
            false,
            None,
            16,
        )
        .err()
        .expect("missing credential must fail");
        assert!(matches!(missing, FetchError::AuthenticationUnavailable(_)));

        let rejected = SparseRegistry::with_dependencies(
            HostContext::new(root.join("rejected")).unwrap(),
            config,
            ScopedCredentials,
            Box::new(FixedStatus(403)),
        )
        .unwrap()
        .get_url(
            &Url::parse("https://a.example/private").unwrap(),
            "test",
            true,
            None,
            16,
        )
        .err()
        .expect("rejected credential must fail");
        assert!(matches!(rejected, FetchError::AuthenticationRejected(_)));
        fs::remove_dir_all(root).unwrap();
    }
}
