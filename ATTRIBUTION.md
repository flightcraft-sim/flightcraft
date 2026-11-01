# Attribution

Every asset and data source in FlightCraft is original or openly licensed, with one row here.
Rust crate dependencies are covered by the licence check (`cargo deny check licenses`), not listed here.

| Item | Source | Licence | Notes |
|---|---|---|---|
| Corvid geometry, coefficient tables, propulsion, and gear data | Original FlightCraft design | MIT OR Apache-2.0 | Engineering estimates; each table documents its derivation in `crates/aero/src/lib.rs`. |
| ISA equations and reference values | US Standard Atmosphere 1976, NOAA/NASA/USAF; ISO 2533 atmosphere relationships | Public domain (US government publication) | Equations independently implemented; geopotential height, 0–20 km. |
| Airfoil design background | NACA Report 824 (1945), Summary of Airfoil Data | Public domain (US government publication) | Cambered four-digit section background only; no digitised coefficient data. |
| Corvid display mesh, training field geometry, and scene shaders | Original procedural artwork in `crates/render` | MIT OR Apache-2.0 | Built in code; no external assets |
| Flight instruments and interface artwork | Original painter geometry in `crates/ui` | MIT OR Apache-2.0 | Built in code |
| Droid Sans (`crates/ui/assets/DroidSans.ttf`) | [Android Open Source Project](https://android.googlesource.com/platform/frameworks/base/), Droid fonts | Apache-2.0 | Copyright 2005–2008 The Android Open Source Project; bundled notice and licence in `crates/ui/assets/DroidSans-NOTICE.txt` |
