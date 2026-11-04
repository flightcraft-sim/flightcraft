#!/usr/bin/env python3
"""Copy the display skin packs pinned in bundled-skins.json into a build's skins/ folder.

One repository holds every skin, one per subdirectory. The lock pins the repository and a commit once and
lists the subdirectories to bundle. Only skin.json, the assets it declares, and the readme, attribution, and
licence files are copied, with the same path-escape checks as bundle-packs.py. A skin whose manifest id
differs from its directory or pin, a name or asset path that is not portable, or a file over the size limit
fails the build.
"""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

EXTRA = ("README.md", "ATTRIBUTION.md", "LICENSE")
MAX_MANIFEST = 64 * 1024
MAX_ASSET = 4 * 1024 * 1024
MAX_SKIN = 8 * 1024 * 1024
MAX_ASSETS = 16
MAX_SKINS = 32


def portable_path(path) -> bool:
    if not isinstance(path, str) or not path or len(path) > 128:
        return False
    return all(
        part and not part.startswith('.') and all(c.isascii() and (c.isalnum() or c in '-_.') for c in part)
        for part in path.split('/')
    )


def fail(message: str) -> None:
    sys.exit(f"bundle-skins: {message}")


def bundle_one(src: pathlib.Path, name: str, dest_root: pathlib.Path) -> int:
    """Copy one skin directory from a checkout. Returns the number of files copied."""
    if not portable_path(name) or '/' in name:
        fail(f"skin directory name {name!r} is not portable")
    root = src.resolve()
    skin_dir = (src / name).resolve()
    if skin_dir.parent != root or skin_dir.is_symlink() or not skin_dir.is_dir():
        fail(f"{name!r} is not a plain directory in the repository")
    manifest_path = skin_dir / "skin.json"
    if not manifest_path.is_file() or manifest_path.stat().st_size > MAX_MANIFEST:
        fail(f"{name} has no skin.json within 64 KiB")
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("kind") != "skin_pack" or manifest.get("id") != name:
        fail(f"{name} manifest id {manifest.get('id')!r} does not match its directory")
    assets = manifest.get("assets", [])
    if not isinstance(assets, list) or len(assets) > MAX_ASSETS:
        fail(f"{name} declares more than {MAX_ASSETS} assets")
    paths = ["skin.json"]
    total = manifest_path.stat().st_size
    for asset in assets:
        rel = asset.get("path") if isinstance(asset, dict) else None
        if not portable_path(rel):
            fail(f"{name} lists asset {rel!r} that is not a portable relative path")
        path = (skin_dir / rel).resolve()
        if skin_dir not in path.parents:
            fail(f"{name} lists {rel!r} outside the skin")
        if not path.is_file() or path.is_symlink():
            fail(f"{name} asset {rel!r} is not a regular file")
        size = path.stat().st_size
        if size > MAX_ASSET:
            fail(f"{name} asset {rel!r} exceeds 4 MiB")
        total += size
        paths.append(rel)
        # Licence text beside an asset travels with it.
        for sibling in path.parent.iterdir():
            if sibling.is_file() and not sibling.is_symlink() and sibling.name.upper().startswith(("OFL", "LICENSE", "COPYING")):
                paths.append(sibling.relative_to(skin_dir).as_posix())
    paths += [p for p in EXTRA if (skin_dir / p).is_file()]
    paths = sorted(set(paths))
    if total > MAX_SKIN:
        fail(f"{name} exceeds 8 MiB")
    dest = dest_root / name
    if dest.exists():
        shutil.rmtree(dest)
    for rel in paths:
        target = dest / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(skin_dir / rel, target)
    # The repository licence applies to every skin.
    repo_licence = src / "LICENSE"
    if not (dest / "LICENSE").exists() and repo_licence.is_file():
        shutil.copy2(repo_licence, dest / "LICENSE")
        paths.append("LICENSE")
    return len(paths)


def main(lock_path: str, out_dir: str) -> None:
    lock = json.loads(pathlib.Path(lock_path).read_text())
    skins = lock.get("skins", [])
    if not isinstance(skins, list) or len(skins) > MAX_SKINS or len(set(skins)) != len(skins):
        fail("skins must be a list of at most 32 distinct ids")
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    if not skins:
        return
    with tempfile.TemporaryDirectory() as tmp:
        src = pathlib.Path(tmp) / "skins"
        subprocess.run(["git", "init", "-q", str(src)], check=True)
        subprocess.run(["git", "-C", str(src), "fetch", "-q", "--depth", "1", lock["repository"], lock["commit"]], check=True)
        subprocess.run(["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"], check=True)
        for name in skins:
            count = bundle_one(src, name, out)
            print(f"bundle-skins: {name} at {lock['commit'][:7]}: {count} files")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: bundle-skins.py <bundled-skins.json> <out/skins>")
    main(sys.argv[1], sys.argv[2])
