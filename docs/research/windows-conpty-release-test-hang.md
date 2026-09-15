# Windows x86_64 Release-Test Hang: ConPTY Shutdown Ownership

Status: diagnosed, mitigated, and remotely verified on 2026-09-15.

## Decision context

The `v1.0.0` release matrix twice stopped making progress in the Windows x86_64
native release-test step. The command was:

```text
cargo +1.88.0 test --release --all-features --locked --target x86_64-pc-windows-msvc
```

The release must retain the complete native test suite. Removing the PTY integration
tests, weakening them to pipe-based tests, or publishing an untested binary were not
acceptable mitigations.

## Observations

### GitHub Actions

In run `34980935644`, job `104420826736` completed all 12 root unit tests and all
32 process integration tests. Its last result was:

```text
test result: ok. 32 passed; 0 failed; ... finished in 2.99s
```

Cargo runs `tests/pty.rs` immediately after `tests/process.rs`. The job produced no
further test result and was cancelled after the release step had spent 16m43s without
progress. Run `34982186340`, job `104428765951`, repeated the same behavior and reached
the job's 35-minute timeout.

The Windows ARM64 job in the same run executed the identical suite successfully:

| Test binary | Result | Test execution time |
|---|---:|---:|
| root unit tests | 12 passed | 0.11s |
| `process.rs` | 32 passed | 11.70s |
| `pty.rs` | 2 passed | 0.59s |
| `recovery_process.rs` | 8 passed | 2.89s |

### Controlled local reproduction

Environment: Windows 11 build 26200 x86_64, Rust 1.88.0, release profile, static CRT, isolated
`CARGO_TARGET_DIR=.temp/v1-windows-release-hang/target`.

The first single-threaded run compiled and ran the entire suite in 112.041s. A warm,
default-parallelism run completed in 4.251s; `pty.rs` completed in 0.45s. Thus the
product child process and assertions do not intrinsically require minutes, and the
failure depends on the Windows ConPTY implementation used by the runner image.

After replacing the strong writer ownership with a weak reference, ten consecutive
single-threaded release runs of `pty.rs` passed. Their measured wall times, including
Cargo startup, were 2.552s for the first incremental rebuild and 1.092–1.157s for the
nine warm repetitions; test execution itself remained 0.58–0.62s.

## Mechanism

`portable-pty` 0.9.0 creates ConPTY with `PSEUDOCONSOLE_INHERIT_CURSOR`. The test's
reader thread must answer the resulting device-status query and therefore previously
held a strong `Arc` to the PTY input writer for its full lifetime. Shutdown did this:

```text
main thread:   drop writer Arc -> drop master / ClosePseudoConsole -> join reader
reader thread: own writer Arc -> blocking ReadFile until ConPTY output EOF
```

On older ConPTY, a blocking read does not necessarily receive EOF merely because the
client process exited. `ClosePseudoConsole` can itself wait for the host to exit. The
strong writer ownership therefore creates a circular shutdown dependency with no
deadline. Windows build 26100 changed close behavior so `ClosePseudoConsole` returns
without waiting, explaining why the Windows 11 ARM64 runner and a current local Windows
installation pass while Windows Server 2022 hangs.

This interpretation is supported by Microsoft's ConPTY maintainers and sources:

- Microsoft Terminal issue [#17688](https://github.com/microsoft/terminal/issues/17688)
  explains both the older blocking `ClosePseudoConsole` behavior and the build-26100
  change to return without waiting.
- Microsoft Terminal issue [#4564](https://github.com/microsoft/terminal/issues/4564)
  records that a blocking read from the ConPTY output pipe may wait indefinitely after
  all clients terminate.
- Microsoft's [ConPTY session guidance](https://learn.microsoft.com/windows/console/creating-a-pseudoconsole-session)
  requires servicing the communication channels on separate threads and warns that
  incorrect synchronous close ordering can deadlock.
- GitHub's runner-images repository lists `windows-2025` as an available x64 image;
  its image is Windows build 26100.

## Changes

1. The PTY reader now holds a `Weak` reference to the input writer. It upgrades only
   while replying to a live cursor-position query. `finish_output` can therefore close
   ConPTY stdin before dropping the master, breaking the ownership cycle.
2. The release x86_64 Windows job uses the generally available `windows-2025` runner.
   This retains all release tests and uses build-26100 ConPTY shutdown semantics. The
   target remains `x86_64-pc-windows-msvc`; the runner selection does not change the
   Rust target triple or static-CRT packaging contract.

The runner change is necessary for publishing the already immutable `v1.0.0` source
tag, whose historical test harness cannot be edited. The ownership fix protects later
source revisions and enables continued testing on older Windows implementations.

## Falsification and follow-up

The diagnosis should be revised if a Windows Server 2022 trace shows the test process
blocked outside ConPTY close/read paths. A direct stack capture from the hosted runner
was unavailable while the job was in progress.

## Remote verification

[Release run `34987421053`](https://github.com/kleedaisuki/prompt-squish/actions/runs/34987421053)
used automation commit `5c8bf1f3dc65ffff3255e78232d1d8cd638b5869` and completed successfully.
All six build jobs, the complete native test commands, packaging, artifact upload, and
the final publish job passed. In particular, Windows x86_64 job `104443061771` ran on
the `windows-2025` label:

| Step | Started (UTC) | Completed (UTC) | Duration | Result |
|---|---|---|---:|---:|
| locked release build | 15:17:59 | 15:22:09 | 4m10s | passed |
| native release tests | 15:22:09 | 15:24:11 | 2m02s | passed |
| smoke/package | 15:24:11 | 15:24:17 | 6s | passed |

The same test step had previously made no progress for 31m45s on `windows-2022` before
the 35-minute job timeout cancelled it. The controlled runner change therefore removed
the release-blocking hang without removing or filtering a test binary.

The publish job created the non-draft, non-prerelease
[`xmlsquish 1.0.0` release](https://github.com/kleedaisuki/prompt-squish/releases/tag/v1.0.0)
with six platform archives and `SHA256SUMS`; GitHub reported all seven assets as uploaded.
