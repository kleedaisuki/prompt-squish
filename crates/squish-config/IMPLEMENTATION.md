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

## Compatibility notes

The schema is deliberately closed: misspellings cannot silently become inert.
Adding a key is consequently an explicit schema evolution. Registry aliases are
case-folded for collision detection while their declared spelling is preserved.
Paths from CLI overrides use the caller's current-base marker (`.`); callers that
need another base should make paths absolute before injection.
