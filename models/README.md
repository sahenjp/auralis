# Models

Milestone 3 provisionally selects the pinned UL-UNAS streaming ONNX artifact
for local evaluation only. It is intentionally not bundled or cleared for
redistribution; the downloaded `*.onnx`/`*.bin` files remain ignored by Git.
The exact URL, revision, size, SHA-256, signal contract, and unresolved weight
terms are recorded in
[`bench/candidates/frozen-set-v1.json`](../bench/candidates/frozen-set-v1.json).

Any downloader or deployment packaging must verify an allowlisted URL, exact
size, and cryptographic hash before atomically activating a model. Do not ship
an artifact until its weight license independently permits the intended use.
