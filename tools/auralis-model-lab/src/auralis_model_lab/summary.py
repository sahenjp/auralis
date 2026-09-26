from __future__ import annotations

import hashlib
import json
from pathlib import Path

import numpy as np

from . import benchmark_provenance


def summarize_bakeoff(
    objective_paths: list[Path],
    perceptual_paths: list[Path],
    output_paths: list[Path],
    report_path: Path,
) -> None:
    if report_path.exists():
        raise FileExistsError(f"refusing to overwrite {report_path}")
    objectives = load_unique(objective_paths, "auralis.objective-bakeoff-score.v1")
    perceptual = load_unique(perceptual_paths, "auralis.perceptual-bakeoff-score.v1")
    outputs = load_unique(output_paths, "auralis.offline-bakeoff-output.v1")
    candidate_ids = set(objectives)
    if candidate_ids != set(perceptual) or candidate_ids != set(outputs):
        raise ValueError("objective, perceptual, and output candidate sets differ")

    candidates = []
    for candidate_id in sorted(candidate_ids):
        objective = objectives[candidate_id][1]
        perception = perceptual[candidate_id][1]
        output = outputs[candidate_id][1]
        objective_cases = index_cases(objective)
        perception_cases = index_cases(perception)
        output_cases = index_cases(output)
        if set(objective_cases) != set(perception_cases) or set(objective_cases) != set(output_cases):
            raise ValueError(f"case IDs differ for {candidate_id}")

        noisy_ids = [
            case_id
            for case_id, case in objective_cases.items()
            if case["condition"] == "noisy"
        ]
        clean_ids = [
            case_id
            for case_id, case in objective_cases.items()
            if case["condition"] == "clean"
        ]
        snr_groups = {}
        for snr_db in (10.0, 5.0, 0.0, -5.0, -10.0):
            ids = [
                case_id
                for case_id in noisy_ids
                if objective_cases[case_id]["requested_snr_db"] == snr_db
            ]
            snr_groups[format_snr(snr_db)] = quality_group(
                ids, objective_cases, perception_cases
            )

        candidates.append(
            {
                "candidate_id": candidate_id,
                "source_reports": {
                    "objective": file_asset(objectives[candidate_id][0]),
                    "perceptual": file_asset(perceptual[candidate_id][0]),
                    "output": file_asset(outputs[candidate_id][0]),
                },
                "signal_path": output["signal_path"],
                "quality": {
                    "noisy_overall": quality_group(
                        noisy_ids, objective_cases, perception_cases
                    ),
                    "by_requested_snr_db": snr_groups,
                    "clean_preservation": clean_group(
                        clean_ids, objective_cases, perception_cases
                    ),
                },
                "runtime": runtime_summary(output),
            }
        )

    report = {
        "schema_id": "auralis.offline-bakeoff-summary.v1",
        "benchmark_code": benchmark_provenance(),
        "candidate_count": len(candidates),
        "candidate_ids": [candidate["candidate_id"] for candidate in candidates],
        "aggregation": {
            "quality_alignment": "each source report's fixed deterministic-latency-compensated fields",
            "statistics": "finite values only; numpy linear quantiles",
            "weighting": "one vote per corpus case unless a field explicitly counts inference operations",
        },
        "limitations": [
            "This is a dimension-preserving summary, not a weighted winner score.",
            "WSL runtime screening is not native Windows realtime acceptance evidence.",
            "Objective and intrusive metrics do not replace blinded listening tests.",
        ],
        "candidates": candidates,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def load_unique(paths: list[Path], schema_id: str) -> dict[str, tuple[Path, dict]]:
    result = {}
    for path in paths:
        value = json.loads(path.read_text(encoding="utf-8"))
        if value.get("schema_id") != schema_id:
            raise ValueError(f"unexpected schema in {path}")
        candidate_id = value["candidate_id"]
        if candidate_id in result:
            raise ValueError(f"duplicate candidate: {candidate_id}")
        result[candidate_id] = (path, value)
    return result


def index_cases(report: dict) -> dict[str, dict]:
    result = {case["case_id"]: case for case in report["cases"]}
    if len(result) != len(report["cases"]):
        raise ValueError("duplicate case ID")
    return result


def quality_group(ids: list[str], objective: dict[str, dict], perceptual: dict[str, dict]) -> dict:
    fixed = [objective[case_id]["quality"]["deterministic_latency_compensated"] for case_id in ids]
    stoi_rows = [perceptual[case_id]["stoi"] for case_id in ids]
    return {
        "case_count": len(ids),
        "si_sdr_db": stats(row["si_sdr_db"] for row in fixed),
        "sdr_db": stats(row["sdr_db"] for row in fixed),
        "noise_attenuation_db": stats(row["noise_attenuation_db"] for row in fixed),
        "speech_projection_gain_db": stats(row["speech_projection_gain_db"] for row in fixed),
        "stoi_input": stats(row["input"] for row in stoi_rows),
        "stoi_output": stats(row["output_deterministic_latency_compensated"] for row in stoi_rows),
        "stoi_delta": stats(
            row["output_deterministic_latency_compensated"] - row["input"]
            for row in stoi_rows
        ),
        "clipped_samples_total": int(sum(row["clipped_samples"] for row in fixed)),
    }


def clean_group(ids: list[str], objective: dict[str, dict], perceptual: dict[str, dict]) -> dict:
    quality = quality_group(ids, objective, perceptual)
    clean = [perceptual[case_id]["clean_preservation"] for case_id in ids]
    quality.update(
        {
            "rms_change_db": stats(row["rms_change_db"] for row in clean),
            "log_spectral_distance_db": stats(
                row["log_spectral_distance_db"] for row in clean
            ),
            "band_4khz_to_8khz_change_db": stats(
                row["band_energy"]["4khz_to_8khz"]["change_db"] for row in clean
            ),
            "band_8khz_to_20khz_change_db": stats(
                row["band_energy"]["8khz_to_20khz"]["change_db"] for row in clean
            ),
            "frame_gain_standard_deviation_db": stats(
                row["frame_gain_stability"]["gain_standard_deviation_db"]
                for row in clean
            ),
        }
    )
    return quality


def runtime_summary(output: dict) -> dict:
    inference_ns = [
        value
        for case in output["cases"]
        for value in (case.get("wall_time") or {}).get("model_inference_ns_raw", [])
    ]
    complete_rtfs = [
        (case.get("wall_time") or {}).get("complete_path_realtime_factor")
        for case in output["cases"]
    ]
    return {
        "environment": output["runtime"],
        "complete_path_realtime_factor_per_case": stats(complete_rtfs),
        "model_inference_wall_time_ms_per_operation": stats(
            np.asarray(inference_ns, dtype=np.float64) / 1_000_000.0
        ),
        "model_inference_operation_count": len(inference_ns),
    }


def stats(values) -> dict | None:
    array = np.asarray([value for value in values if value is not None], dtype=np.float64)
    array = array[np.isfinite(array)]
    if array.size == 0:
        return None
    return {
        "count": int(array.size),
        "mean": float(np.mean(array)),
        "median": float(np.median(array)),
        "p05": float(np.quantile(array, 0.05)),
        "p95": float(np.quantile(array, 0.95)),
        "p99": float(np.quantile(array, 0.99)),
        "minimum": float(np.min(array)),
        "maximum": float(np.max(array)),
    }


def format_snr(snr_db: float) -> str:
    return f"{snr_db:+.0f}"


def file_asset(path: Path) -> dict[str, object]:
    return {
        "path": path.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }
