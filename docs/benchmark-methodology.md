# Benchmark methodology

## Corpus contract

```text
bench/
  corpus/clean/       immutable clean references
  corpus/noise/       immutable noise and interference sources
  corpus/mixtures/    generated, content-addressed mixtures
  outputs/raw/
  outputs/stable/
  outputs/experimental/
  outputs/competitors/
  results/            JSON/CSV reports
```

Every item needs a stable ID, provenance, license, SHA-256, sample rate, channels, scenario tags, speaker identity partition, and generation parameters. Once used for a release comparison, audio and metadata are immutable; corrections create a new corpus version. Training speakers and clips must not enter evaluation partitions.

Synthetic mixtures use deterministic offsets/seeds and at least `+10`, `+5`, `0`, `-5`, and `-10` dB SNR. Scaling is recorded and shared references are retained. Synthetic results are necessary but insufficient; separately captured keyboards, fans, traffic, reverberation, echo, music, and competing-speaker scenes are required before product claims.

## Systems under test

The harness keys output by an opaque `system_id` and accepts raw, stable Auralis, experimental Auralis, and legitimately obtained competitor output. All systems receive the same PCM input. Sample-rate conversion, delay alignment, failures, and manual operations are recorded rather than hidden.

## Metrics

- intrusive: SI-SDR, SDR, STOI, PESQ where legally available;
- non-intrusive: DNSMOS P.835 and personalized DNSMOS where model terms permit;
- signal diagnostics: noise/speech attenuation, clipping, gain, delay, spectral discontinuity;
- systems: realtime factor, per-frame latency distribution, CPU, resident memory, underruns, overruns;
- AEC: ERLE plus single-talk, double-talk, path-change, and render-type breakdown;
- target speaker: target SI-SDR/intelligibility plus interferer leakage, false rejection, and false acceptance.

PESQ is governed by ITU-T P.862 and must not be silently redistributed as project code. STOI/PESQ/DNSMOS versions and model hashes must be pinned in result metadata. DNSMOS is a proxy; Microsoft’s DNS Challenge documentation explicitly treats human subjective evaluation as the gold standard.

The initial `smoke` command uses synthetic tones/noise only to verify determinism, mixing math, processor plumbing, timing, and JSON/CSV schemas. It is not an audio-quality score.

## Blind listening

Each trial randomizes system order using a stored seed and exposes only opaque sample labels. Listeners rate naturalness, intelligibility, noise suppression, artifacts, and overall preference. The private decoding manifest is separated from the presentation manifest. Reports include listener count, trial count, ties, confidence intervals, exclusions, playback requirements, and the exact corpus/system versions.

