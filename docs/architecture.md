# WaveAssert architecture

| | |
|---|---|
| Project | WaveAssert: snapshot regression testing for rendered audio |
| Crate and binary | `waveassert` (library), `waveassert-cli` (package installing the `waveassert` binary). Not yet published on crates.io |
| Document status | Design approved, not implemented. Nothing here is implemented yet |
| Scope of this document | The first usable library and CLI, plus the boundaries of deferred work |

All Rust signatures, command lines and output samples in this document are
**proposed**. They describe the intended design, not existing code.

## Contents

1. [Problem statement and scope](#1-problem-statement-and-scope)
2. [Requirements](#2-requirements)
3. [Public API and CLI design](#3-public-api-and-cli-design)
4. [Core domain model and invariants](#4-core-domain-model-and-invariants)
5. [Modules and responsibility boundaries](#5-modules-and-responsibility-boundaries)
6. [Data flow and processing lifecycle](#6-data-flow-and-processing-lifecycle)
7. [Algorithms and implementation choices](#7-algorithms-and-implementation-choices)
8. [File formats and compatibility](#8-file-formats-and-compatibility)
9. [Errors, diagnostics and reporting](#9-errors-diagnostics-and-reporting)
10. [Performance and resource considerations](#10-performance-and-resource-considerations)
11. [Security and robustness](#11-security-and-robustness)
12. [Dependencies and alternatives](#12-dependencies-and-alternatives)
13. [Testing strategy and quality gates](#13-testing-strategy-and-quality-gates)
14. [Initial usable implementation slice](#14-initial-usable-implementation-slice)
15. [Deferred functionality and non-goals](#15-deferred-functionality-and-non-goals)
16. [Open technical questions](#16-open-technical-questions)

---

## 1. Problem statement and scope

### 1.1 Problem

Audio software is usually regression-tested by rendering a fixed input and
comparing the output with a stored reference. Projects solve this
independently, and the solutions share the same weaknesses:

- **Exact comparison is too strict.** Hashes and byte comparison fail on
  differences far below audibility: dither, changed summation order, fused
  multiply-add on another architecture, or a compiler update.
- **Per-project comparators are reinvented and opaque.** Public examples include
  Surge's golden-render tests
  ([`UnitTestsGOLDEN.cpp`](https://github.com/surge-synthesizer/surge/blob/main/src/surge-testrunner/UnitTestsGOLDEN.cpp))
  and Faust's impulse-response comparison tool
  ([`filesCompare.cpp`](https://github.com/grame-cncm/faust/blob/master/tests/impulse-tests/tools/filesCompare.cpp)).
  Each applies an absolute per-sample tolerance and reports the first
  differing sample. None of them is reusable outside its project.
- **Systematic changes look like random breakage.** A one-frame latency change
  or a small gain change makes almost every sample differ. A per-sample
  comparator reports a large difference but cannot say that a delay or a
  level change explains all of it.
- **Baseline updates are not deliberate.** Regenerating references is often a
  separate script that overwrites files without showing what changed.
  [`insta`](https://insta.rs) solved this for text snapshots with pending
  candidates and an explicit review step. Audio has no equivalent.

Existing tools cover only parts of this. `sndfile-cmp` from
[sndfile-tools](https://github.com/libsndfile/sndfile-tools) compares files
exactly. [pluginval](https://github.com/Tracktion/pluginval) validates plugin
behaviour but does not compare renders with baselines. Plugin-framework test
helpers are tied to their framework.

### 1.2 Scope

A framework-independent Rust library, plus a small CLI, that:

1. Compares a **reference** audio buffer with an **actual** buffer under an
   explicit **policy** of tolerances.
2. Treats sample rate, channel count, length and sample format explicitly, and
   never resamples or reinterprets them silently.
3. Reports peak, RMS and residual metrics per channel, and locates differences
   in time.
4. Optionally estimates a bounded integer latency (lag) and a gain or polarity
   change to *explain* a failure.
5. Keeps **diagnosis** and **acceptance** separate. An estimated lag or gain
   never changes the verdict unless the policy explicitly allows that
   compensation, within an explicit limit, and the report records that it was
   applied. Compensation can never silently turn a regression into a pass:
   a pass that needed compensation has its own verdict.
6. Manages snapshot baselines with a deliberate update and review flow that
   integrates with `cargo test`, `cargo nextest` and CI.

### 1.3 Terminology

| Term | Meaning |
|---|---|
| Frame | One sample per channel at a single time index |
| Reference | The expected audio (a baseline file or a caller-supplied buffer) |
| Actual | The audio produced by the code under test |
| Residual | `actual − gain × reference` sample by sample, after any applied lag; gain is 1 unless compensated |
| dBFS | Decibels relative to full scale, where amplitude `1.0` is 0 dBFS |
| Lag | Integer frame offset; positive lag means the actual output is *later* than the reference: `actual[n + lag] ≈ reference[n]` |
| Policy | The complete set of tolerances, compensation permissions and reporting options for one comparison |
| Baseline | A committed reference file used by a snapshot test |
| Candidate | A pending file written by a failing snapshot test, awaiting review |

## 2. Requirements

### 2.1 Functional requirements

| ID | Requirement |
|---|---|
| F1 | Represent multichannel audio with explicit sample rate, channel count and frame count |
| F2 | Read and write WAV: integer PCM 8/16/24/32-bit and IEEE float 32-bit, including `WAVE_FORMAT_EXTENSIBLE` headers |
| F3 | Construct buffers from interleaved or planar `f32`/`f64` slices supplied by test code |
| F4 | Fail on sample-rate or channel-count mismatch, and on length differences beyond the allowed number of frames |
| F5 | Detect NaN and infinite samples in either input and report the first location and count |
| F6 | Compute per-channel peak and RMS for reference, actual and residual |
| F7 | Evaluate configurable tolerances: residual peak (dBFS), residual RMS (dBFS), residual RMS relative to the reference RMS (dB), peak and RMS level deltas (dB), and optional per-window residual RMS (dBFS) |
| F8 | Provide exact comparison (`Policy::exact()`): any non-zero residual fails |
| F9 | Estimate integer lag within a bounded search range, and gain including polarity, for diagnosis |
| F10 | Apply lag and/or gain compensation only when the policy allows it, and only within the policy's limits |
| F11 | Report windowed diagnostics: first divergence location and the worst windows |
| F12 | Classify a failure as explained by latency, gain, polarity or a combination, or as unexplained |
| F13 | Provide a snapshot assertion for Rust tests with explicit update modes, candidate files and duplicate-name detection |
| F14 | Provide CLI commands: `diff`, `review`, `accept`, `reject` |
| F15 | Produce a human-readable summary and a versioned JSON report |
| F16 | Compensation never silently turns a regression into a pass. A comparison that passes only because policy-permitted compensation was applied gets the distinct verdict `PassWithCompensation`. Its result and report state what was compensated (lag and/or gain), by how much, and how many shift-exposed frames were excluded. The snapshot assertion prints the same facts. An uncompensated pass is always `Pass` |

### 2.2 Non-functional requirements

| ID | Requirement |
|---|---|
| N1 | **Determinism:** the same inputs and policy produce an identical verdict on every platform, and identical report values up to the documented `log10` precision note (§7.8) |
| N2 | **No panics on untrusted input:** malformed WAV files, extreme headers and non-finite samples produce errors or findings, never panics. The only intended panic is the test-assertion macro reporting a failed snapshot |
| N3 | **Bounded resources:** memory is O(frames × channels) for the decoded inputs, plus O(1) per channel for analysis state; decoding enforces configurable limits; lag search cost is bounded by policy |
| N4 | **Fast passing path:** a passing comparison performs one linear pass over the data; diagnostics run only on failure unless compensation is enabled |
| N5 | **Small public API** with `#[non_exhaustive]` on enums and report structs so the API can evolve compatibly |
| N6 | **Portability:** Linux, macOS and Windows; no system libraries; pure Rust |
| N7 | **Rust quality gates:** stable Rust, MSRV 1.85 (edition 2024); `#![forbid(unsafe_code)]`; `cargo clippy --all-targets -- -D warnings`; `cargo fmt --check`; documentation on all public items (`#![deny(missing_docs)]`); `cargo deny check` for licences, advisories, bans and sources |

## 3. Public API and CLI design

### 3.1 Workspace layout

```text
Cargo.toml              # workspace
crates/
  waveassert/            # library: buffer, wav, policy, compare, report, snapshot
  waveassert-cli/        # binary `waveassert`: depends on the library and clap
```

The CLI is a separate package so library users, who are mostly test code, do
not compile `clap`. Environment variables use the `WAVEASSERT_` prefix.

### 3.2 Library API (proposed)

```rust
// ---- buffer ----
pub struct AudioBuffer { /* private */ }

impl AudioBuffer {
    /// Interleaved samples: frame-major, `channels` samples per frame.
    pub fn from_interleaved(sample_rate: u32, channels: u16, samples: &[f32]) -> Result<Self, Error>;
    pub fn from_interleaved_f64(sample_rate: u32, channels: u16, samples: &[f64]) -> Result<Self, Error>;
    /// One slice per channel; all slices must have equal length.
    pub fn from_planar<C: AsRef<[f32]>>(sample_rate: u32, channels: &[C]) -> Result<Self, Error>;
    pub fn from_planar_f64<C: AsRef<[f64]>>(sample_rate: u32, channels: &[C]) -> Result<Self, Error>;

    pub fn sample_rate(&self) -> u32;
    pub fn channels(&self) -> u16;
    pub fn frames(&self) -> usize;
    pub fn channel(&self, index: u16) -> Option<&[f32]>;
    /// Format the samples came from, if read from a file (informational).
    pub fn source_format(&self) -> Option<SampleFormat>;
}

#[non_exhaustive]
pub enum SampleFormat { Int8, Int16, Int24, Int32, Float32 }

// ---- wav ----
pub mod wav {
    pub fn read(path: &Path, limits: &Limits) -> Result<AudioBuffer, Error>;
    pub fn read_from<R: Read + Seek>(reader: R, limits: &Limits) -> Result<AudioBuffer, Error>;
    pub fn write(path: &Path, buffer: &AudioBuffer, format: SampleFormat) -> Result<(), Error>;
}

pub struct Limits { pub max_channels: u16, pub max_frames: u64, pub max_file_bytes: u64 }
impl Default for Limits { /* 64 channels, 2^27 frames, 2 GiB */ }

// ---- policy ----
#[derive(Clone, Debug, PartialEq)]
pub struct Policy { /* private fields, builder methods */ }

/// Residual peak <= -80 dBFS and residual RMS <= -90 dBFS per channel; exact length;
/// lag search 0 (off); no compensation; 10 ms windows; diagnose on failure.
impl Default for Policy { /* ... */ }

impl Policy {
    /// Any non-zero residual sample fails.
    pub fn exact() -> Self;

    pub fn max_residual_peak_dbfs(self, db: f64) -> Self;
    pub fn max_residual_rms_dbfs(self, db: f64) -> Self;
    pub fn max_residual_rms_relative_db(self, db: f64) -> Self;
    pub fn max_peak_delta_db(self, db: f64) -> Self;
    pub fn max_rms_delta_db(self, db: f64) -> Self;
    pub fn max_window_residual_rms_dbfs(self, db: f64) -> Self;
    pub fn clear_tolerances(self) -> Self;

    pub fn max_length_difference(self, frames: u64) -> Self;
    pub fn window(self, duration: Duration) -> Self;

    /// Diagnosis only: search lags in [-frames, +frames] when explaining a failure.
    pub fn lag_search(self, frames: u32) -> Self;
    /// Acceptance: compensate a lag of at most `frames` before evaluating tolerances.
    pub fn allow_lag_compensation(self, frames: u32) -> Self;
    /// Acceptance: compensate a gain change of at most `db` (absolute) before evaluating tolerances.
    /// Polarity inversion is never compensated (see §7.5).
    pub fn allow_gain_compensation(self, db: f64) -> Self;

    pub fn diagnose(self, when: DiagnoseWhen) -> Self;
    pub fn max_reported_windows(self, n: usize) -> Self;

    /// Called by `compare`; public so callers can validate policies up front.
    pub fn validate(&self) -> Result<(), Error>;
}

#[non_exhaustive]
pub enum DiagnoseWhen { OnFailure, Always, Never }

// ---- compare ----
pub fn compare(reference: &AudioBuffer, actual: &AudioBuffer, policy: &Policy) -> Result<Report, Error>;

// ---- report ----
#[non_exhaustive]
pub struct Report {
    pub verdict: Verdict,
    pub failures: Vec<Failure>,
    pub reference: StreamInfo,
    pub actual: StreamInfo,
    pub applied: AppliedCompensation,
    pub channels: Vec<ChannelMetrics>,
    pub first_divergence: Option<Location>,
    pub worst_windows: Vec<WindowMetrics>,
    pub diagnosis: Option<Diagnosis>,
}
#[non_exhaustive]
pub enum Verdict { Pass, PassWithCompensation, Fail }

impl Report {
    /// True for `Pass` and `PassWithCompensation`.
    pub fn passed(&self) -> bool;
    /// True only for `PassWithCompensation`.
    pub fn compensated(&self) -> bool;
    #[cfg(feature = "json")] pub fn to_json(&self) -> String;
}
impl fmt::Display for Report { /* human summary */ }

// ---- snapshot ----
#[macro_export]
macro_rules! assert_audio_snapshot { /* (name, &buffer) | (name, &buffer, &policy) */ }

pub mod snapshot {
    pub struct SnapshotStore { /* root directory */ }
    impl SnapshotStore {
        pub fn new(root: impl Into<PathBuf>) -> Self;
        pub fn check(&self, name: &str, actual: &AudioBuffer, policy: &Policy, mode: UpdateMode)
            -> Result<SnapshotOutcome, Error>;
    }
    #[non_exhaustive] pub enum UpdateMode { No, New, Always }
    impl UpdateMode { pub fn from_env() -> Result<Self, Error>; }
    #[non_exhaustive]
    pub enum SnapshotOutcome {
        Matched(Report),
        Mismatched { report: Report, candidate: Option<PathBuf> },
        MissingBaseline { candidate: Option<PathBuf> },
        BaselineUpdated { report: Option<Report>, path: PathBuf },
    }
}
```

API design notes:

- **Free function `compare`.** There is no comparator object. A comparison is a
  pure function of its inputs.
- **Comparison outcomes are data, not errors.** A mismatched sample rate is a
  `Failure` in the `Report`. `Err` is reserved for problems that prevent a
  comparison: an invalid policy, unreadable input or exceeded limits. A test
  failure therefore always produces a full report.
- **Explicit naming:** `lag_search` is diagnosis, while
  `allow_lag_compensation` affects the verdict. The two meanings cannot be
  set with one flag.
- **Why a macro.** It is needed only to capture the caller's
  `CARGO_MANIFEST_DIR`, `file!()` and `line!()`. It expands to a call to
  `snapshot::assert_snapshot(ctx, name, buffer, policy)` and panics with the
  formatted report on failure, as test assertions do. Custom harnesses use
  `SnapshotStore::check`, which never panics.
- **Integer sample input.** Integer samples reach the library through WAV
  reading. Test code that renders integers converts them itself. That keeps
  the convention of §4.2 in one place, and avoids a generic sample trait.

### 3.3 CLI design (proposed)

```text
waveassert diff <REFERENCE.wav> <ACTUAL.wav> [OPTIONS]
    --max-residual-peak <DBFS>     default -80
    --max-residual-rms <DBFS>      default -90
    --max-residual-rms-rel <DB>
    --max-peak-delta <DB>
    --max-rms-delta <DB>
    --max-window-rms <DBFS>
    --exact                        any non-zero residual fails (overrides tolerances)
    --max-length-diff <FRAMES>     default 0
    --window-ms <MS>               default 10
    --search-lag <FRAMES>          diagnosis only, default 0
    --compensate-lag <FRAMES>      acceptance, default 0 (off)
    --compensate-gain <DB>         acceptance, default off
    --diagnose <on-failure|always|never>
    --json                         print the JSON report to stdout instead of the summary
    --report <PATH>                also write the JSON report to PATH

waveassert review <SNAPSHOT_DIR>               list pending candidates with their summaries
waveassert accept <SNAPSHOT_DIR> (<NAME>... | --all)
waveassert reject <SNAPSHOT_DIR> (<NAME>... | --all)
```

Exit codes follow `cmp` and `diff`:

| Code | Meaning |
|---|---|
| 0 | Within tolerance (`diff`), including a pass with permitted compensation, which the summary and JSON label `pass_with_compensation`; or command succeeded |
| 1 | Difference beyond tolerance (`diff`) |
| 2 | Usage error, unreadable or invalid input, limit exceeded, I/O failure |

`review` exits 0 when there are no pending candidates and 1 when there are,
so CI can fail if candidates were left behind. `review`, `accept` and `reject`
are non-interactive in the first milestone. An interactive review terminal UI
is deferred.

## 4. Core domain model and invariants

### 4.1 `AudioBuffer`

Storage: one contiguous `Vec<f32>` in planar (channel-major) order, plus
`sample_rate: NonZeroU32`, `channels: NonZeroU16` and `frames: usize`.

Invariants (enforced by constructors; fields are private):

1. `samples.len() == frames × channels`.
2. `sample_rate > 0` and `1 ≤ channels ≤ Limits::max_channels`.
3. `frames` may be zero. An empty buffer is valid, and comparisons with it
   are defined.
4. Samples may be any `f32`, including NaN and ±∞. Non-finite samples
   are **findings** reported by `compare`, not construction errors, because a
   render that produces NaN is exactly what a test must report.
5. A buffer is immutable after construction.

**Why `f32` storage.** Baselines are stored as 32-bit float WAV by default, so
an `f32` buffer round-trips through a baseline bit-exactly, and an accepted
snapshot compares as identical even under `Policy::exact()`. With `f64`
storage, an `f64` render would differ from its own `f32` baseline by
about −150 dBFS, and exact policies would fail immediately after acceptance.
`f64` input is converted with round-to-nearest at construction. Precision is
not lost in a way that matters: the `f32` quantization floor, about −144 dBFS
relative to full scale, is below any useful tolerance. All analysis
accumulates in `f64`.

### 4.2 Sample-format conversion

| Source | Conversion to `f32` |
|---|---|
| Int8 (WAV unsigned 8-bit) | `(u − 128) / 128` |
| Int16 | `s / 32768` |
| Int24 | `s / 8388608` |
| Int32 | `s / 2147483648`, rounded to `f32` (precision beyond 24 bits is not representable; documented) |
| Float32 | unchanged |
| Float64 (from slices) | rounded to nearest `f32` |

Writing to integer formats is the inverse, with round-half-to-even. A sample
outside the representable range `[−1.0, 1.0 − 1 LSB]` causes
`Error::WouldClip` instead of silent clipping. Writing non-finite samples to
any format is `Error::NonFiniteSample`.

### 4.3 `Policy`

Fields:

```text
tolerances:
  residual_peak_dbfs:        Option<f64>
  residual_rms_dbfs:         Option<f64>
  residual_rms_relative_db:  Option<f64>
  peak_delta_db:             Option<f64>   # |actual peak − reference peak| in dB
  rms_delta_db:              Option<f64>
  window_residual_rms_dbfs:  Option<f64>
silence_floor_dbfs:          f64           # default −120; see invariant 4
max_length_difference:       u64 frames    # default 0
window:                      Duration      # default 10 ms
lag_search:                  u32 frames    # diagnosis range, default 0
lag_compensation:            Option<u32>   # acceptance limit, default None
gain_compensation:           Option<f64>   # acceptance limit in dB, default None
diagnose:                    DiagnoseWhen  # default OnFailure
max_reported_windows:        usize         # default 8
lag_analysis_max_frames:     u64           # default 2^20
```

Invariants (checked by `Policy::validate`, which returns `InvalidPolicy`):

1. Every dB value is finite, except that tolerance thresholds may be `−∞`.
   `Policy::exact()` is `residual_peak_dbfs = −∞` with all other tolerances
   cleared. NaN is always rejected.
2. Delta and compensation limits are `≥ 0`.
3. Structural checks (sample rate, channel count, length, non-finite samples)
   always apply. A policy with no tolerances is valid, but it checks only
   structure. `clear_tolerances` exists so this is an explicit choice.
4. A relative tolerance requires at least one absolute residual tolerance.
   The relative measure is undefined for silent references. For a channel
   whose reference RMS is below `silence_floor_dbfs`, the relative check is
   skipped and a note is recorded. The absolute tolerances still apply.
5. `lag_compensation ≤ 4096` and `lag_search ≤ 4096` in the first milestone
   (see §7.4 and §16, Q6). Compensation implies a search over at least the
   compensation range.
6. `window` converts to at least one frame at the compared sample rate.

### 4.4 `Report`

```text
Report
  verdict:            Pass | PassWithCompensation | Fail
  failures:           [Failure]             # empty <=> Pass
  reference, actual:  StreamInfo { sample_rate, channels, frames, source_format }
  applied:            AppliedCompensation { lag: i32, gain: f64, excluded: EdgeExclusion }
                                            # lag 0, gain 1.0, nothing excluded when none
  # EdgeExclusion { reference_frames: u32, actual_frames: u32 }: shift-exposed frames (§7.1)
  channels:           [ChannelMetrics]      # one per channel when channel counts match
  first_divergence:   Option<Location { channel, frame, seconds }>
  worst_windows:      [WindowMetrics { channel, start_frame, frames, residual_peak, residual_rms }]
  diagnosis:          Option<Diagnosis>
  notes:              [Note]                # non-failing observations

ChannelMetrics
  reference: Levels { peak, rms }           # linear; dB derived on output
  actual:    Levels { peak, rms }
  residual:  Levels { peak, rms }
  non_finite: Option<NonFinite { input, first_frame, count }>

Failure (non_exhaustive enum)
  SampleRateMismatch { reference, actual }
  ChannelCountMismatch { reference, actual }
  LengthMismatch { reference, actual, allowed }
  NonFiniteSamples { input, channel, first_frame, count }
  ToleranceExceeded { check: Check, channel, measured, limit }
  WindowToleranceExceeded { channel, start_frame, measured, limit }

Diagnosis
  lag:         Option<LagEstimate { frames: i32, correlation: f64, per_channel: [Option<i32>] }>
  gain:        Option<GainEstimate { linear: f64, db: f64, polarity_inverted: bool, per_channel: [Option<f64>] }>
  explanation: Latency | Gain | Polarity | LatencyAndGain | Unexplained
  residual_if_compensated: [Levels]         # per channel, what-if only
  excluded_if_compensated: EdgeExclusion    # shift-exposed frames left out of the what-if (§7.1)
```

Invariants:

1. `verdict == Fail` exactly when `failures` is non-empty.
2. **Diagnosis never affects the verdict.** `Diagnosis` and every what-if metric
   are computed from separate passes. The verdict is a function of the inputs
   and the policy's tolerances and compensation permissions only.
   Changing `diagnose` or `lag_search` never changes `verdict` or `failures`.
3. `applied.lag != 0` only when `lag_compensation` permits that lag, and
   `applied.gain != 1.0` only when `gain_compensation` permits it. The human
   summary always states applied compensation, including on a pass.
   `applied.excluded` is non-zero only when `applied.lag != 0`. Each count is
   at most `|applied.lag|`, and the summary and a `Note` always state it.
6. **No silent compensated pass (F16).** `applied` is non-identity (lag ≠ 0,
   gain ≠ 1) exactly when `verdict == PassWithCompensation`. Compensation is
   tried only after the uncompensated evaluation fails (§7.6). If the
   compensated evaluation also fails, the report shows the uncompensated
   evaluation, with `applied` as identity and `verdict == Fail`, and the
   diagnosis explains the failure.
4. When structural checks fail on sample rate or channel count, no metrics
   are computed (`channels` is empty), because residuals between unrelated
   layouts would be misleading.
5. Report size is bounded: at most `max_reported_windows` windows; per-channel
   vectors are bounded by `Limits::max_channels`.

### 4.5 Snapshot model

```text
<root>/<name>.wav           baseline (committed)
<root>/<name>.new.wav       candidate (pending review, not committed)
<root>/<name>.new.json      JSON report for the candidate's comparison
```

- `root` defaults to `<CARGO_MANIFEST_DIR>/tests/audio_snapshots` for the
  macro. `SnapshotStore::new` accepts any root.
- `name` matches `[A-Za-z0-9_-]+(/[A-Za-z0-9_-]+)*`, and is at most 200
  bytes. `/` creates subdirectories. Absolute paths, `.`, `..`, empty
  segments and a `.new` suffix are rejected (`Error::InvalidSnapshotName`).
- A baseline never contains non-finite samples. The store refuses to write one.

`UpdateMode`, read from `WAVEASSERT_UPDATE`:

| Value | Missing baseline | Mismatch | Match |
|---|---|---|---|
| `no` | fail, write nothing | fail, write nothing | pass; remove stale candidate files |
| `new` | fail, write candidate | fail, write candidate | pass; remove stale candidate files |
| `always` | write baseline, pass with notice | overwrite baseline, pass with notice | pass |

The default is `no` when the `CI` environment variable is set to a value
other than `""`, `0` or `false`, and `new` otherwise. An explicit
`WAVEASSERT_UPDATE` always wins. An invalid value is an error, not a fallback.
The variable is read on every call, so tests in one process can be run in
different modes by the test runner.

Deliberate-update rules:

- Baselines are never created or replaced implicitly. The `new` mode writes
  candidates only. Only `always`, `waveassert accept` or
  `SnapshotStore::accept` replace baselines.
- A failure message always says how to review and accept, and states whether
  a candidate was written.
- `always` prints one line per updated baseline to stderr, so updates appear
  in test logs.
- A snapshot that matches only with compensation (`Matched` with verdict
  `PassWithCompensation`) passes, as the policy permits. The assertion prints
  one stderr line naming the snapshot, the compensated lag and gain, and the
  excluded frames, so the compensated pass is visible in test logs (F16).
- Writes use a temporary file in the same directory followed by an atomic
  rename, so an interrupted run never leaves a truncated baseline.
- Within one process, using the same snapshot name twice for different data
  is detected with a process-wide registry and reported as a failure. Under
  `cargo nextest`, where each test runs in its own process, this check cannot
  work; the documentation says so (§16, Q9).

## 5. Modules and responsibility boundaries

Library crate `waveassert`:

| Module | Responsibility | Depends on |
|---|---|---|
| `buffer` | `AudioBuffer`, `SampleFormat`, conversions, constructors | `error` |
| `wav` | WAV decode/encode via `hound`; `Limits` enforcement | `buffer`, `error` |
| `level` | Linear ↔ dB helpers, dB formatting, `−∞` handling | — |
| `policy` | `Policy`, builder, validation, defaults | `level`, `error` |
| `analysis` (private) | Residual passes, levels, windows, lag correlation, gain fit | `buffer` |
| `compare` | Orchestration: structural checks → compensation → tolerances → diagnosis → `Report` | `analysis`, `policy`, `report` |
| `report` | `Report` types, `Display`, JSON serialization (feature `json`, on by default) | `level` |
| `snapshot` | `SnapshotStore`, `UpdateMode`, names, candidates, atomic writes, accept/reject, macro support | `compare`, `wav`, `report` |
| `error` | `Error` enum | — |

CLI crate `waveassert-cli`: argument parsing, mapping flags to `Policy`,
output selection and exit codes. It contains no comparison logic.

Boundaries:

- `analysis` is the only module with numeric kernels. It works on `&[f32]`
  slices and plain parameters, and knows nothing about policies, files or
  reports. This keeps the DSP code small, testable and replaceable, for
  example with an FFT correlation later.
- `compare` is the only module that decides a verdict.
- `snapshot` is the only module that writes files other than explicit
  `wav::write` calls.
- No module performs network access or spawns processes.

## 6. Data flow and processing lifecycle

```text
 test code / CLI
      │ samples or WAV path
      ▼
 ┌────────────┐   Limits    ┌─────────┐
 │ buffer/wav │◄────────────┤ caller  │
 └─────┬──────┘             └─────────┘
       │ AudioBuffer (reference), AudioBuffer (actual)
       ▼
 ┌────────────────────────── compare ───────────────────────────┐
 │ 1 validate policy                     → Err(InvalidPolicy)   │
 │ 2 structural checks (rate, channels, length, non-finite)     │
 │     → Failures; stop on rate or channel-count mismatch       │
 │ 3 residual pass, uncompensated (one linear pass per channel):│
 │     levels, residual levels, windows, first divergence       │
 │ 4 evaluate tolerances (linear domain) → Failures, or Pass    │
 │ 5 only if 4 failed on tolerances and compensation is allowed:│
 │     estimate lag within ±lag_compensation (never lag_search) │
 │     estimate gain → apply only if |gain dB| ≤ permission     │
 │     re-run 3–4 with the shift-edge rule (§7.1);              │
 │     pass → PassWithCompensation, else keep the result of 4   │
 │ 6 if diagnose = Always, or OnFailure and failed:             │
 │     estimate lag/gain (if not done), what-if residual passes,│
 │     classify explanation          (never touches Failures)   │
 │ 7 assemble immutable Report                                  │
 └──────────────────────────────┬───────────────────────────────┘
                                ▼
          Report ──► Display (human) / JSON / snapshot store
                                │
                     snapshot: decide by UpdateMode
                     ├─ match → remove stale candidates
                     ├─ mismatch/missing + New → write .new.wav + .new.json
                     └─ Always → atomic replace baseline
```

Lifecycle of a snapshot test:

1. The macro builds `SnapshotContext { manifest_dir, file, line }` and resolves
   the store root.
2. The name is validated and registered in the in-process duplicate registry.
3. The mode is read from the environment.
4. The baseline is read with default `Limits`. A missing file is a
   `MissingBaseline` outcome, not an error.
5. `compare(baseline, actual, policy)` runs.
6. The store acts on the outcome according to the mode table in §4.5.
7. The macro panics with the report and update instructions on
   `Mismatched` or `MissingBaseline`, and returns normally otherwise. A
   compensated match first prints its stderr notice (§4.5).

## 7. Algorithms and implementation choices

### 7.1 Alignment model: zero padding plus a shift-edge rule

**Plain comparison (no lag).** Both inputs are treated as zero outside their
stored range. The compared range is `n ∈ [0, max(N_ref, N_act))`, and
`residual[n] = actual[n] − g · reference[n]`, with out-of-range samples equal to 0.

- Every sample of both inputs contributes to the residual exactly once. A
  truncated reverb tail or extra trailing content shows up as residual
  energy.
- A length difference is evaluated twice: by the explicit `LengthMismatch`
  check against `max_length_difference`, and through the residual energy of
  the unmatched tail.

**Shifted comparison (lag `L ≠ 0`).** This applies to a diagnosis what-if or
to policy-permitted lag compensation. Shifting the actual signal by `L`
exposes frames whose counterpart in the other input existed before the
shift and does not exist after it. Comparing those frames with silence would
make every pure latency change look like a large residual: with equal
lengths `N` and `L = 1`, `residual[N−1] = 0 − reference[N−1]`. The **edge
rule** excludes exactly those *shift-exposed* frames, and nothing else:

```text
residual[n] = actual[n + L] − g · reference[n]        (out of range ⇒ 0)

L > 0 (actual is later):
  excluded actual frames     m ∈ [0, min(L, N_act, N_ref))                 # moved before n = 0
  excluded reference frames  n ∈ [max(0, N_act − L), min(N_act, N_ref))    # lost their partner at the tail
L < 0 (actual is earlier): the mirror image, with K = −L:
  excluded reference frames  n ∈ [0, min(K, N_ref, N_act))                 # lost their partner at the head
  excluded actual frames     m ∈ [max(0, N_ref − K), min(N_ref, N_act))    # moved past n = N_ref
```

All other frames, including any length difference that existed *before* the
shift, are still compared under zero padding. Each input loses at most `|L|`
frames, and `|L|` is bounded by `lag_search` or `lag_compensation`. The excluded counts are
recorded in `AppliedCompensation::excluded` or
`Diagnosis::excluded_if_compensated`. When the applied lag is non-zero, a
`Note` and the human summary state them. So no sample is excluded
*silently*: plain comparisons exclude nothing, and shifted comparisons
exclude a bounded, reported edge.

### 7.2 Levels and residual metrics

For each channel, one forward pass accumulates in `f64`:

- `peak = max |x|` and `rms = sqrt(Σ x² / n)` for reference, actual and
  residual. `n` is the number of compared frames: the range of §7.1 minus any
  shift-exposed frames. For RMS this deliberately includes padded zeros, so
  RMS stays comparable between the two inputs.
- window accumulators, which are fixed, non-overlapping, and aligned to frame 0
  of the reference timeline; the last window may be shorter;
- the first index where `|residual|` exceeds the residual-peak tolerance, or
  where it is non-zero under an exact policy.

Summation is sequential in index order. Pairwise or Kahan summation is not needed:
`f64` accumulation of `f32` squares over up to 2^27 frames keeps relative
error far below the 0.01 dB report resolution. The fixed order keeps results
reproducible.

### 7.3 Tolerance evaluation

Thresholds are converted once from dB to linear amplitude
(`10^(dB/20)`, with `−∞` mapping to `0`). Measured values are then compared
in the linear domain with `<=`. dB values appear only in output. This makes
verdicts independent of platform `log10` precision and makes the exact
policy a plain `peak <= 0.0` comparison.

| Check | Measured (per channel) | Fails when |
|---|---|---|
| Residual peak | `residual.peak` | `> limit` |
| Residual RMS | `residual.rms` | `> limit` |
| Residual RMS, relative | `residual.rms / reference.rms` | `> limit`; skipped below the silence floor |
| Peak delta | `|20·log10(actual.peak / reference.peak)|` | `> limit`; a zero peak on one side only fails |
| RMS delta | same with RMS | same |
| Window residual RMS | each window's residual RMS | any window `> limit` |

Every channel must pass every configured check. Overall (all-channel)
figures are reported for information only. Per-channel evaluation is
stricter and easier to reason about than a pooled figure, in which a silent
channel would dilute a broken one.

### 7.4 Integer lag estimation (bounded)

Goal: find the integer `L ∈ [−K, K]` that best aligns actual with reference,
for diagnosis or for permitted compensation.

1. Analyse at most `lag_analysis_max_frames` frames from the start of the
   reference timeline (default 2^20, about 21.8 s at 48 kHz). This bounds the
   cost on long renders.
2. For each candidate `k`, compute the cross-correlation summed over channels
   and the energies of the overlapping parts:
   `C(k) = Σ_c Σ_n actual_c[n+k] · reference_c[n]`, and
   `ρ(k) = C(k) / sqrt(E_act(k) · E_ref(k))`.
   Normalization by overlap energy removes the bias towards `k = 0`.
3. Select `argmax |ρ(k)|`. The sign of `ρ` indicates polarity. Ties are broken
   by smallest `|k|`, then the negative `k`, which is deterministic.
4. If either energy is zero for every `k`, the lag is *undetermined*
   (`None`); silence has no lag.
5. Per-channel lags are computed the same way and reported. If they differ,
   the report adds a channel-skew note. Compensation always uses the single
   global lag, because a processor's latency is normally the same for all
   channels.

Implementation: a direct time-domain loop, `O(channels × frames × (2K+1))`.
With the first-milestone cap `K ≤ 4096` and the frame cap above, the worst
case is bounded and documented. For typical use (`K = 64`, 2 s stereo at
48 kHz) it costs about 12 million multiply-adds and runs only on failure.
An FFT-based correlation (`realfft`) is the planned optimization when larger
search ranges are needed (§16, Q6). It would sit behind the same `analysis`
function, and property tests would check it against the direct method.

Sub-sample (fractional) lag is out of scope for the first milestone. A
fractional delay appears as an unexplained residual with a best integer lag
reported. The documentation states this limit.

### 7.5 Gain and polarity estimation

After lag alignment, the least-squares gain is
`g = Σ actual·reference / Σ reference²`, computed per channel and globally
over the compared range. It is undefined if the reference is silent.
`gain_db = 20·log10|g|`, and `polarity_inverted = g < 0`.

Gain compensation, when permitted, applies the global `g` only if
`|gain_db| ≤ limit` and `g > 0`. **Polarity inversion is never compensated**
because it is almost always a bug. It is diagnosed as the `Polarity`
explanation. The residual is `actual − g·reference`. Scaling the reference
instead of the actual avoids amplifying noise in the actual signal.

### 7.6 Diagnosis classification

The diagnosis asks which permitted-or-not transformation would make the
comparison pass under the same tolerances. Diagnosis needs no extra
thresholds beyond the policy the user already chose.

1. Estimate the lag within `max(lag_search, lag_compensation)`, then the gain
   at that lag.
2. Evaluate what-if residuals, at most four extra linear passes, in this
   order. Any what-if with `L ≠ 0` applies the shift-edge rule of §7.1:
   only the shift-exposed frames are left out, and their counts are recorded
   in `excluded_if_compensated`. The first what-if that passes all
   tolerances is the explanation:
   - latency only (`L ≠ 0`, `g = 1`) → `Latency`
   - gain only (`L = 0`, `g`) → `Gain`, or `Polarity` if `g < 0` and
     `|g| ≈ 1` within the gain-delta tolerance
   - both → `LatencyAndGain`
   - none → `Unexplained`, with the best what-if residual reported so the
     reader can see how much the systematic part accounts for
3. Record the explanation and what-if levels in `Diagnosis`. Do not modify
   `failures` or `verdict`.

**Compensation can never silently turn a regression into a pass.** Permitted
compensation is separate from diagnosis and runs in a fixed order. `compare`
always evaluates the uncompensated residual first. It tries the permitted lag
and/or gain compensation only if that evaluation fails on tolerances. The
compensation lag is searched only within `±lag_compensation`, never within the
diagnosis range `lag_search`, so diagnosis settings cannot change the verdict. A
structural failure is never compensated. If the compensated evaluation
passes, the verdict is `PassWithCompensation`, not `Pass`. The report,
`Display`, JSON and the snapshot assertion state what was compensated (lag in
frames, gain in dB), by how much, and how many shift-exposed frames each input
had excluded. A caller that requires uncompensated passes checks
`report.verdict == Verdict::Pass` or JSON `"verdict": "pass"`.

"Added noise" or "band-limited change" classifications need spectral
analysis and are deferred (§15). The first milestone reports these cases
honestly as `Unexplained`, with windowed location data.

### 7.7 Windows and first divergence

- Window length = `round(window × sample_rate)` frames, at least 1. The
  default is 10 ms, which is 480 frames at 48 kHz.
- The report keeps the `max_reported_windows` windows with the highest
  residual RMS across all channels. Ties are broken by earlier start, then
  lower channel. A bounded min-heap keeps memory at `O(max_reported_windows)`.
- With a window tolerance, every window is checked during the pass, and only
  the first failing window per channel becomes a `Failure`, to keep the
  report bounded.

### 7.8 Determinism notes

- Rust does not contract floating-point operations into fused multiply-add
  unless asked to, and every reduction runs in fixed index order. Linear
  metrics are therefore bit-identical across tier-1 platforms for the same
  inputs.
- `log10` and `powf` come from the platform math library and can differ in
  the last unit of precision. They are used only for dB *display* and for
  converting thresholds once. JSON output rounds dB values to 0.001 dB, and
  verdicts use linear comparisons (§7.3).
- The analysis is single-threaded in the first milestone (§10).

## 8. File formats and compatibility

### 8.1 Audio files

- **WAV (RIFF WAVE)** is the only file format in the first milestone. It
  covers `WAVE_FORMAT_PCM`, `WAVE_FORMAT_IEEE_FLOAT` and
  [`WAVE_FORMAT_EXTENSIBLE`](https://learn.microsoft.com/en-us/windows/win32/coreaudio/waveformatextensible)
  with the PCM or float sub-format. Reading and writing use `hound`.
- Unsupported variants (64-bit float, compressed `fmt` codes, RF64 or BW64
  files larger than 4 GiB) produce `Error::UnsupportedFormat` naming the
  variant. They are never misread.
- Channel masks and extra chunks such as `LIST` and `bext` are ignored on
  read and not written. Speaker layout is out of scope; channels are compared
  by index.
- **Baseline default:** 32-bit float WAV, which round-trips bit-exactly from
  `AudioBuffer`. `SnapshotStore` can be configured to write 24- or 16-bit
  baselines to save repository space. Quantization then becomes part of the
  residual, about −98 dBFS RMS for 16-bit, and the documentation says to
  choose tolerances accordingly. When writing an integer baseline, the store
  reports the quantization residual and refuses to write if the baseline would
  fail its own policy.
- Repository guidance in the user documentation: mark `*.wav` as `binary` in
  `.gitattributes`, keep renders short (a few seconds), and consider Git LFS
  for large suites. As a size reference, 2 s of stereo `f32` at 48 kHz is
  768 KB.

### 8.2 JSON report (schema version 1)

```json
{
  "schema_version": 1,
  "verdict": "fail",
  "reference": { "sample_rate": 48000, "channels": 2, "frames": 96000, "source_format": "float32" },
  "actual":    { "sample_rate": 48000, "channels": 2, "frames": 96000, "source_format": null },
  "policy": { "residual_peak_dbfs": -80.0, "residual_rms_dbfs": -90.0, "lag_search": 64,
              "lag_compensation": null, "gain_compensation_db": null, "window_ms": 10.0 },
  "applied": { "lag": 0, "gain": 1.0, "excluded": { "reference_frames": 0, "actual_frames": 0 } },
  "failures": [
    { "kind": "tolerance_exceeded", "check": "residual_peak", "channel": 0,
      "measured": 0.0288, "measured_db": -30.81, "limit_db": -80.0 }
  ],
  "channels": [
    { "index": 0,
      "reference": { "peak": 0.5, "peak_db": -6.021, "rms": 0.3536, "rms_db": -9.031 },
      "actual":    { "peak": 0.5, "peak_db": -6.021, "rms": 0.3536, "rms_db": -9.031 },
      "residual":  { "peak": 0.0288, "peak_db": -30.81, "rms": 0.0204, "rms_db": -33.81 } }
  ],
  "first_divergence": { "channel": 0, "frame": 1, "seconds": 0.0000208 },
  "worst_windows": [ { "channel": 0, "start_frame": 0, "frames": 480, "residual_peak_db": -30.81, "residual_rms_db": -33.81 } ],
  "diagnosis": { "explanation": "latency", "lag": { "frames": 1, "correlation": 0.9998 },
                 "gain": { "linear": 1.0, "db": 0.0, "polarity_inverted": false },
                 "residual_if_compensated": [ { "peak": 0.0, "peak_db": null, "rms": 0.0, "rms_db": null } ],
                 "excluded_if_compensated": { "reference_frames": 1, "actual_frames": 1 } },
  "notes": []
}
```

The values above are illustrative. The input is a 440 Hz sine at amplitude
0.5, starting at phase 0. The actual render is the same sine delayed by one
frame and cut to the same 96000 frames. The plain comparison fails (no lag
compensation is permitted). The latency what-if removes the shift by
excluding the one shift-exposed frame of each input (§7.1): the actual's
first frame and the reference's last frame. What remains matches exactly,
so the explanation is `latency` and the what-if residual is `−∞ dBFS`.

- `verdict` is one of `"pass"`, `"pass_with_compensation"` or `"fail"`. A
  consumer that tests only for `"pass"` therefore treats a compensated pass as
  not passing, which is the conservative behaviour.
- `−∞ dB` is serialized as `null`, because JSON has no infinity. Linear values
  are always present and finite, so consumers can rely on them.
- `schema_version` increments only on breaking changes. Removing or renaming
  a field or changing a meaning is breaking. Adding fields is not, so
  consumers must ignore unknown fields.
- `waveassert review` reads the `.new.json` files it wrote. A file with an
  unknown `schema_version` is listed as "report unreadable" rather than
  failing the whole command.

### 8.3 Compatibility commitments

- Baselines are plain WAV files, readable by any audio tool. No sidecar is
  required to use a baseline.
- Before 1.0, minor versions may change the API. Changes to default
  tolerances or comparison semantics are listed in the changelog, because
  they can flip verdicts. After 1.0 they are semver-major changes.
- `cargo-semver-checks` runs on the library before any release.
- The MSRV is raised only in minor releases and stated in `Cargo.toml`
  (`rust-version`).

## 9. Errors, diagnostics and reporting

### 9.1 Error type

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("I/O error on {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("invalid WAV data in {path}: {message}")]
    InvalidWav { path: PathBuf, message: String },
    #[error("unsupported WAV variant in {path}: {variant}")]
    UnsupportedFormat { path: PathBuf, variant: String },
    #[error("{what} exceeds limit: {actual} > {limit}")]
    LimitExceeded { what: &'static str, actual: u64, limit: u64 },
    #[error("invalid buffer: {0}")]
    InvalidBuffer(String),
    #[error("invalid policy: {0}")]
    InvalidPolicy(String),
    #[error("invalid snapshot name {name:?}: {reason}")]
    InvalidSnapshotName { name: String, reason: &'static str },
    #[error("invalid WAVEASSERT_UPDATE value {0:?}; expected no, new or always")]
    InvalidUpdateMode(String),
    #[error("sample {value} at channel {channel}, frame {frame} would clip in {format:?}")]
    WouldClip { channel: u16, frame: u64, value: f32, format: SampleFormat },
    #[error("non-finite sample at channel {channel}, frame {frame}; refusing to write")]
    NonFiniteSample { channel: u16, frame: u64 },
}
```

`hound::Error` is mapped into `InvalidWav` or `UnsupportedFormat`, and the
source is kept where useful. The `hound` type is not part of the public API,
so the decoder can be replaced without a breaking change.

### 9.2 Failures versus errors versus notes

| Category | Examples | Effect |
|---|---|---|
| `Err(Error)` | unreadable file, invalid policy, limits | No report; CLI exit 2; snapshot macro panics with the error |
| `Failure` | rate/channel/length mismatch, non-finite samples, tolerance exceeded | Verdict `Fail`; CLI exit 1 |
| Compensated pass | permitted lag/gain compensation needed to pass | Verdict `PassWithCompensation`; CLI exit 0; summary, JSON and snapshot assertion state what was compensated, by how much, and the excluded frames |
| `Note` | compensation applied, relative check skipped on silence, channel skew, integer baseline quantization | Informational; never changes the verdict |

### 9.3 Human summary

The `Display` output is designed for test logs:

- First line: snapshot or file names and `PASSED`, `PASSED WITH COMPENSATION`
  (followed by the lag, gain and excluded frames on the same line) or `FAILED`.
- A table of checks × channels, with limit, measured dB and pass/fail status.
  It is truncated to 16 channels with a "+N more" line.
- First divergence, worst windows (time in seconds and frames), applied
  compensation and the diagnosis in one or two plain sentences.
- For snapshots: baseline path, candidate path (if written), and the exact
  `waveassert accept` command to run.

Output is plain ASCII by default, so CI logs on any platform render it
correctly. Colour is not used in the library. The CLI may add colour when
writing to a terminal.

## 10. Performance and resource considerations

- **Passing path:** structural checks plus one residual pass per channel,
  `O(frames × channels)`, about 10 floating-point operations per sample. A 2 s
  stereo render compares in well under a millisecond in release builds.
- **Debug builds:** `cargo test` builds dependencies without optimization by
  default, and numeric loops are 10–50× slower there. The documentation will
  recommend `[profile.dev.package.waveassert] opt-level = 3` for suites with
  many or long snapshots, and will benchmark the effect.
- **Diagnosis cost:** bounded by `lag_search`, `lag_analysis_max_frames` and at
  most four what-if passes. It runs only on failure by default.
  Compensation, when enabled, adds the lag estimation to every comparison.
  The documentation says so.
- **Memory:** inputs dominate (4 bytes per sample per buffer). Analysis
  allocates `O(channels + max_reported_windows)`. No per-window vectors are
  stored. WAV decoding pre-checks declared sizes against `Limits` before
  allocating, and grows buffers incrementally when the header size is
  unreliable.
- **Parallelism:** none in the first milestone. Test runners already
  parallelize across tests, and adding `rayon` would complicate determinism
  for little gain on short renders. Parallelizing per channel for long
  multichannel files is deferred.
- **Benchmarks:** a `criterion` benchmark suite for the residual pass and the
  lag search is created along with the analysis module. It is run in
  development and is not a CI gate.

## 11. Security and robustness

Threat model: the CLI may be pointed at untrusted WAV files, for example
artifacts from a contributor's pull request. Baseline files in a repository
are also untrusted input when CI runs tests on a contribution.

- **Parsing:** `hound` is a pure-Rust parser. The library adds checks before
  allocation: channel count ≤ `max_channels`, declared frames ≤ `max_frames`,
  and file size ≤ `max_file_bytes`. Truncated data, inconsistent chunk sizes
  and zero channels or sample rate produce `InvalidWav`.
- **No panics:** the library crate denies `clippy::unwrap_used`,
  `clippy::expect_used` and `clippy::panic` outside tests. Integer arithmetic on
  values derived from file headers (sizes, frame counts, offsets) uses checked
  operations, and integer conversions use `try_from`. The snapshot macro's panic is the deliberate test-failure
  signal and lives in one function.
- **Numeric robustness:** NaN and infinity are detected explicitly and
  reported. Division by zero energy produces "undefined" (`None`), never NaN.
- **Filesystem safety:** snapshot names are validated (§4.5), so writes stay
  inside the store root. Candidate and baseline writes go to a temporary file
  in the target directory followed by a rename. A pre-existing symlink at the
  destination is replaced by the rename and is never followed for writing.
  `accept --all` and `reject --all` touch only `*.new.wav` and `*.new.json`
  files below the given directory, and refuse to follow directory symlinks.
- **No code execution, network or environment side effects.** The library
  reads only `WAVEASSERT_UPDATE` and `CI`.
- **Supply chain:** small dependency set (§12); `cargo deny` runs in CI for
  advisories (RustSec), licences, duplicate versions and source restrictions;
  `Cargo.lock` is committed for the CLI.
- **Fuzzing:** a `cargo-fuzz` target for `wav::read_from` with small limits is
  added with the WAV module. It runs locally and on a schedule, not on every
  pull request.

## 12. Dependencies and alternatives

| Need | Choice | Why | Alternatives considered |
|---|---|---|---|
| WAV I/O | [`hound`](https://github.com/ruuda/hound) 3.x | Pure Rust, small, mature, reads and writes int/float PCM including extensible headers | [`symphonia`](https://github.com/pdeljanov/Symphonia): decode-only and much larger; planned later for FLAC and other lossless input (§15). Licence to be reviewed when it is added. `wavers`: smaller user base |
| Errors | [`thiserror`](https://github.com/dtolnay/thiserror) 2.x | Standard derive, no runtime cost | Hand-written `impl Error` |
| JSON | [`serde`](https://serde.rs) + `serde_json` (feature `json`, default on) | Standard | None reasonable |
| Atomic writes | [`tempfile`](https://github.com/Stebalien/tempfile) 3.x | `NamedTempFile::persist` provides same-directory atomic replace on all platforms | Hand-rolled temp names + `std::fs::rename` |
| CLI parsing | [`clap`](https://github.com/clap-rs/clap) 4.x (CLI crate only) | Standard, derive API, good help output | `lexopt`/`pico-args`: smaller, but more hand-written help |
| Sample conversion | none (in-house, about 30 lines) | Trivial and documented in §4.2 | [`dasp`](https://github.com/RustAudio/dasp): broader than needed for six conversion rules |
| FFT (deferred) | [`realfft`](https://github.com/HEnquist/realfft) over [`rustfft`](https://github.com/ejmahler/RustFFT) | Pure Rust, widely used | — |
| Dev: property tests | [`proptest`](https://github.com/proptest-rs/proptest) | Shrinking, stable | `quickcheck` |
| Dev: snapshotting our own text/JSON output | [`insta`](https://insta.rs) | Keeps the report format under review | Hand-written expected strings |
| Dev: CLI tests | [`assert_cmd`](https://github.com/assert-rs/assert_cmd) + `predicates` | Standard | `std::process::Command` |
| Dev: benchmarks | [`criterion`](https://github.com/bheisler/criterion.rs) | Standard | `divan` |
| Dev: deterministic noise | [`rand`](https://github.com/rust-random/rand) + `rand_pcg` with fixed seeds | Reproducible fixtures across platforms | Hand-written xorshift |

The runtime dependencies of the library are `hound`, `thiserror`, `serde`,
`serde_json` and `tempfile`. Licence compatibility is checked with
`cargo deny` once the project licence is chosen. All dependencies above are
currently under permissive licences (MIT and/or Apache-2.0).

## 13. Testing strategy and quality gates

### 13.1 Unit tests

- dB ↔ linear conversion, including `0 → −∞` and `−∞ → 0`.
- Sample-format conversion tables (§4.2): exact round trips for every
  integer depth, the `WouldClip` boundaries, and 8-bit unsigned offsets.
- `AudioBuffer` constructors: length/channel validation, planar ↔
  interleaved equivalence, empty buffers.
- `Policy::validate`: every rule in §4.3.
- Lag tie-breaking and the undetermined case on silence.
- Window partitioning, including a short final window and window size
  clamped to at least 1 frame.
- Snapshot name validation: traversal, absolute paths, reserved suffix, and
  length.

### 13.2 Property tests (`proptest`)

| Property | Statement |
|---|---|
| Identity | For any finite buffer `x`, `compare(x, x, exact)` passes with residual peak `0` |
| Symmetry | With no gain compensation, swapping reference and actual leaves residual peak and RMS unchanged |
| Coverage | In a plain comparison, total frames across windows equal `max(N_ref, N_act)` |
| Edge rule | For any lag `L` and any lengths, each excluded count is `≤ |L|` and matches the formulas of §7.1. With `L = 0` both counts are 0. For equal-length inputs where the actual is the reference delayed by `L` with zero fill, the latency what-if residual is exactly 0 |
| Lag recovery | For seeded noise `r` and shift `k ∈ [−K, K]`, the estimated lag is `k` |
| Gain recovery | For `a = g·r` with `g ∈ [−4, 4] \ {0}`, the estimated gain is within `1e−6` relative, and polarity is correct |
| Diagnosis independence | `verdict` and `failures` are identical for `diagnose ∈ {Never, OnFailure, Always}` and for any `lag_search` |
| Compensation bound | `|applied.lag| ≤ lag_compensation` and `|applied gain dB| ≤ gain_compensation` always |
| No silent compensated pass | `applied` is non-identity exactly when `verdict == PassWithCompensation`; if the uncompensated comparison passes, the verdict is `Pass` whatever compensation is permitted |
| Monotonic tolerance | If a comparison passes at threshold `t`, it passes at any looser threshold |
| Round trip | Writing any finite buffer as float WAV and reading it back is bit-identical |

### 13.3 Fixture tests

Fixtures are **generated in test code** from fixed seeds instead of being
committed as binary files. This keeps the repository small and each case's
intent readable. The cases:

- the reference demonstration: a tone rendered bit-exact, with TPDF dither
  added, and delayed by one frame, all three at the same length as the
  reference. Hash comparison would flag all three as different. The expected
  results are: pass; pass (within the default tolerance); and fail with a
  `Latency` explanation, a what-if residual of `−∞ dBFS`, and
  `excluded_if_compensated` of one reference frame and one actual frame;
- the same delayed tone with `allow_lag_compensation(4)`: verdict
  `PassWithCompensation` (not `Pass`), with `applied.lag = 1`, one
  shift-exposed frame excluded per input, all stated in the summary and JSON,
  and printed by the snapshot assertion;
- the bit-exact tone with `allow_lag_compensation(4)`: verdict `Pass`, with
  `applied` as identity, because compensation is tried only after an
  uncompensated failure;
- a delayed render *and* a truncated tail: the latency what-if still reports
  the length difference that existed before the shift, so the explanation is
  `Unexplained` rather than `Latency`;
- a gain change of 0.1 dB, failing with a `Gain` explanation, then giving
  verdict `PassWithCompensation` with `allow_gain_compensation(0.2)` and the
  applied gain stated;
- polarity inversion: fails, explained as `Polarity`, never compensated;
- a click inserted at a known frame: fails with `first_divergence` and the
  worst window at that frame;
- a truncated tail and extra trailing samples: the length check plus
  residual energy;
- NaN and infinity in the actual buffer, and in the reference;
- silent reference with the relative tolerance (skip note), and silent
  actual against a non-silent reference;
- mismatched sample rate and channel count: failure with empty metrics.

Malformed WAV inputs are built as byte arrays in tests: truncated header,
oversized declared data chunk, zero channels, unsupported format code,
64-bit float, and an extensible header with an unknown sub-format GUID.

### 13.4 Integration tests

- **Snapshot workflow** in a temporary directory: missing baseline under `no`
  (fails, writes nothing), then `new` (writes a candidate and report),
  `accept` (installs the baseline), a re-run (passes and removes the stale
  candidate), a mismatch under `always` (replaces the baseline and prints a
  notice), and `CI=true` defaulting to `no`.
- **Duplicate-name detection** within one process.
- **CLI:** exit codes 0/1/2 for each command; `--json` output parsed and
  validated against the documented field set; `review` exit status with and
  without candidates.
- **Report format:** `insta` snapshots of the human summary and JSON for the
  fixture cases, so format changes are reviewed deliberately.

### 13.5 CI gates

On Linux, macOS and Windows, with stable Rust and the MSRV (1.85):

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace --no-default-features
cargo doc --workspace --no-deps            (RUSTDOCFLAGS="-D warnings")
cargo deny check
```

Lint configuration: `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]`
in both crates, plus the panic-related lints listed in §11 for the library.
No `unsafe` exception is foreseen.

## 14. Initial usable implementation slice

The first milestone delivers a small, complete tool useful without any
spectral analysis. Implementation order, where each step is independently
testable:

1. **Buffer and levels:** `AudioBuffer`, conversions, dB helpers.
2. **WAV I/O with limits:** read and write via `hound`, with malformed-input
   tests and a fuzz target.
3. **Core comparison without diagnosis:** structural checks, residual pass,
   all tolerances, windows, first divergence, `Report`, `Display`, JSON.
   This step alone is a usable tolerance-based comparator.
4. **Snapshots:** `SnapshotStore`, update modes, candidates, atomic writes,
   duplicate detection and the `assert_audio_snapshot!` macro.
5. **CLI:** `diff`, `review`, `accept`, `reject`.
6. **Diagnosis and compensation:** bounded lag search, gain and polarity,
   what-if passes, explanation, and opt-in compensation.
7. **Documentation:** crate docs with a worked example, a tolerance-selection
   guide (dither, integer baselines, FMA differences), and the
   demonstration fixture as a runnable example.

The milestone is complete when the fixture cases in §13.3 pass on all three
platforms, the snapshot workflow in §13.4 works end to end, and the CI gates
in §13.5 are green.

Step 3 comes before diagnosis on purpose. The plain tolerance comparator is
valuable on its own, and the diagnosis layer can be evaluated against it
(§16, Q1).

## 15. Deferred functionality and non-goals

### 15.1 Deferred: possible later work, not commitments

| Item | Notes |
|---|---|
| Band-limited (spectral) residuals | Per-band residual levels via `realfft` to tell broadband noise from a filter change; enables `AddedNoise`/`SpectralChange` explanations |
| FFT-based lag search | Removes the `K ≤ 4096` cap; same results as the direct method, enforced by property tests |
| Sub-sample lag estimation | Parabolic interpolation of the correlation peak, diagnosis only |
| More input formats | FLAC/AIFF via `symphonia`; lossless only |
| Per-platform baseline variants | For example `name.<target_arch>.wav` looked up before `name.wav`, for outputs that legitimately differ by architecture (§16, Q5) |
| Nondeterminism check | Render twice and compare, flagging uninitialised state or thread-order dependence |
| Visual reports | HTML with waveform and spectrogram difference images, as CI artifacts |
| Framework adapters | Thin helpers for [nih-plug](https://github.com/robbert-vdh/nih-plug), [fundsp](https://github.com/SamiPerttu/fundsp) and CLAP test hosts, as separate crates |
| Policy files | `--policy policy.toml` for the CLI, using the `Policy` serde form |
| Interactive review | Terminal UI similar to `cargo insta review` |
| Parallel analysis | Per-channel parallelism for long multichannel files |
| MIDI-driven render fixtures | Belongs in adapters, not the core |

### 15.2 Non-goals

- Rendering audio, hosting plugins or controlling DAWs.
- Perceptual or psychoacoustic quality metrics (PEAQ, ViSQOL, POLQA) and
  "audibility" claims. Tolerances are signal measurements chosen by the user.
- Resampling, sample-rate conversion, or channel remixing to make
  mismatched inputs comparable.
- Lossy formats as baselines.
- Fingerprinting, similarity search or loudness compliance (EBU R128 and
  similar standards have dedicated tools).
- A test runner or test-discovery mechanism.

## 16. Open technical questions

Each question has a recommended default that implementation can proceed
with if no other decision is made.

| # | Question | Recommended default |
|---|---|---|
| Q1 | Do lag/gain diagnosis and the report add enough over "absolute tolerance plus regenerate" to justify their complexity? No ablation on real projects' golden tests has been run. | **The plain comparator is built first** (step 3 in §14), and is a complete, useful tool on its own. **Diagnosis is a removable module:** it is separate, runs only on failure, and never affects the verdict. Evaluate it against real-world golden tests during implementation. If it adds little, ship the comparator and snapshot flow without it. `Report::diagnosis: Option<_>` and the separate `allow_*_compensation` settings allow that without breaking the API |
| Q2 | Default tolerances | Residual peak ≤ −80 dBFS and residual RMS ≤ −90 dBFS per channel. This tolerates float reordering, FMA differences and 16-bit TPDF dither (about −95 dBFS RMS), and fails audible changes. Document it as a starting point rather than an audibility guarantee |
| Q3 | Snapshot location: manifest-relative directory or next to the test source file (the `insta` convention) | `<CARGO_MANIFEST_DIR>/tests/audio_snapshots/`. Binary files are easier to manage, `.gitattributes`/LFS rules stay simple, and the location is the same for unit and integration tests. Revisit if users ask for per-module directories |
| Q4 | Default baseline sample format | 32-bit float, for a bit-exact round trip. 24-bit and 16-bit are opt-in with a quantization report |
| Q5 | Per-platform baseline variants | Not in the first milestone. Recommend tolerances that absorb cross-platform float differences. If a real case needs variants, add architecture-suffixed lookup (§15.1) |
| Q6 | Lag search method and limit | Direct time-domain correlation with `K ≤ 4096` and a 2^20-frame analysis window. Add an FFT path only when a user needs larger ranges, behind the same internal function |
| Q7 | Should `WAVEASSERT_UPDATE=always` make the updating run pass? | Yes, with a stderr notice per baseline (matches `insta`'s `always`). The explicit variable is the deliberate act, and the change is visible in version control before commit |
| Q8 | Channel limit default | 64 channels (`Limits::default`). This covers 3rd-order ambisonics (16) and large beds, and is configurable |
| Q9 | Duplicate snapshot names under `cargo nextest` (one process per test) | Document the limitation. Optionally add a `waveassert review --check-duplicates` scan of test sources later. Do not add cross-process locking |
| Q10 | Should RMS over the compared range include zero padding? | Yes (§7.2), so length differences are visible. The alternative (overlap only) is not offered |
| Q11 | JSON `−∞` representation | `null` for dB fields, with the linear values always present (§8.2) |
| Q12 | Edge rule for shifted comparisons: what happens to frames exposed by a lag what-if or by permitted lag compensation | Exclude only the shift-exposed frames (at most `|L|` per input, §7.1), and count and report them in the diagnosis or in `applied.excluded` and a note. Plain comparison keeps full zero padding. Rejected alternatives: comparing exposed frames with silence (every pure latency change would look unexplained), and trimming both inputs to the overlap (that would hide genuine length differences) |
