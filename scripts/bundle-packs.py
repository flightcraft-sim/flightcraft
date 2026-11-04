#!/usr/bin/env python3
"""Copy the aircraft packs pinned in bundled-packs.json into a desktop build's packs/ folder.

Each pack is fetched at its pinned commit. Only pack.json, the files its manifest lists, and the
licence, readme, and attribution files are copied, so a build carries no tests or tools. A pack whose
pack.json id differs from the pin, or whose manifest names a path outside the pack, fails the build.
"""
import json
import pathlib
import shutil
import struct
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
            paths += sorted(exterior_files(manifest, src))
            for rel in paths:
                path = (src / rel).resolve()
                if src.resolve() not in path.parents:
                    sys.exit(f"bundle-packs: {pin['id']} lists {rel!r} outside the pack")
                target = dest / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, target)
            missing = sorted(rel for rel in exterior_files(manifest, dest) if not (dest / rel).is_file())
            if missing:
                sys.exit(f"bundle-packs: {pin['id']} is missing exterior files {missing}")
            print(f"bundle-packs: {pin['id']} at {pin['commit'][:7]}: {len(paths)} files")


def exterior_files(manifest: dict, root: pathlib.Path | None = None) -> set:
    """All declared LODs and shared PNGs; verify GLB URIs use each entry's inventory."""
    models = {
        value
        for entry in manifest.get("exterior", [])
        for key, value in entry.items()
        if isinstance(value, str) and (key == "model" or (key.startswith("lod") and key[3:].isdigit()))
    }

    textures = {path for entry in manifest.get("exterior", []) for path in entry.get("textures", [])}
    paths = models | textures
    for path in paths:
        portable_path(path)
        if root is not None and not (root / path).is_file():
            sys.exit(f"bundle-packs: missing exterior file {path!r}")
    if root is not None:
        for entry in manifest.get("exterior", []):
            allowed = set(entry.get("textures", []))
            for model in models & {v for v in entry.values() if isinstance(v, str)}:
                path = root / model
                if not path.resolve().is_relative_to(root.resolve()):
                    sys.exit(f"bundle-packs: model escapes pack: {model!r}")
                with path.open('rb') as stream:
                    header = stream.read(20)
                    if len(header) != 20 or header[:4] != b'glTF':
                        sys.exit(f"bundle-packs: invalid GLB: {model!r}")
                    size, kind = struct.unpack_from('<II', header, 12)
                    if size > 64 * 1024 * 1024 or kind != 0x4e4f534a:
                        sys.exit(f"bundle-packs: invalid GLB JSON: {model!r}")
                    document = json.loads(stream.read(size))
                for image in document.get('images', []):
                    if 'uri' in image:
                        portable_path(image['uri'])
                        texture = (pathlib.PurePosixPath(model).parent / image['uri']).as_posix()
                        if texture not in allowed:
                            sys.exit(f"bundle-packs: unlisted texture {texture!r} in {model!r}")
    return paths


def portable_path(path: str) -> None:
    if (not isinstance(path, str) or not path or len(path) > 256 or
            any(not part or part.startswith('.') or not all(c.isascii() and (c.isalnum() or c in '-_.') for c in part)
                for part in path.split('/'))):
        sys.exit(f"bundle-packs: invalid exterior path {path!r}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: bundle-packs.py <bundled-packs.json> <out/packs>")
    main(sys.argv[1], sys.argv[2])
