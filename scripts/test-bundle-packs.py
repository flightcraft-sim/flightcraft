#!/usr/bin/env python3
"""No network: exercise shared-asset completeness and hostile URI rejection."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('bundle', Path(__file__).with_name('bundle-packs.py'))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)

class SharedTextures(unittest.TestCase):
    def test_complete_missing_and_unlisted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest = {'exterior': [{'model': 'plane.glb', 'textures': ['textures/paint.png']}]}
            (root/'textures').mkdir()
            (root/'textures/paint.png').write_bytes(b'PNG')
            def model(uri):
                encoded = json.dumps({'images': [{'uri': uri}]}).encode()
                (root/'plane.glb').write_bytes(struct.pack('<IIIII', 0x46546c67, 2, 20+len(encoded), len(encoded), 0x4e4f534a)+encoded)
            model('textures/paint.png')
            self.assertEqual(bundle.exterior_files(manifest, root), {'plane.glb', 'textures/paint.png'})
            for uri in ['textures/missing.png', '../paint.png', 'https://x/paint.png', '/paint.png', 'textures/%2e.png']:
                model(uri)
                with self.assertRaises(SystemExit):
                    bundle.exterior_files(manifest, root)
            model('textures/paint.png')
            (root/'textures/paint.png').unlink()
            with self.assertRaises(SystemExit):
                bundle.exterior_files(manifest, root)

if __name__ == '__main__':
    unittest.main()
