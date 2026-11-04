#!/usr/bin/env python3
"""No network: exercise skin bundling, path-escape checks, and size limits."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('bundle', Path(__file__).with_name('bundle-skins.py'))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


def make(root: Path, name="demo-skin", assets=None, extra=None):
    d = root / name
    (d / "fonts").mkdir(parents=True)
    (d / "fonts/a.ttf").write_bytes(b"font")
    (d / "fonts/OFL.txt").write_text("licence")
    (d / "README.md").write_text("readme")
    (d / "tests.txt").write_text("not bundled")
    manifest = {"kind": "skin_pack", "id": name,
                "assets": assets if assets is not None else [{"path": "fonts/a.ttf", "kind": "font", "licence": "OFL-1.1"}]}
    manifest.update(extra or {})
    (d / "skin.json").write_text(json.dumps(manifest))
    (root / "LICENSE").write_text("apache")
    return d


class Bundle(unittest.TestCase):
    def run_one(self, **kw):
        with tempfile.TemporaryDirectory() as tmp:
            src, out = Path(tmp) / "src", Path(tmp) / "out"
            src.mkdir()
            make(src, **kw)
            count = bundle.bundle_one(src, "demo-skin", out)
            return count, sorted(p.relative_to(out / "demo-skin").as_posix() for p in (out / "demo-skin").rglob("*") if p.is_file())

    def test_copies_only_listed_files_and_licences(self):
        count, files = self.run_one()
        self.assertEqual(files, ["LICENSE", "README.md", "fonts/OFL.txt", "fonts/a.ttf", "skin.json"])
        self.assertEqual(count, 5)

    def test_hostile_manifests_fail(self):
        for assets in (
            [{"path": "../x.ttf", "kind": "font"}],
            [{"path": "/etc/passwd", "kind": "font"}],
            [{"path": ".hidden", "kind": "font"}],
            [{"path": "fonts/missing.ttf", "kind": "font"}],
            [{"path": "fonts/a.ttf"}] * 17,
        ):
            with self.assertRaises(SystemExit, msg=str(assets)):
                self.run_one(assets=assets)

    def test_id_must_match_directory_and_names_must_be_portable(self):
        with self.assertRaises(SystemExit):
            self.run_one(extra={"id": "other"})
        with tempfile.TemporaryDirectory() as tmp:
            for name in ("../up", "a/b", ".hidden", "sp ace"):
                with self.assertRaises(SystemExit, msg=name):
                    bundle.bundle_one(Path(tmp), name, Path(tmp) / "out")

    def test_oversize_asset_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "src"
            src.mkdir()
            d = make(src)
            (d / "fonts/a.ttf").write_bytes(b"0" * (bundle.MAX_ASSET + 1))
            with self.assertRaises(SystemExit):
                bundle.bundle_one(src, "demo-skin", Path(tmp) / "out")

    def test_symlinked_asset_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "src"
            src.mkdir()
            d = make(src)
            (d / "fonts/a.ttf").unlink()
            (d / "fonts/a.ttf").symlink_to("/etc/hostname")
            with self.assertRaises(SystemExit):
                bundle.bundle_one(src, "demo-skin", Path(tmp) / "out")


if __name__ == '__main__':
    unittest.main()
