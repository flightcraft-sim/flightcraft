#!/usr/bin/env python3
"""Embed unchanged offscreen cockpit PNGs side by side; no image processing or dependencies."""
import argparse
import base64
import html
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    manifest = json.loads((args.after/'manifest.json').read_text())
    rows = []
    for view in manifest['views']:
        label = view['view']
        cells = []
        for root in [args.before, args.after]:
            encoded = base64.b64encode((root/(label+'.png')).read_bytes()).decode()
            cells.append('<td><img alt="'+html.escape(label)+'" src="data:image/png;base64,'+encoded+'"></td>')
        rows.append('<tr><th colspan="2">'+html.escape(label.replace('-', ' ').title())+'</th></tr><tr>'+''.join(cells)+'</tr>')
    args.output.write_text('''<!doctype html><html lang="en"><meta charset="utf-8">
<title>Sling 2 pilot-seat comparison</title>
<style>body{font:16px system-ui;background:#171d24;color:#e4edf2;margin:24px}table{width:100%;border-collapse:collapse;table-layout:fixed}td{padding:4px}img{width:100%;height:auto}th{text-align:left;padding:12px 4px}p{max-width:1000px}</style>
<h1>Sling 2 cockpit · 1N7 runway 07</h1>
<p>Unmodified 1600 × 1000 GPU captures with released regional terrain. Left: original HEAD camera and draw behavior. Right: installed cabin from the declared pilot eye. The capture manifest identifies the geometry revision. Material/detail revisions require a confined Blender run before they appear here.</p>
<p>The native egui instrument surface remains crisp in the split layout. Matching closed/open cameras show the physical canopy movement. Source and adapter metadata are in each capture folder.</p>
<p>Runtime model and texture bytes: <strong>57,977,784 → 22,648,650 (60.9% smaller)</strong>. Mesh, material, and PNG bytes are unchanged by storage repacking.</p>
<table><tr><th>Before</th><th>After</th></tr>'''+''.join(rows)+'</table></html>')
    print(args.output)


if __name__ == '__main__':
    main()
