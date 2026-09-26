from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path

import numpy as np
import soundfile as sf

from . import benchmark_provenance
from .summary import stats

SAMPLE_RATE_HZ = 48_000
WINDOWS_MS = {
    "pre": (-250.0, -50.0),
    "event": (-20.0, 100.0),
    "post": (100.0, 350.0),
}


def analyze_transitions(
    corpus_manifest_path: Path,
    output_manifest_path: Path,
    report_path: Path,
) -> None:
    if report_path.exists():
        raise FileExistsError(f"refusing to overwrite {report_path}")
    corpus = json.loads(corpus_manifest_path.read_text(encoding="utf-8"))
    output = json.loads(output_manifest_path.read_text(encoding="utf-8"))
    if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
        raise ValueError("unsupported transition corpus schema")
    if output.get("schema_id") != "auralis.offline-bakeoff-output.v1":
        raise ValueError("unsupported transition output schema")
    output_cases = {case["case_id"]: case for case in output["cases"]}
    if len(output_cases) != len(corpus["cases"]):
        raise ValueError("transition corpus/output count mismatch")
    offset = output["signal_path"]["offline_output_alignment_offset_samples_at_48khz"]
    corpus_root = corpus_manifest_path.parent
    output_root = output_manifest_path.parent
    events = []
    for case in corpus["cases"]:
        output_case = output_cases.get(case["case_id"])
        if output_case is None:
            raise ValueError(f"missing transition output: {case['case_id']}")
        reference = read_verified(corpus_root, case["reference"])
        noise = read_verified(corpus_root, case["noise_component"])
        enhanced = read_verified(output_root, output_case["output"])
        if reference.size != noise.size or reference.size != enhanced.size:
            raise ValueError(f"transition sample mismatch: {case['case_id']}")
        for event_index, event in enumerate(case["events"]):
            event_result = {
                "event_id": f"{case['case_id']}--event-{event_index:02d}",
                "case_id": case["case_id"],
                "pattern_id": case["pattern_id"],
                "event": event,
                "windows": {},
            }
            for window_id, (start_ms, stop_ms) in WINDOWS_MS.items():
                start = event["time_samples"] + round(start_ms * SAMPLE_RATE_HZ / 1_000.0)
                stop = event["time_samples"] + round(stop_ms * SAMPLE_RATE_HZ / 1_000.0)
                start = max(0, start)
                stop = min(reference.size, stop, enhanced.size - offset)
                event_result["windows"][window_id] = window_metrics(
                    reference[start:stop],
                    noise[start:stop],
                    enhanced[offset + start : offset + stop],
                    start,
                    stop,
                )
            events.append(event_result)

    report = {
        "schema_id": "auralis.transition-analysis.v1",
        "benchmark_code": benchmark_provenance(),
        "candidate_id": output["candidate_id"],
        "corpus_manifest": file_asset(corpus_manifest_path),
        "output_manifest": file_asset(output_manifest_path),
        "alignment": {
            "offset_samples": offset,
            "offset_ms": offset / SAMPLE_RATE_HZ * 1_000.0,
            "source": "candidate output manifest fixed deterministic offset",
            "method": "trim output prefix only; no signal-derived search",
        },
        "window_definitions_ms_relative_to_event": WINDOWS_MS,
        "event_count": len(events),
        "aggregate_by_event_kind": aggregate(events, "kind"),
        "aggregate_by_pattern": aggregate(events, "pattern_id"),
        "limitations": [
            "Window metrics expose local energy, speech projection, and residual behavior but do not identify perceptual artifact type.",
            "Synthetic transients are deterministic controls, not a substitute for real captured keyboards, impacts, or doors.",
            "Blind listening remains required for pumping, metallic artifacts, and naturalness.",
        ],
        "events": events,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def window_metrics(
    reference: np.ndarray,
    noise: np.ndarray,
    output: np.ndarray,
    start_sample: int,
    stop_sample: int,
) -> dict[str, object]:
    if reference.size == 0:
        return {"status": "empty", "start_sample": start_sample, "stop_sample": stop_sample}
    error = output - reference
    reference_energy = energy(reference)
    noise_energy = energy(noise)
    error_energy = energy(error)
    result = {
        "status": "measured",
        "start_sample": start_sample,
        "stop_sample": stop_sample,
        "sample_count": int(reference.size),
        "reference_rms_dbfs": rms_dbfs(reference),
        "input_noise_rms_dbfs": rms_dbfs(noise),
        "output_rms_dbfs": rms_dbfs(output),
        "output_error_rms_dbfs": rms_dbfs(error),
        "clipped_samples": int(np.count_nonzero(np.abs(output) >= 0.999)),
        "speech_projection_gain_db": None,
        "speech_sdr_db": None,
        "noise_attenuation_db": None,
    }
    if reference_energy > 1e-12:
        projection = float(np.dot(reference.astype(np.float64), output.astype(np.float64))) / reference_energy
        result["speech_projection_gain_db"] = db20(abs(projection))
        result["speech_sdr_db"] = db10(reference_energy / max(error_energy, np.finfo(np.float64).eps))
    if noise_energy > 1e-12:
        result["noise_attenuation_db"] = db10(
            noise_energy / max(error_energy, np.finfo(np.float64).eps)
        )
    return result


def aggregate(events: list[dict], grouping: str) -> dict[str, object]:
    groups: dict[str, list[dict]] = {}
    for event in events:
        key = event["event"]["kind"] if grouping == "kind" else event["pattern_id"]
        groups.setdefault(key, []).append(event)
    result = {}
    for key, rows in sorted(groups.items()):
        windows = {}
        for window_id in WINDOWS_MS:
            metrics = [row["windows"][window_id] for row in rows]
            windows[window_id] = {
                field: stats(metric.get(field) for metric in metrics)
                for field in (
                    "reference_rms_dbfs",
                    "input_noise_rms_dbfs",
                    "output_rms_dbfs",
                    "output_error_rms_dbfs",
                    "speech_projection_gain_db",
                    "speech_sdr_db",
                    "noise_attenuation_db",
                    "clipped_samples",
                )
            }
        result[key] = {"event_count": len(rows), "windows": windows}
    return result


def read_verified(root: Path, asset: dict[str, object]) -> np.ndarray:
    path = root / asset["path"]
    if hashlib.sha256(path.read_bytes()).hexdigest() != asset["sha256"]:
        raise ValueError(f"asset hash mismatch: {path}")
    samples, rate = sf.read(path, dtype="float32", always_2d=False)
    if rate != SAMPLE_RATE_HZ or samples.ndim != 1:
        raise ValueError(f"unexpected transition audio: {path}")
    return np.asarray(samples, dtype=np.float32)


def energy(samples: np.ndarray) -> float:
    return float(np.sum(np.square(samples, dtype=np.float64)))


def rms_dbfs(samples: np.ndarray) -> float:
    return db20(math.sqrt(energy(samples) / max(1, samples.size)))


def db20(value: float) -> float:
    return 20.0 * math.log10(max(value, np.finfo(np.float64).eps))


def db10(value: float) -> float:
    return 10.0 * math.log10(max(value, np.finfo(np.float64).eps))


def file_asset(path: Path) -> dict[str, object]:
    return {
        "path": path.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }
