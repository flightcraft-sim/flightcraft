#!/usr/bin/env python3
"""Maintainer-only: resolve upstream revisions and hash every artifact before review.

Run on a connected machine, then review the diff. The app never follows main.
"""
import hashlib
import json
from pathlib import Path
import urllib.request

MODELS = ("tiny.en", "base.en")
FILES = ("config.json", "tokenizer.json", "model.safetensors")
manifest = {"version": 1, "license": "MIT", "source": "https://huggingface.co/openai/whisper-tiny.en", "models": []}
for name in MODELS:
    with urllib.request.urlopen(f"https://huggingface.co/api/models/openai/whisper-{name}", timeout=30) as response:
        revision = json.load(response)["sha"]
    if len(revision) != 40 or any(c not in "0123456789abcdef" for c in revision):
        raise ValueError("invalid immutable upstream revision")
    files = []
    for filename in FILES:
        url = f"https://huggingface.co/openai/whisper-{name}/resolve/{revision}/{filename}"
        digest = hashlib.sha256()
        size = 0
        with urllib.request.urlopen(url, timeout=60) as response:
            while chunk := response.read(1024 * 1024):
                digest.update(chunk)
                size += len(chunk)
        files.append(dict(name=filename, url=url, sha256=digest.hexdigest(), bytes=size))
    manifest["models"].append(dict(id=name, files=files))
Path(__file__).resolve().parents[1].joinpath("assets/speech/models.json").write_text(json.dumps(manifest, indent=2) + "\n")
