# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Custom `resource_attributes` field on `TelemetryConfig` for user-defined OTLP resource key-value pairs
- Test verifying custom resource attributes appear in exported trace protobuf

## [0.9.0] - 2025-04-15

### Added

- Configurable `BackpressureStrategy` (drop / block) for the exporter
- Vector-based end-to-end integration tests
- TLA+ formal specifications for exporter and metrics registry
- cargo-fuzz targets for hex, trace, and log encoding
- Proptest strategies for trace, log, and hex encoding
- Kani proof harnesses for the metrics module
- Remaining practical tests for v1.0.0 readiness
- Verification coverage SVG diagram

### Changed

- Consolidated project layout and applied diagram design system
- Replaced plotters dependency with hand-crafted SVG for architecture diagram
- Excluded non-essential files (`.agent-prompts/`, `.claude/`, `docs/prds/`) from crate package

### Fixed

- Tightened Kani proof harness bounds for tractable verification

## [0.5.1] - 2025-03-20

### Changed

- Updated crates.io README

## [0.5.0] - 2025-03-19

### Added

- Histogram instrument with client-side bucketing
- Exemplar support on Counter, Gauge, and Histogram
- Realistic e-commerce benchmark suite
- `Arc<Vec>` zero-copy collect path and SVG baseline charts
- Configurable cardinality limiting for metrics
- Optimized hashing (blake3) for metric label sets
- Allocation scaling benchmark with SVG table visualization
- 8/10/16-attribute counter and histogram comparison benchmarks
- Baseline and diff flamechart generation

### Changed

- **Breaking:** `TelemetryConfig` fields now accept `String` instead of `&'static str`
- Renamed crate from `ro11y` to `rolly`
- Moved benchmark-methodology.md to `docs/`
- Use Criterion as single source of truth for benchmark methodology

## [0.4.0] - 2025-03-15

### Added

- Deterministic head-based trace sampling with configurable `sampling_rate`
- Kani proofs, proptest, and wire-compatibility test infrastructure

## [0.3.0] - 2025-03-12

### Added

- Native OTLP metrics with Counter and Gauge instruments
- Metrics benchmarks for encoding, counter/gauge ops, and collect
- Proto/trace/log encoding benchmarks

### Changed

- **Breaking:** metrics encoding uses hand-rolled protobuf (no `prost` at runtime)

## [0.2.0] - 2025-03-10

### Changed

- **Breaking:** independent trace and log export configuration — separate endpoint URLs for traces and logs

## [0.1.3] - 2025-03-08

### Added

- Shared field/metric constants module
- Tower middleware feature-gated behind `cfg(feature = "tower")`

### Changed

- Renamed `cf_id` to `request_id` and made `hex_encode` public

## [0.1.0] - 2025-03-05

### Added

- Initial release of the rolly observability crate
- Hand-rolled OTLP protobuf encoding for traces and logs over HTTP
- `tracing`-based `OtlpLayer` subscriber
- Async batch exporter with configurable endpoints
- crates.io packaging and metadata
