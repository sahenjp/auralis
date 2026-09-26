from __future__ import annotations

import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


RATING_DIMENSIONS = (
    ("noise_removal", "Noise removal"),
    ("voice_naturalness", "Voice naturalness"),
    ("artifact_freedom", "Artifacts (5 = none)"),
    ("overall", "Overall"),
)


def create_abx_session(
    spec_path: Path,
    corpus_manifest_path: Path,
    output_manifest_paths: list[Path],
    session_dir: Path,
    private_key_path: Path,
    session_id: str,
    seed_hex: str,
) -> None:
    if session_dir.exists() or private_key_path.exists():
        raise FileExistsError("refusing to overwrite listening session or private key")
    if private_key_path.resolve().is_relative_to(session_dir.resolve()):
        raise ValueError("private key must be stored outside the listener-visible session directory")
    if not session_id or any(character.isspace() for character in session_id):
        raise ValueError("session ID must be non-empty and contain no whitespace")
    try:
        seed = int(seed_hex, 16)
    except ValueError as error:
        raise ValueError("seed must be hexadecimal") from error
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    corpus = json.loads(corpus_manifest_path.read_text(encoding="utf-8"))
    if spec.get("schema_id") != "auralis.abx-session-spec.v1":
        raise ValueError("unsupported ABX session specification")
    if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
        raise ValueError("unsupported ABX corpus manifest")
    corpus_cases = {case["case_id"]: case for case in corpus["cases"]}
    if any(case_id not in corpus_cases for case_id in spec["case_ids"]):
        raise ValueError("ABX specification references missing corpus case")
    outputs = []
    for path in output_manifest_paths:
        manifest = json.loads(path.read_text(encoding="utf-8"))
        if manifest.get("schema_id") != "auralis.offline-bakeoff-output.v1":
            raise ValueError(f"unsupported ABX output manifest: {path}")
        outputs.append(
            (
                manifest["candidate_id"],
                path,
                {case["case_id"]: case for case in manifest["cases"]},
            )
        )
    candidate_ids = [candidate_id for candidate_id, _, _ in outputs]
    if len(candidate_ids) < 2 or len(candidate_ids) != len(set(candidate_ids)):
        raise ValueError("ABX requires at least two unique candidates")
    if any(
        case_id not in cases
        for case_id in spec["case_ids"]
        for _, _, cases in outputs
    ):
        raise ValueError("candidate output is missing an ABX case")

    rng = random.Random(seed)
    session_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = session_dir.parent / f".{session_dir.name}.building"
    if temporary.exists():
        raise FileExistsError(f"stale listening session staging directory: {temporary}")
    audio_dir = temporary / "audio"
    audio_dir.mkdir(parents=True)
    output_by_candidate = {candidate_id: (path, cases) for candidate_id, path, cases in outputs}
    public_trials = []
    private_trials = []
    trial_number = 0
    for case_id in spec["case_ids"]:
        shuffled = candidate_ids.copy()
        rng.shuffle(shuffled)
        pair_count = int(spec["pairs_per_case"])
        for pair_index in range(pair_count):
            left = shuffled[(2 * pair_index) % len(shuffled)]
            right = shuffled[(2 * pair_index + 1) % len(shuffled)]
            if left == right:
                raise ValueError("ABX pair candidates must differ")
            if rng.randrange(2):
                left, right = right, left
            x_label = "A" if rng.randrange(2) == 0 else "B"
            trial_number += 1
            trial_id = f"trial-{trial_number:04d}"
            paths = {}
            assets = {}
            for label, candidate_id in (("A", left), ("B", right)):
                manifest_path, cases = output_by_candidate[candidate_id]
                source = manifest_path.parent / cases[case_id]["output"]["path"]
                verify_asset(source, cases[case_id]["output"])
                destination = audio_dir / f"{trial_id}-{label}.wav"
                shutil.copyfile(source, destination)
                paths[label] = destination
                assets[label] = relative_asset(destination, temporary)
            x_source = paths[x_label]
            x_destination = audio_dir / f"{trial_id}-X.wav"
            shutil.copyfile(x_source, x_destination)
            assets["X"] = relative_asset(x_destination, temporary)
            case = corpus_cases[case_id]
            public_trials.append(
                {
                    "trial_id": trial_id,
                    "condition": case["condition"],
                    "requested_snr_db": case.get("requested_snr_db"),
                    "audio": assets,
                    "instructions": "Identify whether X equals A or B, then rate A versus B without inspecting the private key.",
                }
            )
            private_trials.append(
                {
                    "trial_id": trial_id,
                    "case_id": case_id,
                    "pair_index": pair_index,
                    "A_candidate_id": left,
                    "B_candidate_id": right,
                    "X_label": x_label,
                    "X_candidate_id": left if x_label == "A" else right,
                }
            )
    rng.shuffle(public_trials)
    public_manifest = {
        "schema_id": "auralis.blind-abx-session.v1",
        "session_id": session_id,
        "mode": "ABX plus pairwise preference and category ratings",
        "candidate_count": len(candidate_ids),
        "trial_count": len(public_trials),
        "rating_scale": {"minimum": 1, "maximum": 5, "higher_is_better": True},
        "rating_dimensions": [
            "noise_suppression",
            "speech_naturalness",
            "intelligibility",
            "artifact_freedom",
            "consonant_preservation",
            "transient_behavior",
        ],
        "trials": public_trials,
    }
    public_path = temporary / "session.json"
    public_path.write_text(
        json.dumps(public_manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    private_manifest = {
        "schema_id": "auralis.blind-abx-private-key.v1",
        "session_id": session_id,
        "seed_hex": seed_hex.lower(),
        "session_manifest": file_asset(public_path),
        "source_spec": file_asset(spec_path),
        "corpus_manifest": file_asset(corpus_manifest_path),
        "candidate_outputs": [
            {"candidate_id": candidate_id, "manifest": file_asset(path)}
            for candidate_id, path, _ in outputs
        ],
        "trials": private_trials,
    }
    private_key_path.parent.mkdir(parents=True, exist_ok=True)
    private_key_path.write_text(
        json.dumps(private_manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.rename(session_dir)


def validate_abx_session(session_dir: Path, private_key_path: Path, report_path: Path) -> None:
    if report_path.exists():
        raise FileExistsError(f"refusing to overwrite {report_path}")
    public_path = session_dir / "session.json"
    public = json.loads(public_path.read_text(encoding="utf-8"))
    private = json.loads(private_key_path.read_text(encoding="utf-8"))
    if public.get("schema_id") != "auralis.blind-abx-session.v1":
        raise ValueError("unsupported public ABX schema")
    if private.get("schema_id") != "auralis.blind-abx-private-key.v1":
        raise ValueError("unsupported private ABX schema")
    if public["session_id"] != private["session_id"]:
        raise ValueError("public/private session ID mismatch")
    if file_asset(public_path)["sha256"] != private["session_manifest"]["sha256"]:
        raise ValueError("public session hash mismatch")
    public_trials = {trial["trial_id"]: trial for trial in public["trials"]}
    private_trials = {trial["trial_id"]: trial for trial in private["trials"]}
    if set(public_trials) != set(private_trials):
        raise ValueError("public/private trial set mismatch")
    candidate_ids = [item["candidate_id"] for item in private["candidate_outputs"]]
    public_text = public_path.read_text(encoding="utf-8")
    leaks = [candidate_id for candidate_id in candidate_ids if candidate_id in public_text]
    x_matches = {"A": 0, "B": 0}
    for trial_id, trial in public_trials.items():
        key = private_trials[trial_id]
        if key["A_candidate_id"] == key["B_candidate_id"]:
            raise ValueError(f"same-candidate pair: {trial_id}")
        assets = trial["audio"]
        for asset in assets.values():
            verify_asset(session_dir / asset["path"], asset)
        x_label = key["X_label"]
        if assets["X"]["sha256"] != assets[x_label]["sha256"]:
            raise ValueError(f"X hash mismatch: {trial_id}")
        x_matches[x_label] += 1
    if leaks:
        raise ValueError(f"candidate identity leaked into public manifest: {leaks}")
    report = {
        "schema_id": "auralis.blind-abx-validation.v1",
        "session_id": public["session_id"],
        "session_manifest": file_asset(public_path),
        "private_key": file_asset(private_key_path),
        "trial_count": len(public_trials),
        "x_assignment_counts": x_matches,
        "candidate_identity_leaks": leaks,
        "checks": [
            "all audio hashes match",
            "X is byte-identical to the keyed A or B",
            "A and B candidate identities differ",
            "public manifest contains no candidate IDs",
            "public and private trial sets match",
        ],
        "result": "passed",
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def record_response(
    session_dir: Path,
    results_path: Path,
    listener_id: str,
    trial_id: str,
    x_guess: str,
    preference: str,
    ratings: dict[str, int],
) -> None:
    public_path = session_dir / "session.json"
    public = json.loads(public_path.read_text(encoding="utf-8"))
    trial_ids = {trial["trial_id"] for trial in public["trials"]}
    if trial_id not in trial_ids:
        raise ValueError(f"unknown trial: {trial_id}")
    if not listener_id:
        raise ValueError("listener ID must not be empty")
    if set(ratings) != {"A", "B"} or any(
        value < 1 or value > 5
        for candidate_ratings in ratings.values()
        for value in candidate_ratings.values()
    ):
        raise ValueError("all ratings must be between 1 and 5")
    existing = []
    if results_path.exists():
        existing = [json.loads(line) for line in results_path.read_text(encoding="utf-8").splitlines()]
    if any(
        row["session_id"] == public["session_id"]
        and row["listener_id"] == listener_id
        and row["trial_id"] == trial_id
        for row in existing
    ):
        raise ValueError("response already exists; raw listening results are append-only")
    response = {
        "schema_id": "auralis.blind-abx-response.v1",
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        "session_id": public["session_id"],
        "session_manifest_sha256": file_asset(public_path)["sha256"],
        "listener_id": listener_id,
        "trial_id": trial_id,
        "x_guess": x_guess,
        "preference": preference,
        "ratings": ratings,
    }
    results_path.parent.mkdir(parents=True, exist_ok=True)
    with results_path.open("a", encoding="utf-8") as file:
        file.write(json.dumps(response, ensure_ascii=False, sort_keys=True) + "\n")


def create_rating_session(
    spec_path: Path,
    session_dir: Path,
    private_key_path: Path,
    session_id: str,
    seed_hex: str,
) -> None:
    if session_dir.exists() or private_key_path.exists():
        raise FileExistsError("refusing to overwrite rating session or private key")
    if private_key_path.resolve().is_relative_to(session_dir.resolve()):
        raise ValueError("private key must be outside the listener-visible session directory")
    if not session_id or any(character.isspace() for character in session_id):
        raise ValueError("session ID must be non-empty and contain no whitespace")
    try:
        seed = int(seed_hex, 16)
    except ValueError as error:
        raise ValueError("seed must be hexadecimal") from error
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    if spec.get("schema_id") != "auralis.blind-rating-session-spec.v1":
        raise ValueError("unsupported blind rating session specification")
    groups = spec.get("groups")
    if not isinstance(groups, list) or not groups:
        raise ValueError("rating session requires at least one corpus group")
    case_count = sum(len(group.get("cases", [])) for group in groups)
    if not 10 <= case_count <= 20:
        raise ValueError("quality gate must contain between 10 and 20 cases")

    loaded_groups = []
    candidate_ids = None
    seen_cases: set[tuple[Path, str]] = set()
    for group in groups:
        corpus_path = resolve_spec_path(spec_path, group["corpus_manifest"])
        corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
        if corpus.get("schema_id") != "auralis.offline-bakeoff-corpus.v1":
            raise ValueError(f"unsupported corpus manifest: {corpus_path}")
        corpus_cases = {case["case_id"]: case for case in corpus["cases"]}
        requested_cases = group.get("cases")
        if not isinstance(requested_cases, list) or not requested_cases:
            raise ValueError("each rating group requires cases")
        for requested in requested_cases:
            case_id = requested.get("case_id")
            if case_id not in corpus_cases:
                raise ValueError(f"rating specification references missing case: {case_id}")
            key = (corpus_path, case_id)
            if key in seen_cases:
                raise ValueError(f"duplicate rating case: {case_id}")
            seen_cases.add(key)

        output_manifests = []
        for value in group.get("output_manifests", []):
            path = resolve_spec_path(spec_path, value)
            manifest = json.loads(path.read_text(encoding="utf-8"))
            if manifest.get("schema_id") != "auralis.offline-bakeoff-output.v1":
                raise ValueError(f"unsupported output manifest: {path}")
            cases = {case["case_id"]: case for case in manifest["cases"]}
            missing = [
                requested["case_id"]
                for requested in requested_cases
                if requested["case_id"] not in cases
            ]
            if missing:
                raise ValueError(f"candidate output is missing cases: {missing}")
            output_manifests.append((manifest["candidate_id"], path, cases))
        group_candidate_ids = [candidate_id for candidate_id, _, _ in output_manifests]
        if len(group_candidate_ids) < 1 or len(group_candidate_ids) != len(
            set(group_candidate_ids)
        ):
            raise ValueError("each group requires unique candidate output manifests")
        if candidate_ids is None:
            candidate_ids = group_candidate_ids
        elif group_candidate_ids != candidate_ids:
            raise ValueError("candidate order must be identical for every corpus group")
        loaded_groups.append(
            (corpus_path, corpus_cases, requested_cases, output_manifests)
        )

    assert candidate_ids is not None
    all_candidate_ids = ["raw-input", *candidate_ids]
    if len(all_candidate_ids) > 26:
        raise ValueError("rating session supports at most 26 candidates")
    rng = random.Random(seed)
    session_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary = session_dir.parent / f".{session_dir.name}.building"
    if temporary.exists():
        raise FileExistsError(f"stale rating session staging directory: {temporary}")
    audio_dir = temporary / "audio"
    audio_dir.mkdir(parents=True)
    public_trials = []
    private_trials = []
    source_manifests = []
    try:
        trial_number = 0
        for corpus_path, corpus_cases, requested_cases, outputs in loaded_groups:
            source_manifests.append(
                {
                    "corpus_manifest": file_asset(corpus_path),
                    "candidate_outputs": [
                        {"candidate_id": candidate_id, "manifest": file_asset(path)}
                        for candidate_id, path, _ in outputs
                    ],
                }
            )
            for requested in requested_cases:
                trial_number += 1
                trial_id = f"sample-{trial_number:03d}"
                case_id = requested["case_id"]
                case = corpus_cases[case_id]
                sources = [("raw-input", corpus_path.parent / case["mixture"]["path"], case["mixture"])]
                for candidate_id, manifest_path, output_cases in outputs:
                    output_asset = output_cases[case_id]["output"]
                    sources.append(
                        (candidate_id, manifest_path.parent / output_asset["path"], output_asset)
                    )
                rng.shuffle(sources)
                public_audio = {}
                private_assignments = {}
                for index, (candidate_id, source, expected) in enumerate(sources):
                    verify_asset(source, expected)
                    label = chr(ord("A") + index)
                    destination = audio_dir / f"{trial_id}-{label}.wav"
                    shutil.copyfile(source, destination)
                    public_audio[label] = relative_asset(destination, temporary)
                    private_assignments[label] = {
                        "candidate_id": candidate_id,
                        "source": file_asset(source),
                    }
                public_trials.append(
                    {
                        "trial_id": trial_id,
                        "scenario": requested["scenario"],
                        "condition": case["condition"],
                        "requested_snr_db": case.get("requested_snr_db"),
                        "audio": public_audio,
                    }
                )
                private_trials.append(
                    {
                        "trial_id": trial_id,
                        "case_id": case_id,
                        "assignments": private_assignments,
                    }
                )
        rng.shuffle(public_trials)
        public_manifest = {
            "schema_id": "auralis.blind-rating-session.v1",
            "session_id": session_id,
            "mode": "anonymous multi-candidate quality rating",
            "candidate_count": len(all_candidate_ids),
            "trial_count": len(public_trials),
            "rating_scale": {
                "minimum": 1,
                "maximum": 5,
                "higher_is_better": True,
            },
            "rating_dimensions": [name for name, _ in RATING_DIMENSIONS],
            "signal_policy": "Copied without loudness normalization, delay compensation, or failure masking.",
            "trials": public_trials,
        }
        public_path = temporary / "session.json"
        public_path.write_text(
            json.dumps(public_manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        private_manifest = {
            "schema_id": "auralis.blind-rating-private-key.v1",
            "session_id": session_id,
            "seed_hex": seed_hex.lower(),
            "session_manifest": file_asset(public_path),
            "source_spec": file_asset(spec_path),
            "source_manifests": source_manifests,
            "trials": private_trials,
        }
        private_key_path.parent.mkdir(parents=True, exist_ok=True)
        private_key_path.write_text(
            json.dumps(private_manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        temporary.rename(session_dir)
    except Exception:
        if temporary.exists():
            shutil.rmtree(temporary)
        if private_key_path.exists() and not session_dir.exists():
            private_key_path.unlink()
        raise


def validate_rating_session(
    session_dir: Path, private_key_path: Path, report_path: Path
) -> None:
    if report_path.exists():
        raise FileExistsError(f"refusing to overwrite {report_path}")
    public_path = session_dir / "session.json"
    public = json.loads(public_path.read_text(encoding="utf-8"))
    private = json.loads(private_key_path.read_text(encoding="utf-8"))
    if public.get("schema_id") != "auralis.blind-rating-session.v1":
        raise ValueError("unsupported public rating schema")
    if private.get("schema_id") != "auralis.blind-rating-private-key.v1":
        raise ValueError("unsupported private rating schema")
    if public["session_id"] != private["session_id"]:
        raise ValueError("public/private rating session ID mismatch")
    if file_asset(public_path)["sha256"] != private["session_manifest"]["sha256"]:
        raise ValueError("public rating session hash mismatch")
    public_trials = {trial["trial_id"]: trial for trial in public["trials"]}
    private_trials = {trial["trial_id"]: trial for trial in private["trials"]}
    if set(public_trials) != set(private_trials):
        raise ValueError("public/private rating trial set mismatch")
    candidate_ids = {
        assignment["candidate_id"]
        for trial in private_trials.values()
        for assignment in trial["assignments"].values()
    }
    public_text = public_path.read_text(encoding="utf-8")
    leaks = sorted(candidate_id for candidate_id in candidate_ids if candidate_id in public_text)
    for trial_id, trial in public_trials.items():
        assignments = private_trials[trial_id]["assignments"]
        if set(trial["audio"]) != set(assignments):
            raise ValueError(f"public/private rating labels differ: {trial_id}")
        if len(assignments) != len(candidate_ids):
            raise ValueError(f"candidate count differs: {trial_id}")
        for asset in trial["audio"].values():
            verify_asset(session_dir / asset["path"], asset)
    if leaks:
        raise ValueError(f"candidate identity leaked into public rating session: {leaks}")
    report = {
        "schema_id": "auralis.blind-rating-validation.v1",
        "session_id": public["session_id"],
        "session_manifest": file_asset(public_path),
        "private_key": file_asset(private_key_path),
        "trial_count": len(public_trials),
        "candidate_count": len(candidate_ids),
        "audio_asset_count": sum(len(trial["audio"]) for trial in public_trials.values()),
        "candidate_identity_leaks": leaks,
        "result": "passed",
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def run_rating_session(
    session_dir: Path,
    results_path: Path,
    listener_id: str,
    player_command: str,
    verify_only: bool,
) -> None:
    if not listener_id:
        raise ValueError("listener ID must not be empty")
    public_path = session_dir / "session.json"
    public = json.loads(public_path.read_text(encoding="utf-8"))
    if public.get("schema_id") != "auralis.blind-rating-session.v1":
        raise ValueError("unsupported public rating schema")
    for trial in public["trials"]:
        for asset in trial["audio"].values():
            verify_asset(session_dir / asset["path"], asset)
    player_path = shutil.which(player_command)
    if player_path is None:
        raise FileNotFoundError(f"audio player not found: {player_command}")
    if verify_only:
        print(
            f"validated {public['trial_count']} trials, "
            f"{public['candidate_count']} anonymous candidates, player={player_path}"
        )
        return
    if not sys.stdin.isatty():
        raise RuntimeError("interactive rating requires a terminal")
    completed = load_completed_ratings(results_path, public["session_id"], listener_id)
    session_hash = file_asset(public_path)["sha256"]
    player = FfplayPlayer(player_path)
    try:
        for trial_index, trial in enumerate(public["trials"], start=1):
            labels = sorted(trial["audio"])
            remaining = [
                label for label in labels if (trial["trial_id"], label) not in completed
            ]
            if not remaining:
                continue
            print(
                f"\n[{trial_index}/{public['trial_count']}] {trial['scenario']}\n"
                f"  {', '.join(labels)}: play   Enter: rate   q: save and quit",
                flush=True,
            )
            while True:
                key = read_key()
                label = key.upper()
                if label in labels:
                    player.play(session_dir / trial["audio"][label]["path"])
                elif key in ("\r", "\n"):
                    player.stop()
                    break
                elif key.lower() == "q":
                    player.stop()
                    print(f"\nSaved: {results_path}")
                    return
            for label in remaining:
                ratings = {}
                print(f"\n  Rate {label} (press any {', '.join(labels)} key to compare)")
                for dimension, prompt in RATING_DIMENSIONS:
                    while True:
                        print(f"    {prompt} [1-5]: ", end="", flush=True)
                        key = read_key()
                        replay_label = key.upper()
                        if replay_label in labels:
                            print(replay_label)
                            player.play(
                                session_dir / trial["audio"][replay_label]["path"]
                            )
                            continue
                        if key in "12345":
                            print(key)
                            ratings[dimension] = int(key)
                            break
                        if key.lower() == "q":
                            player.stop()
                            print(f"\nSaved: {results_path}")
                            return
                player.stop()
                response = {
                    "schema_id": "auralis.blind-rating-response.v1",
                    "recorded_at": datetime.now(timezone.utc).isoformat(),
                    "session_id": public["session_id"],
                    "session_manifest_sha256": session_hash,
                    "listener_id": listener_id,
                    "trial_id": trial["trial_id"],
                    "anonymous_label": label,
                    "ratings": ratings,
                }
                append_json_line(results_path, response)
                completed.add((trial["trial_id"], label))
        print(f"\nCompleted. Raw append-only results: {results_path}")
    finally:
        player.stop()


class FfplayPlayer:
    def __init__(self, executable: str) -> None:
        self.executable = executable
        self.process: subprocess.Popen[bytes] | None = None

    def play(self, path: Path) -> None:
        self.stop()
        self.process = subprocess.Popen(
            [
                self.executable,
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostats",
                "-nodisp",
                "-autoexit",
                path.as_posix(),
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def stop(self) -> None:
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=0.5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process = None


def read_key() -> str:
    if os.name == "nt":
        import msvcrt

        return msvcrt.getwch()
    import termios
    import tty

    descriptor = sys.stdin.fileno()
    previous = termios.tcgetattr(descriptor)
    try:
        tty.setcbreak(descriptor)
        return sys.stdin.read(1)
    finally:
        termios.tcsetattr(descriptor, termios.TCSADRAIN, previous)


def load_completed_ratings(
    results_path: Path, session_id: str, listener_id: str
) -> set[tuple[str, str]]:
    completed = set()
    if not results_path.exists():
        return completed
    for line_number, line in enumerate(
        results_path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid results JSONL line {line_number}") from error
        if row.get("session_id") == session_id and row.get("listener_id") == listener_id:
            key = (row["trial_id"], row["anonymous_label"])
            if key in completed:
                raise ValueError(f"duplicate append-only rating result: {key}")
            completed.add(key)
    return completed


def append_json_line(path: Path, value: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as file:
        file.write(json.dumps(value, ensure_ascii=False, sort_keys=True) + "\n")
        file.flush()
        os.fsync(file.fileno())


def resolve_spec_path(spec_path: Path, value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        path = spec_path.parent / path
    return path.resolve()


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
    return {"path": path.as_posix(), "size_bytes": path.stat().st_size, "sha256": digest.hexdigest()}
