"""Pinned offline candidate adapters for Auralis Milestone 3."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path


def benchmark_provenance() -> dict[str, str]:
    """Return a stable source identifier for generated benchmark artifacts."""
    package_root = Path(__file__).resolve().parent
    project_root = package_root.parent.parent
    files = sorted(package_root.rglob("*.py"))
    files.extend(
        path
        for path in (project_root / "pyproject.toml", project_root / "uv.lock")
        if path.is_file()
    )
    digest = hashlib.sha256()
    for path in files:
        digest.update(path.relative_to(project_root).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    revision = os.environ.get("AURALIS_GIT_REVISION")
    if revision:
        return {
            "revision": revision,
            "revision_kind": "git_or_caller_supplied",
            "source_tree_sha256": digest.hexdigest(),
        }
    return {
        "revision": f"source-tree-sha256:{digest.hexdigest()}",
        "revision_kind": "source_tree_sha256",
        "source_tree_sha256": digest.hexdigest(),
    }
