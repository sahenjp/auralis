from __future__ import annotations

import hashlib
import json
import os
import platform
import resource
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.io import wavfile

from . import benchmark_provenance

SAMPLE_RATE_HZ = 48_000


def bakeoff_deepfilter(
    binary: Path,
    model: Path,
    adapter_patch: Path,
    candidate_set_path: Path,
    corpus_manifest_path: Path,
    output_dir: Path,
) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    candidate_id = "deepfilternet3-ll-official"
    verify_frozen_model(candidate_set_path, candidate_id, model)
    if not adapter_patch.is_file():
        raise FileNotFoundError(adapter_patch)
    corpus = json.loads(corpus_manifest_path.read_text(encoding="utf-8"))
    if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
        raise ValueError("unsupported bake-off corpus manifest")
    corpus_root = corpus_manifest_path.parent
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        input_dir = temporary / "inputs"
        native_dir = temporary / "native-pcm16"
        float_dir = temporary / "raw"
        input_dir.mkdir()
        native_dir.mkdir()
        float_dir.mkdir()
        input_paths = []
        input_samples: dict[str, int] = {}
        for case in corpus["cases"]:
            source = corpus_root / case["mixture"]["path"]
            verify_asset(source, case["mixture"])
            source_info = sf.info(source)
            if (
                source_info.samplerate != SAMPLE_RATE_HZ
                or source_info.channels != 1
                or source_info.frames <= 0
            ):
                raise ValueError(f"invalid corpus input: {source}")
            input_samples[case["case_id"]] = source_info.frames
            staged = input_dir / f"{case['case_id']}.wav"
            staged.symlink_to(source.resolve())
            input_paths.append(staged)

        usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
        started = time.perf_counter_ns()
        completed = subprocess.run(
            [
                binary.as_posix(),
                "--model",
                model.as_posix(),
                "--output-dir",
                native_dir.as_posix(),
                *[path.as_posix() for path in input_paths],
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        elapsed_ns = time.perf_counter_ns() - started
        usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
        if completed.returncode != 0:
            raise RuntimeError(
                f"deep-filter failed ({completed.returncode}): {completed.stderr[-2000:]}"
            )

        output_cases = []
        for case in corpus["cases"]:
            native_path = native_dir / f"{case['case_id']}.wav"
            samples, sample_rate = sf.read(native_path, dtype="float32", always_2d=False)
            expected_samples = input_samples[case["case_id"]]
            if sample_rate != SAMPLE_RATE_HZ or samples.ndim != 1:
                raise ValueError(f"invalid DeepFilterNet output: {native_path}")
            if samples.size != expected_samples:
                raise ValueError(
                    f"DeepFilterNet sample count mismatch for {case['case_id']}: "
                    f"expected {expected_samples}, got {samples.size}"
                )
            float_path = float_dir / f"{case['case_id']}.wav"
            wavfile.write(float_path, SAMPLE_RATE_HZ, np.asarray(samples, dtype=np.float32))
            output_cases.append(
                {
                    "case_id": case["case_id"],
                    "condition": case["condition"],
                    "requested_snr_db": case["requested_snr_db"],
                    "input": case["mixture"],
                    "output": relative_asset(float_path, temporary),
                    "native_output": relative_asset(native_path, temporary)
                    | {"sample_format": "signed PCM 16-bit"},
                    "wall_time": None,
                }
            )
        report = {
            "schema_id": "auralis.offline-bakeoff-output.v1",
            "benchmark_code": benchmark_provenance(),
            "candidate_id": candidate_id,
            "candidate_set": file_asset(candidate_set_path),
            "corpus_manifest": file_asset(corpus_manifest_path),
            "model": file_asset(model),
            "adapter_binary": file_asset(binary),
            "adapter_patch": file_asset(adapter_patch),
            "runtime": {
                "platform": platform.platform(),
                "command_mode": "one process, one model initialization, all corpus files",
                "application_wall_time_ms": elapsed_ns / 1_000_000,
                "application_realtime_factor": elapsed_ns
                / (sum(input_samples.values())
                   / SAMPLE_RATE_HZ
                   * 1_000_000_000),
                "child_user_cpu_seconds": usage_after.ru_utime - usage_before.ru_utime,
                "child_system_cpu_seconds": usage_after.ru_stime - usage_before.ru_stime,
                "child_max_rss_kib": usage_after.ru_maxrss,
            },
            "signal_path": {
                "input_sample_rate_hz": SAMPLE_RATE_HZ,
                "model_sample_rate_hz": SAMPLE_RATE_HZ,
                "output_sample_rate_hz": SAMPLE_RATE_HZ,
                "stft_frame_size_samples": 960,
                "stft_hop_size_samples": 480,
                "model_lookahead_samples": 0,
                "model_algorithmic_latency_samples_at_48khz": 480,
                "model_algorithmic_latency_ms": 10.0,
                "offline_delay_compensation": False,
                "offline_output_alignment_offset_samples_at_48khz": 480,
            },
            "reset_policy": "Auralis-pinned adapter patch clones pristine model state for each input file",
            "limitations": [
                "Offline Linux/WSL screening result; not a native Windows acceptance measurement.",
                "The official CLI writes PCM16 WAV; native and deterministic float-decoded copies are both retained.",
                "Raw output retains the 480-sample deterministic STFT delay; reference metrics compensate it explicitly without subtracting it from production latency.",
                "The CLI does not expose per-frame inference timing, so wall time is only reported for the complete batch.",
            ],
            "process_stdout": completed.stdout[-4000:],
            "process_stderr": completed.stderr[-4000:],
            "case_count": len(output_cases),
            "cases": output_cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        shutil.rmtree(input_dir)
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def bakeoff_rnnoise(
    binary: Path,
    model_archive: Path,
    candidate_set_path: Path,
    corpus_manifest_path: Path,
    output_dir: Path,
) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    candidate_id = "rnnoise-main-official-model"
    verify_frozen_model(candidate_set_path, candidate_id, model_archive)
    corpus = json.loads(corpus_manifest_path.read_text(encoding="utf-8"))
    if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
        raise ValueError("unsupported bake-off corpus manifest")
    corpus_root = corpus_manifest_path.parent
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        input_dir = temporary / "native-input-pcm16le"
        native_dir = temporary / "native-output-pcm16le"
        float_dir = temporary / "raw"
        input_dir.mkdir()
        native_dir.mkdir()
        float_dir.mkdir()
        output_cases = []
        process_times_ns = []
        total_input_samples = 0
        usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
        total_started = time.perf_counter_ns()
        for case in corpus["cases"]:
            source_path = corpus_root / case["mixture"]["path"]
            verify_asset(source_path, case["mixture"])
            source, sample_rate = sf.read(source_path, dtype="float32", always_2d=False)
            if sample_rate != SAMPLE_RATE_HZ or source.ndim != 1:
                raise ValueError(f"invalid corpus input: {source_path}")
            total_input_samples += source.size
            frame_count = (source.size + 479) // 480
            padded_samples = frame_count * 480
            padded = np.pad(source, (0, padded_samples - source.size))
            pcm_input = np.clip(np.rint(padded * 32768.0), -32768, 32767).astype("<i2")
            input_path = input_dir / f"{case['case_id']}.pcm"
            native_path = native_dir / f"{case['case_id']}.pcm"
            pcm_input.tofile(input_path)
            started = time.perf_counter_ns()
            completed = subprocess.run(
                [binary.as_posix(), input_path.as_posix(), native_path.as_posix()],
                check=False,
                capture_output=True,
                text=True,
            )
            process_ns = time.perf_counter_ns() - started
            process_times_ns.append(process_ns)
            if completed.returncode != 0:
                raise RuntimeError(
                    f"RNNoise failed for {case['case_id']} ({completed.returncode}): "
                    f"{completed.stderr[-1000:]}"
                )
            native_pcm = np.fromfile(native_path, dtype="<i2")
            expected_native_samples = max(0, padded_samples - 480)
            if native_pcm.size != expected_native_samples:
                raise ValueError(
                    f"RNNoise output length mismatch for {case['case_id']}: "
                    f"expected {expected_native_samples}, got {native_pcm.size}"
                )
            decoded = native_pcm.astype(np.float32) / 32768.0
            decoded = np.pad(decoded, (0, max(0, source.size - decoded.size)))[: source.size]
            float_path = float_dir / f"{case['case_id']}.wav"
            wavfile.write(float_path, SAMPLE_RATE_HZ, decoded)
            output_cases.append(
                {
                    "case_id": case["case_id"],
                    "condition": case["condition"],
                    "requested_snr_db": case["requested_snr_db"],
                    "input": case["mixture"],
                    "output": relative_asset(float_path, temporary),
                    "native_input": relative_asset(input_path, temporary)
                    | {"sample_format": "signed PCM 16-bit little-endian"},
                    "native_output": relative_asset(native_path, temporary)
                    | {"sample_format": "signed PCM 16-bit little-endian"},
                    "wall_time": {"complete_process_ms": process_ns / 1_000_000},
                }
            )
        total_ns = time.perf_counter_ns() - total_started
        usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
        sorted_times = np.sort(np.asarray(process_times_ns, dtype=np.int64))
        report = {
            "schema_id": "auralis.offline-bakeoff-output.v1",
            "benchmark_code": benchmark_provenance(),
            "candidate_id": candidate_id,
            "candidate_set": file_asset(candidate_set_path),
            "corpus_manifest": file_asset(corpus_manifest_path),
            "model": file_asset(model_archive),
            "adapter_binary": file_asset(binary),
            "runtime": {
                "platform": platform.platform(),
                "command_mode": "one portable demo process per corpus case",
                "application_wall_time_ms": total_ns / 1_000_000,
                "application_realtime_factor": total_ns
                / (total_input_samples
                   / SAMPLE_RATE_HZ
                   * 1_000_000_000),
                "per_file_process_ms": {
                    "mean": float(np.mean(sorted_times)) / 1_000_000,
                    "p50": float(np.quantile(sorted_times, 0.50, method="higher")) / 1_000_000,
                    "p95": float(np.quantile(sorted_times, 0.95, method="higher")) / 1_000_000,
                    "p99": float(np.quantile(sorted_times, 0.99, method="higher")) / 1_000_000,
                    "maximum": int(sorted_times[-1]) / 1_000_000,
                },
                "child_user_cpu_seconds": usage_after.ru_utime - usage_before.ru_utime,
                "child_system_cpu_seconds": usage_after.ru_stime - usage_before.ru_stime,
                "child_max_rss_kib": usage_after.ru_maxrss,
            },
            "signal_path": {
                "input_sample_rate_hz": SAMPLE_RATE_HZ,
                "model_sample_rate_hz": SAMPLE_RATE_HZ,
                "output_sample_rate_hz": SAMPLE_RATE_HZ,
                "frame_size_samples": 480,
                "analysis_window_samples": 960,
                "explicit_delayed_spectrum_frames": 1,
                "model_algorithmic_latency_samples_at_48khz": 960,
                "model_algorithmic_latency_ms": 20.0,
                "demo_dropped_initial_output_samples": 480,
                "offline_output_alignment_offset_samples_at_48khz": 480,
            },
            "reset_policy": "new RNNoise state and process for every corpus case",
            "limitations": [
                "Offline Linux/WSL screening result; not a native Windows acceptance measurement.",
                "Portable demo build uses only baseline SSE/SSE2 and includes process startup in per-file timing.",
                "The demo quantizes float corpus input to PCM16 and emits PCM16; exact native input/output bytes are retained.",
                "The official model archive is verified, but the demo embeds generated weights; binary hash is recorded and build provenance must be reproduced before deployment.",
                "The demo drops its first 480-sample all-latency output frame; 480 samples of deterministic residual delay remain for metric alignment.",
            ],
            "case_count": len(output_cases),
            "cases": output_cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def verify_frozen_model(candidate_set: Path, candidate_id: str, model: Path) -> None:
    frozen = json.loads(candidate_set.read_text(encoding="utf-8"))
    candidates = [item for item in frozen["candidates"] if item["candidate_id"] == candidate_id]
    if len(candidates) != 1:
        raise ValueError(f"candidate missing or duplicated: {candidate_id}")
    expected = candidates[0]["artifact"]
    actual = file_asset(model)
    if actual["sha256"] != expected["sha256"] or actual["size_bytes"] != expected["size_bytes"]:
        raise ValueError(f"model artifact mismatch: {candidate_id}")


def verify_asset(path: Path, expected: dict[str, object]) -> None:
    actual = file_asset(path)
    if actual["sha256"] != expected["sha256"] or actual["size_bytes"] != expected["size_bytes"]:
        raise ValueError(f"asset mismatch: {path}")


def relative_asset(path: Path, root: Path) -> dict[str, object]:
    return file_asset(path) | {"path": path.relative_to(root).as_posix()}


def file_asset(path: Path) -> dict[str, object]:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        while chunk := file.read(1024 * 1024):
            digest.update(chunk)
    return {
        "path": path.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": digest.hexdigest(),
    }
