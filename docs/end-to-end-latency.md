# End-to-end latency measurement

Software pipeline timing ends when a processed frame leaves Auralis's output
queue. It excludes ADC, Windows capture buffering, DAC, and the acoustic or
cable path. It must not be labelled end-to-end latency.

## Reproducible correlation procedure

1. Build the release benchmark tool and generate a 48 kHz mono float reference:

   ```bash
   cargo run --release -p auralis-bench -- \
     latency-reference --out bench/work/latency-reference.wav \
     --count 20 --interval-ms 500
   ```

2. Start recording before playback. Route playback through the complete path
   under test. Preferred setups are a physical output-to-input cable or a
   documented Windows loopback path that includes Auralis. Disable unrelated
   effects and automatic gain control. Preserve 48 kHz mono float WAV format.

3. Keep recording until the final pulse and save it with the same time origin
   as the reference: the first reference sample corresponds to the first
   recording sample. If the recorder introduces an unknown leading offset,
   measure and remove that offset before analysis.

4. Correlate every deterministic 127-sample pulse within the bounded positive
   search range:

   ```bash
   cargo run --release -p auralis-bench -- \
     latency-analyze \
     --reference bench/work/latency-reference.wav \
     --recording bench/work/latency-recording.wav \
     --out bench/work/latency-result.json \
     --max-lag-ms 250
   ```

The JSON result records median, p95, p99, maximum, sample count, each detected
lag, and normalized correlation. Inspect low correlation values and clipping
before accepting a run. The tool test recovers a synthetic 240-sample delay as
5.0 ms. No physical end-to-end value is reported until this procedure is run on
the actual Windows signal path.
