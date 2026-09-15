# Project storage namespaces

## Decision

The production composition root separates storage by semantic ownership rather than putting every
artifact below one undifferentiated global directory:

| Storage responsibility | Sharing boundary | Reason |
| --- | --- | --- |
| Content-addressed store (CAS) | Global `manager.storage-root` | A digest names immutable bytes, so verified content is safely reusable across projects. |
| Content-keyed action index | Global `manager.storage-root` | A complete action key includes the inputs and operation recipe; project-independent hits are intentional. |
| Published generation | Canonical project root | It materializes user-visible paths below that project's `target/xmlsquish`. |
| Build catalog/current generation | Canonical project identity | Catalog locators are project-relative and therefore have meaning only inside their owning project. |

The catalog path is:

```text
<storage-root>/catalog/projects/<project-identity-digest>/
```

`<project-identity-digest>` is a domain-separated BLAKE3 digest of the already-canonical project
path. Unix hashes the path's raw bytes; Windows hashes its UTF-16 code units in little-endian order.
This representation is deterministic, does not expose local paths in directory names, and does not
silently merge two projects that happen to use the same target name or relative source locator.

## Invariants

1. Opening one project cannot read or advance another project's current build-catalog generation.
2. A project can still reuse verified CAS blobs and complete action results produced by another
   project.
3. Project discovery supplies the canonical identity. Callers must not hash an unresolved relative
   path.
4. Users do not need to assign a different `XMLSQUISH_HOME` or storage root per project.

## Failure that motivated the boundary

With a single global catalog, building project A and then project B under the same default
`XMLSQUISH_HOME` allowed B to load A's catalog. Because catalog source locators are relative, B
resolved A's locator against B's root and failed with `MGB124` instead of compiling B. Treating this
as a missing-file special case would preserve the invalid data model; namespacing the project-owned
state removes the special case.

## Regression evidence

`tests/process.rs::global_storage_isolates_project_catalogs_while_serving_both_projects` builds two
distinct projects consecutively with one default global home, then inspects each project's unique
target. It also checks that one shared CAS and action index coexist with two catalog namespaces.

