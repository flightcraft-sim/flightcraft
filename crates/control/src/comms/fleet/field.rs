//! The controlled airfield: runway, facilities, and procedure geometry taken from the region data.
use super::craft::*;
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub name: String,
    pub pos: [f64; 2],
}

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub airport: String,
    pub elev_m: f64,
    pub runway: String,
    /// Landing threshold and the grid heading of landing.
    pub thr: [f64; 2],
    pub rh: f64,
    pub length_m: f64,
    pub center: [f64; 2],
    pub stations: Vec<ComStation>,
    pub tower: usize,
    pub ground: Option<usize>,
    /// Approach sector above 3,000 ft and below it; the departure position on the lower channel.
    pub hi: usize,
    pub lo: usize,
    pub dep: usize,
    pub entries: Vec<Entry>,
    pub iaf: [f64; 2],
    pub faf: [f64; 2],
    /// `ILS`, `LOC`, `RNAV`, or `Visual`, with the CIFP identifier (or `visual`).
    pub family: String,
    pub procedure: String,
    pub mag: flightcraft_sim::magnetic::HeadingReference,
    pub left_pattern: bool,
    fallback: ComStation,
}

impl Field {
    pub fn mag_deg(&self, grid_rad: f64) -> u16 {
        let d = self.mag.grid_to_magnetic(grid_rad).to_degrees().round() as i32;
        let d = d.rem_euclid(360) as u16;
        if d == 0 { 360 } else { d }
    }
    pub fn grid_from_mag(&self, deg: u16) -> f64 {
        self.mag.magnetic_to_grid(f64::from(deg).to_radians())
    }
    /// Runway-relative point: `x` metres along the landing direction from the threshold, `y` toward the
    /// circuit side.
    pub fn rw(&self, x: f64, y: f64) -> [f64; 2] {
        let side = if self.left_pattern { 1.0 } else { -1.0 };
        let along = offset(self.thr, self.rh, x);
        // Left of heading is heading minus ninety degrees.
        offset(along, self.rh - core::f64::consts::FRAC_PI_2 * side, y)
    }
    pub fn along(&self, p: [f64; 2]) -> f64 {
        (p[0] - self.thr[0]) * libm::cos(self.rh) + (p[1] - self.thr[1]) * libm::sin(self.rh)
    }
    pub fn cross(&self, p: [f64; 2]) -> f64 {
        -(p[0] - self.thr[0]) * libm::sin(self.rh) + (p[1] - self.thr[1]) * libm::cos(self.rh)
    }
    pub fn st(&self, index: usize) -> &ComStation {
        self.stations.get(index).unwrap_or(&self.fallback)
    }

    pub fn build(s: &Session, airport: &str) -> Result<Self, CommandError> {
        const NAME: &str = "traffic.density";
        let a = crate::regions::airport(s.sim.world(), airport)?.clone();
        let id = a.icao.clone().unwrap_or_else(|| a.ident.clone());
        let stations: Vec<ComStation> = s
            .stations()
            .into_iter()
            .filter(|st| {
                st.airport == id
                    && matches!(
                        st.kind,
                        FacilityKind::Tower
                            | FacilityKind::Ground
                            | FacilityKind::Approach
                            | FacilityKind::Departure
                    )
            })
            .collect();
        let find = |f: &dyn Fn(&ComStation) -> bool| stations.iter().position(f);
        let sector = |st: &ComStation| {
            st.metadata
                .as_ref()
                .map_or(String::new(), |m| m.sector.to_uppercase())
        };
        let tower = find(&|st| st.kind == FacilityKind::Tower).ok_or_else(|| {
            invalid(
                NAME,
                "airport has no imported tower; traffic needs a towered field",
            )
        })?;
        let any_app = find(&|st| st.kind == FacilityKind::Approach)
            .ok_or_else(|| invalid(NAME, "airport has no imported approach control"))?;
        let hi =
            find(&|st| st.kind == FacilityKind::Approach && sector(st).contains("ABV")).unwrap_or(any_app);
        let lo =
            find(&|st| st.kind == FacilityKind::Approach && sector(st).contains("BLW")).unwrap_or(any_app);
        let dep = find(&|st| {
            st.kind == FacilityKind::Departure
                && stations.get(lo).is_some_and(|l| l.channel_khz == st.channel_khz)
        })
        .or_else(|| find(&|st| st.kind == FacilityKind::Departure))
        .unwrap_or(lo);
        let ground = find(&|st| st.kind == FacilityKind::Ground);
        let runway_id = s
            .active_runway(&id)
            .ok_or_else(|| invalid(NAME, "airport has no usable runway"))?;
        let (runway, end) = a
            .runways
            .iter()
            .find_map(|r| r.ends.iter().find(|e| e.ident == runway_id).map(|e| (r, e)))
            .ok_or_else(|| invalid(NAME, "active runway end is missing"))?;
        let other = runway
            .ends
            .iter()
            .find(|e| e.ident != end.ident)
            .ok_or_else(|| invalid(NAME, "missing opposite runway end"))?;
        let project = |lat, lon| {
            s.radio_project(lat, lon)
                .ok_or_else(|| invalid(NAME, "traffic requires a geographic world"))
        };
        let thr = project(end.latitude_deg, end.longitude_deg)?;
        let far = project(other.latitude_deg, other.longitude_deg)?;
        let center = project(a.latitude_deg, a.longitude_deg)?;
        let rh = bearing(thr, far);
        let stations_fallback = stations
            .get(tower)
            .cloned()
            .ok_or_else(|| invalid(NAME, "tower station is missing"))?;
        let mut field = Self {
            airport: id.clone(),
            elev_m: end.elevation_m,
            runway: runway_id.clone(),
            thr,
            rh,
            length_m: runway.length_m,
            center,
            stations,
            tower,
            ground,
            hi,
            lo,
            dep,
            entries: Vec::new(),
            iaf: offset(thr, rh + core::f64::consts::PI, 13.0 * NM),
            faf: offset(thr, rh + core::f64::consts::PI, 6.5 * NM),
            family: "Visual".into(),
            procedure: "visual".into(),
            mag: s.sim.heading_reference(),
            left_pattern: true,
            fallback: stations_fallback,
        };
        field.load_procedures(s, &id);
        Ok(field)
    }

    /// Final-approach fixes and transition entries from the CIFP catalogue, when present.
    fn load_procedures(&mut self, s: &Session, id: &str) {
        let back = self.rh + core::f64::consts::PI;
        let mut entries = Vec::new();
        if let Some(procs) = s.sim.world().procedures() {
            let at = |fix: &str| {
                procs
                    .point(fix)
                    .and_then(|p| s.radio_project(p.position.latitude_deg, p.position.longitude_deg))
            };
            let matching: Vec<_> = procs
                .procedures
                .iter()
                .filter(|p| {
                    p.airport == id
                        && p.kind == flightcraft_world::procedures::ProcedureKind::Approach
                        && p.runway.as_deref().map(|r| r.trim_start_matches("RW"))
                            == Some(self.runway.as_str())
                })
                .collect();
            let chosen = ["I", "L", "R"]
                .iter()
                .find_map(|prefix| matching.iter().find(|p| p.ident.starts_with(prefix)));
            if let Some(p) = chosen
                && let Some(route) = p.routes.iter().find(|r| r.route_type != "A")
            {
                let fixes: Vec<[f64; 2]> = route
                    .legs
                    .iter()
                    .filter(|l| !l.missed)
                    .filter_map(|l| l.fix.as_deref().filter(|f| !f.contains("/PG/")).and_then(at))
                    .collect();
                if let (Some(&iaf), Some(&faf)) = (fixes.first(), fixes.last())
                    && dist(iaf, faf) > NM
                    && dist(iaf, self.thr) > dist(faf, self.thr)
                {
                    self.iaf = iaf;
                    self.faf = faf;
                    self.family = match p.ident.chars().next() {
                        Some('I') => "ILS",
                        Some('L') => "LOC",
                        _ => "RNAV",
                    }
                    .into();
                    self.procedure = p.ident.clone();
                }
            }
            for p in &matching {
                for route in p.routes.iter().filter(|r| r.route_type == "A") {
                    if let Some(leg) = route.legs.first()
                        && let Some(fix) = leg.fix.as_deref()
                        && let Some(pos) = at(fix)
                    {
                        let rel = wrap_pi(bearing(self.thr, pos) - back).abs();
                        let range = dist(self.thr, pos) / NM;
                        if rel <= 1.2 && (8.0..=70.0).contains(&range) {
                            entries.push(Entry {
                                name: fix.rsplit('/').next().unwrap_or(fix).to_string(),
                                pos,
                            });
                        }
                    }
                }
            }
        }
        // Synthetic feeds on the rear side of the runway keep the flow when the catalogue is thin.
        for (i, spread) in [-0.45, 0.0, 0.45].into_iter().enumerate() {
            entries.push(Entry {
                name: format!("FEED{}", i + 1),
                pos: offset(self.iaf, back + spread, 16.0 * NM),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries.dedup_by(|a, b| a.name == b.name);
        self.entries = entries;
    }
}
