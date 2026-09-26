# ADR 0001: Hybrid implementation stack

- Status: accepted
- Date: 2026-08-26

## Problem

Auralis needs a memory-safe, low-overhead realtime core, direct Windows audio integration, mature C/C++ DSP integration, and a productive model research environment.

## Constraints

Windows 11 is primary; CPU realtime is mandatory; callbacks cannot allocate or block; WDK and WebRTC are C/C++ ecosystems; model families must remain replaceable.

## Alternatives

- C++ everywhere: direct ecosystem fit, largest memory/concurrency safety surface.
- Rust everywhere: strong safety, but forces awkward WDK/AEC3 integration.
- Python/managed runtime: productive experiments, unacceptable callback/runtime predictability.
- Hybrid: Rust core and product control, narrow C/C++ platform/DSP adapters, Python research tools.

## Decision

Use the hybrid option. Start the portable engine in Rust. Use C/C++ only behind explicit adapters when platform libraries justify it. Keep Python out of the shipping realtime path.

## Consequences

FFI boundaries need ownership/threading tests and build integration. The core remains model- and backend-neutral. There is no language-purity goal.

## Evidence

CPAL supports WASAPI and device enumeration; Microsoft’s WDK audio samples and WebRTC AEC3 are C++. Links are collected in `docs/architecture.md`.

