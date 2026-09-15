# squish-store implementation notes

## Windows `ERROR_NOT_SAME_DEVICE` during CAS publication (MGB042)

### Observed evidence

On 2026-09-15, the production documentation build used
`C:\Users\STONE\AppData\Roaming\xmlsquish\state\cas` and repeatedly surfaced Win32
error 17 (`ERROR_NOT_SAME_DEVICE`) through `CasError::Io`. The compile path reaches
`Cas::put`; catalog persistence reaches `BlobStore::write_from`, which delegates to
`Cas::put_reader`. Both paths reproduced the same failure.

The CAS root, staging directory, and digest buckets all had the Windows `Encrypted`
attribute. They were ordinary paths on `C:`, not links or junctions. Controlled
probes established:

- .NET `File.Move` successfully moved encrypted files both across those directories
  and between siblings in an actual bucket;
- Rust `fs::rename` failed during real CAS calls. Rust implements it on Windows with
  `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`;
- `tempfile::persist_noclobber` (`MoveFileExW`, flags zero) and an NTFS hard-link
  fallback were tried in the exact product invocation and still returned error 17;
- the project being on `D:` is not causal because both CAS operands are on `C:`;
- the successful .NET control uses `MoveFileW`, identifying the required no-replace
  Windows primitive under this EFS environment.

`put_reader` additionally starts in the algorithm parent because the digest bucket
is unknown until EOF. `Cas::put` already creates a sibling temporary, proving that a
bucket-local copy followed by another Rust `fs::rename` is not a sufficient Windows
fix.

### Production behavior

Both writers first use the portable fast path. Only for Windows error 17 they call a
narrow internal `MoveFileW` wrapper, preserving atomic no-replace publication without
swallowing other errors. The wrapper rejects interior NULs, keeps its NUL-terminated
UTF-16 allocations alive for the call, and is the crate's sole scoped unsafe
allowance. If a concurrent writer already published the digest, its complete bytes
are verified and reused. An invalid existing destination is removed and publication
is retried once; other Win32 failures remain observable.

On non-Windows systems, a genuine `CrossesDevices` result in the streaming path uses
a bounded copy-and-hash into a synced bucket-local temporary followed by the normal
atomic rename.

### Reproduction seam

`Cas::publish_temporary_with` accepts private rename and cross-device-classification
adapters. The unit test injects the first-rename failure deterministically, then uses
the platform fallback and verifies source cleanup, byte/digest integrity, and one
observable `Written` event. The exact EFS product command remains a manual environment
test because CI cannot assume an EFS key.
