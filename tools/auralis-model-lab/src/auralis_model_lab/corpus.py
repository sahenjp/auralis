from __future__ import annotations

import base64
import hashlib
import io
import json
import os
import shutil
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.io import wavfile
from scipy.signal import resample_poly

HF_DATASET_API = "https://huggingface.co/api/datasets/google/fleurs/revision/main"
HF_ROWS_API = "https://datasets-server.huggingface.co/rows"
ZENODO_DEMAND_API = "https://zenodo.org/api/records/1227121"
SOURCE_SAMPLE_RATE_HZ = 16_000
SNR_LEVELS_DB = (10.0, 5.0, 0.0, -5.0, -10.0)
TRANSIENT_SAMPLE_RATE_HZ = 48_000


def fetch_fleurs(spec_path: Path, output_dir: Path) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    validate_spec(spec)
    required_revision = spec["dataset"]["revision"]
    verify_current_revision(required_revision)

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        cases = [
            fetch_fleurs_case(selection, temporary, required_revision, spec)
            for selection in spec["selections"]
        ]
        verify_current_revision(required_revision)
        manifest = {
            "schema_id": "auralis.corpus-manifest.fleurs.v1",
            "corpus_id": spec["corpus_id"],
            "source_spec": file_asset(spec_path),
            "dataset": spec["dataset"],
            "source_sample_rate_hz": SOURCE_SAMPLE_RATE_HZ,
            "target_sample_rate_hz": spec["target_sample_rate_hz"],
            "resampling": {
                "implementation": "scipy.signal.resample_poly",
                "up": 3,
                "down": 1,
                "scipy_version": __import__("scipy").__version__,
            },
            "gender_values": {"0": "male", "1": "female"},
            "cases": cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def fetch_demand(spec_path: Path, output_dir: Path) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    validate_demand_spec(spec)
    remote = fetch_json(ZENODO_DEMAND_API)
    validate_demand_record(spec, remote)
    remote_files = {item["key"]: item for item in remote["files"]}

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        cases = [
            fetch_demand_case(selection, spec, remote_files, temporary)
            for selection in spec["selections"]
        ]
        validate_demand_record(spec, fetch_json(ZENODO_DEMAND_API))
        manifest = {
            "schema_id": "auralis.corpus-manifest.demand.v1",
            "corpus_id": spec["corpus_id"],
            "source_spec": file_asset(spec_path),
            "dataset": spec["dataset"],
            "source_sample_rate_hz": SOURCE_SAMPLE_RATE_HZ,
            "target_sample_rate_hz": spec["target_sample_rate_hz"],
            "segment_duration_seconds": spec["segment_duration_seconds"],
            "channel_policy": "first channel file whose basename is ch01.wav",
            "resampling": {
                "implementation": "scipy.signal.resample_poly",
                "up": 3,
                "down": 1,
                "scipy_version": __import__("scipy").__version__,
            },
            "cases": cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def fetch_voicebank(spec_path: Path, output_dir: Path, cache_dir: Path) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    if spec.get("schema_id") != "auralis.corpus-source.voicebank-clean.v1":
        raise ValueError("unsupported VoiceBank source specification")
    cache_dir.mkdir(parents=True, exist_ok=True)
    archive_assets = {}
    archive_paths = {}
    for kind, archive in spec["archives"].items():
        path = cache_dir / archive["name"]
        if not path.exists():
            download_file(archive["download_url"], path)
        verify_cached_archive(path, archive)
        archive_paths[kind] = path
        archive_assets[kind] = file_asset(path) | {
            "published_md5": archive["md5"],
            "download_url": archive["download_url"],
            "bitstream_uuid": archive["bitstream_uuid"],
        }

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        with zipfile.ZipFile(archive_paths["audio"]) as audio_zip, zipfile.ZipFile(
            archive_paths["transcript"]
        ) as text_zip:
            cases = [
                extract_voicebank_case(selection, spec, temporary, audio_zip, text_zip)
                for selection in spec["selections"]
            ]
        manifest = {
            "schema_id": "auralis.corpus-manifest.voicebank-clean.v1",
            "corpus_id": spec["corpus_id"],
            "source_spec": file_asset(spec_path),
            "dataset": spec["dataset"],
            "archives": archive_assets,
            "sample_rate_hz": spec["sample_rate_hz"],
            "channels": 1,
            "cases": cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def extract_voicebank_case(
    selection: dict[str, object],
    spec: dict[str, object],
    output_dir: Path,
    audio_zip: zipfile.ZipFile,
    text_zip: zipfile.ZipFile,
) -> dict[str, object]:
    utterance_id = selection["utterance_id"]
    audio_member = f"clean_testset_wav/{utterance_id}.wav"
    text_member = f"testset_txt/{utterance_id}.txt"
    audio_bytes = audio_zip.read(audio_member)
    transcript_bytes = text_zip.read(text_member)
    samples, sample_rate = sf.read(io.BytesIO(audio_bytes), dtype="float32", always_2d=False)
    if sample_rate != spec["sample_rate_hz"] or samples.ndim != 1:
        raise ValueError(f"unexpected VoiceBank audio contract: {utterance_id}")
    transcript = transcript_bytes.decode("utf-8").strip()
    speaker = spec["speaker_metadata"][selection["speaker_id"]]
    case_dir = output_dir / selection["case_id"]
    case_dir.mkdir()
    audio_path = case_dir / "clean-48khz.wav"
    transcript_path = case_dir / "transcript.txt"
    audio_path.write_bytes(audio_bytes)
    transcript_path.write_bytes(transcript_bytes)
    return {
        "case_id": selection["case_id"],
        "utterance_id": utterance_id,
        "speaker_id": selection["speaker_id"],
        "gender": speaker["gender"],
        "tags": selection["tags"],
        "transcription": transcript,
        "archive_members": {"audio": audio_member, "transcript": text_member},
        "clean_48khz": relative_asset(audio_path, output_dir)
        | {
            "sample_rate_hz": sample_rate,
            "samples": int(samples.size),
            "peak_absolute": float(np.max(np.abs(samples))),
            "clipped_samples": int(np.count_nonzero(np.abs(samples) >= 0.999)),
        },
        "transcript": relative_asset(transcript_path, output_dir),
    }


def verify_cached_archive(path: Path, expected: dict[str, object]) -> None:
    if path.stat().st_size != expected["size_bytes"]:
        raise ValueError(f"archive size mismatch: {path}")
    digest = hashlib.md5()  # noqa: S324 -- upstream publishes MD5; SHA-256 is also recorded.
    with path.open("rb") as file:
        while chunk := file.read(1024 * 1024):
            digest.update(chunk)
    if digest.hexdigest() != expected["md5"]:
        raise ValueError(f"archive MD5 mismatch: {path}")


def mix_corpus(
    clean_manifest_paths: list[Path],
    noise_manifest_path: Path,
    output_dir: Path,
    corpus_id: str,
) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    if not corpus_id:
        raise ValueError("corpus_id must not be empty")
    clean_manifests = [
        (path, json.loads(path.read_text(encoding="utf-8")))
        for path in clean_manifest_paths
    ]
    noise_manifest = json.loads(noise_manifest_path.read_text(encoding="utf-8"))
    supported_clean_schemas = {
        "auralis.corpus-manifest.fleurs.v1",
        "auralis.corpus-manifest.voicebank-clean.v1",
    }
    if not clean_manifests or any(
        manifest.get("schema_id") not in supported_clean_schemas
        for _, manifest in clean_manifests
    ):
        raise ValueError("unsupported or missing clean corpus manifest")
    if noise_manifest.get("schema_id") != "auralis.corpus-manifest.demand.v1":
        raise ValueError("unsupported noise corpus manifest")

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        clean_sources = [
            source
            for path, manifest in clean_manifests
            for source in load_clean_sources(path, manifest)
        ]
        clean_ids = [source[0] for source in clean_sources]
        if len(clean_ids) != len(set(clean_ids)):
            raise ValueError("clean case IDs must be unique across source manifests")
        noise_sources = load_noise_sources(noise_manifest_path, noise_manifest, temporary)
        cases: list[dict[str, object]] = []
        for clean_id, clean, clean_metadata in clean_sources:
            clean_dir = temporary / "clean" / clean_id
            clean_dir.mkdir(parents=True)
            clean_path = clean_dir / "input.wav"
            wavfile.write(clean_path, 48_000, clean)
            cases.append(
                {
                    "case_id": f"{clean_id}--clean",
                    "condition": "clean",
                    "clean_source": clean_metadata,
                    "reference": relative_asset(clean_path, temporary),
                    "mixture": relative_asset(clean_path, temporary),
                    "noise_component": None,
                    "requested_snr_db": None,
                    "measured_snr_db": None,
                    "gains": {"clean": 1.0, "noise": 0.0, "peak": 1.0},
                }
            )
            for noise_id, noise, noise_metadata in noise_sources:
                for snr_db in SNR_LEVELS_DB:
                    cases.append(
                        write_mixture_case(
                            temporary,
                            clean_id,
                            clean,
                            clean_metadata,
                            noise_id,
                            noise,
                            noise_metadata,
                            snr_db,
                        )
                    )
        manifest = {
            "schema_id": "auralis.offline-bakeoff-corpus.v1",
            "corpus_id": corpus_id,
            "sample_rate_hz": 48_000,
            "channels": 1,
            "snr_generation": {
                "levels_db": list(SNR_LEVELS_DB),
                "rms_scope": "entire clean clip and same-length noise prefix",
                "noise_gain": "clean_rms / (noise_rms * 10^(snr_db / 20))",
                "peak_policy": "apply one common gain to clean and noise only when mixture peak exceeds 0.98",
            },
            "clean_source_manifests": [
                file_asset(path) for path, _ in clean_manifests
            ],
            "noise_source_manifest": file_asset(noise_manifest_path),
            "clean_source_count": len(clean_sources),
            "noise_source_count": len(noise_sources),
            "case_count": len(cases),
            "cases": cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def generate_transient_corpus(
    spec_path: Path,
    clean_manifest_paths: list[Path],
    noise_manifest_path: Path,
    output_dir: Path,
) -> None:
    if output_dir.exists():
        raise FileExistsError(f"refusing to overwrite {output_dir}")
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    if spec.get("schema_id") != "auralis.transient-corpus-spec.v1":
        raise ValueError("unsupported transient corpus specification")
    clean_manifests = [
        (path, json.loads(path.read_text(encoding="utf-8")))
        for path in clean_manifest_paths
    ]
    clean_by_id = {
        case_id: (samples, metadata)
        for path, manifest in clean_manifests
        for case_id, samples, metadata in load_clean_sources(path, manifest)
    }
    selected_ids = spec["clean_case_ids"]
    if len(selected_ids) != len(set(selected_ids)) or any(
        case_id not in clean_by_id for case_id in selected_ids
    ):
        raise ValueError("transient clean selections are missing or duplicated")
    noise_manifest = json.loads(noise_manifest_path.read_text(encoding="utf-8"))
    if noise_manifest.get("schema_id") != "auralis.corpus-manifest.demand.v1":
        raise ValueError("unsupported transient noise manifest")

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{output_dir.name}-", dir=output_dir.parent)
    )
    try:
        noise_by_id = {
            case_id: (samples, metadata)
            for case_id, samples, metadata in load_noise_sources(
                noise_manifest_path, noise_manifest, temporary
            )
        }
        steady_noise_id = spec["steady_noise_case_id"]
        if steady_noise_id not in noise_by_id:
            raise ValueError(f"missing steady noise: {steady_noise_id}")
        steady_noise, steady_metadata = noise_by_id[steady_noise_id]
        duration_samples = round(
            float(spec["duration_seconds"]) * TRANSIENT_SAMPLE_RATE_HZ
        )
        cases = []
        for clean_id in selected_ids:
            clean, clean_metadata = clean_by_id[clean_id]
            for pattern in spec["patterns"]:
                cases.append(
                    write_transient_case(
                        temporary,
                        clean_id,
                        clean,
                        clean_metadata,
                        steady_noise,
                        steady_metadata,
                        duration_samples,
                        pattern,
                    )
                )
        manifest = {
            "schema_id": "auralis.offline-bakeoff-corpus.v1",
            "corpus_id": spec["corpus_id"],
            "sample_rate_hz": TRANSIENT_SAMPLE_RATE_HZ,
            "channels": 1,
            "duration_seconds": spec["duration_seconds"],
            "source_spec": file_asset(spec_path),
            "clean_source_manifests": [file_asset(path) for path in clean_manifest_paths],
            "noise_source_manifest": file_asset(noise_manifest_path),
            "snr_generation": {
                "steady_noise_target_db": spec["steady_noise_target_db"],
                "scope": "RMS over active speech samples versus the same-length steady noise",
                "transient_level_policy": "fixed deterministic waveform followed by one common anti-clipping gain",
            },
            "case_count": len(cases),
            "cases": cases,
        }
        (temporary / "manifest.json").write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def write_transient_case(
    output_dir: Path,
    clean_id: str,
    clean: np.ndarray,
    clean_metadata: dict[str, object],
    steady_noise: np.ndarray,
    steady_metadata: dict[str, object],
    duration_samples: int,
    pattern: dict[str, object],
) -> dict[str, object]:
    reference = np.zeros(duration_samples, dtype=np.float32)
    noise = np.zeros(duration_samples, dtype=np.float32)
    start = round(float(pattern["speech_start_seconds"]) * TRANSIENT_SAMPLE_RATE_HZ)
    stop_seconds = pattern.get("speech_stop_seconds")
    stop = (
        duration_samples
        if stop_seconds is None
        else round(float(stop_seconds) * TRANSIENT_SAMPLE_RATE_HZ)
    )
    speech_length = min(clean.size, max(0, stop - start), duration_samples - start)
    if speech_length <= 0:
        raise ValueError(f"invalid speech interval: {pattern['pattern_id']}")
    reference[start : start + speech_length] = clean[:speech_length]

    events = [
        {
            "kind": "speech-start",
            "time_seconds": start / TRANSIENT_SAMPLE_RATE_HZ,
            "time_samples": start,
        },
        {
            "kind": "speech-stop",
            "time_seconds": (start + speech_length) / TRANSIENT_SAMPLE_RATE_HZ,
            "time_samples": start + speech_length,
        },
    ]
    transient_kind = pattern.get("transient_kind")
    if transient_kind is not None:
        event_times = [float(value) for value in pattern["event_times_seconds"]]
        for event_time in event_times:
            add_transient(noise, event_time, str(transient_kind), clean_id)
            events.append(
                {
                    "kind": transient_kind,
                    "time_seconds": event_time,
                    "time_samples": round(event_time * TRANSIENT_SAMPLE_RATE_HZ),
                }
            )

    steady_interval = pattern.get("steady_noise_interval_seconds")
    if steady_interval is not None:
        noise_start = round(float(steady_interval[0]) * TRANSIENT_SAMPLE_RATE_HZ)
        noise_stop = round(float(steady_interval[1]) * TRANSIENT_SAMPLE_RATE_HZ)
        noise_stop = min(noise_stop, duration_samples)
        length = max(0, noise_stop - noise_start)
        tiled = np.resize(steady_noise, length)
        active = reference != 0.0
        speech_rms = signal_rms(reference[active])
        target_db = float(pattern["steady_noise_target_db"])
        gain = speech_rms / (signal_rms(tiled) * 10.0 ** (target_db / 20.0))
        noise[noise_start:noise_stop] += np.asarray(tiled * gain, dtype=np.float32)
        events.extend(
            [
                {"kind": "steady-noise-start", "time_seconds": noise_start / TRANSIENT_SAMPLE_RATE_HZ, "time_samples": noise_start},
                {"kind": "steady-noise-stop", "time_seconds": noise_stop / TRANSIENT_SAMPLE_RATE_HZ, "time_samples": noise_stop},
            ]
        )

    peak = float(np.max(np.abs(reference + noise)))
    common_gain = min(1.0, 0.98 / peak) if peak > 0.0 else 1.0
    reference = np.asarray(reference * common_gain, dtype=np.float32)
    noise = np.asarray(noise * common_gain, dtype=np.float32)
    mixture = np.asarray(reference + noise, dtype=np.float32)
    case_id = f"{clean_id}--{pattern['pattern_id']}"
    case_dir = output_dir / "transients" / case_id
    case_dir.mkdir(parents=True)
    reference_path = case_dir / "clean-reference.wav"
    noise_path = case_dir / "noise-reference.wav"
    mixture_path = case_dir / "noisy-input.wav"
    wavfile.write(reference_path, TRANSIENT_SAMPLE_RATE_HZ, reference)
    wavfile.write(noise_path, TRANSIENT_SAMPLE_RATE_HZ, noise)
    wavfile.write(mixture_path, TRANSIENT_SAMPLE_RATE_HZ, mixture)
    return {
        "case_id": case_id,
        "condition": "transient",
        "clean_source": clean_metadata,
        "noise_source": {
            "steady": steady_metadata if steady_interval is not None else None,
            "transient_kind": transient_kind,
            "generator": "auralis-model-lab deterministic transient generator v1",
        },
        "pattern_id": pattern["pattern_id"],
        "test_focus": pattern["test_focus"],
        "events": sorted(events, key=lambda event: event["time_samples"]),
        "requested_snr_db": pattern.get("steady_noise_target_db"),
        "measured_snr_db": (
            20.0 * np.log10(signal_rms(reference) / signal_rms(noise))
            if signal_rms(noise) > 0.0
            else None
        ),
        "gains": {"common_peak": common_gain},
        "reference": relative_asset(reference_path, output_dir),
        "noise_component": relative_asset(noise_path, output_dir),
        "mixture": relative_asset(mixture_path, output_dir),
        "peak_absolute": float(np.max(np.abs(mixture))),
        "clipped_samples": int(np.count_nonzero(np.abs(mixture) > 1.0)),
    }


def add_transient(samples: np.ndarray, time_seconds: float, kind: str, salt: str) -> None:
    start = round(time_seconds * TRANSIENT_SAMPLE_RATE_HZ)
    if kind == "keyboard-click":
        length = round(0.006 * TRANSIENT_SAMPLE_RATE_HZ)
        time = np.arange(length, dtype=np.float64) / TRANSIENT_SAMPLE_RATE_HZ
        seed = int.from_bytes(hashlib.sha256(salt.encode()).digest()[:8], "little")
        rng = np.random.default_rng(seed + start)
        waveform = np.exp(-time * 900.0) * (
            0.55 * np.sin(2.0 * np.pi * 6_500.0 * time)
            + 0.35 * rng.standard_normal(length)
        )
    elif kind == "desk-knock":
        length = round(0.18 * TRANSIENT_SAMPLE_RATE_HZ)
        time = np.arange(length, dtype=np.float64) / TRANSIENT_SAMPLE_RATE_HZ
        waveform = np.exp(-time * 28.0) * (
            0.85 * np.sin(2.0 * np.pi * 115.0 * time)
            + 0.35 * np.sin(2.0 * np.pi * 245.0 * time)
        )
    elif kind == "door-slam":
        length = round(0.35 * TRANSIENT_SAMPLE_RATE_HZ)
        time = np.arange(length, dtype=np.float64) / TRANSIENT_SAMPLE_RATE_HZ
        seed = int.from_bytes(hashlib.sha256(salt.encode()).digest()[8:16], "little")
        rng = np.random.default_rng(seed + start)
        waveform = np.exp(-time * 13.0) * (
            0.75 * np.sin(2.0 * np.pi * 72.0 * time)
            + 0.20 * rng.standard_normal(length)
        )
    else:
        raise ValueError(f"unsupported transient kind: {kind}")
    stop = min(samples.size, start + length)
    samples[start:stop] += np.asarray(waveform[: stop - start], dtype=np.float32)


def load_clean_sources(
    manifest_path: Path, manifest: dict[str, object]
) -> list[tuple[str, np.ndarray, dict[str, object]]]:
    result = []
    for case in manifest["cases"]:
        asset = case["clean_48khz"]
        path = manifest_path.parent / asset["path"]
        samples = read_verified_audio(path, asset)
        result.append(
            (
                case["case_id"],
                samples,
                {
                    "corpus_id": manifest["corpus_id"],
                    "case_id": case["case_id"],
                    "tags": case["tags"],
                    "transcription": case["transcription"],
                    "asset": asset,
                },
            )
        )
    return result


def load_noise_sources(
    manifest_path: Path, manifest: dict[str, object], output_dir: Path
) -> list[tuple[str, np.ndarray, dict[str, object]]]:
    result = []
    by_id: dict[str, tuple[np.ndarray, dict[str, object]]] = {}
    for case in manifest["cases"]:
        asset = case["segment"]["noise_48khz"]
        samples = read_verified_audio(manifest_path.parent / asset["path"], asset)
        metadata = {
            "corpus_id": manifest["corpus_id"],
            "case_id": case["case_id"],
            "tags": case["tags"],
            "asset": asset,
            "kind": "single",
        }
        result.append((case["case_id"], samples, metadata))
        by_id[case["case_id"]] = (samples, metadata)

    pairs = (
        ("combined-office-traffic", "demand-office", "demand-street-traffic"),
        ("combined-cafeteria-metro", "demand-cafeteria", "demand-metro"),
        ("combined-living-washing", "demand-domestic-living", "demand-domestic-washing"),
    )
    combined_dir = output_dir / "noise-sources" / "combined"
    combined_dir.mkdir(parents=True)
    for pair_id, left_id, right_id in pairs:
        left, left_metadata = by_id[left_id]
        right, right_metadata = by_id[right_id]
        length = min(left.size, right.size)
        left_gain = 1.0 / signal_rms(left[:length])
        right_gain = 1.0 / signal_rms(right[:length])
        combined = left[:length] * left_gain + right[:length] * right_gain
        peak_gain = 0.9 / max(float(np.max(np.abs(combined))), 0.9)
        combined = np.asarray(combined * peak_gain, dtype=np.float32)
        path = combined_dir / f"{pair_id}.wav"
        wavfile.write(path, 48_000, combined)
        metadata = {
            "corpus_id": manifest["corpus_id"],
            "case_id": pair_id,
            "tags": ["real-noise", "combined", "difficult-mixture"],
            "kind": "equal-rms-combination",
            "components": [left_metadata, right_metadata],
            "component_gains": [left_gain * peak_gain, right_gain * peak_gain],
            "asset": relative_asset(path, output_dir),
        }
        result.append((pair_id, combined, metadata))
    return result


def write_mixture_case(
    output_dir: Path,
    clean_id: str,
    clean: np.ndarray,
    clean_metadata: dict[str, object],
    noise_id: str,
    noise: np.ndarray,
    noise_metadata: dict[str, object],
    snr_db: float,
) -> dict[str, object]:
    if noise.size < clean.size:
        raise ValueError(f"noise {noise_id} is shorter than clean {clean_id}")
    noise = noise[: clean.size]
    clean_gain = 1.0
    noise_gain = signal_rms(clean) / (signal_rms(noise) * 10.0 ** (snr_db / 20.0))
    unscaled = clean + noise * noise_gain
    peak = float(np.max(np.abs(unscaled)))
    peak_gain = min(1.0, 0.98 / peak) if peak > 0.0 else 1.0
    reference = np.asarray(clean * peak_gain, dtype=np.float32)
    noise_component = np.asarray(noise * noise_gain * peak_gain, dtype=np.float32)
    mixture = np.asarray(reference + noise_component, dtype=np.float32)
    measured_snr = 20.0 * np.log10(signal_rms(reference) / signal_rms(noise_component))
    snr_label = f"plus-{int(snr_db)}" if snr_db >= 0 else f"minus-{abs(int(snr_db))}"
    case_id = f"{clean_id}--{noise_id}--snr-{snr_label}db"
    case_dir = output_dir / "mixtures" / case_id
    case_dir.mkdir(parents=True)
    reference_path = case_dir / "clean-reference.wav"
    noise_path = case_dir / "noise-reference.wav"
    mixture_path = case_dir / "noisy-input.wav"
    wavfile.write(reference_path, 48_000, reference)
    wavfile.write(noise_path, 48_000, noise_component)
    wavfile.write(mixture_path, 48_000, mixture)
    return {
        "case_id": case_id,
        "condition": "noisy",
        "clean_source": clean_metadata,
        "noise_source": noise_metadata,
        "requested_snr_db": snr_db,
        "measured_snr_db": float(measured_snr),
        "gains": {"clean": clean_gain, "noise": noise_gain, "peak": peak_gain},
        "reference": relative_asset(reference_path, output_dir),
        "noise_component": relative_asset(noise_path, output_dir),
        "mixture": relative_asset(mixture_path, output_dir),
        "peak_absolute": float(np.max(np.abs(mixture))),
        "clipped_samples": int(np.count_nonzero(np.abs(mixture) > 1.0)),
    }


def read_verified_audio(path: Path, asset: dict[str, object]) -> np.ndarray:
    if hashlib.sha256(path.read_bytes()).hexdigest() != asset["sha256"]:
        raise ValueError(f"asset hash mismatch: {path}")
    samples, sample_rate = sf.read(path, dtype="float32", always_2d=False)
    if sample_rate != 48_000 or samples.ndim != 1 or samples.size != asset["samples"]:
        raise ValueError(f"asset audio contract mismatch: {path}")
    return np.ascontiguousarray(samples, dtype=np.float32)


def signal_rms(samples: np.ndarray) -> float:
    value = float(np.sqrt(np.mean(np.square(samples, dtype=np.float64))))
    if value <= 0.0:
        raise ValueError("cannot mix a zero-RMS signal")
    return value


def fetch_demand_case(
    selection: dict[str, object],
    spec: dict[str, object],
    remote_files: dict[str, dict[str, object]],
    output_dir: Path,
) -> dict[str, object]:
    remote = remote_files[selection["archive"]]
    archive_path = output_dir / f".{selection['case_id']}.zip"
    download_file(remote["links"]["self"], archive_path)
    if archive_path.stat().st_size != selection["size_bytes"]:
        raise ValueError(f"archive size mismatch for {selection['archive']}")
    archive_bytes = archive_path.read_bytes()
    if hashlib.md5(archive_bytes).hexdigest() != selection["md5"]:  # noqa: S324
        raise ValueError(f"archive MD5 mismatch for {selection['archive']}")
    archive_sha256 = hashlib.sha256(archive_bytes).hexdigest()
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        members = [
            item
            for item in archive.infolist()
            if Path(item.filename).name.lower() == "ch01.wav" and not item.is_dir()
        ]
        if len(members) != 1:
            raise ValueError(f"expected one ch01.wav in {selection['archive']}")
        member = members[0]
        source_bytes = archive.read(member)
    archive_path.unlink()

    source, sample_rate = sf.read(io.BytesIO(source_bytes), dtype="float32", always_2d=False)
    if sample_rate != SOURCE_SAMPLE_RATE_HZ or source.ndim != 1:
        raise ValueError(f"unexpected DEMAND audio format in {selection['archive']}")
    start = int(selection["start_seconds"]) * SOURCE_SAMPLE_RATE_HZ
    length = int(spec["segment_duration_seconds"]) * SOURCE_SAMPLE_RATE_HZ
    if start + length > source.size:
        raise ValueError(f"segment exceeds DEMAND source in {selection['archive']}")
    segment = np.ascontiguousarray(source[start : start + length], dtype=np.float32)
    derived = np.asarray(resample_poly(segment, 3, 1), dtype=np.float32)

    case_dir = output_dir / str(selection["case_id"])
    case_dir.mkdir()
    source_path = case_dir / "source-ch01.wav"
    source_path.write_bytes(source_bytes)
    segment_path = case_dir / "noise-segment-16khz.wav"
    derived_path = case_dir / "noise-48khz.wav"
    wavfile.write(segment_path, SOURCE_SAMPLE_RATE_HZ, segment)
    wavfile.write(derived_path, 48_000, derived)
    return {
        "case_id": selection["case_id"],
        "tags": selection["tags"],
        "archive": {
            "name": selection["archive"],
            "size_bytes": selection["size_bytes"],
            "md5": selection["md5"],
            "sha256": archive_sha256,
            "download_url": remote["links"]["self"],
        },
        "archive_member": {
            "path": member.filename,
            "crc32": f"{member.CRC:08x}",
            "source": relative_asset(source_path, output_dir)
            | {"sample_rate_hz": SOURCE_SAMPLE_RATE_HZ, "samples": int(source.size)},
        },
        "segment": {
            "start_seconds": selection["start_seconds"],
            "duration_seconds": spec["segment_duration_seconds"],
            "noise_16khz": relative_asset(segment_path, output_dir)
            | {"sample_rate_hz": SOURCE_SAMPLE_RATE_HZ, "samples": int(segment.size)},
            "noise_48khz": relative_asset(derived_path, output_dir)
            | {"sample_rate_hz": 48_000, "samples": int(derived.size)},
        },
    }


def fetch_fleurs_case(
    selection: dict[str, object],
    output_dir: Path,
    required_revision: str,
    spec: dict[str, object],
) -> dict[str, object]:
    query = urllib.parse.urlencode(
        {
            "dataset": "google/fleurs",
            "config": selection["config"],
            "split": selection["split"],
            "offset": selection["row_index"],
            "length": 1,
        }
    )
    payload = fetch_json(f"{HF_ROWS_API}?{query}")
    rows = payload.get("rows", [])
    if len(rows) != 1 or rows[0].get("row_idx") != selection["row_index"]:
        raise ValueError(f"unexpected row response for {selection['case_id']}")
    row = rows[0]["row"]
    if row["gender"] != selection["expected_gender"]:
        raise ValueError(f"gender changed for {selection['case_id']}")
    audio = row["audio"][0]
    mime_type = audio["type"]
    source_bytes = load_audio_source(audio["src"], required_revision)
    samples, sample_rate = sf.read(
        io.BytesIO(source_bytes), dtype="float32", always_2d=False
    )
    if sample_rate != SOURCE_SAMPLE_RATE_HZ or samples.ndim != 1:
        raise ValueError(f"unexpected source audio format for {selection['case_id']}")
    if int(samples.size) != row["num_samples"]:
        raise ValueError(f"sample count changed for {selection['case_id']}")

    case_dir = output_dir / str(selection["case_id"])
    case_dir.mkdir()
    source_path = case_dir / f"source{mime_extension(mime_type)}"
    source_path.write_bytes(source_bytes)
    derived = np.asarray(resample_poly(samples, 3, 1), dtype=np.float32)
    unscaled_peak = float(np.max(np.abs(derived)))
    peak_limit = float(spec.get("derivation", {}).get("anti_clipping_peak", 1.0))
    peak_gain = min(1.0, peak_limit / unscaled_peak) if unscaled_peak > 0.0 else 1.0
    derived = np.asarray(derived * peak_gain, dtype=np.float32)
    derived_path = case_dir / "clean-48khz.wav"
    wavfile.write(derived_path, 48_000, derived)
    return {
        "case_id": selection["case_id"],
        "config": selection["config"],
        "split": selection["split"],
        "row_index": selection["row_index"],
        "row_id": row["id"],
        "gender": row["gender"],
        "transcription": row["transcription"],
        "tags": selection["tags"],
        "source": relative_asset(source_path, output_dir)
        | {
            "mime_type": mime_type,
            "sample_rate_hz": SOURCE_SAMPLE_RATE_HZ,
            "samples": int(samples.size),
        },
        "clean_48khz": relative_asset(derived_path, output_dir)
        | {
            "sample_rate_hz": 48_000,
            "samples": int(derived.size),
            "resampled_peak_before_gain": unscaled_peak,
            "anti_clipping_peak_limit": peak_limit,
            "anti_clipping_gain": peak_gain,
            "peak_absolute": float(np.max(np.abs(derived))),
            "clipped_samples": int(np.count_nonzero(np.abs(derived) >= 0.999)),
        },
    }


def validate_spec(spec: dict[str, object]) -> None:
    if spec.get("schema_id") != "auralis.corpus-source.fleurs.v1":
        raise ValueError("unsupported FLEURS source specification")
    if spec.get("target_sample_rate_hz") != 48_000:
        raise ValueError("FLEURS target sample rate must be 48 kHz")
    selections = spec.get("selections")
    if not isinstance(selections, list) or not selections:
        raise ValueError("FLEURS source specification has no selections")
    case_ids = [selection["case_id"] for selection in selections]
    if len(case_ids) != len(set(case_ids)):
        raise ValueError("FLEURS case_id values must be unique")


def validate_demand_spec(spec: dict[str, object]) -> None:
    if spec.get("schema_id") != "auralis.corpus-source.demand.v1":
        raise ValueError("unsupported DEMAND source specification")
    if spec.get("target_sample_rate_hz") != 48_000:
        raise ValueError("DEMAND target sample rate must be 48 kHz")
    selections = spec.get("selections")
    if not isinstance(selections, list) or not selections:
        raise ValueError("DEMAND source specification has no selections")
    case_ids = [selection["case_id"] for selection in selections]
    archives = [selection["archive"] for selection in selections]
    if len(case_ids) != len(set(case_ids)) or len(archives) != len(set(archives)):
        raise ValueError("DEMAND case_id and archive values must be unique")


def validate_demand_record(spec: dict[str, object], remote: dict[str, object]) -> None:
    dataset = spec["dataset"]
    if remote.get("id") != dataset["zenodo_record_id"]:
        raise ValueError("DEMAND Zenodo record ID changed")
    if remote.get("doi") != dataset["doi"]:
        raise ValueError("DEMAND DOI changed")
    if remote["metadata"]["license"]["id"] != dataset["license"].lower():
        raise ValueError("DEMAND license metadata changed")
    remote_files = {item["key"]: item for item in remote["files"]}
    for selection in spec["selections"]:
        item = remote_files.get(selection["archive"])
        if item is None:
            raise ValueError(f"DEMAND archive disappeared: {selection['archive']}")
        if item["size"] != selection["size_bytes"]:
            raise ValueError(f"DEMAND archive size changed: {selection['archive']}")
        if item["checksum"] != f"md5:{selection['md5']}":
            raise ValueError(f"DEMAND archive checksum changed: {selection['archive']}")


def verify_current_revision(required_revision: str) -> None:
    actual = fetch_json(HF_DATASET_API).get("sha")
    if actual != required_revision:
        raise RuntimeError(
            f"google/fleurs revision changed: required {required_revision}, current {actual}"
        )


def fetch_json(url: str) -> dict[str, object]:
    request = urllib.request.Request(url, headers={"User-Agent": "Auralis-M3-corpus/1"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.load(response)
        except (urllib.error.URLError, TimeoutError):
            if attempt == 4:
                raise
            time.sleep(2**attempt)
    raise AssertionError("unreachable")


def load_audio_source(value: str, required_revision: str) -> bytes:
    if value.startswith("data:"):
        header, encoded = value.split(",", 1)
        if not header.endswith(";base64"):
            raise ValueError("audio data URL is not base64 encoded")
        return base64.b64decode(encoded, validate=True)
    parsed = urllib.parse.urlparse(value)
    if parsed.scheme != "https" or parsed.netloc != "datasets-server.huggingface.co":
        raise ValueError("audio asset is not hosted by the expected dataset server")
    if not parsed.path.startswith("/cached-assets/google/fleurs/"):
        raise ValueError("audio asset path is outside google/fleurs")
    if f"/{required_revision}/" not in parsed.path:
        raise ValueError("audio asset URL does not contain the pinned revision")
    return fetch_bytes(value)


def fetch_bytes(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "Auralis-M3-corpus/1"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read()
        except (urllib.error.URLError, TimeoutError):
            if attempt == 4:
                raise
            time.sleep(2**attempt)
    raise AssertionError("unreachable")


def download_file(url: str, output: Path) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "Auralis-M3-corpus/1"})
    try:
        with urllib.request.urlopen(request, timeout=120) as response, output.open("wb") as file:
            while chunk := response.read(1024 * 1024):
                file.write(chunk)
    except BaseException:
        output.unlink(missing_ok=True)
        raise


def mime_extension(mime_type: str) -> str:
    extensions = {"audio/wav": ".wav", "audio/x-wav": ".wav", "audio/flac": ".flac"}
    if mime_type not in extensions:
        raise ValueError(f"unsupported audio MIME type: {mime_type}")
    return extensions[mime_type]


def file_asset(path: Path) -> dict[str, object]:
    return {
        "path": path.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def relative_asset(path: Path, base: Path) -> dict[str, object]:
    return file_asset(path) | {"path": path.relative_to(base).as_posix()}
