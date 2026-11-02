# Data size projection

Measured terrain v2/v1 ratio: 0.223. Lossless WebP/PNG ratio: 0.614.

Projected national terrain: 3.04 GB; charts: 3.82 GB; total: 6.86 GB (decimal). Target: about 5–8 GB.

This extrapolates the two showcase regions across 9.83 million square kilometres at 60 m terrain spacing and chart zooms 8–11, with 40° representative latitude. It counts one chart coverage per point. Regional overlaps are not a national inventory: relief, Alaska, islands, chart duplication, and latitude can change these estimates substantially. The JSON records measured byte counts and assumptions. A full inventory must confirm the target as cells are added.

Charts use the pure-Rust MIT/Apache-2.0 image-webp lossless encoder. Every measured tile is decoded again and compared byte-for-byte in RGBA. Terrain error is at most 0.125 m at stored samples; the original training valley stays exact.

Desktop data comes from GitHub releases. Pages includes only the showcase regions and refuses a site reaching 1 GB. The data base URL is configurable without credentials; no additional hosting service is required.
