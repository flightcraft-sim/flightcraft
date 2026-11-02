# Panel fidelity adoption report

The optional pack fields preserve the existing equipment demo's definitions and defaults.
A separate runnable example, `examples/packs/panel-fidelity-demo`, exercises the options.
The engine adds original hardware styling, a composed PFD page, a placed autopilot controller,
a narrow navigator layout, bounded captions, switched backup buses, alternating landing lights,
and a read-only equipment artwork registry. See [equipment](equipment.md) for semantics.

## New pack fields

Paths are relative to a panel layer unless marked as systems fields. Existing shell outlines,
glare-shield outlines, screws, display composition fields, and rocker sublabels are retained.

| Path | New fields or enum values |
|---|---|
| `equipment.shell` | `centre_console`, `palette` |
| `equipment.shell.palette` | `face`, `glare_shield`, `bezel`, `placard_text` |
| `equipment.elements[]` | `appearance` |
| `appearance` | `style`, `binding`, `bezel`, `controller`, `navigator_layout`, `skin`, `synthetic_vision` |
| `appearance.bezel` | `color`, `edge_trim`, `margins`, `softkeys`, `softkey_count`, `corner_knobs`, `card_slot` |
| `appearance.controller` | `sections`, `controls` |
| `appearance.controller.sections[]` | `label`, `rect` |
| `appearance.controller.controls[]` | `action`, `rect`, `lit_color` |
| `element.layout` for touch displays | New `integrated_pfd` value |
| `element.switch` for rockers | New `ems_backup`, `efis_backup`, `landing_wigwag` values |
| Systems `definition.modern` | `backup_buses`, `wigwag` |
| Systems `definition.modern.backup_buses[]` | `circuit`, `capacity_ah`, `voltage`, `loads` |
| Systems `definition.modern.wigwag` | `left_load`, `right_load`, `half_period_s` |

Style values are `black_rocker`, `push_pull_breaker`, `rounded_pull_handle`, `silver_key`,
`louvred_eyeball`, `elt_plate`, `red_t_handle`, `arc_dimmer`, and `cabin_heat`. Bindings are
`lane_a` and `lane_b`. Navigator layouts are `classic` and `home_columns`. Controller actions
are `ap`, `fd`, `yd`, `lvl`, `hdg`, `nav`, `apr`, `vs`, `alt`, `ias`, `vnav`, `heading_knob`,
`altitude_knob`, and `pitch_wheel`.

New equipment system actions are `backup_power` (`circuit`, `on`) and `wigwag` (`on`).
Snapshots add `backup_buses` (`circuit`, `on`, `remaining_ah`, `voltage`), `wigwag`, and
`wigwag_elapsed_s`. The command and aircraft schemas include these definitions.

## Verification

The review artifacts include original-demo before/after panels, compact panels, and a separate
fidelity example with navigator, autopilot, and display crops. The initial fidelity implementation retained six pixel-identical original-demo captures.
The caption/inset review now deliberately changes navigator typography and the no-region map
view. Current before/after captures and pixel difference reports live under `out/regression/`. The original demo uses a flat
world in this checkout because the optional regional terrain tiles are absent; the preview
harness uses identical scene setup for both captures. Reference-derived comparisons are
written under ignored `out/comparison/`.

Tests cover old defaults, optional field round trips, hostile geometry and colours, contrast,
style compatibility, backup-bus isolation and depletion, breaker isolation, guarded lane
binding and actual faults, deterministic alternating lights, caption bounds at measured
spacing, narrow navigator captions, registry artwork and preserved host hit regions, and
comparison output boundaries and scoring. Runtime limitations: backup batteries recharge only
on reset, and VNAV selects the existing GPS glidepath mode. CPU comparison captures omit GPU
synthetic-vision callbacks; the running application supplies that existing renderer.

Review validation: workspace clippy with warnings denied and generated schema checks passed.
The touched aero/UI/xtask run recorded 124 passes, two failures, and 11 ignored manual previews.
Both failures are existing synthetic-vision tests unable to open the absent regional terrain
files. The remaining UI suite passes all 68 tests when those two are excluded; all four control
fidelity tests also pass. Manual preview captures and the comparison command passed.

The caption audit checks all 274 visible captions at 1250 × 700, 1728 × 968, and 1100 × 620,
with both open and closed guards: zero overlaps. It also checks lamp/label/guard separation.
Inset audits check interior pixel diversity and movement in geographic and local worlds,
and ensure terrain insets leave shared MAP settings unchanged. Navigator tile and frequency
font sizes are checked for equality.

Artifacts are `out/equipment-fidelity/panel.png` and its crops,
`out/regression/{before,after}/`, `out/regression/*-before-after.png`,
`out/regression/regression.json`, `out/regression/caption-audit.json`,
`out/regression/inset-audit.json`, and `out/comparison/measured/`.
The original-demo regression deliberately changes the corrected navigator and map views;
outside those installations, channel differences are at most one RGB level.

Review corrections add `terrain` and `synthetic_vision` values to inset `content`; an integrated
PFD's bottom-right composition entry selects its second inset. Navigator tile and frequency
captions have shared sizes, bound toggle guards no longer cover captions or lamps, and the
whole-panel overlap check also found and corrected fuel-selector title clearance. All preview
and regression artifacts are regenerated beneath ignored `out/`.
