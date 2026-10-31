# WaveAssert

**Snapshot regression testing for rendered audio in Rust.**

WaveAssert (crate and binary name `waveassert`) is a planned Rust library and small command-line
tool for checking that rendered audio still matches a stored reference within
explicit, documented tolerances, and for explaining *why* it does not when a
check fails.

> **Status: design stage. Nothing described here is implemented yet.**
> This repository currently contains this README, the
> [architecture design](docs/architecture.md) and the
> [implementation roadmap](docs/roadmap.md). There is no code, no published
> crate and no stable API. All code and command examples below are
> **proposed** and may change during review.

---

## The problem

Authors of synthesizers, audio effects, codecs, game-audio engines and other
DSP code need regression tests that answer one question: *did this change
alter the rendered output beyond an acceptable tolerance?*

The common approaches all have well-known problems:

- **Byte or hash comparison** (`cmp`, `md5sum`) fails on differences nobody can
  hear: dither, a different summation order, fused multiply-add on another CPU,
  or a compiler upgrade.
- **Hand-written comparators** inside each project (an absolute per-sample
  tolerance over an array) work, but every project rewrites them, and they
  report only "sample 18 231 differs by 0.0041" with no context.
- **Unexplained failures.** A one-sample latency change or a 0.1 dB gain change
  makes every sample differ. The test fails, but the failure message does not
  say that the whole difference is a delay or a level change.
- **Ad-hoc baseline updates.** Regenerating reference files is often a
  separate script, so it is easy to overwrite a baseline without looking at
  what changed.

`waveassert` aims to provide one small, framework-independent comparison core
with tolerances expressed in decibels, diagnostics that locate and explain
differences, and a deliberate snapshot update and review flow.

## Intended users and workflows

- **Plugin and DSP library authors** who render a fixed input through their
  processor in `cargo test` and want to compare it with a checked-in baseline.
- **Codec and audio-engine developers** who compare decoder or mixer output
  against reference renders produced by another build or platform.
- **CI maintainers** who need a command-line `diff` for two WAV files with a
  clear exit code and a machine-readable report.

Typical workflow:

1. A test renders audio into a buffer and calls the snapshot assertion.
2. The first run has no baseline: the test fails and writes a *candidate* file.
3. The developer reviews the candidate and explicitly accepts it. The baseline
   is committed alongside the test.
4. Later runs compare against the baseline. Inaudible differences inside the
   tolerance pass. Larger differences fail with a report that names the
   failing channel, the time window where the difference starts, and whether
   a latency or gain change explains it.
5. If the change is intended, the developer reviews and accepts the new
   candidate. Updates never happen implicitly, and never in CI unless
   explicitly requested.

## Example (proposed API, not implemented)

A snapshot test inside a Rust test suite:

```rust
// PROPOSED API: does not compile today.
use waveassert::{assert_audio_snapshot, AudioBuffer, Policy};

#[test]
fn hall_reverb_tail_is_stable() {
    let rendered: Vec<f32> = render_reverb_impulse_response(); // your code
    let buffer = AudioBuffer::from_interleaved(48_000, 2, &rendered).unwrap();

    let policy = Policy::default()
        .max_residual_peak_dbfs(-80.0)
        .max_residual_rms_dbfs(-90.0)
        .lag_search(64); // diagnose up to 64 frames of latency change on failure

    assert_audio_snapshot!("hall_reverb_tail", &buffer, &policy);
}
```

Suppose a change delays the output by one frame and the render length stays
the same. The test fails, and the report explains the cause. Because the
policy does not allow latency compensation, the latency change is
*diagnosed*, not *accepted*. When the diagnosis removes the delay, it leaves
out only the frames the shift exposes (one at each end here) and says so. The
figures below are illustrative, for a 440 Hz test tone at half scale:

```text
audio snapshot 'hall_reverb_tail' FAILED   (illustrative output)
  check                limit        ch0           ch1
  residual peak        -80.0 dBFS   -30.8 dBFS    -30.8 dBFS   FAIL
  residual RMS         -90.0 dBFS   -33.8 dBFS    -33.8 dBFS   FAIL
  first divergence     ch0, frame 1 (0.00002 s)
  diagnosis            actual is 1 frame later than the baseline; with that
                       delay removed the residual is -inf dBFS (identical),
                       excluding the 1 shift-exposed frame of each input.
                       Latency compensation is not allowed by this policy.
  candidate written    tests/audio_snapshots/hall_reverb_tail.new.wav
```

Comparing two files from a shell or CI job:

```console
$ waveassert diff reference.wav actual.wav --max-residual-rms -90 --search-lag 64 --json
$ echo $?    # 0 = within tolerance, 1 = difference found, 2 = usage or input error
```

## Planned capabilities

First milestone (see [Initial implementation slice](docs/architecture.md#14-initial-usable-implementation-slice)):

- An in-memory `AudioBuffer` with explicit sample rate, channel count and
  frame count. Sample formats are converted to `f32` by documented rules.
- WAV reading and writing (16/24/32-bit integer PCM and 32-bit float) with
  resource limits for untrusted files.
- Per-channel peak, RMS and residual (difference) metrics, in linear units and
  in dB.
- Configurable tolerances: residual peak, residual RMS, residual relative to
  the reference level, peak and RMS level deltas, and optional per-window
  residual limits.
- Structural checks: sample rate, channel count, length, and NaN or infinite
  samples.
- Bounded integer-lag (latency) and gain/polarity estimation for diagnosis.
  Compensation is applied only when the policy explicitly allows it, and
  within an explicit limit. A shifted comparison leaves out only the frames
  the shift exposes, and reports how many. A pass that needed compensation
  gets its own verdict (`PassWithCompensation`), stating what was compensated
  and by how much. It never looks like a plain pass.
- Time-windowed diagnostics: where the difference begins and which windows
  differ most.
- A snapshot assertion macro with an explicit update mode
  (`WAVEASSERT_UPDATE`), candidate files, and `review` / `accept` / `reject`
  commands.
- Human-readable failure messages and a versioned JSON report.
- A CLI with `diff`, `review`, `accept` and `reject`, using `cmp`-style exit
  codes.

Later, if the first milestone proves useful: band-limited (spectral)
residuals, more input formats, per-platform baseline variants, a
render-twice nondeterminism check, and visual difference reports. These are
described as deferred work in the architecture document. They are not
commitments.

## Non-goals

- Rendering audio, hosting plugins (VST3, CLAP, AU) or driving a DAW. Your test
  produces samples; `waveassert` compares them.
- Perceptual quality scoring (PEAQ, ViSQOL, POLQA) or "sounds the same"
  judgements. Tolerances are signal-level measurements.
- Resampling or sample-rate conversion. A sample-rate mismatch is a failure.
- Audio fingerprinting or similarity search.
- Lossy formats (MP3, AAC, Vorbis, Opus) as baselines.
- Replacing a test runner. `waveassert` works inside `cargo test` and
  `cargo nextest`.

## Technical overview

- **Language:** Rust (stable, edition 2024, MSRV 1.85), `#![forbid(unsafe_code)]`.
- **Layout:** a library crate (`waveassert`) and a separate CLI crate
  (`waveassert-cli`, installing a binary named `waveassert`), so library users
  do not pull in CLI dependencies.
- **Pipeline:** load or construct buffers → validate structure → compute the
  residual (with any compensation the policy allows) → evaluate tolerances
  → run diagnostics only when needed → build an immutable report.
- **Determinism:** fixed-order `f64` accumulation and tolerance checks done
  in linear units, so verdicts do not depend on platform `log10`
  precision.
- **Dependencies (planned):** [`hound`](https://crates.io/crates/hound) for
  WAV, [`serde`](https://serde.rs) / `serde_json` for reports,
  [`thiserror`](https://crates.io/crates/thiserror) for errors,
  [`tempfile`](https://crates.io/crates/tempfile) for atomic baseline writes, and
  [`clap`](https://crates.io/crates/clap) in the CLI only.

The full design covers requirements, the domain model and its invariants, algorithms,
file formats, error model, testing strategy and open questions. See
**[docs/architecture.md](docs/architecture.md)**.

## Project maturity

| Area | State |
|---|---|
| Architecture design | Approved |
| Implementation roadmap | Draft, awaiting review |
| Library code | Not started |
| CLI | Not started |
| Tests and fixtures | Not started |
| Published crate | None. `waveassert` is not yet published on crates.io |
| Licence | Not yet selected |

## Contributing

The project is not yet accepting code contributions; the design is under
review. Comments on the architecture document are welcome once the
repository is published.
