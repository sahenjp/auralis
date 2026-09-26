from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import sys
import time
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import onnxruntime as ort
import scipy
from scipy.io import wavfile
from scipy.signal import resample_poly
import soundfile as sf

from .corpus import (
    fetch_demand,
    fetch_fleurs,
    fetch_voicebank,
    generate_transient_corpus,
    mix_corpus,
)
from .external import bakeoff_deepfilter, bakeoff_rnnoise
from .quality import analyze_bakeoff, diagnose_alignment
from .summary import summarize_bakeoff
from .transitions import analyze_transitions
from . import benchmark_provenance
from .listening import (
    create_abx_session,
    create_rating_session,
    record_response,
    run_rating_session,
    validate_abx_session,
    validate_rating_session,
)

INPUT_RATE_HZ = 48_000
MODEL_RATE_HZ = 16_000
FFT_SAMPLES = 512
HOP_SAMPLES = 256
MODEL_ALGORITHM_DELAY_SAMPLES = FFT_SAMPLES - HOP_SAMPLES
FRONTENDS = ("official-centered", "causal-left-padded")


@dataclass(frozen=True)
class CandidateContract:
    candidate_id: str
    window: str
    cache_shapes: dict[str, tuple[int, ...]]


CONTRACTS = {
    "gtcrn": CandidateContract(
        candidate_id="gtcrn-dns3-streaming-onnx",
        window="sqrt_hann",
        cache_shapes={
            "conv_cache": (2, 1, 16, 16, 33),
            "tra_cache": (2, 3, 1, 1, 16),
            "inter_cache": (2, 1, 33, 16),
        },
    ),
    "ul-unas": CandidateContract(
        candidate_id="ul-unas-dns3-streaming-onnx",
        window="hann",
        cache_shapes={
            "conv_cache": (1, 5358),
            "tfa_cache": (1, 402),
            "inter_cache": (1, 1056),
        },
    ),
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    process = subparsers.add_parser("process")
    process.add_argument("--candidate", choices=sorted(CONTRACTS), required=True)
    process.add_argument("--model", type=Path, required=True)
    process.add_argument("--input", type=Path, required=True)
    process.add_argument("--output", type=Path, required=True)
    process.add_argument("--report", type=Path, required=True)
    process.add_argument("--threads", type=int, default=1)
    process.add_argument("--frontend", choices=FRONTENDS, default="official-centered")
    fetch = subparsers.add_parser("fetch-fleurs")
    fetch.add_argument("--spec", type=Path, required=True)
    fetch.add_argument("--out-dir", type=Path, required=True)
    demand = subparsers.add_parser("fetch-demand")
    demand.add_argument("--spec", type=Path, required=True)
    demand.add_argument("--out-dir", type=Path, required=True)
    voicebank = subparsers.add_parser("fetch-voicebank")
    voicebank.add_argument("--spec", type=Path, required=True)
    voicebank.add_argument("--out-dir", type=Path, required=True)
    voicebank.add_argument("--cache-dir", type=Path, required=True)
    mix = subparsers.add_parser("mix-corpus")
    mix.add_argument("--clean-manifest", type=Path, action="append", required=True)
    mix.add_argument("--noise-manifest", type=Path, required=True)
    mix.add_argument("--out-dir", type=Path, required=True)
    mix.add_argument("--corpus-id", required=True)
    transients = subparsers.add_parser("generate-transient-corpus")
    transients.add_argument("--spec", type=Path, required=True)
    transients.add_argument("--clean-manifest", type=Path, action="append", required=True)
    transients.add_argument("--noise-manifest", type=Path, required=True)
    transients.add_argument("--out-dir", type=Path, required=True)
    bakeoff = subparsers.add_parser("bakeoff")
    bakeoff.add_argument("--candidate", choices=sorted(CONTRACTS), required=True)
    bakeoff.add_argument("--model", type=Path, required=True)
    bakeoff.add_argument("--candidate-set", type=Path, required=True)
    bakeoff.add_argument("--corpus-manifest", type=Path, required=True)
    bakeoff.add_argument("--out-dir", type=Path, required=True)
    bakeoff.add_argument("--threads", type=int, default=1)
    bakeoff.add_argument("--frontend", choices=FRONTENDS, default="official-centered")
    verify = subparsers.add_parser("verify-official-fixture")
    verify.add_argument("--candidate", choices=sorted(CONTRACTS), required=True)
    verify.add_argument("--model", type=Path, required=True)
    verify.add_argument("--input", type=Path, required=True)
    verify.add_argument("--expected", type=Path, required=True)
    verify.add_argument("--output", type=Path, required=True)
    verify.add_argument("--report", type=Path, required=True)
    analyze = subparsers.add_parser("analyze-bakeoff")
    analyze.add_argument("--corpus-manifest", type=Path, required=True)
    analyze.add_argument("--output-manifest", type=Path, required=True)
    analyze.add_argument("--report", type=Path, required=True)
    alignment = subparsers.add_parser("diagnose-alignment")
    alignment.add_argument("--reference", type=Path, required=True)
    alignment.add_argument("--output", type=Path, required=True)
    alignment.add_argument("--report", type=Path, required=True)
    alignment.add_argument("--minimum-offset-samples", type=int, default=0)
    alignment.add_argument("--maximum-offset-samples", type=int, required=True)
    summary = subparsers.add_parser("summarize-bakeoff")
    summary.add_argument("--objective-report", type=Path, action="append", required=True)
    summary.add_argument("--perceptual-report", type=Path, action="append", required=True)
    summary.add_argument("--output-manifest", type=Path, action="append", required=True)
    summary.add_argument("--report", type=Path, required=True)
    transition = subparsers.add_parser("analyze-transitions")
    transition.add_argument("--corpus-manifest", type=Path, required=True)
    transition.add_argument("--output-manifest", type=Path, required=True)
    transition.add_argument("--report", type=Path, required=True)
    listen = subparsers.add_parser("create-abx-session")
    listen.add_argument("--spec", type=Path, required=True)
    listen.add_argument("--corpus-manifest", type=Path, required=True)
    listen.add_argument("--output-manifest", type=Path, action="append", required=True)
    listen.add_argument("--session-dir", type=Path, required=True)
    listen.add_argument("--private-key", type=Path, required=True)
    listen.add_argument("--session-id", required=True)
    listen.add_argument("--seed-hex", required=True)
    validate_listen = subparsers.add_parser("validate-abx-session")
    validate_listen.add_argument("--session-dir", type=Path, required=True)
    validate_listen.add_argument("--private-key", type=Path, required=True)
    validate_listen.add_argument("--report", type=Path, required=True)
    response = subparsers.add_parser("record-listening-response")
    response.add_argument("--session-dir", type=Path, required=True)
    response.add_argument("--results-jsonl", type=Path, required=True)
    response.add_argument("--listener-id", required=True)
    response.add_argument("--trial-id", required=True)
    response.add_argument("--x-guess", choices=("A", "B"), required=True)
    response.add_argument("--preference", choices=("A", "B", "tie"), required=True)
    for rating in (
        "noise-suppression",
        "speech-naturalness",
        "intelligibility",
        "artifact-freedom",
        "consonant-preservation",
        "transient-behavior",
    ):
        response.add_argument(f"--a-{rating}", type=int, required=True)
        response.add_argument(f"--b-{rating}", type=int, required=True)
    create_rating = subparsers.add_parser("create-rating-session")
    create_rating.add_argument("--spec", type=Path, required=True)
    create_rating.add_argument("--session-dir", type=Path, required=True)
    create_rating.add_argument("--private-key", type=Path, required=True)
    create_rating.add_argument("--session-id", required=True)
    create_rating.add_argument("--seed-hex", required=True)
    validate_rating = subparsers.add_parser("validate-rating-session")
    validate_rating.add_argument("--session-dir", type=Path, required=True)
    validate_rating.add_argument("--private-key", type=Path, required=True)
    validate_rating.add_argument("--report", type=Path, required=True)
    run_rating = subparsers.add_parser("run-rating-session")
    run_rating.add_argument("--session-dir", type=Path, required=True)
    run_rating.add_argument("--results-jsonl", type=Path, required=True)
    run_rating.add_argument("--listener-id", required=True)
    run_rating.add_argument("--player", default="ffplay")
    run_rating.add_argument("--verify-only", action="store_true")
    deepfilter = subparsers.add_parser("bakeoff-deepfilter")
    deepfilter.add_argument("--binary", type=Path, required=True)
    deepfilter.add_argument("--model", type=Path, required=True)
    deepfilter.add_argument("--adapter-patch", type=Path, required=True)
    deepfilter.add_argument("--candidate-set", type=Path, required=True)
    deepfilter.add_argument("--corpus-manifest", type=Path, required=True)
    deepfilter.add_argument("--out-dir", type=Path, required=True)
    rnnoise = subparsers.add_parser("bakeoff-rnnoise")
    rnnoise.add_argument("--binary", type=Path, required=True)
    rnnoise.add_argument("--model-archive", type=Path, required=True)
    rnnoise.add_argument("--candidate-set", type=Path, required=True)
    rnnoise.add_argument("--corpus-manifest", type=Path, required=True)
    rnnoise.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "process":
        run_process(args)
    elif args.command == "fetch-fleurs":
        fetch_fleurs(args.spec, args.out_dir)
    elif args.command == "fetch-demand":
        fetch_demand(args.spec, args.out_dir)
    elif args.command == "fetch-voicebank":
        fetch_voicebank(args.spec, args.out_dir, args.cache_dir)
    elif args.command == "mix-corpus":
        mix_corpus(
            args.clean_manifest, args.noise_manifest, args.out_dir, args.corpus_id
        )
    elif args.command == "generate-transient-corpus":
        generate_transient_corpus(
            args.spec, args.clean_manifest, args.noise_manifest, args.out_dir
        )
    elif args.command == "bakeoff":
        run_bakeoff(args)
    elif args.command == "verify-official-fixture":
        verify_official_fixture(args)
    elif args.command == "analyze-bakeoff":
        analyze_bakeoff(args.corpus_manifest, args.output_manifest, args.report)
    elif args.command == "diagnose-alignment":
        diagnose_alignment(
            args.reference,
            args.output,
            args.report,
            args.minimum_offset_samples,
            args.maximum_offset_samples,
        )
    elif args.command == "summarize-bakeoff":
        summarize_bakeoff(
            args.objective_report,
            args.perceptual_report,
            args.output_manifest,
            args.report,
        )
    elif args.command == "analyze-transitions":
        analyze_transitions(args.corpus_manifest, args.output_manifest, args.report)
    elif args.command == "create-abx-session":
        create_abx_session(
            args.spec,
            args.corpus_manifest,
            args.output_manifest,
            args.session_dir,
            args.private_key,
            args.session_id,
            args.seed_hex,
        )
    elif args.command == "validate-abx-session":
        validate_abx_session(args.session_dir, args.private_key, args.report)
    elif args.command == "record-listening-response":
        record_response(
            args.session_dir,
            args.results_jsonl,
            args.listener_id,
            args.trial_id,
            args.x_guess,
            args.preference,
            {
                "A": {
                    "noise_suppression": args.a_noise_suppression,
                    "speech_naturalness": args.a_speech_naturalness,
                    "intelligibility": args.a_intelligibility,
                    "artifact_freedom": args.a_artifact_freedom,
                    "consonant_preservation": args.a_consonant_preservation,
                    "transient_behavior": args.a_transient_behavior,
                },
                "B": {
                    "noise_suppression": args.b_noise_suppression,
                    "speech_naturalness": args.b_speech_naturalness,
                    "intelligibility": args.b_intelligibility,
                    "artifact_freedom": args.b_artifact_freedom,
                    "consonant_preservation": args.b_consonant_preservation,
                    "transient_behavior": args.b_transient_behavior,
                },
            },
        )
    elif args.command == "create-rating-session":
        create_rating_session(
            args.spec,
            args.session_dir,
            args.private_key,
            args.session_id,
            args.seed_hex,
        )
    elif args.command == "validate-rating-session":
        validate_rating_session(args.session_dir, args.private_key, args.report)
    elif args.command == "run-rating-session":
        run_rating_session(
            args.session_dir,
            args.results_jsonl,
            args.listener_id,
            args.player,
            args.verify_only,
        )
    elif args.command == "bakeoff-deepfilter":
        bakeoff_deepfilter(
            args.binary,
            args.model,
            args.adapter_patch,
            args.candidate_set,
            args.corpus_manifest,
            args.out_dir,
        )
    elif args.command == "bakeoff-rnnoise":
        bakeoff_rnnoise(
            args.binary,
            args.model_archive,
            args.candidate_set,
            args.corpus_manifest,
            args.out_dir,
        )


def run_process(args: argparse.Namespace) -> None:
    if args.threads < 1:
        raise ValueError("--threads must be at least one")
    for path in (args.output, args.report):
        if path.exists():
            raise FileExistsError(f"refusing to overwrite {path}")
        path.parent.mkdir(parents=True, exist_ok=True)

    contract = CONTRACTS[args.candidate]
    source, sample_rate = sf.read(args.input, dtype="float32", always_2d=False)
    if sample_rate != INPUT_RATE_HZ or source.ndim != 1:
        raise ValueError("input must be mono 48 kHz PCM")
    source = np.ascontiguousarray(source, dtype=np.float32)

    total_started = time.perf_counter_ns()
    down_started = time.perf_counter_ns()
    model_input = np.asarray(resample_poly(source, 1, 3), dtype=np.float32)
    down_ns = elapsed_ns(down_started)

    session_started = time.perf_counter_ns()
    session_options = ort.SessionOptions()
    session_options.intra_op_num_threads = args.threads
    session_options.inter_op_num_threads = 1
    session_options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    session_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    session = ort.InferenceSession(
        args.model.as_posix(),
        sess_options=session_options,
        providers=["CPUExecutionProvider"],
    )
    session_init_ns = elapsed_ns(session_started)
    validate_session_contract(session, contract)

    enhanced_16k, stages = process_spectral_frames(
        model_input, session, contract, args.frontend
    )
    up_started = time.perf_counter_ns()
    enhanced_48k = np.asarray(resample_poly(enhanced_16k, 3, 1), dtype=np.float32)
    up_ns = elapsed_ns(up_started)
    enhanced_48k = exact_length(enhanced_48k, source.size)
    total_ns = elapsed_ns(total_started)

    # scipy writes a stable IEEE-float RIFF header. libsndfile adds a PEAK
    # chunk containing the current time, which makes byte hashes differ even
    # when every PCM sample is identical.
    wavfile.write(args.output, INPUT_RATE_HZ, enhanced_48k)
    report = {
        "schema_id": "auralis.offline-candidate-runtime.v1",
        "benchmark_code": benchmark_provenance(),
        "candidate_id": contract.candidate_id,
        "input": asset(args.input),
        "output": asset(args.output),
        "model": asset(args.model),
        "runtime": {
            "python": sys.version,
            "platform": platform.platform(),
            "numpy": np.__version__,
            "scipy": scipy.__version__,
            "onnxruntime": ort.__version__,
            "execution_provider": "CPUExecutionProvider",
            "intra_op_threads": args.threads,
            "inter_op_threads": 1,
            "execution_mode": "ORT_SEQUENTIAL",
            "process_affinity": affinity(),
        },
        "signal_path": {
            "input_sample_rate_hz": INPUT_RATE_HZ,
            "model_sample_rate_hz": MODEL_RATE_HZ,
            "output_sample_rate_hz": INPUT_RATE_HZ,
            "input_samples": int(source.size),
            "model_samples": int(model_input.size),
            "output_samples": int(enhanced_48k.size),
            "stft_frame_size_samples": FFT_SAMPLES,
            "stft_hop_size_samples": HOP_SAMPLES,
            "window": contract.window,
            "frontend": args.frontend,
            "centered_offline_padding": args.frontend == "official-centered",
            "model_lookahead_samples": 0,
            "model_algorithmic_latency_samples_at_16khz": MODEL_ALGORITHM_DELAY_SAMPLES,
            "model_algorithmic_latency_ms": MODEL_ALGORITHM_DELAY_SAMPLES
            / MODEL_RATE_HZ
            * 1000,
            "offline_output_alignment_offset_samples_at_48khz": (
                0 if args.frontend == "official-centered" else MODEL_ALGORITHM_DELAY_SAMPLES * 3
            ),
            "resampler_algorithmic_latency_samples": None,
            "resampler_latency_status": "unknown_for_production; scipy batch resample_poly compensates FIR group delay",
        },
        "wall_time": {
            "session_initialization_ms": ns_to_ms(session_init_ns),
            "downsampling_ms": ns_to_ms(down_ns),
            "analysis_ms": ns_to_ms(stages["analysis_ns"]),
            "model_inference": timing_summary(stages["inference_ns"]),
            "synthesis_ms": ns_to_ms(stages["synthesis_ns"]),
            "upsampling_ms": ns_to_ms(up_ns),
            "complete_path_ms": ns_to_ms(total_ns),
            "complete_path_realtime_factor": total_ns
            / (source.size / INPUT_RATE_HZ * 1_000_000_000),
        },
        "limitations": [
            "Offline Linux/WSL screening result; not a native Windows acceptance measurement.",
            "SciPy batch resampling wall time is included, but its production streaming delay and memory are not measured.",
            "The official repositories demonstrate frame-wise model inference over an offline STFT; this runner uses an explicitly causal left-padded frontend and requires waveform equivalence review.",
        ],
    }
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_bakeoff(args: argparse.Namespace) -> None:
    if args.threads < 1:
        raise ValueError("--threads must be at least one")
    if args.out_dir.exists():
        raise FileExistsError(f"refusing to overwrite {args.out_dir}")
    contract = CONTRACTS[args.candidate]
    verify_frozen_model(args.candidate_set, contract.candidate_id, args.model)
    corpus = json.loads(args.corpus_manifest.read_text(encoding="utf-8"))
    if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
        raise ValueError("unsupported bake-off corpus manifest")

    args.out_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.out_dir.parent / f".{args.out_dir.name}-{os.getpid()}"
    temporary.mkdir()
    try:
        session_started = time.perf_counter_ns()
        session = create_session(args.model, args.threads)
        session_init_ns = elapsed_ns(session_started)
        validate_session_contract(session, contract)
        output_cases = []
        corpus_root = args.corpus_manifest.parent
        for case in corpus["cases"]:
            input_path = corpus_root / case["mixture"]["path"]
            verify_asset(input_path, case["mixture"])
            source, sample_rate = sf.read(input_path, dtype="float32", always_2d=False)
            if sample_rate != INPUT_RATE_HZ or source.ndim != 1:
                raise ValueError(f"invalid corpus input: {input_path}")
            output, timing = process_complete_path(
                np.ascontiguousarray(source, dtype=np.float32),
                session,
                contract,
                args.frontend,
            )
            output_path = temporary / "raw" / f"{case['case_id']}.wav"
            output_path.parent.mkdir(exist_ok=True)
            wavfile.write(output_path, INPUT_RATE_HZ, output)
            output_cases.append(
                {
                    "case_id": case["case_id"],
                    "condition": case["condition"],
                    "requested_snr_db": case["requested_snr_db"],
                    "input": case["mixture"],
                    "output": relative_asset(output_path, temporary),
                    "wall_time": timing,
                }
            )
        report = {
            "schema_id": "auralis.offline-bakeoff-output.v1",
            "benchmark_code": benchmark_provenance(),
            "candidate_id": contract.candidate_id,
            "candidate_set": asset(args.candidate_set),
            "corpus_manifest": asset(args.corpus_manifest),
            "model": asset(args.model),
            "runtime": runtime_metadata(args.threads),
            "session_initialization_ms": ns_to_ms(session_init_ns),
            "reset_policy": "zero all model caches independently before every corpus case",
            "signal_path": signal_path_metadata(contract, args.frontend),
            "case_count": len(output_cases),
            "cases": output_cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, args.out_dir)
    except BaseException:
        import shutil

        shutil.rmtree(temporary, ignore_errors=True)
        raise


def verify_official_fixture(args: argparse.Namespace) -> None:
    for path in (args.output, args.report):
        if path.exists():
            raise FileExistsError(f"refusing to overwrite {path}")
        path.parent.mkdir(parents=True, exist_ok=True)
    source, source_rate = sf.read(args.input, dtype="float32", always_2d=False)
    expected, expected_rate = sf.read(args.expected, dtype="float32", always_2d=False)
    if source_rate != MODEL_RATE_HZ or expected_rate != MODEL_RATE_HZ:
        raise ValueError("official fixtures must be 16 kHz")
    if source.ndim != 1 or expected.ndim != 1:
        raise ValueError("official fixtures must be mono audio")
    contract = CONTRACTS[args.candidate]
    session = create_session(args.model, 1)
    validate_session_contract(session, contract)
    actual, stages = process_spectral_frames(
        np.ascontiguousarray(source, dtype=np.float32),
        session,
        contract,
        "official-centered",
    )
    compared_samples = min(actual.size, expected.size)
    compared_actual = actual[:compared_samples].astype(np.float64)
    compared_expected = expected[:compared_samples].astype(np.float64)
    difference = compared_actual - compared_expected
    target_energy = float(np.sum(np.square(compared_expected, dtype=np.float64)))
    error_energy = float(np.sum(np.square(difference, dtype=np.float64)))
    scale = float(np.dot(compared_expected, compared_actual)) / max(
        target_energy, np.finfo(np.float64).eps
    )
    projection = compared_expected * scale
    si_error = compared_actual - projection
    report = {
        "schema_id": "auralis.official-fixture-equivalence.v1",
        "candidate_id": contract.candidate_id,
        "frontend": "official-centered",
        "input": asset(args.input),
        "expected": asset(args.expected),
        "model": asset(args.model),
        "output": None,
        "sample_rate_hz": MODEL_RATE_HZ,
        "input_samples": int(source.size),
        "expected_samples": int(expected.size),
        "output_samples": int(actual.size),
        "compared_samples": int(compared_samples),
        "comparison": {
            "maximum_absolute_error": float(np.max(np.abs(difference))),
            "root_mean_square_error": float(np.sqrt(np.mean(np.square(difference)))),
            "sdr_db": float(
                10.0
                * np.log10(
                    max(target_energy, np.finfo(np.float64).eps)
                    / max(error_energy, np.finfo(np.float64).eps)
                )
            ),
            "si_sdr_db": float(
                10.0
                * np.log10(
                    max(float(np.sum(projection * projection)), np.finfo(np.float64).eps)
                    / max(float(np.sum(si_error * si_error)), np.finfo(np.float64).eps)
                )
            ),
        },
        "inference": timing_summary(stages["inference_ns"]),
        "interpretation": "Expected WAV is 16-bit PCM, so byte/sample equality is not expected from float inference.",
    }
    wavfile.write(args.output, MODEL_RATE_HZ, actual)
    report["output"] = asset(args.output)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def process_complete_path(
    source: np.ndarray,
    session: ort.InferenceSession,
    contract: CandidateContract,
    frontend: str,
) -> tuple[np.ndarray, dict[str, object]]:
    total_started = time.perf_counter_ns()
    down_started = time.perf_counter_ns()
    model_input = np.asarray(resample_poly(source, 1, 3), dtype=np.float32)
    down_ns = elapsed_ns(down_started)
    enhanced_16k, stages = process_spectral_frames(model_input, session, contract, frontend)
    up_started = time.perf_counter_ns()
    enhanced_48k = np.asarray(resample_poly(enhanced_16k, 3, 1), dtype=np.float32)
    up_ns = elapsed_ns(up_started)
    enhanced_48k = exact_length(enhanced_48k, source.size)
    total_ns = elapsed_ns(total_started)
    return enhanced_48k, {
        "downsampling_ms": ns_to_ms(down_ns),
        "analysis_ms": ns_to_ms(stages["analysis_ns"]),
        "model_inference": timing_summary(stages["inference_ns"]),
        "model_inference_ns_raw": stages["inference_ns"],
        "synthesis_ms": ns_to_ms(stages["synthesis_ns"]),
        "upsampling_ms": ns_to_ms(up_ns),
        "complete_path_ms": ns_to_ms(total_ns),
        "complete_path_realtime_factor": total_ns
        / (source.size / INPUT_RATE_HZ * 1_000_000_000),
    }


def create_session(model: Path, threads: int) -> ort.InferenceSession:
    session_options = ort.SessionOptions()
    session_options.intra_op_num_threads = threads
    session_options.inter_op_num_threads = 1
    session_options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    session_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    return ort.InferenceSession(
        model.as_posix(), sess_options=session_options, providers=["CPUExecutionProvider"]
    )


def verify_frozen_model(candidate_set: Path, candidate_id: str, model: Path) -> None:
    frozen = json.loads(candidate_set.read_text(encoding="utf-8"))
    candidates = [item for item in frozen["candidates"] if item["candidate_id"] == candidate_id]
    if len(candidates) != 1:
        raise ValueError(f"candidate missing or duplicated in frozen set: {candidate_id}")
    expected = candidates[0]["artifact"]
    actual = asset(model)
    if actual["sha256"] != expected["sha256"] or actual["size_bytes"] != expected["size_bytes"]:
        raise ValueError(f"model does not match frozen metadata: {candidate_id}")


def verify_asset(path: Path, expected: dict[str, object]) -> None:
    actual = asset(path)
    if actual["sha256"] != expected["sha256"] or actual["size_bytes"] != expected["size_bytes"]:
        raise ValueError(f"corpus asset does not match manifest: {path}")


def relative_asset(path: Path, root: Path) -> dict[str, object]:
    return asset(path) | {"path": path.relative_to(root).as_posix()}


def runtime_metadata(threads: int) -> dict[str, object]:
    return {
        "python": sys.version,
        "platform": platform.platform(),
        "numpy": np.__version__,
        "scipy": scipy.__version__,
        "onnxruntime": ort.__version__,
        "execution_provider": "CPUExecutionProvider",
        "intra_op_threads": threads,
        "inter_op_threads": 1,
        "execution_mode": "ORT_SEQUENTIAL",
        "process_affinity": affinity(),
    }


def signal_path_metadata(contract: CandidateContract, frontend: str) -> dict[str, object]:
    return {
        "input_sample_rate_hz": INPUT_RATE_HZ,
        "model_sample_rate_hz": MODEL_RATE_HZ,
        "output_sample_rate_hz": INPUT_RATE_HZ,
        "stft_frame_size_samples": FFT_SAMPLES,
        "stft_hop_size_samples": HOP_SAMPLES,
        "window": contract.window,
        "frontend": frontend,
        "centered_offline_padding": frontend == "official-centered",
        "model_lookahead_samples": 0,
        "model_algorithmic_latency_samples_at_16khz": MODEL_ALGORITHM_DELAY_SAMPLES,
        "model_algorithmic_latency_samples_at_48khz": MODEL_ALGORITHM_DELAY_SAMPLES * 3,
        "model_algorithmic_latency_ms": MODEL_ALGORITHM_DELAY_SAMPLES / MODEL_RATE_HZ * 1000,
        "offline_output_alignment_offset_samples_at_48khz": (
            0 if frontend == "official-centered" else MODEL_ALGORITHM_DELAY_SAMPLES * 3
        ),
        "resampler_algorithmic_latency_samples": None,
        "resampler_latency_status": "unknown_for_production; scipy batch resample_poly compensates FIR group delay",
    }


def process_spectral_frames(
    samples: np.ndarray,
    session: ort.InferenceSession,
    contract: CandidateContract,
    frontend: str,
) -> tuple[np.ndarray, dict[str, object]]:
    if frontend not in FRONTENDS:
        raise ValueError(f"unsupported frontend: {frontend}")
    # torch.hann_window defaults to periodic=True. np.hanning(N) is symmetric,
    # so generate N+1 points and drop the duplicated endpoint.
    window = np.hanning(FFT_SAMPLES + 1)[:-1].astype(np.float32)
    if contract.window == "sqrt_hann":
        window = np.sqrt(window).astype(np.float32)
    if frontend == "official-centered":
        return process_centered_frames(samples, session, contract, window)
    return process_causal_frames(samples, session, contract, window)


def process_centered_frames(
    samples: np.ndarray,
    session: ort.InferenceSession,
    contract: CandidateContract,
    window: np.ndarray,
) -> tuple[np.ndarray, dict[str, object]]:
    padded = np.pad(samples, (FFT_SAMPLES // 2, FFT_SAMPLES // 2), mode="reflect")
    frame_offsets = list(range(0, padded.size - FFT_SAMPLES + 1, HOP_SAMPLES))
    synthesis_length = frame_offsets[-1] + FFT_SAMPLES
    overlap = np.zeros(synthesis_length, dtype=np.float32)
    normalization = np.zeros(synthesis_length, dtype=np.float32)
    caches = new_caches(contract)
    inference_ns: list[int] = []
    analysis_ns = 0
    synthesis_ns = 0
    for offset in frame_offsets:
        analysis_started = time.perf_counter_ns()
        spectrum = np.fft.rfft(padded[offset : offset + FFT_SAMPLES] * window).astype(
            np.complex64
        )
        analysis_ns += elapsed_ns(analysis_started)
        enhanced, inference_time = infer_spectrum(spectrum, session, caches)
        inference_ns.append(inference_time)
        synthesis_started = time.perf_counter_ns()
        time_frame = np.fft.irfft(enhanced, n=FFT_SAMPLES).astype(np.float32) * window
        overlap[offset : offset + FFT_SAMPLES] += time_frame
        normalization[offset : offset + FFT_SAMPLES] += window * window
        synthesis_ns += elapsed_ns(synthesis_started)
    reconstructed = np.divide(
        overlap,
        normalization,
        out=np.zeros_like(overlap),
        where=normalization > 1e-8,
    )
    centered = reconstructed[FFT_SAMPLES // 2 :]
    return exact_length(centered, samples.size), {
        "analysis_ns": analysis_ns,
        "inference_ns": inference_ns,
        "synthesis_ns": synthesis_ns,
    }


def process_causal_frames(
    samples: np.ndarray,
    session: ort.InferenceSession,
    contract: CandidateContract,
    window: np.ndarray,
) -> tuple[np.ndarray, dict[str, object]]:
    history = np.zeros(FFT_SAMPLES - HOP_SAMPLES, dtype=np.float32)
    overlap = np.zeros(FFT_SAMPLES, dtype=np.float32)
    normalization = np.zeros(FFT_SAMPLES, dtype=np.float32)
    caches = new_caches(contract)
    output_chunks: list[np.ndarray] = []
    inference_ns: list[int] = []
    analysis_ns = 0
    synthesis_ns = 0

    padded_count = ((samples.size + HOP_SAMPLES - 1) // HOP_SAMPLES) * HOP_SAMPLES
    padded = np.pad(samples, (0, padded_count - samples.size))
    for offset in range(0, padded.size, HOP_SAMPLES):
        hop = padded[offset : offset + HOP_SAMPLES]
        analysis_started = time.perf_counter_ns()
        frame = np.concatenate((history, hop))
        spectrum = np.fft.rfft(frame * window).astype(np.complex64)
        analysis_ns += elapsed_ns(analysis_started)
        enhanced, inference_time = infer_spectrum(spectrum, session, caches)
        inference_ns.append(inference_time)

        synthesis_started = time.perf_counter_ns()
        time_frame = np.fft.irfft(enhanced, n=FFT_SAMPLES).astype(np.float32) * window
        overlap += time_frame
        normalization += window * window
        emitted = np.divide(
            overlap[:HOP_SAMPLES],
            normalization[:HOP_SAMPLES],
            out=np.zeros(HOP_SAMPLES, dtype=np.float32),
            where=normalization[:HOP_SAMPLES] > 1e-8,
        )
        output_chunks.append(emitted)
        overlap[:-HOP_SAMPLES] = overlap[HOP_SAMPLES:]
        overlap[-HOP_SAMPLES:] = 0
        normalization[:-HOP_SAMPLES] = normalization[HOP_SAMPLES:]
        normalization[-HOP_SAMPLES:] = 0
        history[:] = hop
        synthesis_ns += elapsed_ns(synthesis_started)

    output = np.concatenate(output_chunks)[: samples.size]
    return output, {
        "analysis_ns": analysis_ns,
        "inference_ns": inference_ns,
        "synthesis_ns": synthesis_ns,
    }


def new_caches(contract: CandidateContract) -> dict[str, np.ndarray]:
    return {
        name: np.zeros(shape, dtype=np.float32)
        for name, shape in contract.cache_shapes.items()
    }


def infer_spectrum(
    spectrum: np.ndarray,
    session: ort.InferenceSession,
    caches: dict[str, np.ndarray],
) -> tuple[np.ndarray, int]:
    model_input = np.stack((spectrum.real, spectrum.imag), axis=-1)[None, :, None, :]
    feeds = {"mix": np.ascontiguousarray(model_input, dtype=np.float32), **caches}
    inference_started = time.perf_counter_ns()
    outputs = session.run(None, feeds)
    inference_ns = elapsed_ns(inference_started)
    enhanced = outputs[0][0, :, 0, 0] + 1j * outputs[0][0, :, 0, 1]
    for name, value in zip(caches, outputs[1:], strict=True):
        caches[name] = np.ascontiguousarray(value, dtype=np.float32)
    return enhanced, inference_ns


def validate_session_contract(session: ort.InferenceSession, contract: CandidateContract) -> None:
    input_names = [value.name for value in session.get_inputs()]
    expected = ["mix", *contract.cache_shapes]
    if input_names != expected:
        raise ValueError(f"unexpected ONNX inputs: expected {expected}, got {input_names}")
    if session.get_providers() != ["CPUExecutionProvider"]:
        raise ValueError(f"unexpected execution providers: {session.get_providers()}")


def timing_summary(values_ns: list[int]) -> dict[str, float | int]:
    values = np.sort(np.asarray(values_ns, dtype=np.int64))
    return {
        "operations": int(values.size),
        "mean_ms": ns_to_ms(int(np.mean(values))),
        "p50_ms": ns_to_ms(int(np.quantile(values, 0.50, method="higher"))),
        "p95_ms": ns_to_ms(int(np.quantile(values, 0.95, method="higher"))),
        "p99_ms": ns_to_ms(int(np.quantile(values, 0.99, method="higher"))),
        "maximum_ms": ns_to_ms(int(values[-1])),
    }


def exact_length(samples: np.ndarray, length: int) -> np.ndarray:
    if samples.size >= length:
        return np.ascontiguousarray(samples[:length], dtype=np.float32)
    return np.pad(samples, (0, length - samples.size)).astype(np.float32)


def asset(path: Path) -> dict[str, object]:
    return {
        "path": path.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def affinity() -> list[int] | None:
    if hasattr(os, "sched_getaffinity"):
        return sorted(os.sched_getaffinity(0))
    return None


def elapsed_ns(started: int) -> int:
    return time.perf_counter_ns() - started


def ns_to_ms(value: int) -> float:
    return value / 1_000_000


if __name__ == "__main__":
    main()
