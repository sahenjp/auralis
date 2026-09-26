# ADR 0007: Explicit minimal startup pre-roll

- Status: accepted
- Date: 2026-08-26

## Problem

Starting render immediately caused a startup underrun before capture and the
worker could produce a complete frame. Enlarging every queue would hide the
ordering problem and add steady-state latency.

## Decision

Render waits for an explicit pre-roll target. In automatic mode, the first
render callback latches `ceil(callback_frames / 480)`, capped by the existing
output queue capacity. An operator may request a fixed target for experiments.
During pre-roll, render writes silence and increments dedicated pre-roll
metrics; it does not increment underrun metrics. After startup, empty output is
an underrun.

## Consequences

Startup is deterministic and separately observable. The measured target is
included in the latency breakdown. No queue capacity was increased. A callback
larger than total queue capacity remains a configuration limit and is reported
rather than silently hidden.
