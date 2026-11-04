# Pilot-seat interiors and shared exterior maps

An exterior can declare `interior: {"pilot_eye_m": [-2.30, -0.205, -0.72]}`.
Coordinates are meters in the same pack body frame as the meshes: forward, right, down.
They are converted using the current loaded CG, like the meshes, so changing payload cannot
move the eye through the dash. Values must be finite and within ten meters of the body origin.
The camera uses this metadata for every aircraft identity, independent of canopy type or node names.
Cockpit mode draws the complete authored LOD0 cabin and exterior, including posed sliding glass,
frames, and handles. The 15 mm near plane preserves nearby trim during head movement.
Packs without the metadata retain their procedural cockpit fallback.

The existing split layout remains: 3D cabin/outside above, the pack's actual `panel.json`
instruments below. Mapping the whole panel to a perspective quad would reduce small-text readability
and require a new inverse-projection interaction path. Keeping egui's native panel surface preserves
its scale, tested fidelity, and exact click targets. The 3D dash and glareshield are context; the lower
panel is the active instrument surface. Look controls turn the camera through the cabin without
resampling the instruments. No panel definition or instrument drawing code changed.

## Shared PNG contract

`exterior.textures` explicitly lists at most eight pack-root PNG paths. Each GLB's image `uri`
is a portable relative path resolved against that GLB's directory, and must resolve to a listed
texture. Embedded PNGs remain supported. No external buffers, absolute paths, traversal, URL/data
URIs, percent escapes, or network requests are supported. Transports fetch/read the manifest inventory
only, with existing regular-file/symlink checks, 64 MiB per file, 128 MiB per pack, 4096² per image,
and 320 MiB decoded pixels per model. PNG headers and binary bounds are checked before decoding.
A per-install cache shares decoded immutable pixels among LODs. Browser fetching uses the same
`asset_paths()` inventory as native reads. Bundling checks actual GLB image references against each
entry's inventory and checks every model and PNG before and after copying.

LODs share the exact same PNG bytes, UVs, and material channels. The normal/color-correct runtime
filter starts LOD1 uploads at 1024² and LOD2 at 512²; LOD0 retains the device-supported resolution
(up to 4096²). There are no extra lower-resolution downloads. GPU mip generation still operates in
linear light for color and renormalizes tangent normals. No new dependencies were added. The updated pack is version 0.4.0; the flight lead must land both
worktrees together and update the bundled-pack pin after committing the pack.

The installed Sling V10 maps were **4096²**, rather than the 2048² estimated in the task.
Lossless repacking reduced GLBs from 21,367,124 / 18,581,908 / 18,028,752 bytes to
3,703,328 / 917,672 / 364,392 bytes. Their one shared set is 17,663,258 bytes.
Combined model/texture download: **57,977,784 → 22,648,650 bytes (60.9% smaller)**.
Redundant standalone review bakes are removed from the installed pack; Blender output retains
those review images. Texture hashes and exact sizes are in `out/cockpit/file-sizes.json` and the
pack's `docs/exterior-storage.json`. Repacking does not change mesh accessor bytes or image bytes.

## Repeat the proof

```sh
export CARGO_TARGET_DIR="$HOME/builds/cargo-target/flightcraft/fc-cockpit"
export PKG_CONFIG_PATH="$HOME/builds/pkgconfig-shim"
export CCACHE_DISABLE=1
export FLIGHTCRAFT_SLING_PACK="$HOME/builds/worktrees/sling-exterior"
export XDG_RUNTIME_DIR=/tmp
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
cargo +1.98.0 test --offline -j 4 -p flightcraft-ui render_sling_cockpit_proof -- --ignored --nocapture
```

The proof uses the actual GLBs, native pack loader, camera head-slew path, PBR renderer, and egui
panel, at FAA 1N7 runway 07, with released regional terrain. It does not claim imagery, photogrammetry,
a real window, or hardware-GPU performance. It captures forward,
left, right, down, rear-left, and matching closed/open canopy cameras. It checks that head movement
changes the scene, opening changes the same camera, and instrument pixels remain unchanged during
head movement.
The after manifest includes exact look angles, aircraft state, canopy fraction, geometry revision,
and adapter. The stopped propeller is visible in forward views. The baseline reproduces the original
regression: its matched closed/open images are identical, so the canopy visibility assertion fails.

The baseline is captured with the original HEAD `crates/render/src/lib.rs` camera/draw decisions,
using the same decoded meshes and maps. The storage-only change is lossless. Restore the edited
renderer after capturing the baseline; do not leave the old file installed. Output is under
`out/cockpit/before/` and `out/cockpit/after/`, with side-by-side `out/cockpit/comparison.html`.

## Confined Blender follow-up

The installed geometry remains accepted **V10**. Its existing cabin is now viewed from the seat.
Queue `~/builds/codex-jobs/blender-queue/fc-cockpit-sling2-interior-v11.py` is byte-identical to the
pack's current `scripts/blender-exterior.py`. It adds matte panel coating, procedural woven fabric,
anodized controls, side panels, armrests, pedals, a throttle grip, and a curved dash and coaming.
The dash follows a domed shoulder profile and an aft-bowed lip; the lining slopes inward beneath the rails.
These are original modeling estimates. No Blender execution or acceptance is claimed by this lane.
The installer validates the Blender receipt, source hash, all LOD budgets, containment, canopy
hardware attachment, and continuous swept clearance before replacing assets, then externalizes
the maps and updates both manifests. Repeat the engine proof after installation.

Flight lead's real-window shots: at 1N7 runway 07, paused with the engine stopped, capture forward,
left (−71.6° yaw), right (+71.6°), panel down (−37.2° pitch), rear-left (−120.3° yaw), then the
same left/up camera (−37.2° yaw, +18.3° pitch) with canopy closed and fully open. Keep the panel
visible at the same window size. Also click a panel control and drag the physical canopy bow to
confirm interaction in the packaged desktop build; repeat forward/down/open in the browser.

The queue also contains `fc-cockpit-sling2-interior-v11-engine-proof.sh OUTPUT`, a foreground
install/proof runner for flight lead after reviewing the Blender output. It refreshes the installed
storage/hash receipt, compiled pack asset fixture, pilot-seat captures, all-LOD canopy check, and
comparison page. It does not stage or commit.

## Validation in this lane

Rust 1.98.0, four build jobs, one cargo command at a time, and the fc-cockpit target directory.
Passed: 65 aero/render unit tests with the actual repacked Sling, all 10 pack fidelity tests,
20 Blender authoring tests, the storage repack test, the bundler hostile/missing-image test,
and actual-pack copy/hash verification. The pack fidelity run uses local engine path patches
recorded in `out/cockpit/pack-check-config.toml`.

Clippy with `-D warnings` passed for aero/render/UI (all targets) and the pack harness.
`cargo xtask schemas --check` passed. `cargo xtask web --check` compiled all CI web packages
with warnings denied and validated 102,501,478 bytes of showcase data. `cargo fmt --all` ran in
both worktrees. No new dependencies, staging, or commits. Full workspace CI remains the
flight lead's pre-commit check. The confined V11 Blender run and its geometry/material proof
remain pending; the current captures show accepted V10 geometry.
