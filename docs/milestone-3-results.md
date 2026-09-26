# Milestone 3 — Reproducible denoiser bake-off and first production engine

Measurement and artifact dates: 2026-08-27 to 2026-09-17. This document is the technical
decision record for the current uncommitted Auralis workspace. It is not a
claim about physical microphone-to-speaker latency or about superiority over
any commercial product.

## Executive technical summary

Four candidate families passed the initial evidence gate and were evaluated on
the same frozen 48 kHz corpus: DeepFilterNet3-LL, GTCRN, UL-UNAS, and RNNoise.
The deterministic benchmark controls passed, the offline corpus contained 644
cases, and the transition corpus contained 36 cases expanding to 160 measured
events per candidate. The raw outputs, hashes, metrics, alignment reports, and
runtime artifacts remain under `bench/work/`.

Milestone status is **provisional local acceptance**: the measurements support
UL-UNAS as the current technical choice for the Auralis `balanced` engine, but
weight licensing, human listening, complete historical host provenance, and
physical latency remain release blockers.

The current frozen corpus is reproducible but incomplete against the requested
coverage envelope: it lacks independent recordings for several noise categories
and controlled microphone-distance labels; those gaps are called out below.

UL-UNAS had the best aggregate noisy-corpus SI-SDR and the only positive
aggregate STOI change in this bake-off, while its complete 48 -> 16 -> 48 kHz
path remained
well below realtime CPU demand. Its official streaming ONNX adapter completed
the recorded native Windows 60-second, 10-minute, and 30-minute runs with zero
underruns, overruns, xruns, deadline misses, and inference failures. The
30-minute run used the unchanged Milestone 2 queue capacity and target fill.

This is a provisional technical selection, not a redistributable release
approval. The source repositories are permissively licensed, but none of the
four frozen model artifacts has a separately explicit weight license in the
evidence collected here. UL-UNAS therefore remains blocked for shipping until
the upstream weight terms are clarified. RNNoise remains the lightweight
48 kHz reference/fallback. DeepFilterNet3-LL is rejected for the first live
round because its native Windows run produced 43 input overruns, one processing
deadline miss, and a 470.449 ms processing maximum in only 10 seconds. GTCRN
remains an offline comparison candidate and was not integrated into the live
pipeline.

The blind listening harness is implemented and its 16-trial core, quality-gate,
and transient session integrity checks pass, but no human response JSONL has
been recorded. Consequently the decision is evidence-backed but not a final
subjective preference result. Physical acoustic/device end-to-end latency also
remains unknown.

## Evidence inventory

The machine-readable realtime roll-up is
[`bench/raw/realtime/milestone-3-realtime-summary-v1.json`](../bench/raw/realtime/milestone-3-realtime-summary-v1.json).
It deliberately uses `null` for CPU model, SIMD feature, affinity, and OS-build
host fields that the historical native runs did not record. A separately
timestamped host snapshot from the same Windows machine and a new 60-second
UL-UNAS run are retained at
`bench/raw/realtime/native-host-snapshot-2026-09-05.json`; that sidecar is not
retroactively attached to the older 30-minute artifacts. Candidate raw
artifacts retain runtime version/build, execution provider, and requested thread
settings; no missing host value is inferred.

Important source artifacts include:

| Evidence | Path | SHA-256 |
|---|---|---|
| Gate A candidate freeze | `bench/candidates/frozen-set-v1.json` | `c25f7bd16b0682cb49fc7deb7b9e41c820ade2f20979c519057e9ab7bf182234` |
| License audit snapshot | `bench/candidates/license-audit-2026-09-05.json` | `7a52d5f9c3ae0d89c1c5b64589b8c0edd89ee5d8ece570cee7b04735c04b78ec` |
| Gate C finalist scope | `bench/candidates/gate-c-finalists-v1.json` | `74624295b59bbbed8847b5123b10742b2cf60b5da751c73f04b19bd5447c25dd` |
| Benchmark foundation controls | `bench/work/foundation-m3-final/results/foundation.json` | `42be3f394d8707acd296f989a5b6e8c5bd413ffa5a12380e2ed5ada10493be5f` |
| Offline aggregate | `bench/work/metrics/offline-core-v3-summary.json` | `0bd01f12847e487ca033f2f04525c1253935fb4b9a6a78e192b4f77963c2838a` |
| Offline repeat aggregate | `bench/work/metrics/offline-core-v3-repeat-summary.json` | `c779177a2ea29cc0647ed068c80aa74e413fd0f0c0ea76368033a72fddd61875` |
| Native Milestone 2 reference | `bench/work/native-windows-30m-accepted.json` | `46a623945ef7b7863efe9650b6543e98bfbe6047065f0b1f08a5e5be824443bc` |
| Native UL-UNAS 30 min | `bench/work/native-windows-ulunas-30m.json` | `b0e5035b7522a73234be565ae034e924d25470c01e26f7cce659dbd2d9441241` |
| Native UL-UNAS target-fill 2.0, 30 min | `bench/work/native-windows-ulunas-target-2.0-30m-followup.json` | `05a44efd6b41d7efadb1a7cc0c43f029d7cf6902c53e2ab4f9a043172d0b4bf1` |
| Native UL-UNAS target-fill 2.5, 30 min | `bench/work/native-windows-ulunas-target-2.5-30m-followup.json` | `16a0b5206617e6200e2748c73dfb825ba0a99b14944760e9cfb8e8d0b5642123` |
| Native RNNoise 10 min | `bench/work/native-windows-rnnoise-10m.json` | `aaabff253cf486106929df172df5239a3e6d9eaebee3f9ee9bc3c311ab934133` |
| Native RNNoise follow-up 10 s | `bench/work/native-windows-rnnoise-10s-followup.json` | `56aaecb6635940695a6673c8f7d10078667755da22b90fe6c224da2c7400c503` |
| Native RNNoise follow-up 30 min | `bench/work/native-windows-rnnoise-30m-followup.json` | `506984328a766e34d9080f13ff3d405fb996de6c8e2370f54f146624a1d5eaf2` |
| Native UL-UNAS follow-up 60 s | `bench/work/native-windows-ulunas-60s-followup.json` | `293a434a3309806c4f9d9a39ee7d19ecef0a3edaf556670ff9a13f2827c46213` |
| Native UL-UNAS host-envelope 60 s | `bench/work/native-windows-ulunas-host-envelope-60s.json` | `8a409a3bab55fd4f159e93d79f6441b0715a51e3a63ea1f7683880641630303a` |
| Windows host snapshot sidecar | `bench/raw/realtime/native-host-snapshot-2026-09-05.json` | `5c704e44af7303ff6f57e1f5701b67dac28f96c5b98961096da93d893fc008bd` |
| Native UL-UNAS requested pre-roll 0, 60 s | `bench/work/native-windows-ulunas-preroll0-60s.json` | `469db3692de2886fad41c0bc53129920591d30f71f3a82859bbb93bb23c56d53` |
| Native UL-UNAS GUI-path recheck, 60 s | `bench/work/native-windows-ulunas-gui-60s-20260916.json` | `7eeac78f017dfbe7ae07a96f057d6ce0560263dfa8ca46b1ecfbb9cb97e61dc8` |
| Realtime follow-up roll-up | `bench/raw/realtime/milestone-3-realtime-followup-2026-09-05.json` | `692daff2c66b52867e8e05edd797eedfda64c21dde5ce8061789eac9739a1230` |
| Model-memory method validation | `bench/raw/realtime/model-memory-followup-2026-09-05.json` | `536ca287914dc72a0ab60cf9454f213462f6e240a06726ba1b878e74b6a954e1` |
| Model-memory repeatability | `bench/raw/realtime/model-memory-repeat-2026-09-05.json` | `14d0c9341d21501aa906a93405c5910e3e55a871f9dcada2a233bded7f9cc9a4` |
| Realtime roll-up | `bench/raw/realtime/milestone-3-realtime-summary-v1.json` | `c8d043d4d98c83c7720a7e92e04f5812a8d704e263f2bfa8388358e2285b2d1d` |

Generated audio and detailed per-case JSON remain ignored working artifacts by
design. They are not replaced by the Markdown tables below.

## Gate A — candidate freeze

The frozen set is intentionally four families rather than a superficial list.
Every artifact was pinned to an upstream revision and SHA-256 before it entered
the bake-off. Source and model-weight terms were checked independently.

| Candidate | Upstream revision | Artifact / SHA-256 | Native signal contract | Streaming/runtime | Gate A license result |
|---|---|---|---|---|---|
| DeepFilterNet3-LL | `Rikorose/DeepFilterNet` `d375b2d8309e0935d165700c91da9de862a99c31` | `models/DeepFilterNet3_ll_onnx.tar.gz`, 36,359,660 bytes; `5998e58e8ba0e09bb76986ef97b84afa065a571ef282d4a1222f341e3251cf3a` | 48 kHz full-band; frame 960, hop 480; zero lookahead; declared structural delay 480 samples / 10 ms | Official Rust `libDF`/Tract; stateful; no resampling | Source MIT OR Apache-2.0; weight terms not separately stated; local evaluation only |
| GTCRN | `Xiaobin-Rong/gtcrn` `502ebfab64da7c4a9af78dcb9c6ceef1ebb01c73` | `stream/onnx_models/gtcrn_simple.onnx`, 535,190 bytes; `b4718df6228e7bdf1a8a435cf98f838636eb2fd331acabf86ba87c5192ebcb87` | 16 kHz, 8 kHz bandwidth; frame 512, hop 256; causal/no lookahead; 16 ms model hop | Official stateful ONNX; ORT CPU; 48/16/48 adapter required | Source MIT; weight terms not separately stated; do not redistribute |
| UL-UNAS | `Xiaobin-Rong/ul-unas` `00f7c700da43d38347f30a6ccebd86fcbc798e07` | `ulunas_onnx/onnx_models/ulunas_stream_simple.onnx`, 788,967 bytes; `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b` | 16 kHz, 8 kHz bandwidth; frame 512, hop 256; causal/no lookahead; 16 ms model hop; verified processor delay 2,208 samples / 46 ms at 48 kHz | Official stateful ONNX with explicit caches; ORT CPU; 48/16/48 adapter required | Source MIT; weight terms not separately stated; do not redistribute |
| RNNoise | `xiph/rnnoise` `70f1d256acd4b34a572f999a05c87bf00b67730d` | Official `rnnoise_data` archive, 58,603,099 bytes; `0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37` | 48 kHz full-band; 480-sample hop, 960-sample analysis window; verified processor delay 960 samples / 20 ms at 48 kHz | Native C library behind a narrow FFI; stateful; no resampling | Source BSD-3-Clause; archive has no independent weight license; local/reference use only |

The frozen UL-UNAS ID is `ul-unas-dns3-streaming-onnx`. Historical native
artifacts that report `ul-unas-official-streaming-onnx` are retained under the
identity alias recorded in `frozen-set-v1.json`; their hashes and measurements
are not rewritten.

The metadata records parameter counts and upstream MAC claims separately from
Auralis measurements. GTCRN reports approximately 48.2k parameters and 33M
MAC/s upstream; UL-UNAS reports approximately 171k parameters and 35M MAC/s
upstream. Those claims are not substituted for Auralis runtime measurements.

The dated license audit snapshot
`bench/candidates/license-audit-2026-09-05.json` records the exact checked
archive entries and public clarification-issue states. It confirms that the
source licenses are permissive, while no pinned model archive has an explicit
weight license; the resulting commercial-use and redistribution decision is
therefore `none_cleared_for_redistribution`.

### License matrix

| Candidate | Source-code license | Weight/artifact license | Commercial use | Redistribution |
|---|---|---|---|---|
| DeepFilterNet3-LL | MIT OR Apache-2.0 | Not separately stated in the pinned archive/repository evidence | Source permits it; weight clearance required | Do not redistribute before written clarification |
| GTCRN | MIT | Not separately stated | Source permits it; weight clearance required | Do not redistribute before written clarification |
| UL-UNAS | MIT | Not separately stated | Source permits it; weight clearance required | Do not redistribute before written clarification |
| RNNoise | BSD-3-Clause | Official generated-weight archive has no independent license file | Source permits it; weight clearance required | Do not redistribute before written clarification |

The model hash proves artifact identity, not legal permission. The frozen
metadata records the primary-source URLs and the exact uncertainty for each
candidate.

### Rejected at Gate A

- Standard-lookahead DeepFilterNet3 was not added as a fifth family. The official
  low-latency artifact is the relevant within-family representative.
- Third-party DeepFilterNet conversions were rejected because the official
  low-latency artifact is available with pinned provenance.
- GPU-only systems were rejected because a useful CPU baseline is mandatory and
  no additional primary-source candidate justified displacing a frozen family.

## Gate B — benchmark foundation

The dependency-free foundation harness passed all controls in
`bench/work/foundation-m3-final/results/foundation.json`:

| Control | Result |
|---|---|
| Identity passthrough | 48,000 samples; repeat hash identical; SI-SDR/SDR 186.54 dB |
| Known -6 dB gain | 48,000 samples; repeat hash identical; measured gain -6.0205999 dB |
| Known 20 ms delay | 48,000 samples; repeat hash identical; raw SDR 1.3785 dB and deterministic-latency-compensated SDR 186.4261 dB at offset 960 samples |
| Timestamp/frame sequence | 100 frames, strictly increasing 480-sample starts, final end 48,000 |
| Loudness handling | Expected and measured gain agree within 0.0001 dB |
| Intentional degradation detection | Passed for the delayed and gain fixtures |

The controls were run again through the same Rust command without overwriting
the earlier output. The offline summary was also regenerated. The only summary
hash difference is the expected absolute-versus-relative source path spelling;
all source report hashes and metric values are unchanged.

The paced CLI simulation also waits for each processed frame before consuming
the corresponding render frame. This keeps the device-free control deterministic
at shutdown and prevents a scheduler race from being misreported as a final
simulation underrun; it does not change the native callback architecture.

Reproduction commands from a clean checkout are:

```bash
cargo run -p auralis-bench -- foundation --out-dir bench/work/foundation-m3-final
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke-m3-final
```

Both commands refuse to overwrite a non-empty output directory.

## Corpus and benchmark methodology

The v3 offline manifest is
`bench/work/corpus/auralis-m3-offline-core-v3/manifest.json`:

The requested-category audit is machine-readable in
`bench/manifests/noise-coverage-m3-v1.json`. It intentionally marks missing
categories instead of treating synthetic transient controls as real recordings.

- 644 mono cases at 48 kHz;
- 14 clean speech sources and 9 noise paths: six real DEMAND environments and
  three deterministic equal-RMS combinations of those environments;
- clean speech plus +10, +5, 0, -5, and -10 dB mixtures;
- deterministic gain `clean_rms / (noise_rms * 10^(snr_db / 20))`;
- one common gain applied to both components only when the mixture peak exceeds
  0.98;
- original clean, noise, mixture, processed raw output, and hashes retained.

The speech set includes male/female voices, English and Japanese clips, quiet
and loud material, and dedicated consonant coverage. The selected clean
manifests do not provide microphone-distance labels, so close/medium/far-field
conditions are not claimed. The offline noise
paths are the six DEMAND labels (domestic living, domestic washing, office,
cafeteria, street traffic, and metro) plus three deterministic combinations.
The transient corpus separately contains deterministic keyboard-like clicks,
repeated keyboard impacts, desk knocks, and door slams. Independent recordings
for mouse clicks, fans, air conditioning, vacuum, dishes, rain, wind, crowd,
television, music, game/speaker leakage, and nearby conversation are not yet in
the frozen offline corpus and remain a coverage blocker. Source manifests retain
the provenance limitations instead of inventing microphone-distance labels.

The transition manifest is
`bench/work/corpus/auralis-m3-transient-core-v1/manifest.json`. It contains 36
cases and covers silence-to-click-to-speech, speech-click-speech, repeated
keyboard, desk knock, door slam, speech onset/stop during steady noise, and
quiet-to-loud-transient transitions. The expansion produces 160 measured event
windows per candidate.

### Alignment policy

Each output manifest declares its deterministic production/frontend delay. The
reference metrics retain both `raw_unaligned` and
`deterministic_latency_compensated` values. Compensation trims exactly the
declared offset and truncates the common tail; it never searches freely for a
best delay. The diagnostic command can run a bounded DC-removed
cross-correlation search, but its result is explicitly diagnostic and is not
used to conceal temporal deformation.

For the offline centered ONNX screening path, the 16 kHz model hop is 256
samples (768 samples when expressed at 48 kHz). SciPy batch resampling
compensates its FIR group delay, so the offline resampler algorithmic delay is
recorded as unknown for production rather than silently treated as zero. The
native UL-UNAS adapter independently measured 2,208 samples / 46.0 ms by a
constrained correlation test after warm-up (coefficient 0.910599). This is
production-path structural delay, not inference wall time.

### Metrics and limitations

The bake-off records SI-SDR, SDR, STOI, noise attenuation, speech projection
gain, clipping, clean-speech RMS/spectral distortion, band energy changes,
frame-gain stability, transition-window residuals, and runtime factors. PESQ
and DNSMOS were not added to the result truth because their dependency/model
licensing and version provenance were not frozen for this local run. STOI is an
intrusive intelligibility predictor, not a naturalness ground truth. No single
metric is treated as decisive.

## Offline objective results

The following values are means over the 630 noisy cases or 14 clean cases in
`offline-core-v3-summary.json`. They are Auralis measurements, not upstream
paper numbers.

### Noisy mixtures and clean preservation

| Candidate | Noisy SI-SDR (dB) | Noisy SDR (dB) | STOI delta | Noise attenuation (dB) | Clean SI-SDR (dB) | Clean STOI delta | Clean RMS change (dB) | Clean 8–20 kHz change (dB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| DeepFilterNet3-LL | 6.191 | 9.076 | -0.0518 | 9.070 | 23.628 | -0.1214 | -2.811 | -0.785 |
| GTCRN | 5.475 | 6.655 | -0.0060 | 6.655 | 18.678 | -0.0595 | -0.646 | -12.769 |
| RNNoise | 6.826 | 8.209 | -0.0175 | 8.202 | 13.141 | -0.1262 | -0.522 | -0.132 |
| UL-UNAS | **7.025** | 8.005 | **+0.0057** | 8.005 | 19.584 | -0.0760 | -0.395 | **-13.112** |

UL-UNAS is strongest on the aggregate noisy objective values, but its clean
8–20 kHz reduction is a real penalty. The result is not a free quality win:
the model is 16 kHz/8 kHz bandwidth and the high-frequency loss must remain in
the product decision.

The requested-SNR SI-SDR means for UL-UNAS were 13.027 dB at +10 dB, 10.493 dB
at +5 dB, 7.637 dB at 0 dB, 4.215 dB at -5 dB, and -0.245 dB at -10 dB. The
corresponding STOI deltas were -0.0322, -0.0123, +0.0113, +0.0299, and +0.0320.
The full per-SNR and per-case values remain in the machine-readable reports.

### Clean-speech regression details

Clean preservation is a separate scorecard, not a footnote to noise removal.
The frame-gain standard-deviation means were 11.687 dB (DeepFilterNet3-LL),
5.850 dB (GTCRN), 87.382 dB (RNNoise), and 6.589 dB (UL-UNAS). The RNNoise
value and its large transition-state excursions are why it is retained as a
lightweight control rather than the quality engine. All candidates had zero
clipped samples in the clean aggregate; the full clipping counts are retained
per case.

## Transient and state-transition results

The transition reports are `bench/work/metrics/*-transient-v1.json`. Selected
event-window speech SDR means (dB) are:

| Pattern | DeepFilterNet3-LL | GTCRN | RNNoise | UL-UNAS |
|---|---:|---:|---:|---:|
| speech → click → speech | 6.45 | 7.80 | 1.65 | **7.97** |
| speech + repeated keyboard | **10.75** | 8.37 | 5.86 | 8.90 |
| steady noise → speech onset | **5.33** | -2.55 | 0.00 | -3.54 |
| speech + door slam | 5.20 | 5.00 | 0.05 | **6.29** |

All four transition corpora reported zero clipped samples in the selected
windows. These local-energy/projection measurements expose onset damage,
release behavior, and suppression overshoot, but cannot label metallic noise,
pumping, or naturalness reliably. Representative raw transition WAVs are kept
under each candidate's processed manifest for the listening harness.

The Japanese sibilant and English `/s/`, `/f/`, `/th/`, `/t/`, `/k/` cases are
included in the dedicated consonant and listening manifests. No conclusion is
drawn from aggregate SI-SDR alone for those clips.

## CPU and realtime-factor measurements

Offline screening ran one ORT CPU execution thread and one inter-op thread in
the pinned Python environment. Complete-path mean RTFs (including the declared
resampling/frontend path) were:

| Candidate | Complete-path mean RTF | Model-operation median / p95 / p99 / max (ms) | Notes |
|---|---:|---:|---|
| DeepFilterNet3-LL | 0.2051 | not exposed by official CLI | 48 kHz full-band; complete batch wall time includes the official process |
| GTCRN | 0.04369 | 0.574 / 0.866 / 1.098 / 16.714 | WSL2 screening, ORT CPU |
| RNNoise | 0.04499 | per-file process timing only: 321.444 / 754.078 / 792.596 / 865.757 | PCM16 portable demo includes process startup |
| UL-UNAS | 0.05117 | 0.660 / 1.146 / 1.758 / 32.399 | WSL2 screening, ORT CPU, full 48/16/48 path |

The native Windows production measurements below supersede these WSL numbers
for acceptance. No discrete GPU is required or used by the accepted path.

## Gate C — finalist selection

Gate C retained only UL-UNAS and RNNoise for live integration:

- UL-UNAS: best aggregate noisy objective result, positive aggregate STOI
  delta, and a credible full-path CPU result; accepted as the quality finalist.
- RNNoise: native 48 kHz, small integration surface, and useful low-CPU
  reference; retained as the lightweight control.
- GTCRN: good clean preservation and low offline RTF, but lower aggregate
  noisy SI-SDR than UL-UNAS and no native Windows live evidence in this round.
- DeepFilterNet3-LL: full-band and attractive offline quality, but rejected for
  realtime after the native deadline/overrun failure described below.

This gate is not the final subjective or licensing decision.

## Common realtime interface and integration

`auralis-core` now exposes a model-independent `Denoiser` contract for native
rate, frame/hop, lookahead, algorithmic delay, statefulness, and reset
semantics. The product-facing `FrameProcessor` remains the only 48 kHz/480
sample boundary used by the audio engine. Model-specific code lives in
`crates/auralis-denoisers`:

- UL-UNAS uses the official ONNX graph, fixed IO binding, preallocated caches,
  fixed-capacity FIFOs, and Rubato FFT 48/16/48 resampling;
- RNNoise uses a narrow dynamically loaded C FFI with exclusive worker state;
- DeepFilterNet3-LL uses the pinned official `libDF`/Tract runtime and remains
  available for explicit experiments only.

Inference is performed by the processing worker. Capture and render callbacks
continue to contain only bounded format conversion, fixed-size copying,
wait-free queue operations, and atomic metrics. They do not allocate, block,
log, access the filesystem, or invoke inference. The stable CPAL/WASAPI engine
was not replaced. New CLI runtime reports use denoiser-runtime schema version 2
and include target OS/family/arch, pointer width, detected x86 SIMD features,
affinity-change status, and the
queried OS version; the saved native artifacts below predate these fields and
retain null historical host values, although their denoiser-runtime objects
retain provider, runtime build, and requested thread settings.
New CLI reports also record `measurement_started_unix_seconds` and copy
`AURALIS_GIT_REVISION` when the caller supplies it; the saved native artifacts
predate those provenance fields.
New CLI runs also record a same-process resident-memory delta around denoiser
load as `model_load_envelope_bytes` and `model_memory_measurement`. The
load-envelope value covers the model, runtime, and persistent adapter
allocations; `model_memory_bytes` remains null because a weight-only allocation
query is not available.

New Python model-lab processing, bake-off, metric, and transition reports now
include a `benchmark_code` object. A caller-supplied `AURALIS_GIT_REVISION` is
retained when present; otherwise the report uses a deterministic SHA-256 of the
model-lab source tree and lockfiles as a revision surrogate. The historical
Python artifacts used for this bake-off predate that field, so their missing
revision is recorded rather than retroactively inferred.

The historical native files listed below use denoiser-runtime schema version 1.
Their `software_latency.incremental_software_pipeline_latency_*` fields are the
legacy structural-path values captured before the paired-baseline distinction
was added; they are not measured baseline deltas. The normalized realtime
roll-up uses the current terminology and keeps the paired incremental value
explicitly unmeasured.

## Native Windows realtime results

All values below are read from the saved native JSON artifacts. `transport` is
the existing capture-frame-timestamp to output-dequeue measurement. It is not
physical E2E latency. `total software` adds the candidate's declared/verified
structural path delay and the separately measured drift-resampler delay; it is
the production software accounting value, not an acoustic measurement.

| Run | Duration | Target fill / pre-roll | Underrun / overrun / xrun | Queue mean (min–max) | Processing mean / max (ms) | Inference p50 / p95 / p99 / max (ms) | CPU avg | Peak RSS |
|---|---:|---|---|---|---:|---|---:|---:|
| Milestone 2 passthrough reference | 30 min | 3.5 / 3 frames effective | 0 / 0 / 0 | 3.499 (2.525–3.598) | 0.029 / 1.251 | n/a | 4.894% | 18,239,488 |
| UL-UNAS | 60 s | 3.5 / 1 | 0 / 0 / 0 | 3.162 (2.996–3.390) | see artifact | 1.01 / 1.53 / 1.94 / 3.221 | artifact | artifact |
| UL-UNAS follow-up | 60 s | 3.5 / 1 | 0 / 0 / 0 | 3.164 (2.996–3.394) | 1.088 / 3.955 | 1.60 / 2.14 / 2.63 / 3.856 | 15.574% | 43,503,616 |
| UL-UNAS | 10 min | 3.5 / 1 | 0 / 0 / 0 | 3.422 (2.258–3.854) | 0.804 / 4.136 | 1.20 / 1.50 / 1.76 / 4.061 | 11.797% | 43,716,608 |
| UL-UNAS | 30 min | 3.5 / 1 | 0 / 0 / 0 | 3.627 (1.996–3.904) | 0.730 / 4.612 | **1.10 / 1.35 / 1.56 / 4.513** | **11.024%** | **43,945,984** |
| UL-UNAS host envelope | 60 s | 3.5 / 1 | 0 / 0 / 0 | 3.843 (2.800–3.996) | 0.893 / 2.665 | 1.34 / 1.69 / 2.12 / 2.606 | 13.431% | 43,446,272 |
| UL-UNAS requested pre-roll 0 | 60 s | 3.5 / 0 (effective 3) | 0 / 0 / 0 | 3.162 (2.996–3.390) | 0.944 / 3.474 | 1.40 / 1.71 / 2.19 / 3.411 | 14.829% | 43,515,904 |
| RNNoise | 60 s | 3.5 / 1 | 0 / 0 / 0 | 3.163 (2.996–3.390) | see artifact | 0.65 / 0.76 / 0.87 / 3.984 | artifact | artifact |
| RNNoise | 10 min | 3.5 / 1 | 0 / 0 / 0 | 3.540 (1.996–3.721) | 0.635 / 4.008 | 0.65 / 0.76 / 0.87 / 3.984 | 10.683% | 28,655,616 |
| RNNoise follow-up | 10 s | 3.5 / 1 | 0 / 0 / 0 | 3.000 (2.996–3.010) | 0.604 / 6.875 | 0.58 / 0.78 / 1.00 / 6.875 | 11.443% | 28,667,904 |
| RNNoise | 30 min | 3.5 / 1 | 0 / 0 / 0 | 3.500 (2.996–3.598) | 0.567 / 5.021 | **0.56 / 0.75 / 0.92 / 4.968** | **10.566%** | **29,282,304** |
| DeepFilterNet3-LL | 10 s | 3.5 / 1 | 0 / 43 / 0 | not accepted | 2.591 / **470.449** | 2.07 / 2.50 / 2.69 / **470.400** | 28.844% | 133,369,856 |

The DeepFilterNet row's `overrun` is input overrun frames. It also recorded one
processing deadline miss; no longer soak was started after this failure.

The additional UL-UNAS 60-second run used the same stable 3.5-frame target and
also recorded zero losses, zero deadline misses, and zero inference failures. It
is retained as follow-up evidence, not treated as a replacement for the native
30-minute acceptance run.

UL-UNAS's native 30-minute inference histogram had 112,506 operations and zero
histogram overflow or failure observations. The saved result reports MMCSS
`Pro Audio` registration success and zero correction errors. RNNoise's 30-minute
follow-up also recorded zero callback loss, deadline misses, and xruns. It uses
a separately hashed locally rebuilt scalar DLL; its CPU/RSS values are retained
as a distinct runtime measurement rather than silently merged with the earlier
10-minute binary.

The separate host-envelope run used the same native Windows machine and pinned
UL-UNAS artifact; its host CPU/OS/SIMD details are in the sidecar listed above.
The paired executable predates the newer in-process host wrapper, so the sidecar
is deliberately not presented as retroactive provenance for the historical
30-minute runs.

On 2026-09-16 the current Windows release executable was run through the same
WASAPI path again. The 60-second muted UL-UNAS characterize artifact recorded
zero input overruns, output overruns, underrun callbacks, xruns, processing
deadline misses, and inference failures; inference p95 was 1.77 ms, average
CPU was 13.19%, peak RSS was 57,815,040 bytes, and measured software-pipeline
latency was 49.070 ms average / 52.783 ms maximum. The GUI was then started on
Windows loopback, enumerated the native devices, started the Balanced session,
reported a running state with zero underruns/xruns, and stopped cleanly. That
short GUI check measured 1.83 ms inference p95 and 29.000 ms software-pipeline
latency; it is a usability smoke check, not a replacement for the 30-minute
soak or a physical end-to-end measurement.

## Queue/pre-roll latency Pareto

The stable reference remains the default. Lower target-fill experiments changed
only the target-fill setting and used the same four-frame ring and one-frame
startup pre-roll. They are evidence, not a new default:

| UL-UNAS run | Duration | Target fill | Queue mean (min–max) | Slope equivalent | Transport avg / max (ms) | Losses | Decision |
|---|---:|---:|---|---:|---:|---|---|
| Stable candidate | 30 min | 3.5 | 3.627 (1.996–3.904) | +0.053 ppm | 41.994 / 43.794 | 0 / 0 / 0 | **accepted finalist profile** |
| Lower fill | 10 min | 2.5 | 2.831 (2.396–3.990) | -2.039 ppm | 41.004 / 44.196 | 0 / 0 / 0 | short-run comparison; see 30-min row |
| Lower fill | 30 min | 2.5 | 2.502 (2.404–2.996) | -0.121 ppm | 34.624 / 46.468 | 0 / 0 / 0 | **passed soak; not default** |
| Lower fill | 10 min | 2.0 | 2.259 (0.946–3.956) | -9.020 ppm | 32.137 / 41.869 | 0 / 0 / 0 | short-run comparison; see 30-min row |
| Lower fill | 30 min | 2.0 | 2.001 (1.808–2.996) | -0.242 ppm | 26.115 / 36.488 | 0 / 0 / 0 | **passed soak; not default** |
| Requested pre-roll | 60 s | 3.5, requested 0 | 3.162 (2.996–3.390) | +73.730 ppm | 45.442 / 47.416 | 0 / 0 / 0 | effective target remained 3 frames; no latency benefit |

The 2.5-frame 30-minute run lowers the measured transport average by
approximately 7.37 ms relative to the 3.5-frame candidate run, with no losses
and a bounded 2.404-frame minimum. It is a validated lower-latency option but
does not replace the known-stable reference by itself. The 2.0-frame run lowers
transport by approximately 9.86 ms, but its minimum fill approaches one frame
and its slope is materially larger in magnitude; its completed 30-minute soak
recorded zero losses but remains a non-default profile. The 2.5-frame run's
accounted total software pipeline is 81.957 ms
average / 93.802 ms maximum, and the 2.0-frame run is 73.448 ms average /
83.821 ms maximum; both retain the same 47.333 ms structural processing path as
the 3.5-frame profile. The two lower-fill runs are validated Pareto options but
remain non-default until startup behavior and repeated native evidence justify
changing the reference profile. A requested zero-frame pre-roll was measured for
60 seconds, but the first 1,056-sample callback forced the existing automatic
effective target to three processing frames (30 ms); it therefore produced no
valid lower-startup operating point. The stable 30-minute run already covers
that effective target, so no duplicate 30-minute run was started.

## Formal production software-latency budget

The quantities remain separate:

1. `model_algorithmic_latency`: structural model/frame/state delay;
2. `model_inference_wall_time`: elapsed worker inference time;
3. `structural_path_addition`: candidate processing-path delay reported in
   samples and milliseconds; `incremental_software_pipeline_latency` remains
   `null` until a paired passthrough transport delta is measured;
4. `total_software_pipeline_latency`: measured transport plus structural path
   accounting;
5. `physical_e2e_latency`: acoustic/device loopback measurement, currently
   unknown.

### UL-UNAS balanced profile

| Component | Samples at 48 kHz | Milliseconds | Evidence/interpretation |
|---|---:|---:|---|
| Hardware/device latency | unknown | unknown | physical ADC/DAC and device endpoint delay; not measured |
| Capture buffering | unknown | unknown | device/driver capture buffering; not isolated from the physical path |
| Milestone 2 measured transport reference, average / max | — | 45.964 / 59.611 | native 30-minute passthrough artifact |
| Model hop (256 at 16 kHz) | 768 | 16.000 | structural; not inference wall time |
| Downsampler delay | 240 | 5.000 | Rubato adapter accounting |
| Upsampler delay | 240 | 5.000 | Rubato adapter accounting |
| Deterministic 48 kHz/256-hop phase/framing | 960 | 20.000 | included in constrained 2,208-sample verification |
| Candidate processor structural delay | **2,208** | **46.000** | measured by constrained correlation, coefficient 0.910599 |
| Drift-resampler delay | 64 | 1.333 | worker-side Rubato delay, kept separate |
| Processing-path algorithmic delay | **2,272** | **47.333** | processor + drift resampler; not physical E2E |
| Worker inference wall time | — | mean 1.113; p50 1.10; p95 1.35; p99 1.56; max 4.513 | timing histogram; not added as algorithmic delay |
| Native measured transport, average / max | — | 41.994 / 43.794 | candidate artifact |
| Accounted total software pipeline, average / max | — | **89.328 / 91.128** | transport + 47.333 ms structural path |
| Render buffering | unknown | unknown | device/driver render buffering; not isolated from the physical path |

The candidate transport average is 3.970 ms below the accepted reference run,
but that does not erase the 47.333 ms candidate structural path. The paired
transport difference and the structural addition are reported separately; no
unconstrained metric alignment or inference-time substitution removes model
lookahead.

### RNNoise reference profile

RNNoise's corresponding values are 960 samples / 20.000 ms processor delay,
64 samples / 1.333 ms drift-resampler delay, and 1,024 samples / 21.333 ms
processing-path delay. Its 10-minute native transport was 41.898 / 45.890 ms
average/max, giving an accounted software total of 63.231 / 67.223 ms. The
native inference distribution was p50 0.65, p95 0.76, p99 0.87, and maximum
3.984 ms.

The RNNoise budget has the same unmeasured hardware/device, capture-buffering,
and render-buffering components listed above; those values are intentionally
`unknown` and are not folded into the software total.

Neither total is physical E2E latency. Acoustic propagation and any future
virtual endpoint also remain outside this software budget until a loopback test
is performed.

## Blind listening harness

The local harness stores opaque labels, randomized order, a private mapping, and
append-only response files. Validation results:

| Session | Candidate/audio scope | Trials | Result |
|---|---|---:|---|
| `core-sanity-v1` | ABX integrity controls | 16 | passed; no identity leaks |
| `transient-sanity-v1` | transition integrity controls | 16 | passed; no identity leaks |
| `quality-gate-v1` | 5 systems, 80 audio assets | 16 | passed; no identity leaks |

`human_response_count` is zero in the Gate C evidence. The sessions prove the
harness plumbing and blind mappings, not a listener preference. The next run
must use the existing anonymized session plus append-only JSONL responses for
noise suppression, naturalness, intelligibility, artifacts, consonant
preservation, transient behavior, and overall preference.

## Decision

### Selected initial engine

Select **UL-UNAS official streaming ONNX as the provisional Auralis balanced
engine**. This choice is based on the measured Pareto position:

- highest aggregate noisy SI-SDR (7.025 dB) and positive aggregate STOI delta
  (+0.0057) in the frozen v3 corpus;
- credible complete-path WSL RTF (0.0512) including 48/16/48 conversion;
- native Windows 30-minute zero-loss run with bounded queue drift and bounded
  inference p99/max;
- explicit, verified 2,208-sample processor delay and 2,272-sample path delay;
- integration behind the existing worker-only processing boundary.

This is not a claim that UL-UNAS is perceptually best in every condition. Its
8–20 kHz clean-speech loss is measured and remains a production risk.

### Retained reference

Retain **RNNoise** as the low-latency/full-band reference and fallback. It is
native 48 kHz and has the lowest native inference percentiles of the integrated
models, but its clean-speech and transition stability metrics are not strong
enough to call it the quality winner.

### Rejected finalists/candidates

- **DeepFilterNet3-LL:** rejected for live production in this milestone due to
  43 input overruns, one deadline miss, 470.449 ms processing maximum, and high
  RSS in the native 10-second run. Its full-band offline quality does not waive
  realtime failure.
- **GTCRN:** not integrated after Gate C. Its offline CPU result is excellent,
  but aggregate noisy SI-SDR trails UL-UNAS and there is no native Windows live
  acceptance artifact in this milestone.
- **All four for redistribution:** blocked until each model-weight license is
  independently clarified. Repository source licenses alone are insufficient.

If the licensing question cannot be resolved, the explicit release decision is
“none sufficient for redistribution”; the local technical adapter and measured
UL-UNAS choice remain useful without shipping the artifact.

## Deployment implications

- Keep the pinned model outside Git and verify the allowlisted URL, exact size,
  and SHA-256 before loading.
- Ship ONNX Runtime CPU with one intra-op and one inter-op thread first; GPU is
  optional and not part of the baseline.
- Keep the 48/16/48 resampling cost and high-frequency bandwidth loss visible in
  product metadata and release notes.
- Preserve the Milestone 2 passthrough profile and the 3.5-frame stable target
  as the reference until lower-fill profiles complete comparable soak testing.
- Do not place model loading, inference, filesystem access, or logging in the
  CPAL callbacks.
- Treat weight-license clarification, host/SIMD provenance capture, and model
  memory measurement as release blockers, not documentation polish.

## Known limitations

- No physical acoustic/device end-to-end latency has been measured.
- No human listening responses have been recorded; only harness sanity and
  anonymization validation passed.
- RNNoise now has a verified 30-minute zero-loss native run; the runtime DLL is
  a separately hashed local scalar build and its memory/CPU values are not
  directly merged with the earlier 10-minute artifact.
- UL-UNAS target fills 2.5 and 2.0 both have verified 30-minute zero-loss soaks
  and lower software transport, but neither is the default; the 2.0 profile's
  minimum fill was 1.808 frames.
- A requested zero-frame startup pre-roll was evaluated for 60 seconds, but the
  first 1,056-sample callback forced the existing automatic effective target to
  three frames; no lower-startup operating point was demonstrated.
- Historical native artifacts lack CPU model, SIMD feature, affinity, and
  OS-build fields. A same-host CPU/OS/SIMD sidecar now exists for the separate
  60-second UL-UNAS envelope run, but it is not retroactive provenance for the
  historical 30-minute records. Candidate runtime version/build, provider, and
  requested thread settings are present in the raw denoiser-runtime objects.
- Linux/WSL UL-UNAS method-validation artifacts record a 26,794,667-byte mean
  denoiser-load RSS delta across three fresh processes (range 26,767,360–
  26,845,184 bytes); see `bench/raw/realtime/model-memory-repeat-2026-09-05.json`.
  Native Windows finalist artifacts still need regeneration with this field, and
  the value is a runtime load envelope rather than weight-only memory.
- PESQ and DNSMOS are not present in the current metric truth.
- The 16 kHz candidates necessarily lose bandwidth above 8 kHz; the measured
  clean high-band changes are not hidden in the aggregate score.
- The frozen offline corpus does not yet include independent recordings for all
  requested noise categories; the transient controls are synthetic and are not
  a substitute for that expansion.
- The selected clean-speech sources do not expose controlled close, medium, or
  far-field recording-distance labels.
- Weight licenses and Auralis's own project license are unresolved.

## Reproducible verification performed in this workspace

The following checks were actually run:

```text
cargo fmt --all                                  passed
cargo check --workspace --all-targets --all-features       passed
cargo clippy --workspace --all-targets --all-features -- -D warnings  passed
cargo test --workspace --all-features            52 passed, 6 ignored
cargo build --release --workspace                passed
cargo run -p auralis-cli -- simulate 2          passed
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke-m3-final  passed
cargo run -p auralis-bench -- foundation --out-dir bench/work/foundation-m3-final  passed
```

The release simulation was rerun after adding provenance fields; it produced
100 frames with zero output underrun callbacks. The noise-coverage manifest was
validated against the generated DEMAND, offline, and transient manifests. An
additional simulation with `AURALIS_GIT_REVISION=test-revision` verified that
the revision is copied into the output JSON.

After the runtime-report schema update, a fresh release simulation also
verified schema version 2, the canonical UL-UNAS candidate ID,
`model_memory_bytes: null`, a populated `model_load_envelope_bytes`, and a
null paired transport delta. Fresh foundation and smoke runs were also written
to separate audit directories; all controls passed. Those duplicate audit
directories, along with superseded corpus/process outputs and local build
environments, were removed during the post-milestone workspace cleanup. The
final control run is retained at `bench/work/foundation-m3-final` and the final
smoke run at `bench/work/smoke-m3-final`. The existing blind rating session was
revalidated without creating any listener responses.

The model-load memory probe was rerun in three fresh release processes with
`cargo run --release -p auralis-cli -- simulate 1 --denoiser ul-unas --model
models/ulunas_stream_simple.onnx`. The load RSS delta was 26,767,360,
26,845,184, and 26,771,456 bytes (mean 26,794,667 bytes; population standard
deviation 35,760 bytes). These are Linux/WSL method-validation results, not
native Windows acceptance values.

The continuation audit then reran the current release simulation with the
canonical UL-UNAS ID: 100 frames completed with zero inference failures,
schema version 2, a 2,208-sample processor delay, a 26,103,808-byte load
envelope, and a null paired transport delta. A fresh model-lab 48/16/48 run
also reproduced the frozen UL-UNAS SHA-256 and exact input/output sample count.
Fresh foundation, smoke, and all three blind-session validation commands were
run in separate non-overwriting directories; raw fixture hashes and validation
flags matched the earlier controls.

The frozen UL-UNAS model was downloaded from its pinned official raw URL and
verified as `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`.
The ignored model-specific latency test then passed:

```text
UL-UNAS constrained correlation lag: 2208 samples (46.000 ms), coefficient 0.910599
```

On 2026-09-05 a separate native Windows UL-UNAS 60-second host-envelope run
completed with zero input/output losses, xruns, processing deadline misses, and
inference failures. The paired host snapshot records Windows 11 build 26200,
AMD Ryzen 7 5700X (8 cores/16 logical processors), and the observed SIMD set;
the run and snapshot are linked by
`bench/raw/realtime/milestone-3-realtime-followup-2026-09-05.json`. A requested
zero-frame pre-roll control also completed with zero losses, but the automatic
callback-size guard selected an effective three-frame target.

The continuation audit also ran the model-lab help, Python compilation, and a
fresh UL-UNAS 48/16/48 single-case process. The new report contained a
non-null `benchmark_code.revision`
(`source-tree-sha256:7d624fda2c7556251af494442dae17e2dcae94e67e73cfe2ccf0650abe0ed3a1`)
and preserved the input/output sample count; an `AURALIS_GIT_REVISION` override
was verified to be retained verbatim. This does not rewrite the historical
bake-off reports, whose missing code revision remains an explicit provenance
limitation.

`cargo check --target x86_64-pc-windows-msvc --workspace --all-targets` was
also attempted from WSL. It failed in the third-party `tract-linalg` build
script because the available GNU compiler is not supported for the MSVC
target. This is not native Windows execution evidence; the native Windows
results above came from the recorded Windows release runs.

A direct MSVC release-build attempt used LLVM `clang-cl`/`llvm-lib`; compilation
reached the final crates but the host had no `link.exe`, so linking failed. A
second `cargo-xwin` build used the cached Windows SDK/CRT and `lld-link`, with
temporary case aliases for the SDK's `DirectML.lib` and `PathCch.lib`. The
GUI-inclusive rebuild on 2026-09-17 produced a PE32+
`target/windows-release/auralis-cli.exe` (SHA-256
`da19d31d3309ef101a547fe306798cbd2a8ecb7315e4fd6d6fddb03e8308ba7f`) and its
required `DirectML.dll` sidecar (SHA-256
`9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1`). PE
dependency inspection passed. This proves an executable artifact can be
assembled, but not that it runs on a native device; Windows 11 device
enumeration and capture/render execution remain the required acceptance check.

## Local control GUI

After the Milestone 3 engine decision, a small loopback-only control GUI was
added to `auralis-cli gui`. It serves a single screen with the selected visual
direction: start/stop, input/output device selection, profile and local model
path, plus live queue, underrun/xrun, CPU/RSS, inference p95, algorithmic
latency, and software-pipeline latency. The GUI calls the existing CPAL run
path through a cancellable worker session; no callback code or denoiser
implementation is moved into the UI. The browser server binds to
`127.0.0.1` by default and has no remote API or telemetry path.

The route was exercised locally with `GET /`, `GET /api/state`,
`GET /api/devices`, and passthrough/UL-UNAS start/stop requests. WSL exposed
PulseAudio rather than Windows WASAPI, so this is API/interaction evidence only;
native Windows visual and device verification remains pending.

## Next recommended milestone

Before AEC or target-speaker work, close the Milestone 3 release blockers:

1. obtain explicit source/weight commercial and redistribution terms for
   UL-UNAS (and the RNNoise fallback), or record that no candidate is shippable;
2. rebuild/run the current CLI on Windows so CPU model, SIMD features, runtime
   build, thread/affinity settings, and the model-load RSS envelope are embedded
   in each finalist artifact (a same-host sidecar exists for one UL-UNAS run);
3. repeat the lower-fill profile if a future release needs more than the current
   single 30-minute Pareto confirmation, without changing the Milestone 2
   reference by default;
4. collect blinded human responses on the existing randomized sessions,
   especially Japanese sibilants, English consonants, clean speech, and
   transients;
5. run the documented physical/loopback latency procedure.

Only after those results are accepted should the project define the next
milestone. AEC, target-speaker extraction, and virtual microphone work remain
explicitly out of scope; the GUI is limited to the local control surface above.
