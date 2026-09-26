# Benchmark data area

See [`docs/benchmark-methodology.md`](../docs/benchmark-methodology.md) and the
[model-lab reproduction commands](../tools/auralis-model-lab/README.md). Audio
files and generated result files are ignored by Git by default because
provenance, privacy, and licensing must be reviewed before inclusion.

Run the dependency-free corpus smoke path through the Rust tool:

```bash
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke
```

Committed metadata for a real frozen corpus must include licenses and content hashes. Do not place private microphone recordings here without explicit consent and a retention policy.
