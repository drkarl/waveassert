# WaveAssert implementation roadmap

| | |
|---|---|
| Project | WaveAssert: snapshot regression testing for rendered audio |
| Source of truth | [docs/architecture.md](architecture.md). This roadmap sequences that design. It does not change it |
| Document status | Draft for review. Nothing described here is implemented yet |
| Planning horizon | Four milestones, from an empty repository to a release-ready 0.1 |

Section references such as §7.1 point into the architecture document.

## 1. Summary

| Milestone | Delivers | Issues | Estimate (engineer-hours) |
|---|---|---|---|
| **M1. Tolerance comparator and `diff`** | `waveassert diff a.wav b.wav` and `waveassert::compare` with tolerances, windows, human and JSON reports, CI on three platforms | 12 | 73 – 111 |
| **M2. Snapshot workflow** | `assert_audio_snapshot!`, update modes, candidates, `review` / `accept` / `reject` | 6 | 38 – 57 |
| **M3. Diagnosis and permitted compensation** | Bounded lag and gain estimation, the shift-edge rule, `PassWithCompensation`, CLI flags | 7 | 50 – 74 |
| **M4. Documentation and release readiness** | Tolerance guide, worked example, benchmarks, licence, semver checks, publish dry run | 3 | 20 – 31 |
| **Total** | | **28** | **181 – 273** |

Estimates are focused engineering hours for one engineer who knows Rust and
basic DSP. They include the tests and documentation each issue requires and
the engineer's own review fixes. They exclude waiting for CI queues, external
decisions and calendar gaps. Every range is a plausible low and high, not a
commitment. The most likely total is near the middle, about 225 hours. At
about 25 focused hours a week that is roughly nine calendar weeks for one
engineer.

**Usable early.** After M1 the project is already a complete tolerance-based
comparator, as both the library and a command-line tool. The architecture
makes the same point in §14 and Q1: the diagnosis layer (M3) is a removable
module that must earn its place. M2 and M3 are independent of each other apart
from one notice in the snapshot assertion (issue 23), so with two engineers
they can run in parallel.

### Deliberate deviation from §14

§14 lists the CLI as one step after snapshots. This roadmap splits it:
`waveassert diff` ships in M1, because it makes the comparator usable on its
own and exercises the report and exit-code design early. `review`, `accept`
and `reject` arrive in M2 together with the store they operate on. The final
deliverables are the same as in §14.

## 2. How the work is ordered

```text
M1  01 workspace+CI ─ 02 errors,dB ─ 03 AudioBuffer ─┬─ 04 WAV read ─┬─ 05 WAV write
                                                      │               └─ 12 fuzz target
                                       06 Policy ─────┤
                                                      └─ 07 analysis ─ 08 compare+Report ─ 09 Display+JSON ─ 10 CLI diff ─ 11 fixtures
M2  13 names+UpdateMode ─ 14 SnapshotStore ─┬─ 15 macro+duplicates ─┬─ 17 workflow tests
                                            ├─ 16 accept/reject+CLI ┘
                                            └─ 18 integer baselines
M3  19 shifted ranges ─┬─ 20 lag estimate ──┐
                       └─ 21 gain, polarity ┴─ 22 diagnosis ─ 23 compensation ─ 24 CLI flags ─ 25 lag fixtures
M4  26 docs+example ─ 27 benchmarks ─ 28 release readiness
```

Critical path: 01 → 02 → 03 → 07 → 08 → 09 → 10 for M1, then
13 → 14 → 15 → 16 → 17 for M2, then 19 → 20 → 22 → 23 → 24 → 25 for M3.
The full dependency list is in the tables below and in the front matter of each
issue draft.

## 3. Cross-cutting quality gates

These apply to every pull request from M1 onward (§13.5). Issue 01 sets them up.

| Gate | Command or rule |
|---|---|
| Format | `cargo fmt --all --check` |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Tests | `cargo test --workspace --all-features` and `cargo test --workspace --no-default-features` |
| Docs | `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"` |
| Supply chain | `cargo deny check` (advisories, licences, bans, sources) |
| Platforms | Linux, macOS and Windows on stable, plus Linux on the MSRV (1.85) |
| Code rules | `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, and in the library `clippy::unwrap_used`, `expect_used` and `panic` denied outside tests |
| Review | Behaviour changes that can flip a verdict get a changelog entry (§8.3) |

Project conventions fixed in M1: edition 2024, `rust-version = "1.85"`,
resolver 3 (so dependency resolution respects the MSRV), and a `CHANGELOG.md`
with an "Unreleased" section.

## 4. Milestones

### 4.1 M1. Tolerance comparator and `diff` (73 – 111 h)

**Goal.** Compare two audio buffers or two WAV files under explicit tolerances
and get a clear verdict, report and exit code. No snapshots, no diagnosis.

| # | Task | Depends on | Estimate |
|---|---|---|---|
| 01 | Workspace, lint configuration and CI gates | none | 5 – 8 h |
| 02 | Error type and dB helpers | 01 | 3 – 5 h |
| 03 | `AudioBuffer` and sample-format conversion | 01, 02 | 6 – 9 h |
| 04 | WAV reader with resource limits | 03 | 8 – 12 h |
| 05 | WAV writer | 03, 04 | 5 – 8 h |
| 06 | `Policy` and validation (comparison subset) | 02 | 6 – 9 h |
| 07 | Analysis kernels: levels, residual, windows, first divergence | 03 | 8 – 12 h |
| 08 | `compare`, structural checks, tolerance evaluation, `Report` types | 06, 07 | 10 – 14 h |
| 09 | Human summary and JSON report (schema version 1) | 08 | 8 – 12 h |
| 10 | CLI `diff` with exit codes | 04, 08, 09 | 6 – 9 h |
| 11 | Comparator fixture and property test suite | 05, 08, 09 | 5 – 8 h |
| 12 | Fuzz target for WAV reading | 04 | 3 – 5 h |

Design points fixed here so later milestones do not break the API:

- `Verdict` has all three variants (`Pass`, `PassWithCompensation`, `Fail`)
  and `Report` already carries `applied`, `diagnosis` and `notes`. In M1,
  `applied` is always the identity, `diagnosis` is `None`, and
  `PassWithCompensation` is unreachable. A test asserts that. JSON schema
  version 1 is therefore complete from the first release and M3 only fills
  fields in.
- All structs and enums that can grow are `#[non_exhaustive]`.
- Plain comparisons use the documented zero-padding rule (§7.1) from the start.

**Quality gates.** The cross-cutting gates, plus: every rule of §4.3 has a
`Policy::validate` test; the WAV malformed-input table of §13.3 passes; the
identity, symmetry, coverage, monotonic-tolerance and round-trip properties of
§13.2 pass; `insta` snapshots cover the human summary and JSON of each
non-lag fixture; CLI exit codes 0, 1 and 2 are tested with `assert_cmd`.

**Exit criteria.** `cargo install --path crates/waveassert-cli` produces a
`waveassert diff` that works on 16/24/32-bit integer and float WAV files; CI is
green on all three platforms and the MSRV; the fuzz target has run locally
without findings for at least 10 minutes.

**Checkpoint inside M1.** After issue 08 the library comparator is usable from
Rust code. Issues 09 to 12 make it pleasant and safe to use.

### 4.2 M2. Snapshot workflow (38 – 57 h)

**Goal.** The `insta`-like loop for audio: first run fails and writes a
candidate, the developer reviews and accepts it, later runs compare against the
committed baseline. Updates are always deliberate.

| # | Task | Depends on | Estimate |
|---|---|---|---|
| 13 | Snapshot names, paths and `UpdateMode` | 02 | 4 – 6 h |
| 14 | `SnapshotStore::check`: outcomes, candidates, atomic writes | 04, 05, 08, 13 | 10 – 14 h |
| 15 | `assert_audio_snapshot!` and duplicate-name detection | 14 | 6 – 9 h |
| 16 | `accept` / `reject` API and CLI `review`, `accept`, `reject` | 10, 14 | 8 – 12 h |
| 17 | Snapshot workflow integration tests | 15, 16 | 5 – 8 h |
| 18 | Configurable integer baselines with quantization check | 05, 14 | 5 – 8 h |

**Quality gates.** The cross-cutting gates, plus the full mode table of §4.5
(missing baseline, mismatch and match under `no`, `new` and `always`),
`CI=true` defaulting to `no`, explicit `WAVEASSERT_UPDATE` winning, invalid
values being an error, atomic replacement tested by injecting a failure between
write and rename, name-traversal tests (including Windows-style separators),
and symlink refusal tested on Unix (skipped with a visible reason where the
platform cannot create symlinks).

**Exit criteria.** The end-to-end workflow of §13.4 passes on all three
platforms. A new project can add one snapshot test, run it, run
`waveassert review`, `waveassert accept`, and see it pass, using only the
README. Issue 18 may slip to M4 without blocking the milestone, because 32-bit
float baselines are the default (Q4).

### 4.3 M3. Diagnosis and permitted compensation (50 – 74 h)

**Goal.** Explain failures (latency, gain, polarity) and, only when the policy
explicitly permits it, accept them with a distinct verdict. This is where the
corrected overlap semantics live (§7.1, F16): plain comparisons keep zero
padding; shifted comparisons exclude exactly the shift-exposed frames; every
excluded frame is counted and reported; compensation never produces a plain
`Pass`.

| # | Task | Depends on | Estimate |
|---|---|---|---|
| 19 | Shifted alignment ranges and the edge rule | 07 | 8 – 12 h |
| 20 | Bounded integer lag estimation | 19 | 8 – 12 h |
| 21 | Gain and polarity estimation | 19 | 4 – 6 h |
| 22 | Diagnosis classification and what-if passes | 08, 09, 20, 21 | 10 – 14 h |
| 23 | Opt-in compensation and `PassWithCompensation` | 15, 22 | 10 – 15 h |
| 24 | CLI flags for search, compensation and diagnosis | 10, 22, 23 | 4 – 6 h |
| 25 | Diagnosis and compensation fixture suite | 15, 23, 24 | 6 – 9 h |

**Decision gate G3 (after issue 22).** §16 Q1 asks whether diagnosis adds
enough value over plain tolerances. Before starting issue 23, run the
diagnosis on the M1 fixtures and on at least two realistic golden-test
scenarios (for example a reverb tail and a decoder output, rendered with a
deliberate one-frame delay and a 0.1 dB gain change) and record in the pull
request whether the explanation is correct and useful. If it is not, stop M3
after issue 22 and keep diagnosis as an optional, documented module. This is
the only point in the roadmap with a planned go or no-go.

**Quality gates.** The cross-cutting gates, plus all properties in §13.2 that
concern M3: edge rule (checked against an independent, deliberately naive
pairing implementation), lag recovery, gain recovery, diagnosis independence,
compensation bound and no silent compensated pass. All lag fixtures of §13.3
pass, including the delayed render with a truncated tail, which must be
`Unexplained`, and the bit-exact tone with compensation permitted, which must
be `Pass` with identity `applied`.

**Exit criteria.** Every fixture of §13.3 passes on three platforms. The
summary, JSON and snapshot assertion state the compensated lag, gain and
excluded frames. Changing `diagnose` or `lag_search` never changes a verdict.

### 4.4 M4. Documentation and release readiness (20 – 31 h)

| # | Task | Depends on | Estimate |
|---|---|---|---|
| 26 | Crate docs, tolerance-selection guide and runnable example | 25 | 10 – 15 h |
| 27 | Criterion benchmarks and the debug-build recommendation | 07, 20 | 4 – 6 h |
| 28 | Release readiness: licence, metadata, semver checks, dry run | 12, 26, 27 | 6 – 10 h |

**Quality gates.** The cross-cutting gates, plus: doctests for the worked
example; `cargo publish --dry-run` for both crates; `cargo semver-checks` on
the library against its first baseline; the README status table matches
reality.

**Exit criteria.** A tagged 0.1 candidate that passes every gate. Publishing
to crates.io and creating releases are outside this roadmap and need the
owner's explicit action.

## 5. Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | `hound` behaves differently from the design assumptions: extensible headers, 8-bit data, 64-bit float, header sizes that lie, error variants that do not map cleanly to `UnsupportedFormat` | Medium | Medium | Issue 04 starts with a small matrix of hand-built files before writing any wrapper. The `hound` type stays out of the public API, so a thin in-house header validator, or a different decoder, can replace it without a breaking change |
| R2 | Platform differences in `sin`, `log10` and `powf` make generated fixtures or `insta` snapshots differ by one unit in the last place across Linux, macOS and Windows | Medium | Medium | Verdicts use linear comparisons (§7.3). Report text rounds dB to 0.1 and JSON to 0.001. Test tones come from the pure-Rust `libm` crate as a dev-dependency, and fixtures are derived from a single generated array so shifted copies are exactly equal. See also the note in section 7 |
| R3 | Off-by-one errors in the shift-edge rule, especially for negative lags and unequal lengths | High | High | Issue 19 isolates the rule in one function with a naive pairing implementation as an oracle for property tests, and issue 25 pins the documented examples |
| R4 | Compensation drifts towards silent acceptance (for example by letting diagnosis settings influence the verdict) | Low | High | Verdict is computed in a separate pass from diagnosis. Property tests for diagnosis independence, compensation bound and no silent compensated pass are required by issues 22 and 23 |
| R5 | Direct time-domain lag search is slow at the upper limits. At `K = 4096`, 2^20 frames and 2 channels it is about 1.7 × 10^10 multiply-adds, which is seconds in a release build and many minutes in an unoptimized test build | Medium | Medium | The default `lag_search` is 0, estimation runs only on failure, and `lag_analysis_max_frames` bounds it. Issue 27 measures real cost, and issue 26 documents the `opt-level` recommendation. FFT correlation stays deferred until a user needs it |
| R6 | Windows rename-over-existing and virus scanners make atomic writes fail intermittently | Low | Medium | `tempfile::persist` plus a small bounded retry limited to Windows sharing violations, covered by a stress test in issue 17 |
| R7 | Snapshot tests that run in parallel collide on candidate files | Low | Medium | Names are unique per store, writes are atomic, the in-process registry catches duplicates, and the nextest limitation is documented (Q9) |
| R8 | A newer dependency raises its own MSRV above 1.85 | Medium | Low | Resolver 3 with `rust-version` set, an MSRV job in CI, and pinned `Cargo.lock` for the CLI |
| R9 | Diagnosis proves less useful than hoped (Q1) | Medium | Low | Gate G3. M1 and M2 stand alone as a useful product |
| R10 | Scope growth from the deferred list in section 6 | Medium | Medium | Deferred items are not issues. Each has a stated trigger, and none is scheduled before 0.1 |

Items that need a decision before they bite are collected in section 7.

## 6. Deferred work (not part of the committed roadmap)

These items come from §15.1. They have no issues and no estimates here. The
trigger says what evidence would justify planning them.

| Item | Trigger to plan it |
|---|---|
| Band-limited (spectral) residuals, `AddedNoise` and `SpectralChange` explanations | Users report many `Unexplained` failures that are in fact filter or noise changes |
| FFT-based lag search | A user needs `K` above 4096 or analysis windows where the direct method is too slow (R5) |
| Sub-sample lag estimation | Fractional-delay processors produce `Unexplained` failures |
| More input formats (FLAC, AIFF) | Requests from codec projects; needs a licence review of `symphonia` |
| Per-platform baseline variants | A real case where tolerances cannot absorb cross-platform differences (Q5) |
| Render-twice nondeterminism check | Requests; independent of the rest |
| HTML visual reports | CI users ask for artifacts they can open |
| Framework adapters (nih-plug, fundsp, CLAP test hosts) | After a stable 0.x API; separate crates |
| Policy files (`--policy policy.toml`) | CLI users with many repeated flags |
| Interactive review terminal UI | After the non-interactive commands prove too limited |
| Parallel analysis | Long multichannel renders where analysis dominates |
| MIDI-driven fixtures | Belongs in adapters, not the core |

## 7. Open points and notes

**Needs a decision (owner).**

1. **Licence.** The README says none is selected. Needed before issue 28. The
   recommended default is dual MIT or Apache-2.0, matching the Rust ecosystem
   and the dependencies listed in §12. Issue 01 configures `cargo deny` so
   workspace crates are not licence-checked until then.
2. **Crate names.** Confirm `waveassert` and `waveassert-cli` are free on
   crates.io before issue 28 (the design assumes they are).

**Clarifications of the architecture found while planning.** None of these
change the design. Each is resolved in the issue named, and mirrored back into
the architecture document by that issue.

| Point | Resolution |
|---|---|
| §7.2 defines the compared-frame count for shifted comparisons only as "the range of §7.1 minus shift-exposed frames" | Issue 19 states the exact range and denominator in code documentation and adds a worked example to §7.1 |
| §4.5 says mode `no` "writes nothing", and also says a matching run removes stale candidates | Issue 14 implements removal of stale candidates on a match in every mode, and documents that this is the only file change `no` makes |
| The README lists 16/24/32-bit integer PCM and omits 8-bit, while F2 and §4.2 include 8-bit | Issue 04 supports 8-bit as specified in F2, and issue 26 corrects the README wording |
| Overlap-energy normalization in the lag estimator (§7.4) can favour lags with a tiny overlap when `K` approaches the signal length | Issue 20 ignores candidate lags whose overlap is under half of the shorter analysed input, and documents it as the default. The default is owner-approved |
| Test tones need a deterministic sine across platforms (R2) | `libm` is added as a dev-dependency only. Nothing changes at runtime |
| The Report invariants in §4.4 are numbered 1, 2, 3, 6, 4, 5 | Cosmetic. Issue 26 renumbers them |

**Estimate basis.** Per-issue ranges were built from the size of the module
described in the architecture, the number of test cases the architecture
requires for it, and a margin for unfamiliar APIs (mostly `hound`,
`tempfile::persist` on Windows, and `insta` workflow). M1 is the largest
milestone because it also carries workspace, CI and three-platform setup. M3
has the widest range because the edge rule and its oracle tests are the most
likely place to find surprises.
