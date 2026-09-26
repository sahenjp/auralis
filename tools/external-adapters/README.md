# External candidate adapters

These files are reproducibility inputs for local-only Milestone 3 evaluation.
They do not vendor candidate source or model weights.

## DeepFilterNet per-file reset

The official `deep-filter` CLI reuses one stateful `DfTract` across every input
file. That leaks recurrent/STFT state between independent corpus cases. Apply
`deepfilternet-reset-per-file.patch` to the frozen DeepFilterNet checkout before
building the evaluation binary:

```bash
git -C /path/to/DeepFilterNet apply \
  /path/to/Auralis/tools/external-adapters/deepfilternet-reset-per-file.patch
cargo build --manifest-path /path/to/DeepFilterNet/Cargo.toml \
  --release -p deep_filter --bin deep-filter \
  --no-default-features --features bin,tract,wav-utils,transforms
```

The patch clones one never-processed `DfTract` before each mono input. Validate
the adapter by processing the same WAV through two differently named symlinks
in one invocation and requiring byte-identical output hashes. The patch does not
resolve model-weight redistribution uncertainty; model artifacts remain local.
