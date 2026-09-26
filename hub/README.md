---
language: [en, ja]
tags: [audio, speech-enhancement, denoising, ul-unas]
---

# Auralis UL-UNAS evaluation

This repository contains evaluation information only. It does not host or
redistribute model weights. The upstream weight terms for the pinned artifact
have not been independently clarified, so the artifact is not cleared for
redistribution.

## Evaluated artifact

- Model: UL-UNAS DNS3 streaming ONNX
- Upstream: [Xiaobin-Rong/ul-unas](https://github.com/Xiaobin-Rong/ul-unas)
- Revision: `00f7c700da43d38347f30a6ccebd86fcbc798e07`
- Artifact: `ulunas_onnx/onnx_models/ulunas_stream_simple.onnx`
- SHA-256: `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`
- Signal path: 48 kHz mono input, 48/16/48 kHz resampling, 16 kHz model, 8 kHz output bandwidth

## Auralis measurements

On the frozen Auralis v3 corpus (630 noisy cases), the candidate averaged
7.025 dB SI-SDR and a +0.0057 STOI change. Its 8–20 kHz clean-speech energy
change was -13.112 dB, reflecting the model's 8 kHz bandwidth limit.

A native Windows 11 30-minute run recorded zero input/output losses, underruns,
xruns, processing deadline misses, or inference failures. This is a software
stability result, not a physical microphone-to-speaker latency measurement.
No human listening responses have been collected; objective scores do not
establish perceived quality or preference.

## Intended use and limitations

The measurements support local evaluation of speech denoising. They do not
establish performance for every speaker, room, noise type, or device. The model
can remove high-frequency speech detail above 8 kHz. The result is not a claim
of superiority over commercial products.

## Reproducibility

The model adapter, pinned candidate metadata, corpus manifests, evaluation
methodology, and full results are in the [Auralis source repository](https://github.com/sahenjp/auralis):

- `tools/auralis-model-lab/README.md`
- `bench/candidates/frozen-set-v1.json`
- `docs/benchmark-methodology.md`
- `docs/milestone-3-results.md`

Dataset audio and generated audio are not included here. Refer to the Auralis
repository for dataset provenance and licensing notes.
