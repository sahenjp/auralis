# ADR 0002: Bounded fixed-frame realtime pipeline

- Status: accepted
- Date: 2026-08-26

## Problem

Capture/output deadlines must not inherit unpredictable neural or DSP execution time, while latency and overload remain visible.

## Constraints

Callbacks cannot block, allocate, log, or run inference. Memory and latency must be bounded. Processing models commonly use 10 ms hops at 48 kHz.

## Alternatives

- Process everything in callbacks: minimum queueing, missed deadlines couple directly to audio glitches.
- Blocking channels: easy coordination, unbounded callback wait.
- Unbounded queues: avoid drops temporarily, permit runaway latency/memory.
- Fixed frames over wait-free bounded SPSC queues: explicit latency and overload.

## Decision

Use mono 48 kHz `f32`, 480-sample frames, one SPSC queue on each side of one processing worker, and four-frame default capacity. Drop newest on overrun; output silence on underrun; atomically count both.

## Consequences

Device-rate mismatch and clock drift need a later asynchronous resampler/controller. The queue’s capacity is not permission to operate four frames deep. Processing and model lookahead must fit the latency budget.

## Evidence

`rtrb` documents fixed construction-time allocation and wait-free, lock-free reads/writes. Model framing will be revisited only with measured evidence.

