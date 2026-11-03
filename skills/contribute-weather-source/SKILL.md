---
name: contribute-weather-source
description: Add or repair a FlightCraft live-weather endpoint definition, including local station pages, extraction fixtures, and advisory metadata. Use for weather-source contributions, not simulator weather physics.
---

# Contribute a weather source

Read the repository's AGENTS.md and CONTRIBUTING.md, then [live weather](../../docs/live-weather.md)
and [the source schema](../../schema/weather-source.json). Start from
[the DigiWx example](../../examples/weather-sources/1n7-digiwx.json).

Confirm which airport the endpoint actually reports for. Nearby official METARs retain their own
station identifier; do not manufacture an ICAO identifier by adding K. In particular, 1N7's
DigiWx page is advisory only and has no NWS-disseminated METAR. Official nearby stations include
KFWN, K12N, KMPO, and KSMQ. Discovery uses actual METAR reports, not stationinfo availability.

Choose the smallest extraction: a preset, stable element id, restricted id/class selector,
JSON pointer, plain body, or linear-time regex with exactly one capture group. Supplement missing
METAR fields rather than replacing good parsed data. Source wind directions are true. Verify
station time, units, fractions, and cloud heights AGL. Unknown values must remain unknown.

HTTP requires explicit allow_http. Advisory-only sources need a nonempty disclaimer, visible
with observations and spoken transcripts. Do not suppress warnings to make a source test pass.
Browser support requires actual server CORS permission; do not introduce a proxy or credentials.
Respect refresh/backoff and use only a GET. Do not execute page scripts or form postbacks.

Test with weather.source_test and inspect its exact matches and parsed fields before saving.
Network access is for an authorized manual test only; CI uses original minimal fixtures and a
127.0.0.1 fake server. Never check a provider's HTML page into the repository. Write invented
fixture values with the same relevant element ids, and attribute the endpoint as a runtime-only
source. Document any source-specific limitation and privacy implication.

Run the affected control tests, clippy with warnings denied, schema generation/check, browser
check, and formatting using the lane's external CARGO_TARGET_DIR and job limit. Follow the user's
staging and commit instructions. Report test results and any live-fetch or preview limitation.
