#!/usr/bin/env python3
"""Copy the aircraft packs pinned in bundled-packs.json into a desktop build's packs/ folder.

Each pack is fetched at its pinned commit. Only pack.json, the files its manifest lists, and the
licence, readme, and attribution files are copied, so a build carries no tests or tools. A pack whose
pack.json id differs from the pin, or whose manifest names a path outside the pack, fails the build.
"""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

EXTRA = ("LICENSE", "README.md", "ATTRIBUTION.md", "data/sources.json")


def main(lock_path: str, out_dir: str) -> None:
    lock = json.loads(pathlib.Path(lock_path).read_text())
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for pin in lock["packs"]:
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp) / "pack"
            subprocess.run(["git", "init", "-q", str(src)], check=True)
            subprocess.run(["git", "-C", str(src), "fetch", "-q", "--depth", "1", pin["repository"], pin["commit"]], check=True)
            subprocess.run(["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"], check=True)
            manifest = json.loads((src / "pack.json").read_text())
            if manifest.get("id") != pin["id"]:
                sys.exit(f"bundle-packs: {pin['repository']} has id {manifest.get('id')!r}, pinned as {pin['id']!r}")
            dest = out / pin["id"]
            if dest.exists():
                shutil.rmtree(dest)
            paths = ["pack.json"] + [f["path"] for f in manifest.get("files", [])] + [p for p in EXTRA if (src / p).is_file()]
            for rel in paths:
                path = (src / rel).resolve()
                if src.resolve() not in path.parents:
                    sys.exit(f"bundle-packs: {pin['id']} lists {rel!r} outside the pack")
                target = dest / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, target)
            print(f"bundle-packs: {pin['id']} at {pin['commit'][:7]}: {len(paths)} files")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: bundle-packs.py <bundled-packs.json> <out/packs>")
    main(sys.argv[1], sys.argv[2])
