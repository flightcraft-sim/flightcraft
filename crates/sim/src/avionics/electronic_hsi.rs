//! Independent heading unit with great-circle direct-to guidance and attitude reversion.
use super::{electronic_attitude::ElectronicAttitude, invalid};
use crate::{Readout, SimError, State, systems::SystemsState};
use flightcraft_world::World;
use schemars::JsonSchema;
use serde::Serialize;

use super::navigation::{cross_track, inverse_nm as inverse};
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize, JsonSchema)]
pub struct ElectronicHsi {
    pub reversion_attitude: ElectronicAttitude,
    pub heading_bug_deg: f64,
    pub course_deg: f64,
    pub gps_fix: bool,
    pub destination: Option<String>,
    pub distance_nm: Option<f64>,
    pub bearing_deg: Option<f64>,
    pub lateral_deviation_nm: Option<f64>,
    /// Positive right of course, clamped at the displayed full-scale stops.
    pub cdi: Option<f64>,
    pub full_scale_nm: f64,
    pub ground_speed_kt: Option<f64>,
    pub track_deg: Option<f64>,
    pub wind_from_deg: Option<f64>,
    pub wind_speed_kt: Option<f64>,
    pub reversion: bool,
    pub manual_reversion: bool,
    #[serde(skip)]
    destination_position: Option<[f64; 2]>,
    #[serde(skip)]
    leg_origin: Option<[f64; 2]>,
}
impl Default for ElectronicHsi {
    fn default() -> Self {
        Self {
            reversion_attitude: ElectronicAttitude::with_seed(0x432a_e9b7, 1800.),
            heading_bug_deg: 0.,
            course_deg: 0.,
            gps_fix: false,
            destination: None,
            distance_nm: None,
            bearing_deg: None,
            lateral_deviation_nm: None,
            cdi: None,
            full_scale_nm: 5.,
            ground_speed_kt: None,
            track_deg: None,
            wind_from_deg: None,
            wind_speed_kt: None,
            reversion: false,
            manual_reversion: false,
            destination_position: None,
            leg_origin: None,
        }
    }
}
impl ElectronicHsi {
    pub(crate) fn nav_guidance(&self, world: &World, state: State) -> Option<crate::nav_source::NavGuidance> {
        use crate::nav_source::NavSource;
        if !self.gps_fix {
            return None;
        }
        let region = world.region()?;
        let leg = crate::nav_source::ReceiverLeg {
            start_deg: self.leg_origin?,
            end_deg: self.destination_position?,
            projection_origin_deg: [region.origin.latitude_deg, region.origin.longitude_deg],
        };
        let [n, e, _] = state.position_m;
        leg.guidance(n, e)
    }

    pub(crate) fn hash_words(&self) -> Vec<u64> {
        let mut words = self.reversion_attitude.hash_words();
        words.extend([self.gps_fix, self.reversion, self.manual_reversion].map(u64::from));
        words.extend([self.heading_bug_deg, self.course_deg, self.full_scale_nm].map(f64::to_bits));
        words.extend(
            [
                self.distance_nm,
                self.bearing_deg,
                self.lateral_deviation_nm,
                self.cdi,
                self.ground_speed_kt,
                self.track_deg,
                self.wind_from_deg,
                self.wind_speed_kt,
            ]
            .map(|v| v.map_or(u64::MAX, f64::to_bits)),
        );
        if let Some(destination) = &self.destination {
            words.extend(destination.bytes().map(u64::from));
        }
        for pair in [self.destination_position, self.leg_origin] {
            words.push(u64::from(pair.is_some()));
            if let Some(pair) = pair {
                words.extend(pair.map(f64::to_bits));
            }
        }
        words
    }
    pub(crate) fn direct_to(
        &mut self,
        ident: &str,
        world: &World,
        state: State,
        declination_rad: f64,
    ) -> Result<(), SimError> {
        if ident.is_empty() || ident.len() > 8 || !ident.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(invalid("invalid airport ident"));
        }
        let airport = world
            .airports()
            .and_then(|a| {
                a.airports.iter().find(|a| {
                    a.ident.eq_ignore_ascii_case(ident)
                        || a.icao.as_ref().is_some_and(|v| v.eq_ignore_ascii_case(ident))
                })
            })
            .ok_or_else(|| invalid("airport is not in the active region"))?;
        let [n, e, _] = state.position_m;
        let origin = world.geodetic(n, e).ok_or_else(|| invalid("NO GPS"))?;
        let position = [airport.latitude_deg, airport.longitude_deg];
        self.course_deg = (inverse(origin, position).1 - declination_rad.to_degrees()).rem_euclid(360.);
        self.destination = Some(airport.ident.clone());
        self.destination_position = Some(position);
        self.leg_origin = Some(origin);
        Ok(())
    }
    pub(crate) fn update(
        &mut self,
        state: State,
        truth: Readout,
        systems: &SystemsState,
        world: &World,
        attitude_failed: bool,
        advance: bool,
    ) {
        self.reversion_attitude
            .update("electronic_hsi", state, truth, systems, advance);
        self.reversion = self.manual_reversion || attitude_failed;
        let [n, e, _] = state.position_m;
        let position = world.geodetic(n, e);
        self.gps_fix =
            self.reversion_attitude.power.powered && position.is_some() && world.region().is_some();
        self.distance_nm = None;
        self.bearing_deg = None;
        self.lateral_deviation_nm = None;
        self.cdi = None;
        self.ground_speed_kt = None;
        self.track_deg = None;
        self.wind_from_deg = None;
        self.wind_speed_kt = None;
        if !self.gps_fix {
            return;
        }
        self.ground_speed_kt = Some(truth.ground_speed_mps * 1.94384449);
        // Resolve local grid vectors into true meridian bearings through world.geodetic.
        let [vn, ve, _] = state.velocity_mps;
        let true_bearing = |north: f64, east: f64| {
            world
                .geodetic(n + north, e + east)
                .zip(position)
                .map(|(to, from)| inverse(from, to).1)
        };
        self.track_deg = if truth.ground_speed_mps > 0.1 {
            true_bearing(vn, ve).map(|d| (d - truth.magnetic_declination_rad.to_degrees()).rem_euclid(360.))
        } else {
            None
        };
        // Wind triangle: true air-data velocity rotated into NED, subtracted from GPS velocity.
        let air = crate::math::rotate(
            state.attitude,
            [
                truth.true_airspeed_mps * libm::cos(truth.alpha_rad) * libm::cos(truth.beta_rad),
                truth.true_airspeed_mps * libm::sin(truth.beta_rad),
                truth.true_airspeed_mps * libm::sin(truth.alpha_rad) * libm::cos(truth.beta_rad),
            ],
        );
        let [an, ae, _] = air;
        let (wn, we) = (vn - an, ve - ae);
        self.wind_speed_kt = Some(libm::hypot(wn, we) * 1.94384449);
        self.wind_from_deg = Some(if libm::hypot(wn, we) > 0.1 {
            true_bearing(-wn, -we)
                .map(|d| (d - truth.magnetic_declination_rad.to_degrees()).rem_euclid(360.))
                .unwrap_or(0.)
        } else {
            0.
        });
        if let (Some(pos), Some(dest), Some(origin)) = (position, self.destination_position, self.leg_origin)
        {
            let (distance, bearing) = inverse(pos, dest);
            self.distance_nm = Some(distance);
            self.bearing_deg = Some((bearing - truth.magnetic_declination_rad.to_degrees()).rem_euclid(360.));
            self.full_scale_nm = if distance <= 30. { 1. } else { 5. };
            let cross = cross_track(
                origin,
                pos,
                self.course_deg + truth.magnetic_declination_rad.to_degrees(),
            );
            self.lateral_deviation_nm = Some(cross);
            self.cdi = Some((cross / self.full_scale_nm).clamp(-1., 1.));
        }
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    fn geographic_world() -> World {
        use flightcraft_world::{Airports, HeightTile, Region, Surface};
        let mut region =
            Region::from_json(include_str!("../../../../assets/terrain/new-york/region.json")).unwrap();
        region.southwest_m = [-100000., -100000.];
        region.tiles_north = 1;
        region.tiles_east = 1;
        region.spacing_m = 1000.;
        region.tile_size = 257;
        region.tiles = vec!["000.fcht".into()];
        let airports = Airports::from_json(
            include_bytes!("../../../../assets/terrain/new-york/airports.json"),
            &region,
        )
        .unwrap();
        let tile = HeightTile::new(
            region.southwest_m,
            1000.,
            257,
            vec![100.; 257 * 257],
            vec![Surface::Grass; 256 * 256],
        )
        .unwrap();
        World::from_region(region, vec![tile])
            .unwrap()
            .with_airports(airports)
            .unwrap()
    }
    #[test]
    fn independent_battery_alignment_and_geodetic_cdi_scaling() {
        let world = geographic_world();
        let region = world.region().unwrap();
        let projection = flightcraft_world::geo::LocalProjection::new(
            region.origin.latitude_deg,
            region.origin.longitude_deg,
        )
        .unwrap();
        let mut h = ElectronicHsi::default();
        let systems = SystemsState::default();
        for _ in 0..5400 {
            h.update(
                State::default(),
                Readout::default(),
                &systems,
                &world,
                false,
                true,
            );
        }
        assert!(h.reversion_attitude.attitude_valid);
        assert!(h.reversion_attitude.power.on_battery);
        for ident in ["1N7", "KMMU"] {
            let airport = world
                .airports()
                .unwrap()
                .airports
                .iter()
                .find(|a| a.ident == ident || a.icao.as_deref() == Some(ident))
                .unwrap();
            let dest = [airport.latitude_deg, airport.longitude_deg];
            let [n, e] = projection.project(dest[0] - 0.7, dest[1]).unwrap();
            let origin = State {
                position_m: [n, e, -1000.],
                ..State::default()
            };
            h.direct_to(ident, &world, origin, 0.).unwrap();
            // A northbound path: one nm right, and 6 nm along, gives 0.2 en-route scale.
            let east = dest[1] + 1. / (60. * libm::cos(dest[0].to_radians()));
            let [n, e] = projection.project(dest[0] - 0.6, east).unwrap();
            let state = State {
                position_m: [n, e, -1000.],
                ..State::default()
            };
            h.update(state, Readout::default(), &systems, &world, false, false);
            assert_eq!(h.full_scale_nm, 5.);
            assert!((h.cdi.unwrap() - 0.2).abs() < 0.003);
            let [n, e] = projection.project(dest[0] - 0.1, east).unwrap();
            h.update(
                State {
                    position_m: [n, e, -1000.],
                    ..state
                },
                Readout::default(),
                &systems,
                &world,
                false,
                false,
            );
            assert_eq!(h.full_scale_nm, 1.);
            assert!((h.cdi.unwrap() - 1.).abs() < 0.003);
            // Left and right signs are symmetric near the destination.
            let [n, e] = projection.project(dest[0] - 0.1, 2. * dest[1] - east).unwrap();
            h.update(
                State {
                    position_m: [n, e, -1000.],
                    ..state
                },
                Readout::default(),
                &systems,
                &world,
                false,
                false,
            );
            assert!((h.cdi.unwrap() + 1.).abs() < 0.003);
        }
        h.reversion_attitude.power.battery_remaining_s = crate::DT_S;
        h.update(State::default(), Readout::default(), &systems, &world, true, true);
        assert!(!h.reversion_attitude.power.powered && !h.gps_fix && h.reversion);
    }
    #[test]
    fn hand_computed_cross_track_near_regional_airports() {
        for origin in [[40.971145_f64, -74.997475], [40.799062_f64, -74.414725]] {
            // One minute of latitude is approximately one nm. On a northbound leg,
            // one minute of longitude divided by cos(latitude) gives one nm right.
            let right = [
                origin[0],
                origin[1] + 1. / (60. * libm::cos(origin[0].to_radians())),
            ];
            assert!((cross_track(origin, right, 0.) - 1.).abs() < 0.002);
            assert!((cross_track(origin, right, 180.) + 1.).abs() < 0.002);
            assert!(cross_track(origin, [origin[0] + 0.1, origin[1]], 0.).abs() < 1e-10);
        }
    }
    #[test]
    fn distance_and_bearing_match_independent_haversine_calculation() {
        let from = [40.971145_f64, -74.997475_f64];
        let to = [40.799062_f64, -74.414725_f64];
        let p = from[0].to_radians();
        let q = to[0].to_radians();
        let dl = (to[1] - from[1]).to_radians();
        let h = libm::pow(libm::sin((q - p) / 2.), 2.)
            + libm::cos(p) * libm::cos(q) * libm::pow(libm::sin(dl / 2.), 2.);
        let distance = 2. * 6371008.8 * libm::asin(libm::sqrt(h)) / 1852.;
        let bearing = libm::atan2(
            libm::sin(dl) * libm::cos(q),
            libm::cos(p) * libm::sin(q) - libm::sin(p) * libm::cos(q) * libm::cos(dl),
        )
        .to_degrees()
        .rem_euclid(360.);
        let actual = inverse(from, to);
        assert!((actual.0 - distance).abs() < 0.1);
        assert!((actual.1 - bearing).abs() < 1.);
    }
}
