//! The nearest pages of the landscape touch EFIS: lists of the nearest VORs, NDBs, intersections,
//! and airspace, and the rules that pick and order them.
//!
//! The unit lists facilities within 200 NM, nearest first, and reports "None Within 200 NM" when
//! there are none. Airspace is listed by its status: inside now, or the time to entry when the
//! aircraft would enter it within 10 minutes at its present speed, track, and altitude;
//! otherwise "No Immediate Airspace". The list-building here is pure; the session fills in the
//! facilities from its navigation data.
use flightcraft_aero::modern::NearestPage;
use schemars::JsonSchema;
use serde::Serialize;

/// Search radius for nearest facilities.
pub const SEARCH_RADIUS_NM: f64 = 200.;
/// Rows kept per list.
pub const MAX_ROWS: usize = 15;
/// How far ahead the airspace list looks.
pub const LOOKAHEAD_S: u32 = 600;
/// Spacing of the look-ahead samples.
pub const LOOKAHEAD_STEP_S: u32 = 60;

/// A VOR, NDB, or intersection near the aircraft.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema, serde::Deserialize)]
pub struct NearestFacility {
    pub ident: String,
    pub name: String,
    /// Magnetic bearing from the aircraft.
    pub bearing_rad: f64,
    pub distance_nm: f64,
    /// VOR and NDB frequency, kHz.
    pub frequency_khz: Option<f64>,
    /// Facility class: VOR, VOR/DME, VORTAC, NDB, or empty.
    pub detail: String,
}

/// A piece of airspace the aircraft is in or about to enter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, serde::Deserialize)]
pub struct NearestAirspace {
    pub name: String,
    pub class: Option<String>,
    /// Seconds until entry; zero when inside now.
    pub entry_s: u32,
}

/// Facilities within the radius, nearest first, at most [`MAX_ROWS`].
pub fn keep_nearest(mut list: Vec<NearestFacility>) -> Vec<NearestFacility> {
    list.retain(|f| f.distance_nm.is_finite() && f.distance_nm <= SEARCH_RADIUS_NM);
    list.sort_by(|a, b| {
        a.distance_nm
            .total_cmp(&b.distance_nm)
            .then_with(|| a.ident.cmp(&b.ident))
    });
    list.truncate(MAX_ROWS);
    list
}

/// One airspace volume as sampled at a moment: identifier, name, class.
pub type SampledVolume = (String, String, Option<String>);

/// Build the airspace list from samples of the airspace containing the aircraft at each
/// look-ahead step, sample 0 being now. A volume present at sample 0 is inside; one first
/// present at a later sample is entered at that sample's time.
pub fn airspace_entries(samples: &[Vec<SampledVolume>]) -> Vec<NearestAirspace> {
    let mut out: Vec<NearestAirspace> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (index, sample) in samples.iter().enumerate() {
        let entry_s = (index as u32) * LOOKAHEAD_STEP_S;
        if entry_s > LOOKAHEAD_S {
            break;
        }
        for (id, name, class) in sample {
            if seen.insert(id.clone()) {
                out.push(NearestAirspace {
                    name: name.clone(),
                    class: class.clone(),
                    entry_s,
                });
            }
        }
    }
    out.sort_by_key(|a| a.entry_s);
    out.truncate(MAX_ROWS);
    out
}

/// Status text: "Inside", or "Entry in m:ss".
pub fn airspace_status(entry_s: u32) -> String {
    if entry_s == 0 {
        "Inside".to_owned()
    } else {
        format!("Entry {}:{:02}", entry_s / 60, entry_s % 60)
    }
}

/// Display text of one facility row: identifier, bearing, distance, frequency.
pub fn facility_row(page: NearestPage, f: &NearestFacility) -> String {
    let frequency = f.frequency_khz.map_or(String::new(), |khz| match page {
        NearestPage::Ndbs => format!("{khz:.0}"),
        _ => format!("{:.2}", khz / 1000.),
    });
    format!(
        "{:<6} {:03.0}° {:>5.1} NM {}",
        f.ident,
        f.bearing_rad.to_degrees().rem_euclid(360.),
        f.distance_nm,
        frequency
    )
    .trim_end()
    .to_owned()
}

/// Display text of one airspace row: name, class, status.
pub fn airspace_row(a: &NearestAirspace) -> String {
    let class = a
        .class
        .as_deref()
        .map_or(String::new(), |c| format!(" Class {c}"));
    format!("{}{} {}", a.name, class, airspace_status(a.entry_s))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fac(ident: &str, nm: f64) -> NearestFacility {
        NearestFacility {
            ident: ident.into(),
            name: ident.into(),
            bearing_rad: 0.5,
            distance_nm: nm,
            frequency_khz: Some(113_800.),
            detail: "VOR".into(),
        }
    }

    #[test]
    fn nearest_lists_sort_by_distance_and_stop_at_200_nm() {
        let list = keep_nearest(vec![
            fac("FAR", 250.),
            fac("B", 20.),
            fac("A", 5.),
            fac("EDGE", 200.),
        ]);
        let idents: Vec<_> = list.iter().map(|f| f.ident.as_str()).collect();
        assert_eq!(idents, ["A", "B", "EDGE"]);
        assert!(keep_nearest(vec![fac("FAR", 201.)]).is_empty());
        assert!(keep_nearest(vec![fac("NAN", f64::NAN)]).is_empty());
    }

    #[test]
    fn lists_keep_fifteen_rows_with_a_stable_tie_order() {
        let many: Vec<_> = (0..40).map(|i| fac(&format!("F{i:02}"), 10.)).collect();
        let list = keep_nearest(many);
        assert_eq!(list.len(), MAX_ROWS);
        assert_eq!(list[0].ident, "F00");
        assert_eq!(list[14].ident, "F14");
    }

    #[test]
    fn airspace_lists_inside_first_then_entries_by_time() {
        let v = |id: &str, class: &str| (id.to_owned(), format!("{id} area"), Some(class.to_owned()));
        let samples = vec![
            vec![v("E1", "E")],
            vec![v("E1", "E")],
            vec![v("E1", "E"), v("D1", "D")],
            vec![v("D1", "D")],
            vec![v("D1", "D"), v("B1", "B")],
        ];
        let list = airspace_entries(&samples);
        let rows: Vec<_> = list.iter().map(|a| (a.name.as_str(), a.entry_s)).collect();
        assert_eq!(rows, [("E1 area", 0), ("D1 area", 120), ("B1 area", 240)]);
        assert_eq!(airspace_status(0), "Inside");
        assert_eq!(airspace_status(125), "Entry 2:05");
        assert_eq!(airspace_row(&list[1]), "D1 area Class D Entry 2:00");
    }

    #[test]
    fn nothing_ahead_is_an_empty_list_with_the_guides_text() {
        assert!(airspace_entries(&[vec![], vec![], vec![]]).is_empty());
        assert!(airspace_entries(&[]).is_empty());
        assert_eq!(NearestPage::Airspace.empty_text(), "No Immediate Airspace");
        assert_eq!(NearestPage::Vors.empty_text(), "None Within 200 NM");
        // Samples past ten minutes are ignored.
        let late: Vec<Vec<SampledVolume>> = (0..=12)
            .map(|i| {
                if i == 12 {
                    vec![("X".into(), "X".into(), None)]
                } else {
                    vec![]
                }
            })
            .collect();
        assert!(airspace_entries(&late).is_empty());
    }

    #[test]
    fn rows_show_identifier_bearing_distance_and_frequency() {
        let f = fac("JFK", 12.34);
        let text = facility_row(NearestPage::Vors, &f);
        assert!(text.starts_with("JFK"), "{text}");
        assert!(
            text.contains("029°") && text.contains("12.3 NM") && text.ends_with("113.80"),
            "{text}"
        );
        let ndb = NearestFacility {
            frequency_khz: Some(350.),
            ..fac("AB", 3.)
        };
        assert!(facility_row(NearestPage::Ndbs, &ndb).ends_with("350"));
        let int = NearestFacility {
            frequency_khz: None,
            ..fac("ROBER", 8.)
        };
        assert!(facility_row(NearestPage::Intersections, &int).ends_with("NM"));
    }

    #[test]
    fn the_modelled_pages_are_listed_in_the_units_order() {
        let order: Vec<_> = NearestPage::ALL.iter().map(|p| p.abbreviation()).collect();
        assert_eq!(order, ["Apt", "VOR", "NDB", "INT", "ASPC"]);
    }
}
