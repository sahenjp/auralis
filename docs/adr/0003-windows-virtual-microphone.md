# ADR 0003: Windows virtual microphone direction

- Status: proposed; implementation deferred to Phase 7
- Date: 2026-08-26

## Problem

Discord, Zoom, Teams, browsers, games, and recording applications must see `Auralis Microphone` as an ordinary capture endpoint.

## Constraints

Compatibility, latency, signing, HLK, installation safety, recovery, and maintenance matter. DSP/ML does not belong in kernel mode. Development must not install unsigned drivers.

## Alternatives

- Application-only WASAPI bridge: useful for monitor output, does not itself expose a new system capture endpoint.
- APO: user-mode DSP integrated with an associated audio endpoint/driver; useful but distribution and endpoint association do not by themselves provide an independent product endpoint.
- WaveRT virtual audio endpoint derived from the SysVAD architecture: normal endpoint semantics, highest signing/packaging/testing burden.
- Third-party virtual cable dependency: fastest prototype, weak product control and supportability.

## Decision

Prototype a minimal signed WaveRT virtual capture endpoint whose only job is bounded transport to/from the unprivileged Auralis engine. Keep all enhancement in user mode. Before implementation, validate IPC latency, failure behavior, signing path, HLK plan, and whether a supported user-mode-only endpoint option has appeared.

## Consequences

Phase 1 outputs to a normal render device and proves the engine only. Phase 7 needs Windows-native C/C++/WDK work, a dedicated test machine, signed packages, and production changes beyond copying SysVAD.

## Evidence

Microsoft’s SysVAD sample exposes WDM virtual audio devices and explicitly requires trusted signing for normal loading. Microsoft documents APOs as effects associated with audio endpoints and drivers.

