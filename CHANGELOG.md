# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace with the `waveassert` library crate and the `waveassert-cli`
  crate that installs a binary named `waveassert`; both are compiling
  placeholders.
- Lint configuration for both crates, a `cargo deny` policy for advisories,
  licences, bans and sources, and the CI quality gates.
- `scripts/check.sh`, which runs every gate locally.
