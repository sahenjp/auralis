# ADR 0004: Benchmark gate before model adoption

- Status: accepted
- Date: 2026-08-26

## Problem

Model reputation and isolated demo clips cannot establish naturalness, suppression, realtime performance, license suitability, or superiority.

## Constraints

The product must handle stationary, transient, music, echo, reverberation, and interfering-speaker cases on CPU. Some metrics and corpora have redistribution restrictions.

## Alternatives

- Select one popular model immediately.
- Optimize subjective demos only.
- Establish a versioned corpus, systems harness, objective metrics, runtime metrics, and blind listening gate first.

## Decision

Adopt the benchmark gate. The first harness proves deterministic mixing, identical input routing, result schemas, and timing. Model candidates are retained only after measured improvements and license review.

## Consequences

Initial progress appears less dramatic but remains comparable. Synthetic smoke fixtures are not treated as speech-quality evidence. Captured and blinded tests are mandatory before competitive claims.

## Evidence

The Microsoft DNS Challenge uses P.835 subjective dimensions and states that human subjective evaluation is the gold standard; DNSMOS is a proxy.

