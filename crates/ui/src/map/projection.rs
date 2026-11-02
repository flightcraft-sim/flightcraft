use eframe::egui::{self, Pos2, Rect};
use flightcraft_control::{MapState, MapUp};
use flightcraft_world::{GeoPoint, geo::LocalProjection};
/// One affine view in the terrain region's transverse Mercator frame, in metres.
#[derive(Clone, Copy)]
pub struct View {
    pub projection: LocalProjection,
    pub rect: Rect,
    pub centre: [f64; 2],
    pub angle: f64,
    pub pixels_per_m: f64,
    pub aircraft_grid_heading_rad: f64,
    pub magnetic_bearing_deg: Option<f64>,
    pub north_grid_rad: f64,
}
impl View {
    pub fn new(s: &flightcraft_control::Session, state: &MapState, rect: Rect) -> Option<Self> {
        if !rect.is_finite() || rect.width() <= 0. || rect.height() <= 0. {
            return None;
        }
        let region = s.sim().world().region()?;
        let projection =
            LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg).ok()?;
        let [n, e, _] = s.sim().state().position_m;
        let centre = state
            .centre
            .and_then(|p| projection.project(p.latitude_deg, p.longitude_deg).ok())
            .unwrap_or([n, e]);
        let position = |[north, east]: [f64; 2]| {
            projection
                .unproject(north, east)
                .ok()
                .map(|[latitude_deg, longitude_deg]| GeoPoint {
                    latitude_deg,
                    longitude_deg,
                })
        };
        let aircraft = position([n, e])?;
        let true_north = |point: GeoPoint| -> Option<f64> {
            let [an, ae] = projection.project(point.latitude_deg, point.longitude_deg).ok()?;
            let next = point.destination(0., 100.);
            let [bn, be] = projection.project(next.latitude_deg, next.longitude_deg).ok()?;
            Some((be - ae).atan2(bn - an))
        };
        let north_grid_rad = true_north(position(centre)?)?;
        let aircraft_north_grid = true_north(aircraft)?;
        let declination = flightcraft_world::geomag::MagneticModel::wmm2025()
            .ok()?
            .declination(aircraft.latitude_deg, aircraft.longitude_deg, s.simulation_date())
            .ok()?
            .to_radians();
        // Physical attitude and velocity stay in the grid frame, independently of pilot-facing
        // readout changes. Convert to magnetic for the reference, then back into that frame for
        // rotation, avoiding a second variation correction when readout headings become magnetic.
        let [w, x, y, z] = s.sim().state().attitude;
        let aircraft_grid_heading_rad = (2.0 * (w * z + x * y))
            .atan2(1.0 - 2.0 * (y * y + z * z))
            .rem_euclid(std::f64::consts::TAU);
        let [vn, ve, _] = s.sim().state().velocity_mps;
        let grid_track = if vn.hypot(ve) > 0.5 {
            ve.atan2(vn)
        } else {
            aircraft_grid_heading_rad
        };
        let magnetic =
            |grid: f64| (grid - aircraft_north_grid - declination).rem_euclid(std::f64::consts::TAU);
        let magnetic_bearing = match state.orientation {
            MapUp::NorthUp => None,
            MapUp::HeadingUp => Some(magnetic(aircraft_grid_heading_rad)),
            MapUp::TrackUp => Some(magnetic(grid_track)),
        };
        let angle = magnetic_bearing.map_or(north_grid_rad, |bearing| {
            bearing + declination + aircraft_north_grid
        });
        Some(Self {
            projection,
            rect,
            centre,
            angle,
            aircraft_grid_heading_rad,
            magnetic_bearing_deg: magnetic_bearing.map(f64::to_degrees),
            north_grid_rad,
            pixels_per_m: f64::from(rect.width().min(rect.height())) / (state.range_nm * 1852. * 2.),
        })
    }
    pub fn local(&self, [n, e]: [f64; 2]) -> Pos2 {
        let [cn, ce] = self.centre;
        let (n, e) = (n - cn, e - ce);
        let (sin, cos) = self.angle.sin_cos();
        self.rect.center()
            + egui::vec2(
                ((e * cos - n * sin) * self.pixels_per_m) as f32,
                (-(n * cos + e * sin) * self.pixels_per_m) as f32,
            )
    }
    pub fn local_at(&self, p: Pos2) -> [f64; 2] {
        let d = p - self.rect.center();
        let (x, y) = (
            f64::from(d.x) / self.pixels_per_m,
            -f64::from(d.y) / self.pixels_per_m,
        );
        let (sin, cos) = self.angle.sin_cos();
        let [cn, ce] = self.centre;
        [cn + y * cos - x * sin, ce + x * cos + y * sin]
    }
    pub fn point(&self, p: GeoPoint) -> Option<Pos2> {
        Some(self.local(self.projection.project(p.latitude_deg, p.longitude_deg).ok()?))
    }
    pub fn position(&self, p: Pos2) -> Option<GeoPoint> {
        let [n, e] = self.local_at(p);
        let [latitude_deg, longitude_deg] = self.projection.unproject(n, e).ok()?;
        Some(GeoPoint {
            latitude_deg,
            longitude_deg,
        })
    }
}
