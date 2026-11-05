//! Independent painted-runway alignment gate.
//!
//! The imagery is resampled into a runway-aligned raster (along-track `u`, cross-track `v`, metres)
//! centred on the FAA runway axis from the region data. Painted features are then found by plain
//! image processing (no learned model): a luminance threshold taken from the raster's own
//! pavement/paint statistics, connected-component centreline dashes with a Theil-Sen line fit, and
//! full-width transverse threshold bars from the along-track paint-fraction profile. Their positions
//! are compared with the FAA runway ends and surveyed displaced thresholds. Nothing here reads
//! the georegistration it is judging: the FAA coordinates only seed the search window.
use flightcraft_world::{geo::LocalProjection, scenery};
use serde::Serialize;

/// Acceptance tolerance: every measured end offset must be within this horizontal distance.
pub const GATE_M: f64 = 2.0;
const STEP_M: f64 = 0.25;
const FT_M: f64 = 0.3048;

/// NAIP imagery assembled in EPSG:3857, north-up, pixel-area registered.
pub struct Mosaic {
    pub bounds: [f64; 4],
    pub side: usize,
    pub rgb: Vec<[u8; 3]>,
}
impl Mosaic {
    pub fn new(bounds: [f64; 4], side: usize) -> Self {
        Self {
            bounds,
            side,
            rgb: vec![[0; 3]; side * side],
        }
    }
    pub fn paste(&mut self, x0: usize, y0: usize, image: &image::RgbaImage) {
        for (x, y, p) in image.enumerate_pixels() {
            let (px, py) = (x0 + x as usize, y0 + y as usize);
            if px < self.side && py < self.side {
                self.rgb[py * self.side + px] = [p.0[0], p.0[1], p.0[2]];
            }
        }
    }
    fn at(&self, x: i64, y: i64) -> [f32; 3] {
        let max = self.side as i64 - 1;
        let p = self.rgb[(y.clamp(0, max) as usize) * self.side + x.clamp(0, max) as usize];
        [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]
    }
    /// Bilinear sample at a geodetic position; `None` outside the mosaic.
    pub fn sample(&self, lat: f64, lon: f64) -> Option<[f32; 3]> {
        let [mx, my] = scenery::mercator(lat, lon).ok()?;
        self.sample_mercator(mx, my)
    }
    pub fn sample_mercator(&self, mx: f64, my: f64) -> Option<[f32; 3]> {
        let [w, s, e, n] = self.bounds;
        let px = (mx - w) / (e - w) * self.side as f64 - 0.5;
        let py = (n - my) / (n - s) * self.side as f64 - 0.5;
        if px < 0. || py < 0. || px > self.side as f64 - 1. || py > self.side as f64 - 1. {
            return None;
        }
        let (x0, y0) = (libm::floor(px), libm::floor(py));
        let (fx, fy) = ((px - x0) as f32, (py - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut out = [0_f32; 3];
        for (c, value) in out.iter_mut().enumerate() {
            let a = self.at(x0, y0)[c] * (1. - fx) + self.at(x0 + 1, y0)[c] * fx;
            let b = self.at(x0, y0 + 1)[c] * (1. - fx) + self.at(x0 + 1, y0 + 1)[c] * fx;
            *value = a * (1. - fy) + b * fy;
        }
        Some(out)
    }
}

/// One runway end taken from the region data plus the FAA displaced-threshold survey.
#[derive(Clone, Debug)]
pub struct EndReference {
    pub ident: String,
    pub lat: f64,
    pub lon: f64,
    pub displaced_threshold_m: f64,
    /// Surveyed displaced-threshold position (FAA NASR), a point independent of the region ends.
    pub displaced_lat_lon: Option<[f64; 2]>,
}
#[derive(Clone, Debug)]
pub struct RunwayReference {
    pub airport: String,
    pub designator: String,
    pub width_m: f64,
    pub ends: [EndReference; 2],
}

/// FAA 28-day NASR 2026-10-29 APT_RWY_END (3rd-party survey 2014-08-11) displaced thresholds.
/// Region-generic: other areas register their own references here or pass none and fail closed.
pub fn displaced_threshold(region: &str, airport: &str, end: &str) -> Option<(f64, [f64; 2])> {
    match (region, airport, end) {
        ("eastern-pa", "1N7", "07") => Some((318., [40.96947391, -75.00141666])),
        ("eastern-pa", "1N7", "25") => Some((235., [40.97290411, -74.99332825])),
        _ => None,
    }
}

/// Builds the reference for `airport`'s first runway from region `airports.json`.
pub fn reference_from_region(
    region: &str,
    airports: &serde_json::Value,
    airport: &str,
) -> Result<RunwayReference, String> {
    let apt = airports
        .get("airports")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter()
                .find(|a| a.get("ident").and_then(|v| v.as_str()) == Some(airport))
        })
        .ok_or("airport missing from region data")?;
    let runway = apt
        .get("runways")
        .and_then(|v| v.as_array())
        .and_then(|r| r.first())
        .ok_or("airport has no runway")?;
    let width_m = runway
        .get("width_m")
        .and_then(|v| v.as_f64())
        .ok_or("runway width")?;
    let ends = runway
        .get("ends")
        .and_then(|v| v.as_array())
        .ok_or("runway ends")?;
    let mut parsed = Vec::new();
    for end in ends {
        let ident = end
            .get("ident")
            .and_then(|v| v.as_str())
            .ok_or("end ident")?
            .to_owned();
        let lat = end
            .get("latitude_deg")
            .and_then(|v| v.as_f64())
            .ok_or("end latitude")?;
        let lon = end
            .get("longitude_deg")
            .and_then(|v| v.as_f64())
            .ok_or("end longitude")?;
        let (feet, at) =
            displaced_threshold(region, airport, &ident).map_or((0., None), |(f, p)| (f, Some(p)));
        parsed.push(EndReference {
            ident,
            lat,
            lon,
            displaced_threshold_m: feet * FT_M,
            displaced_lat_lon: at,
        });
    }
    let [a, b]: [EndReference; 2] = parsed.try_into().map_err(|_| "runway needs exactly two ends")?;
    Ok(RunwayReference {
        airport: airport.into(),
        designator: runway
            .get("designator")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        width_m,
        ends: [a, b],
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct Centreline {
    pub dashes: usize,
    pub dash_length_m: f64,
    /// Painted centreline minus the FAA axis, positive to the right of end A toward end B, at midfield.
    pub cross_track_m: f64,
    pub heading_error_deg: f64,
    pub residual_rms_m: f64,
    pub cross_at_end_a_m: f64,
    pub cross_at_end_b_m: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct EndMeasurement {
    pub ident: String,
    pub bar_found: bool,
    /// Distances along the FAA axis from end A.
    pub bar_inboard_u_m: Option<f64>,
    pub bar_outboard_u_m: Option<f64>,
    /// Midline of the painted bar; the surveyed threshold point is judged against this.
    pub bar_centre_u_m: Option<f64>,
    pub bar_width_m: Option<f64>,
    pub bar_cross_centre_m: Option<f64>,
    pub bar_span_m: Option<f64>,
    pub expected_threshold_u_m: f64,
    /// Bar centre minus the FAA-expected threshold along the axis (positive toward end B). The
    /// FAA survey does not say which part of the bar its point marks, so the bar's midline is
    /// the unbiased comparison; both edges are reported too.
    pub along_error_m: Option<f64>,
    /// Painted centreline at this end's threshold minus the FAA axis.
    pub cross_error_m: Option<f64>,
    pub horizontal_error_m: Option<f64>,
    /// Same offset resolved to local north/east, metres.
    pub offset_north_east_m: Option<[f64; 2]>,
    /// Bar centre minus the surveyed FAA displaced-threshold position, when supplied.
    pub vs_surveyed_displaced_threshold_m: Option<[f64; 2]>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Audit {
    pub passed: bool,
    pub gate_m: f64,
    pub airport: String,
    pub runway: String,
    pub runway_length_m: f64,
    pub centreline: Option<Centreline>,
    pub ends: Vec<EndMeasurement>,
    /// Mean of the end offsets, north/east metres: a systematic registration shift when persistent.
    pub systematic_offset_north_east_m: Option<[f64; 2]>,
    pub method: &'static str,
    pub notes: Vec<String>,
}

pub(super) struct Frame {
    projection: LocalProjection,
    d: [f64; 2],
    r: [f64; 2],
    pub length: f64,
}
impl Frame {
    pub fn new(a: [f64; 2], b: [f64; 2]) -> Result<Self, String> {
        let projection = LocalProjection::new(a[0], a[1]).map_err(|e| e.to_string())?;
        let [n, e] = projection.project(b[0], b[1]).map_err(|e| e.to_string())?;
        let length = libm::hypot(n, e);
        if length < 100. {
            return Err("runway too short for alignment".into());
        }
        let d = [n / length, e / length];
        Ok(Self {
            projection,
            d,
            r: [-d[1], d[0]],
            length,
        })
    }
    fn lat_lon(&self, u: f64, v: f64) -> Result<[f64; 2], String> {
        self.projection
            .unproject(u * self.d[0] + v * self.r[0], u * self.d[1] + v * self.r[1])
            .map_err(|e| e.to_string())
    }
    pub fn uv(&self, lat: f64, lon: f64) -> Result<[f64; 2], String> {
        let [n, e] = self.projection.project(lat, lon).map_err(|e| e.to_string())?;
        Ok([n * self.d[0] + e * self.d[1], n * self.r[0] + e * self.r[1]])
    }
    fn north_east(&self, du: f64, dv: f64) -> [f64; 2] {
        [du * self.d[0] + dv * self.r[0], du * self.d[1] + dv * self.r[1]]
    }
}

/// Runway-aligned raster. Row 0 is the left-most cross-track position (`v` minimum).
pub struct Strip {
    pub u0: f64,
    pub v0: f64,
    pub cols: usize,
    pub rows: usize,
    pub rgb: Vec<[u8; 3]>,
    luma: Vec<f32>,
}
impl Strip {
    fn u(&self, c: usize) -> f64 {
        self.u0 + (c as f64 + 0.5) * STEP_M
    }
    fn v(&self, r: usize) -> f64 {
        self.v0 + (r as f64 + 0.5) * STEP_M
    }
    fn col(&self, u: f64) -> i64 {
        libm::floor((u - self.u0) / STEP_M) as i64
    }
    fn row(&self, v: f64) -> i64 {
        libm::floor((v - self.v0) / STEP_M) as i64
    }
    fn luma_at(&self, c: usize, r: usize) -> f32 {
        self.luma[r * self.cols + c]
    }
}
fn resample(mosaic: &Mosaic, frame: &Frame, margin_u: f64, half_v: f64) -> Result<Strip, String> {
    let u0 = -margin_u;
    let cols = libm::ceil((frame.length + 2. * margin_u) / STEP_M) as usize;
    let rows = libm::ceil(2. * half_v / STEP_M) as usize;
    let mut strip = Strip {
        u0,
        v0: -half_v,
        cols,
        rows,
        rgb: vec![[0; 3]; cols * rows],
        luma: vec![0.; cols * rows],
    };
    for r in 0..rows {
        for c in 0..cols {
            let [lat, lon] = frame.lat_lon(strip.u(c), strip.v(r))?;
            let p = mosaic
                .sample(lat, lon)
                .ok_or("alignment strip extends beyond the imagery mosaic")?;
            strip.rgb[r * cols + c] = [p[0] as u8, p[1] as u8, p[2] as u8];
            strip.luma[r * cols + c] = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        }
    }
    Ok(strip)
}

fn percentile(sorted: &[f32], q: f64) -> f32 {
    if sorted.is_empty() {
        return 0.;
    }
    let i = libm::round(q * (sorted.len() - 1) as f64) as usize;
    sorted[i.min(sorted.len() - 1)]
}
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    if values.is_empty() {
        return f64::NAN;
    }
    let m = values.len() / 2;
    if values.len() % 2 == 1 {
        values[m]
    } else {
        (values[m - 1] + values[m]) / 2.
    }
}

/// Luminance separating paint from pavement, from the strip's own statistics.
fn paint_threshold(strip: &Strip, length: f64, half_width: f64) -> f32 {
    let mut values = Vec::new();
    for r in 0..strip.rows {
        if strip.v(r).abs() > half_width {
            continue;
        }
        for c in 0..strip.cols {
            let u = strip.u(c);
            if (0. ..=length).contains(&u) {
                values.push(strip.luma_at(c, r));
            }
        }
    }
    values.sort_by(|a, b| a.total_cmp(b));
    (percentile(&values, 0.5) + percentile(&values, 0.997)) / 2.
}

fn centreline(strip: &Strip, frame: &Frame, threshold: f32) -> Result<Centreline, String> {
    let (lo, hi) = (0.2 * frame.length, 0.8 * frame.length);
    // Cross-track histogram of paint near the axis fixes the search window for the dashes.
    let bins = 40;
    let mut histogram = vec![0_u32; bins];
    for r in 0..strip.rows {
        let v = strip.v(r);
        if v.abs() >= 4. {
            continue;
        }
        let bin = ((v + 4.) / 8. * bins as f64) as usize;
        for c in 0..strip.cols {
            let u = strip.u(c);
            if u >= lo && u <= hi && strip.luma_at(c, r) > threshold {
                histogram[bin.min(bins - 1)] += 1;
            }
        }
    }
    let peak = histogram
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| **n)
        .map(|(i, _)| i)
        .ok_or("empty histogram")?;
    let v_guess = -4. + (peak as f64 + 0.5) * 8. / bins as f64;
    // Connected components of paint inside a narrow corridor around the guess.
    let r_lo = strip.row(v_guess - 1.2).max(0) as usize;
    let r_hi = (strip.row(v_guess + 1.2).max(0) as usize).min(strip.rows - 1);
    let c_lo = strip.col(lo).max(0) as usize;
    let c_hi = (strip.col(hi).max(0) as usize).min(strip.cols - 1);
    let (w, h) = (c_hi - c_lo + 1, r_hi - r_lo + 1);
    let mask: Vec<bool> = (0..w * h)
        .map(|i| strip.luma_at(c_lo + i % w, r_lo + i / w) > threshold)
        .collect();
    let mut seen = vec![false; w * h];
    let mut dashes = Vec::<(f64, f64, f64)>::new(); // u centroid, v centroid, length
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut n, mut su, mut sv) = (0_f64, 0_f64, 0_f64);
        let (mut cmin, mut cmax, mut rmin, mut rmax) = (usize::MAX, 0, usize::MAX, 0);
        while let Some(i) = stack.pop() {
            let (c, r) = (i % w, i / w);
            n += 1.;
            su += strip.u(c_lo + c);
            sv += strip.v(r_lo + r);
            (cmin, cmax, rmin, rmax) = (cmin.min(c), cmax.max(c), rmin.min(r), rmax.max(r));
            for (dc, dr) in [(-1_i64, 0_i64), (1, 0), (0, -1), (0, 1)] {
                let (nc, nr) = (c as i64 + dc, r as i64 + dr);
                if nc < 0 || nr < 0 || nc >= w as i64 || nr >= h as i64 {
                    continue;
                }
                let j = nr as usize * w + nc as usize;
                if mask[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let length = (cmax - cmin + 1) as f64 * STEP_M;
        let width = (rmax - rmin + 1) as f64 * STEP_M;
        // A centreline dash is long and thin; numerals, aircraft and stray pixels are not.
        // Dashes touching the corridor edge are rejected: a clipped component has a biased centroid.
        if length >= 8. && width <= 1.6 && rmin > 0 && rmax < h - 1 && cmin > 0 && cmax < w - 1 {
            dashes.push((su / n, sv / n, length));
        }
    }
    if dashes.len() < 6 {
        return Err(format!(
            "only {} centreline dashes found; need at least 6",
            dashes.len()
        ));
    }
    // Theil-Sen line v = a + b u.
    let mut slopes = Vec::new();
    for i in 0..dashes.len() {
        for j in i + 1..dashes.len() {
            let du = dashes[j].0 - dashes[i].0;
            if du.abs() > 20. {
                slopes.push((dashes[j].1 - dashes[i].1) / du);
            }
        }
    }
    let slope = median(&mut slopes);
    let mut intercepts: Vec<f64> = dashes.iter().map(|d| d.1 - slope * d.0).collect();
    let intercept = median(&mut intercepts);
    let rms = libm::sqrt(
        dashes
            .iter()
            .map(|d| (d.1 - (intercept + slope * d.0)).powi(2))
            .sum::<f64>()
            / dashes.len() as f64,
    );
    let mut lengths: Vec<f64> = dashes.iter().map(|d| d.2).collect();
    Ok(Centreline {
        dashes: dashes.len(),
        dash_length_m: median(&mut lengths),
        cross_track_m: intercept + slope * frame.length / 2.,
        heading_error_deg: libm::atan(slope).to_degrees(),
        residual_rms_m: rms,
        cross_at_end_a_m: intercept,
        cross_at_end_b_m: intercept + slope * frame.length,
    })
}

struct Bar {
    inboard: f64,
    outboard: f64,
    cross_centre: f64,
    span: f64,
}
/// Finds the full-width transverse bar within `[from, to]` along-track. `inward_is_positive`
/// is true when the runway interior lies toward larger `u`.
fn find_bar(
    strip: &Strip,
    threshold: f32,
    line: &Centreline,
    length: f64,
    width: f64,
    (from, to): (f64, f64),
    inward_is_positive: bool,
) -> Option<Bar> {
    let half = (width / 2. - 2.).max(2.);
    let (c_lo, c_hi) = (
        strip.col(from).max(0) as usize,
        (strip.col(to).max(0) as usize).min(strip.cols - 1),
    );
    let fraction = |c: usize| -> f64 {
        let u = strip.u(c);
        let centre = line.cross_at_end_a_m + (line.cross_at_end_b_m - line.cross_at_end_a_m) * u / length;
        let (mut paint, mut total) = (0_u32, 0_u32);
        for r in 0..strip.rows {
            if (strip.v(r) - centre).abs() <= half {
                total += 1;
                paint += u32::from(strip.luma_at(c, r) > threshold);
            }
        }
        f64::from(paint) / f64::from(total.max(1))
    };
    let profile: Vec<f64> = (c_lo..=c_hi).map(fraction).collect();
    let mut best: Option<(usize, usize, f64)> = None;
    let mut i = 0;
    while i < profile.len() {
        if profile[i] < 0.75 {
            i += 1;
            continue;
        }
        let start = i;
        while i < profile.len() && profile[i] >= 0.75 {
            i += 1;
        }
        let run = (i - start) as f64 * STEP_M;
        let mean = profile[start..i].iter().sum::<f64>() / (i - start) as f64;
        if (1. ..=8.).contains(&run) && best.is_none_or(|b| mean > b.2) {
            best = Some((start, i - 1, mean));
        }
    }
    let (first, last, _) = best?;
    // Edge positions: where the paint fraction crosses 0.5, interpolated between columns.
    let edge = |inner: usize, outer: Option<usize>| -> f64 {
        let (a, b) = (profile[inner], outer.map_or(0., |o| profile[o]));
        let t = if (a - b).abs() < 1e-9 {
            0.5
        } else {
            (a - 0.5) / (a - b)
        };
        let u_inner = strip.u(c_lo + inner);
        let u_outer = outer.map_or(u_inner + STEP_M, |o| strip.u(c_lo + o));
        u_inner + (u_outer - u_inner) * t
    };
    let left = edge(first, first.checked_sub(1));
    let right = edge(last, (last + 1 < profile.len()).then_some(last + 1));
    let (u_lo, u_hi) = (left, right);
    // Cross-track extent from the mean paint density across the bar's columns.
    let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for r in 0..strip.rows {
        let n = (first..=last)
            .filter(|c| strip.luma_at(c_lo + c, r) > threshold)
            .count();
        if n * 2 > last - first + 1 {
            vmin = vmin.min(strip.v(r) - STEP_M / 2.);
            vmax = vmax.max(strip.v(r) + STEP_M / 2.);
        }
    }
    // Restrict to the contiguous pavement span about the axis.
    if !vmin.is_finite() || vmax - vmin < width * 0.5 {
        return None;
    }
    let (inboard, outboard) = if inward_is_positive {
        (u_hi, u_lo)
    } else {
        (u_lo, u_hi)
    };
    Some(Bar {
        inboard,
        outboard,
        cross_centre: (vmin + vmax) / 2.,
        span: vmax - vmin,
    })
}

/// Measures the painted runway in `mosaic` against the FAA reference.
pub fn measure(mosaic: &Mosaic, reference: &RunwayReference) -> Result<(Audit, Strip), String> {
    let [a, b] = &reference.ends;
    let frame = Frame::new([a.lat, a.lon], [b.lat, b.lon])?;
    let strip = resample(mosaic, &frame, 60., 40.)?;
    let threshold = paint_threshold(&strip, frame.length, reference.width_m / 2.);
    let mut notes = Vec::new();
    let line = centreline(&strip, &frame, threshold);
    let mut audit = Audit {
        passed: false,
        gate_m: GATE_M,
        airport: reference.airport.clone(),
        runway: reference.designator.clone(),
        runway_length_m: frame.length,
        centreline: None,
        ends: Vec::new(),
        systematic_offset_north_east_m: None,
        method: "runway-frame resample; luminance-threshold paint mask; connected-component centreline dashes with Theil-Sen fit; full-width transverse threshold bars from the along-track paint fraction",
        notes: Vec::new(),
    };
    let line = match line {
        Ok(line) => line,
        Err(error) => {
            notes.push(error);
            audit.notes = notes;
            return Ok((audit, strip));
        }
    };
    let zone = (0.25 * frame.length).min(220.);
    let mut vectors = Vec::new();
    let mut all_found = true;
    for (index, end) in reference.ends.iter().enumerate() {
        let first = index == 0;
        let expected = if first {
            end.displaced_threshold_m
        } else {
            frame.length - end.displaced_threshold_m
        };
        let window = if first {
            (0., zone)
        } else {
            (frame.length - zone, frame.length)
        };
        let bar = find_bar(
            &strip,
            threshold,
            &line,
            frame.length,
            reference.width_m,
            window,
            first,
        );
        let mut m = EndMeasurement {
            ident: end.ident.clone(),
            bar_found: bar.is_some(),
            bar_inboard_u_m: None,
            bar_outboard_u_m: None,
            bar_centre_u_m: None,
            bar_width_m: None,
            bar_cross_centre_m: None,
            bar_span_m: None,
            expected_threshold_u_m: expected,
            along_error_m: None,
            cross_error_m: None,
            horizontal_error_m: None,
            offset_north_east_m: None,
            vs_surveyed_displaced_threshold_m: None,
        };
        if let Some(bar) = bar {
            let bar_centre = (bar.inboard + bar.outboard) / 2.;
            let along = bar_centre - expected;
            let centre = line.cross_at_end_a_m
                + (line.cross_at_end_b_m - line.cross_at_end_a_m) * bar_centre / frame.length;
            m.bar_centre_u_m = Some(bar_centre);
            m.bar_inboard_u_m = Some(bar.inboard);
            m.bar_outboard_u_m = Some(bar.outboard);
            m.bar_width_m = Some((bar.inboard - bar.outboard).abs());
            m.bar_cross_centre_m = Some(bar.cross_centre);
            m.bar_span_m = Some(bar.span);
            m.along_error_m = Some(along);
            m.cross_error_m = Some(centre);
            m.horizontal_error_m = Some(libm::hypot(along, centre));
            m.offset_north_east_m = Some(frame.north_east(along, centre));
            vectors.push(frame.north_east(along, centre));
            if let Some([lat, lon]) = end.displaced_lat_lon {
                let [u, v] = frame.uv(lat, lon)?;
                m.vs_surveyed_displaced_threshold_m = Some([bar_centre - u, centre - v]);
            }
        } else {
            all_found = false;
            notes.push(format!("runway {} threshold bar not found", end.ident));
        }
        audit.ends.push(m);
    }
    if !vectors.is_empty() {
        let n = vectors.len() as f64;
        audit.systematic_offset_north_east_m = Some([
            vectors.iter().map(|v| v[0]).sum::<f64>() / n,
            vectors.iter().map(|v| v[1]).sum::<f64>() / n,
        ]);
    }
    audit.passed = all_found
        && libm::fabs(line.cross_track_m) <= GATE_M
        && audit
            .ends
            .iter()
            .all(|e| e.horizontal_error_m.is_some_and(|h| h <= GATE_M));
    audit.centreline = Some(line);
    audit.notes = notes;
    Ok((audit, strip))
}

/// Writes the three overlay crops (end A, midfield, end B) stacked vertically.
/// Cyan: FAA axis and expected inboard threshold. Yellow: measured centreline.
/// Green: measured bar edges. Magenta: surveyed FAA displaced-threshold point.
pub fn overlay(
    strip: &Strip,
    audit: &Audit,
    reference: &RunwayReference,
) -> Result<image::RgbaImage, String> {
    let scale = 4_u32;
    let (span_u, span_v) = (130.0_f64, 40.0_f64);
    let line = audit.centreline.as_ref();
    let length = audit.runway_length_m;
    let panels = [
        (-20.0, true),
        (length / 2. - span_u / 2., false),
        (length - span_u + 20., true),
    ];
    let (pw, ph) = (
        (span_u * f64::from(scale)) as u32,
        (span_v * f64::from(scale)) as u32,
    );
    let mut out = image::RgbaImage::new(pw, ph * 3 + 4);
    for (k, (u_start, _)) in panels.iter().enumerate() {
        let y_off = k as u32 * (ph + 2);
        for py in 0..ph {
            for px in 0..pw {
                let u = u_start + f64::from(px) / f64::from(scale);
                let v = -span_v / 2. + f64::from(py) / f64::from(scale);
                let (c, r) = (strip.col(u), strip.row(v));
                let pixel = if c >= 0 && r >= 0 && (c as usize) < strip.cols && (r as usize) < strip.rows {
                    let p = strip.rgb[r as usize * strip.cols + c as usize];
                    image::Rgba([p[0], p[1], p[2], 255])
                } else {
                    image::Rgba([0, 0, 0, 255])
                };
                out.put_pixel(px, y_off + py, pixel);
            }
        }
        let mut vline = |u: f64, color: [u8; 4]| {
            let px = ((u - u_start) * f64::from(scale)) as i64;
            if (0..i64::from(pw)).contains(&px) {
                for py in 0..ph {
                    out.put_pixel(px as u32, y_off + py, image::Rgba(color));
                }
            }
        };
        for (index, end) in reference.ends.iter().enumerate() {
            let expected = if index == 0 {
                end.displaced_threshold_m
            } else {
                length - end.displaced_threshold_m
            };
            vline(expected, [0, 255, 255, 255]);
            if let Some(m) = audit.ends.get(index) {
                for u in [m.bar_inboard_u_m, m.bar_outboard_u_m].into_iter().flatten() {
                    vline(u, [0, 255, 0, 255]);
                }
            }
        }
        vline(0., [0, 160, 255, 255]);
        vline(length, [0, 160, 255, 255]);
        let mut hline = |v: f64, color: [u8; 4]| {
            let py = ((v + span_v / 2.) * f64::from(scale)) as i64;
            if (0..i64::from(ph)).contains(&py) {
                for px in 0..pw {
                    out.put_pixel(px, y_off + py as u32, image::Rgba(color));
                }
            }
        };
        hline(0., [0, 255, 255, 255]);
        if let Some(line) = line {
            let mid = u_start + span_u / 2.;
            hline(
                line.cross_at_end_a_m + (line.cross_at_end_b_m - line.cross_at_end_a_m) * mid / length,
                [255, 230, 0, 255],
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paints a synthetic runway whose true position is shifted from the FAA record by
    /// (north, east) metres, using geodesy independent of the detector's frame code.
    fn synthetic(shift_ne: [f64; 2], reference: &RunwayReference) -> Mosaic {
        let [a, b] = &reference.ends;
        let projection = LocalProjection::new(a.lat, a.lon).unwrap();
        let [n, e] = projection.project(b.lat, b.lon).unwrap();
        let length = libm::hypot(n, e);
        let d = [n / length, e / length];
        let r = [-d[1], d[0]];
        let [cx, cy] = scenery::mercator((a.lat + b.lat) / 2., (a.lon + b.lon) / 2.).unwrap();
        let side = 4096_usize;
        let half = 0.3 * 2048. / libm::cos(a.lat.to_radians());
        let mut mosaic = Mosaic::new([cx - half, cy - half, cx + half, cy + half], side);
        let pixel = 2. * half / side as f64;
        let width = reference.width_m;
        // Metres per Mercator unit and the local north/east of the mosaic centre come from exact
        // projections; over 1.2 km the affine map differs from them by about 3 cm.
        let [clat, clon] = scenery::inverse_mercator(cx, cy).unwrap();
        let [cn, ce] = projection.project(clat, clon).unwrap();
        let [lat_n, lon_n] = scenery::inverse_mercator(cx, cy + 100.).unwrap();
        let [lat_e, lon_e] = scenery::inverse_mercator(cx + 100., cy).unwrap();
        let [n_n, e_n] = projection.project(lat_n, lon_n).unwrap();
        let [n_e, e_e] = projection.project(lat_e, lon_e).unwrap();
        let (dn_dy, de_dy, dn_dx, de_dx) = (
            (n_n - cn) / 100.,
            (e_n - ce) / 100.,
            (n_e - cn) / 100.,
            (e_e - ce) / 100.,
        );
        for py in 0..side {
            for px in 0..side {
                let (mx, my) = ((px as f64 + 0.5) * pixel - half, half - (py as f64 + 0.5) * pixel);
                let pn = cn + dn_dx * mx + dn_dy * my - shift_ne[0];
                let pe = ce + de_dx * mx + de_dy * my - shift_ne[1];
                let (u, v) = (pn * d[0] + pe * d[1], pn * r[0] + pe * r[1]);
                let mut color = [78_u8, 112, 52]; // grass
                if (0. ..=length).contains(&u) && v.abs() <= width / 2. {
                    color = [70, 70, 74]; // asphalt
                    let dash = u > 110. && u < length - 110. && v.abs() < 0.45 && (u % 40.) < 24.;
                    let bar = |from: f64| (u - from).abs() < 1.5 && v.abs() < width / 2. - 1.;
                    let keys = |from: f64, sign: f64| {
                        let t = (u - from) * sign;
                        (1.8..48.).contains(&t)
                            && (v.abs() > 2.)
                            && (v.abs() % 2.4) < 1.2
                            && v.abs() < width / 2. - 1.5
                    };
                    let [da, db] = [a.displaced_threshold_m, b.displaced_threshold_m];
                    if dash
                        || bar(da)
                        || bar(length - db)
                        || keys(da + 1.5, 1.)
                        || keys(length - db - 1.5, -1.)
                    {
                        color = [235, 235, 235];
                    }
                }
                mosaic.rgb[py * side + px] = color;
            }
        }
        mosaic
    }
    fn reference() -> RunwayReference {
        let airports: serde_json::Value =
            serde_json::from_str(include_str!("../../../assets/terrain/eastern-pa/airports.json")).unwrap();
        reference_from_region("eastern-pa", &airports, "1N7").unwrap()
    }

    #[test]
    fn recovers_a_known_registration_offset() {
        let reference = reference();
        for shift in [[0., 0.], [1.0, -0.5], [-0.6, 1.1]] {
            let mosaic = synthetic(shift, &reference);
            let (audit, _) = measure(&mosaic, &reference).unwrap();
            let [n, e] = audit.systematic_offset_north_east_m.unwrap();
            assert!(
                (n - shift[0]).abs() < 0.35 && (e - shift[1]).abs() < 0.35,
                "{shift:?} measured {n} {e}"
            );
            let line = audit.centreline.as_ref().unwrap();
            assert!(line.heading_error_deg.abs() < 0.05);
            assert!(audit.ends.iter().all(|m| m.bar_found));
            let h = libm::hypot(shift[0], shift[1]);
            assert_eq!(audit.passed, h < GATE_M - 0.4, "{shift:?} {audit:#?}");
        }
    }
    #[test]
    fn rejects_a_registration_shift_beyond_the_gate_and_reports_it() {
        let reference = reference();
        let mosaic = synthetic([2.5, 3.0], &reference);
        let (audit, _) = measure(&mosaic, &reference).unwrap();
        assert!(!audit.passed);
        let [n, e] = audit.systematic_offset_north_east_m.unwrap();
        assert!((n - 2.5).abs() < 0.4 && (e - 3.0).abs() < 0.4, "{n} {e}");
    }
    /// The measurement of the real fetched NAIP imagery (`--ignored real_1n7_measurement`
    /// regenerates it). The gate must be satisfied by the recorded numbers themselves.
    #[test]
    fn recorded_real_1n7_measurement_satisfies_the_gate() {
        let recorded: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/scenery/1n7-alignment-measurement.json"
        ))
        .unwrap();
        assert_eq!(recorded["airport"], "1N7");
        assert_eq!(recorded["gate_m"].as_f64().unwrap(), GATE_M);
        assert_eq!(recorded["passed"], true);
        let centre = recorded["centreline"]["cross_track_m"].as_f64().unwrap();
        assert!(centre.abs() <= GATE_M && recorded["centreline"]["dashes"].as_u64().unwrap() >= 6);
        let ends = recorded["ends"].as_array().unwrap();
        assert_eq!(ends.len(), 2);
        for end in ends {
            assert_eq!(end["bar_found"], true);
            let along = end["along_error_m"].as_f64().unwrap();
            let cross = end["cross_error_m"].as_f64().unwrap();
            let horizontal = end["horizontal_error_m"].as_f64().unwrap();
            assert!((libm::hypot(along, cross) - horizontal).abs() < 1e-9);
            assert!(horizontal <= GATE_M, "{end}");
            // The painted bar is a physical 10 ft (3.05 m) stripe, blurred by the 0.3 m imagery.
            let width = end["bar_width_m"].as_f64().unwrap();
            assert!((2.6..=3.8).contains(&width), "{width}");
            // And the surveyed FAA displaced-threshold point lies on the painted bar.
            let surveyed = end["vs_surveyed_displaced_threshold_m"].as_array().unwrap();
            assert!(surveyed[0].as_f64().unwrap().abs() < width / 2. + 0.5);
        }
    }
    #[test]
    fn no_paint_fails_closed() {
        let reference = reference();
        let mut mosaic = synthetic([0., 0.], &reference);
        mosaic.rgb.fill([78, 112, 52]);
        let (audit, _) = measure(&mosaic, &reference).unwrap();
        assert!(!audit.passed && audit.centreline.is_none());
    }
    #[test]
    fn overlay_has_three_panels() {
        let reference = reference();
        let mosaic = synthetic([0., 0.], &reference);
        let (audit, strip) = measure(&mosaic, &reference).unwrap();
        let image = overlay(&strip, &audit, &reference).unwrap();
        assert_eq!(image.width(), 520);
        assert!(image.height() > 480);
    }
    #[test]
    #[ignore = "reads the real fetched 1N7 imagery cache"]
    fn real_1n7_measurement() {
        let cache = std::env::var("FLIGHTCRAFT_1N7_CACHE").unwrap_or_else(|_| {
            format!(
                "{}/builds/codex-jobs/scenery/eastern-pa/1n7-test/cache",
                std::env::var("HOME").unwrap()
            )
        });
        let out = std::env::var("FLIGHTCRAFT_ALIGNMENT_OUT").ok();
        let (audit, strip, reference) =
            crate::scenery::alignment_for_cache(std::path::Path::new(&cache)).unwrap();
        println!("{}", serde_json::to_string_pretty(&audit).unwrap());
        if let Some(out) = out {
            overlay(&strip, &audit, &reference).unwrap().save(out).unwrap();
        }
    }
}
