# Weather and simulated instrument practice

Visual weather belongs to the deterministic simulation session. The renderer reads a snapshot;
clouds, fog, and lights never change flight dynamics. Existing seeded wind, shear, and Dryden
turbulence retain their behaviour and magnetic command reference. Synthetic vision deliberately
ignores all visual weather, including night, and retains its existing sensor failure rules.

Open **Weather** for presets, up to three layers, visibility, obscuration, precipitation, time of
day, wind, turbulence, and the hood. The outside HUD reports the current layers and conditions.
Cloud heights use **feet MSL**, visibility uses **statute miles**, and wind speed uses **m/s**.
Presets are VFR (clear, 10 SM), MVFR (1,800 OVC, 4 SM haze), IFR (600 OVC, 2 SM mist),
LIFR (200 OVC, ½ SM fog), and clear night. Preset cloud heights are MSL too: for a ceiling of
600 ft above an airport, add that runway threshold's elevation to the base. Presets retain wind.
Rain multiplies surface visibility by 0.75; snow multiplies it by 0.5.

## Commands

All three commands have input and output schemas in `schema/commands.json` and are automatically
available through the MCP registry. `schema/weather.json` describes a complete visual state.

```json
{"preset":"ifr"}
```

Send that input to `weather.set`, or patch individual fields:

```json
{"layers":[{"base_ft_msl":1000,"top_ft_msl":4000,"coverage":"OVC"}],"visibility_sm":2,"obscuration":"mist","precipitation":"rain","time_of_day":"night"}
```

Coverage is `FEW`, `SCT`, `BKN`, or `OVC`. Bases must be below tops; both are finite and within
−1,500 to 60,000 ft MSL. Visibility must be finite and within 0.125–100 SM. Overlapping layers
are supported. Obscuration is `none`, `mist`, `fog`, or `haze`; precipitation is `none`, `rain`, or
`snow`; time of day is `day`, `dusk`, `night`, or `dawn`. A preset applies before explicit fields.
Invalid patches leave both visual weather and wind untouched.

The existing wind input remains supported. Supply all five fields together to replace wind and
restart its seeded gust process. Omit the group to preserve it:

```json
{"wind_from_deg":270,"wind_speed_mps":5,"shear":true,"turbulence":"light","seed":42}
```

`weather.state` takes `{}` and returns `conditions`, magnetic `weather` wind, `hood`,
`simulated_instrument_time`, failures, and aircraft systems. `sim.state` also includes visual
conditions, hood state, and practice totals. `view.hood` takes `{"enabled":true}` or
`{"enabled":false}`. Enabling it selects cockpit view in the app; remove it before selecting
chase view. New keymaps bind **F8** to the hood, with the same configurable binding machinery as
other actions. Existing timed JSON-lines scripts provide the phase-one scenario hook:

```jsonl
{"at_s":0,"command":"weather.set","input":{"preset":"ifr"}}
{"at_s":0,"command":"view.hood","input":{"enabled":true}}
{"at_s":30,"command":"view.hood","input":{"enabled":false}}
```

Hood occlusion is applied to the outside camera image, including during head pan. Cockpit framing,
the glare shield, instruments, and synthetic-vision displays remain visible. No dialog is opened.

## Flight record and determinism

`simulated_instrument_time.hood_s` and `.imc_s` are independent cumulative fixed-step seconds.
They overlap when both conditions apply; do not sum them. IMC counts time inside a spatial cloud
or with effective visibility below 3 SM. Hood counts time with the device enabled. Pausing,
reading state, or rendering does not advance either counter; explicit simulation steps do.
A flight reset clears visual weather, the hood, and these totals. The UI's wind reset behaviour
remains unchanged. These are **simulated instrument practice totals**, never FAA logbook credit.
They are available in the exported command snapshot; no persistent logbook is implemented.

The clear default adds no words to the established trajectory hash. Explicit visual weather and
practice totals enter the hash in stable field order; they leave the rigid-body trajectory alone.
Cloud coverage uses a fixed analytic spatial field shared by the simulation and shader, with no
wall clock or random generator. OVC is continuous; lower coverages have deterministic holes.

## Clouds, fog, and lights

Three analytic noise slabs give cloud depth without volume textures, compute shaders, or a
ray-march loop. Ray/slab intersections obscure terrain above the base and create a sharp exit
below it. Inside cloud the outside image is uniformly grey, with no horizon. Distance extinction
uses 3.912 divided by effective visibility in metres, the 2% contrast convention. Mist, fog,
and haze alter the scattering colour; precipitation additionally reduces effective visibility.
Night and twilight reduce environmental illumination; emissive lights remain readable.

Runway lighting uses cached luminous impostors with geometric occlusion. Runway edge
fixtures have 60 m spacing, with amber near the far end. Threshold/end fixtures show green toward
the approach and red toward the runway. Optional `RunwayEnd.lighting` metadata accepts
`approach: none | alsf2 | malsr | odals` and `visual_guidance: papi | vasi`. PAPI/VASI colours
change with the camera's vertical angle. An explicit `approach: none` suppresses the fallback.
The bundled FAA GIS runway feed has no lighting columns, so existing airport files omit the
metadata. Unknown approach lighting uses ALSF-2 for a runway with an ILS glideslope, MALSR for a
localizer-only runway, ODALS for another
published instrument approach, and no approach system otherwise. These fallbacks are simulated
training fixtures, not claims about a real airport's installed equipment. Unknown PAPI/VASI is
not invented. The original fictional smoke field has a MALSR and PAPI.

## Previews and checks

Restore the locked data, then run the ignored `render_weather_previews` UI test. It captures
actual GPU pixels and the original instrument panel at 1280 × 800 under `out/weather/`:

- `kabe-06-700ft-cloud.png`: 700 ft AGL inside a 600 ft AGL overcast;
- `kabe-06-500ft-breakout.png`: below the base, with the approach lights in 2 SM mist;
- `kabe-06-500ft-night.png`: the same breakout at night;
- `kabe-06-hood.png`: cockpit hood, with the panel visible;
- `kabe-06-vfr.png`: clear reference.

Companion `*-msl-*.png` (and `kabe-06-msl-hood.png` / `kabe-06-msl-vfr.png`)
images repeat all five cases at literal 700/500 ft MSL with a 600 ft MSL cloud base,
near the ILS decision height. Both altitude references are explicit in each caption.

The harness checks cloud pixels, approach-light contrast, hood occlusion during head pan, and
unchanged panel pixels. It writes mean and p95 wall-clock frame costs over 120 warmed frames,
including GPU completion, for each preview and the original smoke field. Set
`FLIGHTCRAFT_TEST_BACKEND=gl` to exercise native GLES with WebGL 2 limits. Native GLES timings
are a compatibility measurement, not a browser frame-rate claim. Browser measurements, when
available, are recorded separately with the renderer and build profile.

## Limitations

Edge and threshold installation availability is assumed when metadata is absent. Lighting is a
bounded visual approximation, without calibrated candela photometry, bloom,
sequenced flashers, terrain-following approach towers, or published intensity controls.
Approach fixtures follow an extrapolated threshold plane, so uneven approach terrain can occlude
them. PAPI/VASI geometry and nominal 3° settings are illustrative when metadata only names a type.
Cloud structure is static, with coarse FEW/SCT/BKN coverage, and no icing, cloud shadows, lightning,
or individual rain/snow particles. Time of day is an explicit illumination setting, not an
astronomical clock. The full scenario validator, live METARs, logbook persistence, and graded
instrument tasks remain future training phases. This is an offline simulator, not a certified
training device or a source for real-world weather, lighting, or navigation decisions.

See [training](training.md#6-instrument-flying), [synthetic vision](glass.md#synthetic-vision),
[geographic data](terrain.md), and the [design specification](SPEC.md).

For a browser timing run, package with `cargo xtask web --debug`, create `out/weather`, and
link `dist/weather` to `../out/weather`. Serve `dist` on localhost, start a headless Chromium
with a DevTools port and WebGL 2 enabled, then run:

```sh
node scripts/weather-bench.mjs 9348 http://127.0.0.1:8080 out/weather debug
```

The harness compares VFR, IFR, and night on the original smoke field. It measures callback cost
with WebGL completion, presentation intervals, and frame rate after warmup, saves screenshots,
and records browser errors. Browser startup must succeed before these results are meaningful.

## Rust integration

`world::Conditions` is separate from the existing copyable wind `Weather`, preserving wind starts
and existing calm-flight hashes. `Sim::conditions`, `set_conditions`, `hood`, `set_hood`, and
`instrument_time` expose the validated session model. `StateOutput` adds `conditions`, `hood`,
and `simulated_instrument_time`; the weather command output adds these to `Systems` as well.
Typed `control::SetWeather` calls now use optional patch fields instead of the former `weather`
wrapper. Supply all five optional wind fields together when replacing wind. `GetWeather` and
`SetHood` implement the new commands.

Render hosts pass `Scene::set_weather(conditions, hood)` before drawing. `Palette` adds
`light_red`, `light_green`, and `light_amber` tokens. `RunwayEnd.lighting` is optional serialized
`RunwayLighting`; projected `RunwayStrip.lighting` carries one optional value per end. Existing
serialized airport files remain compatible. Rust constructors should initialize unknown lighting
to `None` or `[None; 2]`. The optional `Scene::lights_visible` presentation switch supports
controlled light-contrast captures. Synthetic-vision hosts retain their existing API.
