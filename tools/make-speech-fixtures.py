#!/usr/bin/env python3
"""Generate original calls with installed Linux platform TTS (eSpeak NG and Flite)."""
import hashlib
import json
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[1] / "tests/fixtures/speech"
root.mkdir(parents=True, exist_ok=True)
calls = [
    ("ready", "Allentown tower, Sling three seven one foxtrot charlie, with information alpha, ready for departure runway zero six, request closed traffic."),
    ("takeoff", "Runway zero six, cleared for takeoff, left closed traffic, Sling three seven one foxtrot charlie."),
    ("crosswind", "Allentown tower, Sling three seven one foxtrot charlie, left crosswind runway zero six."),
    ("downwind", "Allentown tower, Sling three seven one foxtrot charlie, left downwind runway zero six."),
    ("pattern", "Enter left downwind runway zero six, Sling three seven one foxtrot charlie."),
    ("base", "Allentown tower, Sling three seven one foxtrot charlie, left base runway zero six."),
    ("land", "Runway zero six, cleared to land, Sling three seven one foxtrot charlie."),
    ("final", "Allentown tower, Sling three seven one foxtrot charlie, final runway zero six."),
    ("around", "Going around runway zero six, Sling three seven one foxtrot charlie."),
    ("clear", "Allentown tower, Sling three seven one foxtrot charlie, clear of the runway."),
]
manifest = []
for name, text in calls:
    path = root / f"{name}.wav"
    subprocess.run(["/usr/bin/espeak-ng", "-v", "en-us", "-s", "155", "-w", str(path), text], check=True)
    manifest.append(dict(file=path.name, text=text, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
# A student retries rejected speech by saying the call clearly again. Use a
# second installed platform voice; original first-attempt clips remain intact.
retries = [
    ("ready", "Allentown tower. Sling three seven one foxtrot charlie. With information alpha. Ready for departure runway zero six. Request closed traffic."),
    ("pattern", "Sling three seven one foxtrot charlie. Enter left downwind runway zero six."),
    ("clear", "Allentown tower. Sling three seven one foxtrot charlie. Clear of runway zero six."),
]
for original, text in retries:
    path = root / f"{original}-retry.wav"
    subprocess.run(["/usr/bin/flite", "-voice", "rms", "-t", text, "-o", str(path)], check=True)
    manifest.append(dict(file=path.name, text=text, sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                         retry_of=f"{original}.wav", generator="Flite rms (installed platform TTS)"))
# A separate 4.83-second call for the five-second release-latency target.
path = root / "latency.wav"
text = "Runway zero six, cleared to land, Sling three seven one foxtrot charlie."
subprocess.run(["/usr/bin/espeak-ng", "-v", "en-us", "-s", "175", "-w", str(path), text], check=True)
manifest.append(dict(file=path.name,text=text,sha256=hashlib.sha256(path.read_bytes()).hexdigest(),benchmark_only=True))
(root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

# Adversarial acoustic evidence: these differ from the live-state prompt.
negative = []
for name, text in [
    ("wrong-runway", "Runway two four, cleared to land, Sling three seven one foxtrot charlie."),
    ("wrong-callsign", "Allentown tower, Sling three seven niner foxtrot charlie, left downwind runway zero six."),
]:
    path = root / f"{name}.wav"
    subprocess.run(["/usr/bin/espeak-ng", "-v", "en-us", "-s", "155", "-w", str(path), text], check=True)
    negative.append(dict(file=path.name, text=text, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
(root / "negative-manifest.json").write_text(json.dumps(negative, indent=2) + "\n")
