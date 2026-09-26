# ADR 0006: Decouple device callback periods from processing frames

- Status: accepted
- Date: 2026-08-26

## Problem

The DSP contract uses exactly 480 samples at 48 kHz, while a Windows endpoint
may invoke capture and render callbacks with a different or varying frame
count. Treating 10 ms as both contracts makes ordinary callback batching look
like an xrun and makes the model dictate hardware scheduling.

## Decision

Keep 480 samples as the processing frame and accept arbitrary positive callback
frame counts at both device boundaries. Use construction-time fixed buffers to
split and merge frames. The callback path remains bounded and performs only
conversion, fixed-size copies, SPSC operations, and atomic metric updates.

Use the backend's default buffer request unless a characterization run
explicitly supplies another supported size. Record requested buffer size,
actual callback histogram, actual stream buffer size, and WASAPI engine period
as distinct values.

## Consequences

The processing engine remains independent of WASAPI periods. Irregular callback
sequences can be tested deterministically. A larger callback can require more
than one pre-roll processing frame, and that cost must appear in latency
accounting.
