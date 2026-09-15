# Configuration domain implementation map

This crate implements the configuration boundary described by dependency-source
protocol section 3 and the CLI presentation precedence contract.

## Ownership and data flow

`ConfigLoader` receives a `ConfigHome`, optional workspace root, and ordered CLI
`KEY=VALUE` strings. The host—not this crate—decides whether the home came from
`XMLSQUISH_HOME` or a platform convention and handles any environment/capability
adapter. The load order is compiled defaults, `<home>/config.toml`,
`<workspace>/.xmlsquish/config.toml`, and CLI strings in their supplied order.
There are no process-global or network reads.

`[new] vcs` is a closed, typed policy with the values `"git"` and `"none"`.
It defaults to Git and participates in the same user → workspace → CLI
precedence and provenance chain as every other scalar. The executable host may
translate an environment value or `new --vcs` into a final invocation override;
this pure crate deliberately does not read process environment variables. The
resolved `Config::new.vcs` is therefore the single configuration-domain input
to invocation assembly, avoiding separate stringly-typed defaults.

`PartialConfig` is the deserialization boundary. `validate_keys` first walks the
span-preserving immutable `toml_edit::Document` so key and value byte ranges refer
to the exact original source. `merge` validates and normalizes domain values,
resolves file-relative paths, updates `Config`, and
appends an `ExplainEntry`. `State::finish` validates cross-registry alias and
stable-ID uniqueness. `Provenance` therefore contains a weak-to-strong chain for
every effective scalar value, including defaults and dynamically named registry
leaves.

Registry configuration stores only logical IDs, sparse index locators, and an
opaque non-secret credential lookup scope. Token/password fields are outside the
schema and are rejected as unknown keys; credential acquisition remains an
adapter port concern.

Multiple aliases may intentionally name one stable registry ID. They form one
resolver domain only when their canonical index and authentication scope agree.
Conflicting endpoint/auth definitions are rejected at the later alias's layer;
case-fold-equivalent alias tokens remain ambiguous and are always rejected.

## Production environment credential contract

The first production credential adapter is intentionally smaller than Cargo's
pluggable credential-provider system: it reads an immutable snapshot of process
environment variables and never reads or writes a credential file, invokes a
credential subprocess, prompts, or accepts a token in TOML or on the command
line. This is an ephemeral CI/operator injection mechanism, not a general secret
manager. A future OS keychain or broker can implement the same port without
changing resolution or transport behavior. Cargo's registry-specific environment
lookup is the production precedent for this narrow first step; unlike Cargo's
historical token file, xmlsquish does not add a plaintext persistence fallback
([Cargo registry authentication](https://doc.rust-lang.org/cargo/reference/registry-authentication.html)).

`auth-scope` is the non-secret lookup identity. It is not an alias and it is not
an authorization value. The exact configured string maps to an environment stem
as follows:

1. If it matches `[A-Za-z][A-Za-z0-9_-]{0,63}`, ASCII-uppercase it and replace
   `-` with `_`.
2. Otherwise use `H_` followed by the complete uppercase hexadecimal SHA-256 of
   its UTF-8 bytes.
3. Reject an effective configuration in which two distinct scope strings map to
   the same stem. Reusing the exact same scope is intentional credential sharing.

The readable form preserves existing values such as `corp-read`; the hashed form
keeps the current default (the canonical registry ID) bounded and portable. The
complete digest, rather than a truncated one, keeps naming deterministic without
adding a collision-resolution registry. Environment names are ASCII-uppercase
and are compared ASCII-case-insensitively on every platform. Two supplied names
which fold to the same name are an error, even on a case-sensitive host, so a CI
job behaves the same on Linux, macOS, and Windows.

For the configured sparse-index origin, the authorization header value is read
from:

```text
XMLSQUISH_REGISTRY_<SCOPE_STEM>_AUTHORIZATION
```

For every other origin, including an authenticated download CDN or a
cross-origin redirect, it is read only from:

```text
XMLSQUISH_REGISTRY_<SCOPE_STEM>_ORIGIN_<ORIGIN_SHA256>_AUTHORIZATION
```

`ORIGIN_SHA256` is the complete uppercase hexadecimal SHA-256 of the canonical
ASCII origin `https://host[:non-default-port]`. The value is the exact HTTP
`Authorization` field value; the adapter does not prepend `Bearer`, trim it, or
interpret its scheme. Empty, non-Unicode, or invalid HTTP header values are
configuration errors. Missing variables mean "credential unavailable", not an
empty credential. Diagnostics may name the environment variable and canonical
origin required to recover, but must never include the variable value.

Aliases never participate in lookup. All aliases for one stable registry ID
already must agree on index and exact `auth-scope`; consequently they select one
primary origin and one environment namespace. The stable ID is passed to the
credential port as a consistency key, while `auth-scope` selects the namespace.
The request origin is an authority boundary: the primary variable is usable only
for the configured index origin. A different origin receives a credential only
when its exact origin-specific variable exists. Same-origin redirects reuse the
primary lookup; cross-origin redirects discard the preceding value and perform a
new lookup. This is an application of least privilege rather than trust inferred
from redirect adjacency (Saltzer and Schroeder,
[The Protection of Information in Computer Systems](https://doi.org/10.1109/PROC.1975.9939)).

Example:

```toml
[registries.corp]
id = "https://packages.example.com/xmlsquish"
index = "sparse+https://index.example.com/xmlsquish/"
auth-scope = "corp-read"
```

```text
# index.example.com
XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION

# packages.example.com, if config.json requires authorization there
XMLSQUISH_REGISTRY_CORP_READ_ORIGIN_19E791B81441962D42D1BBF666CA11B91AF5C9D50324D91868D601B31E00FECD_AUTHORIZATION
```

The adapter captures only this prefixed namespace once at process composition
and retains only matching names/values in memory. Its type must not implement a
value-revealing `Debug`, `Display`, serialization, or event conversion. Verbose
and trace modes do not weaken this rule. Environment injection is convenient but
does not provide OS-backed at-rest protection and may be visible to sufficiently
privileged same-host observers; keychain/broker adapters remain a compatible
future extension, not part of the first production contract.

Implementation ownership is deliberately explicit:

- `squish-config` owns and tests the pure `AuthScope::environment_stem()`
  mapping plus cross-registry stem-collision validation. It never reads an
  environment variable.
- `squish-fetch::RegistryConfig` gains `auth_scope: String`.
  `CredentialPort::authorization` gains that scope argument, returns
  `Result<Option<AuthorizationValue>, CredentialError>`, and is invoked again
  for every redirect origin. `AuthorizationValue` exposes its bytes only to the
  HTTP request builder and always formats as `<redacted>`.
- `squish-host` owns `EnvironmentCredentials::from_snapshot(routes, variables)`.
  `routes` contain stable registry ID, auth scope, and canonical primary index
  origin; `variables` are injected `(OsString, OsString)` pairs. The constructor
  filters the `XMLSQUISH_REGISTRY_` namespace and detects folded duplicates.
  It does not call `std::env` itself, preserving the host's no-process-global
  composition contract.
- The executable boundary captures `std::env::vars_os()` once, constructs this
  adapter, and injects it through the existing `HostConfig.credentials` port.
  `NoCredentials` remains the explicit adapter for tests and deployments with
  no private registry.

Focused tests must cover safe and hashed scope vectors, normalized-scope
collisions, exact scope reuse, primary and cross-origin variable names,
case-folded environment duplicates, invalid/non-Unicode header values, alias
invariance, redirect re-lookup, missing versus rejected authentication, and
secret absence from `Debug`, errors, source events, NDJSON, human, verbose, and
trace output. The redirect fixture must use distinguishable primary and
cross-origin values so merely retaining the first header cannot pass.

## Compatibility notes

The schema is deliberately closed: misspellings cannot silently become inert.
Adding a key is consequently an explicit schema evolution. Registry aliases are
case-folded for collision detection while their declared spelling is preserved.
Paths from CLI overrides use the caller's current-base marker (`.`); callers that
need another base should make paths absolute before injection.
