#!/usr/bin/env python3
"""Audit registry license fields in Candle/tokenizers' resolved dependency closure.

Usage: python3 tools/check-speech-licenses.py /tmp/metadata.json [output.json]
Generate metadata with cargo metadata --locked --offline --format-version 1
--features flightcraft-ui/whisper (all target-specific dependency branches).
This supplements, and does not replace, CI's cargo-deny license-text scan.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib


def approved(expression, allowed):
    tokens = re.findall(r"\(|\)|[^\s()]+", expression.replace("/", " OR "))
    if not tokens:
        return False
    pos = 0

    def atom():
        nonlocal pos
        token = tokens[pos]
        pos += 1
        if token == "(":
            value = either()
            assert tokens[pos] == ")"
            pos += 1
            return value
        if pos < len(tokens) and tokens[pos] == "WITH":
            token += " WITH " + tokens[pos + 1]
            pos += 2
        return token in allowed

    def both():
        nonlocal pos
        value = atom()
        while pos < len(tokens) and tokens[pos] == "AND":
            pos += 1
            right = atom()
            value = value and right
        return value

    def either():
        nonlocal pos
        value = both()
        while pos < len(tokens) and tokens[pos] == "OR":
            pos += 1
            right = both()
            value = value or right
        return value

    result = either()
    assert pos == len(tokens), expression
    return result


root = Path(__file__).resolve().parent.parent
metadata = json.loads(Path(sys.argv[1]).read_text())
allowed = tomllib.loads((root / "deny.toml").read_text())["licenses"]["allow"]
packages = {p["id"]: p for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
pending = [p["id"] for p in packages.values() if p["name"] in
           {"candle-core", "candle-nn", "candle-transformers", "tokenizers"}]
closure = set()
while pending:
    key = pending.pop()
    if key in closure:
        continue
    closure.add(key)
    pending.extend(nodes[key]["dependencies"])
baseline = tomllib.loads(subprocess.check_output(
    ["git", "show", "HEAD:Cargo.lock"], cwd=root, text=True))
old = {(p["name"], p["version"]) for p in baseline["package"]}
rows = []
for key in sorted(closure, key=lambda key: (packages[key]["name"], packages[key]["version"])):
    package = packages[key]
    manifest = Path(package["manifest_path"])
    license_field = tomllib.loads(manifest.read_text())["package"].get("license", "")
    rows.append({"name": package["name"], "version": package["version"],
                 "license": license_field, "approved": approved(license_field, allowed),
                 "new": (package["name"], package["version"]) not in old,
                 "manifest": str(manifest)})
report = {"method": __doc__.strip(), "packages": len(rows),
          "new_packages": sum(row["new"] for row in rows),
          "unapproved": [row for row in rows if not row["approved"]],
          "dependencies": rows}
if len(sys.argv) > 2:
    Path(sys.argv[2]).write_text(json.dumps(report, indent=2) + "\n")
print(f'{len(rows)} packages; {report["new_packages"]} new; {len(report["unapproved"])} unapproved')
sys.exit(bool(report["unapproved"]))
