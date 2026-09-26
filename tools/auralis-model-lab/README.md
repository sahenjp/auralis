# Auralis model lab

This pinned Python environment is an offline research adapter, not production
audio code. It runs the official GTCRN and UL-UNAS streaming ONNX graphs one
spectral frame at a time with one ONNX Runtime CPU thread.

The complete measured path is:

```text
48 kHz WAV -> polyphase downsample -> 16 kHz STFT -> stateful ONNX frames
           -> ISTFT -> polyphase upsample -> 48 kHz WAV
```

The JSON report keeps downsampling, frontend, inference, synthesis, and
upsampling wall time separate. The SciPy batch resamplers compensate their FIR
group delay, so their timings are suitable for offline quality screening but
their algorithmic latency is explicitly `null`. This adapter therefore cannot
be used as production-latency evidence.

Generated processing, bake-off, metric, and transition reports include a
`benchmark_code` object. When
`AURALIS_GIT_REVISION` is set its value is retained; otherwise a SHA-256 of the
Python source tree plus `pyproject.toml` and `uv.lock` is used as the revision
surrogate. Existing historical reports may predate this field and are not
rewritten.

The relative-path commands below assume the current directory is
`tools/auralis-model-lab`.

Create the locked environment and run one candidate:

```bash
uv sync --python 3.12 --frozen
uv run --frozen auralis-model-lab process \
  --candidate gtcrn \
  --model /path/to/gtcrn_simple.onnx \
  --input /path/to/input-48k-mono.wav \
  --output /path/to/output-48k-mono.wav \
  --report /path/to/runtime.json
```

The ONNX graphs and their SHA-256 values must match
`bench/candidates/frozen-set-v1.json`. This tool does not download models.

Fetch the pinned FLEURS clean-speech subset with revision and per-file hash
verification:

```bash
uv run --frozen auralis-model-lab fetch-fleurs \
  --spec ../../bench/manifests/fleurs-clean-core-v2.source.json \
  --out-dir ../../bench/work/corpus/fleurs-clean-core-v2
```

The fetcher verifies the upstream `main` revision both before and after the
download. Its manifest records source bytes, deterministic 48 kHz derivatives,
transcripts, row indexes, and hashes. FLEURS does not label microphone distance
or recording-room conditions, so Auralis does not infer those attributes.

Fetch the pinned six-category DEMAND real-noise subset:

```bash
uv run --frozen auralis-model-lab fetch-demand \
  --spec ../../bench/manifests/demand-noise-core-v1.source.json \
  --out-dir ../../bench/work/corpus/demand-noise-core-v1
```

This verifies the Zenodo record metadata and every archive's size and published
MD5, then records a SHA-256 as well. It retains the exact first-channel member,
the selected 16 kHz segment, and a deterministic 48 kHz derivative. The six
official DEMAND environment labels are not relabeled as noise types that the
dataset does not claim to contain.

Fetch the pinned VoiceBank clean-speech subset (the archive cache is separate
from the derived corpus):

```bash
uv run --frozen auralis-model-lab fetch-voicebank \
  --spec ../../bench/manifests/voicebank-fullband-clean-v1.source.json \
  --out-dir ../../bench/work/corpus/voicebank-fullband-clean-v1 \
  --cache-dir ../../bench/work/corpus/cache/voicebank
```

Build the frozen cross-product at +10, +5, 0, -5, and -10 dB, plus clean input:

```bash
uv run --frozen auralis-model-lab mix-corpus \
  --clean-manifest ../../bench/work/corpus/fleurs-clean-core-v2/manifest.json \
  --clean-manifest ../../bench/work/corpus/voicebank-fullband-clean-v1/manifest.json \
  --noise-manifest ../../bench/work/corpus/demand-noise-core-v1/manifest.json \
  --out-dir ../../bench/work/corpus/auralis-m3-offline-core-v3 \
  --corpus-id auralis-m3-offline-core-v3
```

The mix manifest preserves the untouched clean condition and, for every noisy
case, the exact scaled clean reference, noise reference, noisy input, gains,
requested/measured SNR, clipping count, and hashes. One common peak gain is
applied to both components only when their sum would exceed 0.98.

Generate the deterministic transition corpus from the same clean/noise sources:

```bash
uv run --frozen auralis-model-lab generate-transient-corpus \
  --spec ../../bench/manifests/transient-core-v1.spec.json \
  --clean-manifest ../../bench/work/corpus/fleurs-clean-core-v2/manifest.json \
  --clean-manifest ../../bench/work/corpus/voicebank-fullband-clean-v1/manifest.json \
  --noise-manifest ../../bench/work/corpus/demand-noise-core-v1/manifest.json \
  --out-dir ../../bench/work/corpus/auralis-m3-transient-core-v1
```

Run the reproducible ONNX bake-off (one fresh output directory per candidate;
the command verifies the candidate artifact against the frozen set):

```bash
for candidate in gtcrn ul-unas; do
  case "$candidate" in
    gtcrn) model=/path/to/gtcrn_simple.onnx ;;
    ul-unas) model=/path/to/ulunas_stream_simple.onnx ;;
  esac
  uv run --frozen auralis-model-lab bakeoff \
    --candidate "$candidate" \
    --model "$model" \
    --candidate-set ../../bench/candidates/frozen-set-v1.json \
    --corpus-manifest ../../bench/work/corpus/auralis-m3-offline-core-v3/manifest.json \
    --out-dir "../../bench/work/processed/${candidate}-official-centered-core-v3"
done
```

The DeepFilterNet and RNNoise runs use their pinned official/native adapter
commands (`bakeoff-deepfilter` and `bakeoff-rnnoise`) because their upstream
tools have different input contracts. Analyze every output with
`analyze-bakeoff`, then use `summarize-bakeoff` to retain the per-candidate
metrics and alignment metadata; all output directories must be new or empty.

## Fast blind quality rating

Create and validate the 16-case gate from the repository root. The public
session contains only anonymous A-E WAV files; the candidate mapping stays in
the separate private key.

```bash
cd tools/auralis-model-lab
uv run --frozen auralis-model-lab create-rating-session \
  --spec ../../bench/manifests/quality-gate-v1.spec.json \
  --session-dir ../../bench/work/listening/sessions/quality-gate-v1 \
  --private-key ../../bench/work/listening/private/quality-gate-v1-key.json \
  --session-id quality-gate-v1 \
  --seed-hex a120260831
uv run --frozen auralis-model-lab validate-rating-session \
  --session-dir ../../bench/work/listening/sessions/quality-gate-v1 \
  --private-key ../../bench/work/listening/private/quality-gate-v1-key.json \
  --report ../../bench/work/listening/validation/quality-gate-v1.json
```

Run the keyboard-only player and rating flow:

```bash
uv run --frozen auralis-model-lab run-rating-session \
  --session-dir ../../bench/work/listening/sessions/quality-gate-v1 \
  --results-jsonl ../../bench/work/listening/results/quality-gate-v1.jsonl \
  --listener-id local-user
```

Press `A`-`E` to switch playback, `Enter` to rate, `1`-`5` for each score, and
`q` to save and exit. Results are append-only JSONL and never contain candidate
names. Audio is copied without normalization or delay compensation.
