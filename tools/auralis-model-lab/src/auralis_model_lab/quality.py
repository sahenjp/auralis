from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path

import numpy as np
import soundfile as sf
from pystoi import stoi
from scipy.signal import correlate, stft

from . import benchmark_provenance

SAMPLE_RATE_HZ = 48_000


def diagnose_alignment(
    reference_path: Path,
    output_path: Path,
    report_path: Path,
    minimum_offset_samples: int,
    maximum_offset_samples: int,
) -> None:
    if report_path.exists():
        raise FileExistsError(f"refusing to overwrite {report_path}")
    if minimum_offset_samples < 0 or maximum_offset_samples < minimum_offset_samples:
        raise ValueError("invalid constrained offset range")
    reference, reference_rate = sf.read(
        reference_path, dtype="float32", always_2d=False
    )
    output, output_rate = sf.read(output_path, dtype="float32", always_2d=False)
    if (
        reference_rate != SAMPLE_RATE_HZ
        or output_rate != SAMPLE_RATE_HZ
        or reference.ndim != 1
        or output.ndim != 1
    ):
        raise ValueError("alignment diagnostics require mono 48 kHz audio")
    if maximum_offset_samples >= output.size:
        raise ValueError("maximum offset must be smaller than output")

    reference = np.asarray(reference - np.mean(reference), dtype=np.float64)
    output = np.asarray(output - np.mean(output), dtype=np.float64)
    cross = correlate(output, reference, mode="full", method="fft")
    base = reference.size - 1
    candidates = []
    for offset in range(minimum_offset_samples, maximum_offset_samples + 1):
        length = min(reference.size, output.size - offset)
        numerator = float(cross[base + offset])
        denominator = math.sqrt(
            signal_energy(reference[:length]) * signal_energy(output[offset : offset + length])
        )
        candidates.append(numerator / max(denominator, np.finfo(np.float64).eps))
    scores = np.asarray(candidates, dtype=np.float64)
    best_index = int(np.argmax(scores))
    best_offset = minimum_offset_samples + best_index
    report = {
        "schema_id": "auralis.alignment-diagnostic.v1",
        "reference": file_asset(reference_path),
        "output": file_asset(output_path),
        "sample_rate_hz": SAMPLE_RATE_HZ,
        "search": {
            "method": "DC-removed normalized cross-correlation using FFT correlation",
            "minimum_offset_samples": minimum_offset_samples,
            "maximum_offset_samples": maximum_offset_samples,
            "minimum_offset_ms": minimum_offset_samples / SAMPLE_RATE_HZ * 1_000.0,
            "maximum_offset_ms": maximum_offset_samples / SAMPLE_RATE_HZ * 1_000.0,
            "best_offset_samples": best_offset,
            "best_offset_ms": best_offset / SAMPLE_RATE_HZ * 1_000.0,
            "best_normalized_correlation": float(scores[best_index]),
        },
        "policy": "Diagnostic only. The constrained signal-derived result must not replace declared or verified deterministic production latency, and must not be used to hide unstable delay or temporal deformation.",
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def analyze_bakeoff(
    corpus_manifest_path: Path,
    output_manifest_path: Path,
    report_path: Path,
) -> None:
    if report_path.exists():
        raise FileExistsError(f"refusing to overwrite {report_path}")
    corpus = json.loads(corpus_manifest_path.read_text(encoding="utf-8"))
    outputs = json.loads(output_manifest_path.read_text(encoding="utf-8"))
    if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
        raise ValueError("unsupported corpus manifest")
    if outputs.get("schema_id") != "auralis.offline-bakeoff-output.v1":
        raise ValueError("unsupported output manifest")
    output_cases = {case["case_id"]: case for case in outputs["cases"]}
    if len(output_cases) != len(outputs["cases"]) or len(output_cases) != len(corpus["cases"]):
        raise ValueError("case count or uniqueness mismatch")
    offset_48k = outputs["signal_path"]["offline_output_alignment_offset_samples_at_48khz"]
    corpus_root = corpus_manifest_path.parent
    output_root = output_manifest_path.parent
    cases = []
    for case in corpus["cases"]:
        output_case = output_cases.get(case["case_id"])
        if output_case is None:
            raise ValueError(f"output missing case: {case['case_id']}")
        reference = read_verified(corpus_root, case["reference"])
        mixture = read_verified(corpus_root, case["mixture"])
        enhanced = read_verified(output_root, output_case["output"])
        aligned_reference, aligned_output = align(reference, enhanced, offset_48k)
        result = {
            "case_id": case["case_id"],
            "condition": case["condition"],
            "requested_snr_db": case["requested_snr_db"],
            "input_sha256": case["mixture"]["sha256"],
            "output_sha256": output_case["output"]["sha256"],
            "stoi": {
                "input": float(stoi(reference, mixture, SAMPLE_RATE_HZ, extended=False)),
                "output_raw_unaligned": float(
                    stoi(reference, enhanced, SAMPLE_RATE_HZ, extended=False)
                ),
                "output_deterministic_latency_compensated": float(
                    stoi(aligned_reference, aligned_output, SAMPLE_RATE_HZ, extended=False)
                ),
            },
            "clipped_samples_raw": int(np.count_nonzero(np.abs(enhanced) >= 0.999)),
            "input_clipped_samples": int(np.count_nonzero(np.abs(mixture) >= 0.999)),
        }
        if case["condition"] == "clean":
            result["clean_preservation"] = clean_preservation(
                aligned_reference, aligned_output
            )
        cases.append(result)
    report = {
        "schema_id": "auralis.perceptual-bakeoff-score.v1",
        "benchmark_code": benchmark_provenance(),
        "candidate_id": outputs["candidate_id"],
        "corpus_manifest": file_asset(corpus_manifest_path),
        "output_manifest": file_asset(output_manifest_path),
        "alignment": {
            "offset_samples": offset_48k,
            "offset_ms": offset_48k / SAMPLE_RATE_HZ * 1000.0,
            "source": "candidate output manifest; no signal-derived search",
        },
        "metric_implementations": {
            "stoi": {
                "package": "pystoi",
                "version": "0.4.1",
                "upstream": "https://github.com/mpariente/pystoi",
                "license": "MIT",
                "extended": False,
            },
            "spectral": {
                "implementation": "Auralis scipy.signal.stft",
                "frame_samples": 1024,
                "hop_samples": 512,
                "window": "periodic Hann",
            },
        },
        "limitations": [
            "STOI is an intrusive intelligibility predictor, not a naturalness or preference score.",
            "RMS change is not a perceptual loudness measurement.",
            "Spectral metrics are descriptive and must not replace listening tests.",
            "A band-change result is null when the clean reference has less than -60 dB of total energy in that band.",
        ],
        "case_count": len(cases),
        "cases": cases,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def clean_preservation(reference: np.ndarray, output: np.ndarray) -> dict[str, object]:
    return {
        "rms_change_db": db_ratio(signal_energy(output), signal_energy(reference)),
        "log_spectral_distance_db": log_spectral_distance(reference, output),
        "band_energy": {
            "4khz_to_8khz": band_energy_change(reference, output, 4_000, 8_000),
            "8khz_to_20khz": band_energy_change(reference, output, 8_000, 20_000),
        },
        "frame_gain_stability": frame_gain_stability(reference, output),
    }


def log_spectral_distance(reference: np.ndarray, output: np.ndarray) -> float:
    frequencies, _, reference_stft = stft(
        reference,
        fs=SAMPLE_RATE_HZ,
        window="hann",
        nperseg=1024,
        noverlap=512,
        boundary=None,
        padded=False,
    )
    _, _, output_stft = stft(
        output,
        fs=SAMPLE_RATE_HZ,
        window="hann",
        nperseg=1024,
        noverlap=512,
        boundary=None,
        padded=False,
    )
    included = (frequencies >= 80.0) & (frequencies <= 20_000.0)
    reference_db = 20.0 * np.log10(np.maximum(np.abs(reference_stft[included]), 1e-8))
    output_db = 20.0 * np.log10(np.maximum(np.abs(output_stft[included]), 1e-8))
    return float(np.mean(np.sqrt(np.mean(np.square(reference_db - output_db), axis=0))))


def band_energy_change(
    reference: np.ndarray, output: np.ndarray, low_hz: int, high_hz: int
) -> dict[str, object]:
    frequencies = np.fft.rfftfreq(reference.size, 1.0 / SAMPLE_RATE_HZ)
    included = (frequencies >= low_hz) & (frequencies < high_hz)
    reference_spectrum = np.fft.rfft(reference)
    output_spectrum = np.fft.rfft(output)
    reference_energy = float(np.sum(np.square(np.abs(reference_spectrum[included]))))
    output_energy = float(np.sum(np.square(np.abs(output_spectrum[included]))))
    total_energy = float(np.sum(np.square(np.abs(reference_spectrum))))
    fraction_db = db_ratio(reference_energy, total_energy)
    if fraction_db < -60.0:
        return {
            "status": "insufficient_reference_energy",
            "reference_fraction_db": fraction_db,
            "change_db": None,
        }
    return {
        "status": "measured",
        "reference_fraction_db": fraction_db,
        "change_db": db_ratio(output_energy, reference_energy),
    }


def frame_gain_stability(reference: np.ndarray, output: np.ndarray) -> dict[str, float | int]:
    frame_samples = 960
    reference_frames = frame_view(reference, frame_samples)
    output_frames = frame_view(output, frame_samples)
    reference_rms = np.sqrt(np.mean(np.square(reference_frames, dtype=np.float64), axis=1))
    output_rms = np.sqrt(np.mean(np.square(output_frames, dtype=np.float64), axis=1))
    threshold = float(np.max(reference_rms)) * 0.01
    active = reference_rms >= threshold
    gains = 20.0 * np.log10(
        np.maximum(output_rms[active], np.finfo(np.float64).eps)
        / np.maximum(reference_rms[active], np.finfo(np.float64).eps)
    )
    return {
        "frame_samples": frame_samples,
        "active_threshold_db_below_peak": -40.0,
        "active_frames": int(gains.size),
        "gain_mean_db": float(np.mean(gains)),
        "gain_standard_deviation_db": float(np.std(gains)),
        "gain_p05_db": float(np.quantile(gains, 0.05)),
        "gain_p95_db": float(np.quantile(gains, 0.95)),
    }


def frame_view(samples: np.ndarray, frame_samples: int) -> np.ndarray:
    count = samples.size // frame_samples
    if count == 0:
        raise ValueError("signal is too short for frame metric")
    return samples[: count * frame_samples].reshape(count, frame_samples)


def align(
    reference: np.ndarray, output: np.ndarray, offset_samples: int
) -> tuple[np.ndarray, np.ndarray]:
    length = min(reference.size, output.size - offset_samples)
    if length <= 0:
        raise ValueError("alignment offset exceeds output")
    return reference[:length], output[offset_samples : offset_samples + length]


def read_verified(root: Path, asset: dict[str, object]) -> np.ndarray:
    path = root / asset["path"]
    if hashlib.sha256(path.read_bytes()).hexdigest() != asset["sha256"]:
        raise ValueError(f"asset hash mismatch: {path}")
    samples, sample_rate = sf.read(path, dtype="float32", always_2d=False)
    if sample_rate != SAMPLE_RATE_HZ or samples.ndim != 1:
        raise ValueError(f"unexpected audio format: {path}")
    return np.ascontiguousarray(samples, dtype=np.float32)


def signal_energy(samples: np.ndarray) -> float:
    return float(np.sum(np.square(samples, dtype=np.float64)))


def db_ratio(numerator: float, denominator: float) -> float:
    epsilon = np.finfo(np.float64).eps
    return 10.0 * math.log10(max(numerator, epsilon) / max(denominator, epsilon))


def file_asset(path: Path) -> dict[str, object]:
    return {
        "path": path.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }
