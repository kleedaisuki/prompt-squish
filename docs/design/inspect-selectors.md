# Typed Inspect Selectors

**Status:** Implemented  
**Decision scope:** The boundary between CLI syntax, protocol requests, manager catalog lookup,
and kernel result validation for `xmlsquish inspect`.

## Invariant

An artifact path and an artifact identity are different domain values:

- `InspectView::Artifact(ProjectPath)` is a user-facing path selector.
- `InspectView::Provenance(ArtifactId)` is an identity selector used by typed internal or protocol
  clients that already possess the canonical artifact identity.
- Both produce `InspectResult::Provenance`, but only the identity selector requires the returned
  artifact ID to equal the requested value.

The CLI parses `inspect artifact PATH` exactly once into `ProjectPath`; it must not duplicate the
same bytes into a synthetic `ArtifactId` or an out-of-band manager setting.

## Resolution Path

```text
CLI PATH
  -> InspectView::Artifact(ProjectPath)
  -> project containment validation
  -> authoritative build-catalog path lookup
  -> Artifact { real ArtifactId, digest, kind, ... }
  -> product validation
  -> provenance lookup by real ArtifactId
  -> InspectResult::Provenance
  -> kernel validates the requested result shape
```

This separation prevents a catalog entry whose content identity naturally differs from its
publication path from being rejected as an identity mismatch. It also keeps path containment and
catalog authority in the manager instead of distributing path/ID guesses across the CLI, manager,
and kernel.

## Evidence

- `crates/squish-protocol/src/lib.rs`: typed selector and result/request matching policy.
- `crates/squish-cli/src/lib.rs`: single parse of the artifact path.
- `crates/squish-manager/src/inspect.rs`: path-to-artifact resolution followed by real-ID evidence
  lookup.
- `crates/squish-manager/tests/inspect.rs`: a kernel-dispatched path whose path differs from its
  artifact ID succeeds.

