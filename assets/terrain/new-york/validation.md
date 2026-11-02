# Validation: new-york

Dataset bytes: 50339002. Procedure parse failures: 0.

| Type | Count |
|---|---:|
| airports | 26 |
| comms | 226 |
| fixes | 887 |
| holds | 15 |
| ils | 7 |
| navaids | 21 |
| points | 821 |
| procedures | 193 |
| routes | 316 |
| runways | 46 |
| terrain_tiles | 88 |
| volumes | 32 |

Elevation spot checks use up to 32 airports in identifier order after runway preparation, with a 15 m tolerance. Runway polygons are checked against published lengths with a 5% tolerance. Warnings require review; they are not silently corrected. JSON contains every measured value.
