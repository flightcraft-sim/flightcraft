# Validation: eastern-pa

Dataset bytes: 49671412. Procedure parse failures: 0.

| Type | Count |
|---|---:|
| airports | 43 |
| fixes | 760 |
| holds | 15 |
| ils | 3 |
| navaids | 20 |
| points | 592 |
| procedures | 125 |
| routes | 378 |
| runways | 61 |
| terrain_tiles | 110 |
| volumes | 27 |

Elevation spot checks use up to 32 airports in identifier order after runway preparation, with a 15 m tolerance. Runway polygons are checked against published lengths with a 5% tolerance. Warnings require review; they are not silently corrected. JSON contains every measured value.

- 7N8: FAA elevation and terrain differ by 17.00 m
