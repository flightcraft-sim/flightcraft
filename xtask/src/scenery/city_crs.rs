//! Explicit source CRS handling for offline CityGML. Unknown identifiers fail closed.
use super::city_mesh;
use glam::{DMat4, DVec3};
const SURVEY_FOOT: f64 = 1200. / 3937.;
#[derive(Clone, Copy, Debug)]
enum Horizontal {
    NyFeet,
    NyMeters,
    LatLon,
    LonLat,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Crs {
    horizontal: Horizontal,
    z_scale: f64,
}
fn epsg(text: &str) -> Option<u32> {
    let code = if let Some(s) = text.strip_prefix("EPSG:") {
        s
    } else if let Some(s) = text.strip_prefix("urn:ogc:def:crs:EPSG:") {
        s.split_once(':')?.1
    } else if let Some(s) = text.strip_prefix("http://www.opengis.net/def/crs/EPSG/0/") {
        s
    } else if let Some(s) = text.strip_prefix("https://www.opengis.net/def/crs/EPSG/0/") {
        s
    } else if let Some(s) = text.strip_prefix("http://www.opengis.net/gml/srs/epsg.xml#") {
        s
    } else {
        return None;
    };
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    code.parse().ok()
}
impl Crs {
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        if let Some(parts) = text.strip_prefix("urn:ogc:def:crs,") {
            let parts: Vec<_> = parts.split(',').collect();
            if parts.len() != 2 {
                return Err("Unsupported compound CityGML CRS".into());
            }
            let h = format!("urn:ogc:def:{}", parts.first().ok_or("Compound CRS")?);
            let z = format!("urn:ogc:def:{}", parts.get(1).ok_or("Compound CRS")?);
            let mut crs = Self::parse(&h)?;
            crs.z_scale = match epsg(&z) {
                Some(5703) => 1.,
                Some(6360) => SURVEY_FOOT,
                _ => return Err("Unknown CityGML vertical CRS; no datum is guessed".into()),
            };
            return Ok(crs);
        }
        let horizontal = match epsg(text) {
            Some(2263 | 6539) => Horizontal::NyFeet,
            Some(32118) => Horizontal::NyMeters,
            Some(4326 | 4979) => Horizontal::LatLon,
            _ => {
                if matches!(
                    text,
                    "OGC:CRS84"
                        | "urn:ogc:def:crs:OGC:1.3:CRS84"
                        | "http://www.opengis.net/def/crs/OGC/1.3/CRS84"
                ) {
                    Horizontal::LonLat
                } else {
                    return Err(format!("Unknown CityGML srsName: {text}"));
                }
            }
        };
        Ok(Self {
            horizontal,
            z_scale: if matches!(horizontal, Horizontal::NyFeet) {
                SURVEY_FOOT
            } else {
                1.
            },
        })
    }
    pub(super) fn local(
        self,
        mut p: [f64; 3],
        units: Option<&str>,
        axes: Option<&str>,
        inverse: DMat4,
    ) -> Result<DVec3, String> {
        if p.iter().any(|n| !n.is_finite()) {
            return Err("Nonfinite CityGML coordinate".into());
        }
        if let Some(axes) = axes {
            let axes: Vec<_> = axes.split_whitespace().collect();
            if axes.len() != 3 {
                return Err("CityGML requires three explicit axes".into());
            }
            if !matches!(
                axes.get(2).copied().unwrap_or("").to_ascii_lowercase().as_str(),
                "z" | "h" | "height" | "up" | "elevation" | "ellipsoidalheight" | "orthometricheight"
            ) {
                return Err("Unknown CityGML height axis".into());
            }
            let a = axes.first().copied().unwrap_or("").to_ascii_lowercase();
            let b = axes.get(1).copied().unwrap_or("").to_ascii_lowercase();
            let reverse = match self.horizontal {
                Horizontal::NyFeet | Horizontal::NyMeters => match (a.as_str(), b.as_str()) {
                    ("x" | "e" | "east" | "easting", "y" | "n" | "north" | "northing") => false,
                    ("n" | "north" | "northing", "e" | "east" | "easting") => true,
                    _ => return Err("Unknown projected CityGML axis order".into()),
                },
                Horizontal::LatLon => match (a.as_str(), b.as_str()) {
                    ("lat" | "latitude", "lon" | "long" | "longitude") => false,
                    ("lon" | "long" | "longitude", "lat" | "latitude") => true,
                    _ => return Err("Unknown geographic CityGML axis order".into()),
                },
                Horizontal::LonLat => match (a.as_str(), b.as_str()) {
                    ("lon" | "long" | "longitude", "lat" | "latitude") => false,
                    ("lat" | "latitude", "lon" | "long" | "longitude") => true,
                    _ => return Err("Unknown geographic CityGML axis order".into()),
                },
            };
            if reverse {
                p.swap(0, 1);
            }
        }
        let z_scale = if let Some(units) = units {
            let labels: Vec<_> = units.split_whitespace().collect();
            if labels.len() != 3 {
                return Err("CityGML requires three unit labels".into());
            }
            for label in labels.iter().take(2) {
                let valid = match self.horizontal {
                    Horizontal::NyFeet => {
                        matches!(*label, "ft" | "foot" | "ftUS" | "us-ft" | "US_survey_foot")
                    }
                    Horizontal::NyMeters => matches!(*label, "m" | "meter" | "metre"),
                    Horizontal::LatLon | Horizontal::LonLat => matches!(*label, "deg" | "degree" | "degrees"),
                };
                if !valid {
                    return Err("CityGML horizontal unit conflicts with srsName".into());
                }
            }
            match labels.get(2).copied() {
                Some("m" | "meter" | "metre") => 1.,
                Some("ftUS" | "us-ft" | "US_survey_foot") => SURVEY_FOOT,
                Some("ft" | "foot") => {
                    if matches!(self.horizontal, Horizontal::NyFeet) {
                        SURVEY_FOOT
                    } else {
                        0.3048
                    }
                }
                _ => return Err("Unknown CityGML vertical unit".into()),
            }
        } else {
            self.z_scale
        };
        let [x, y, z] = p;
        let (lat, lon) = match self.horizontal {
            Horizontal::NyFeet => inverse_lcc(x * SURVEY_FOOT, y * SURVEY_FOOT)?,
            Horizontal::NyMeters => inverse_lcc(x, y)?,
            Horizontal::LatLon => (x, y),
            Horizontal::LonLat => (y, x),
        };
        city_mesh::local(inverse, lat, lon, z * z_scale)
    }
}
/// GRS80 / NY Long Island Lambert conformal conic (2SP), original EPSG-parameter equations.
fn inverse_lcc(easting: f64, northing: f64) -> Result<(f64, f64), String> {
    let a = 6_378_137.;
    let flattening = 1. / 298.257_222_101;
    let eccentricity: f64 = f64::sqrt(flattening * (2. - flattening));
    let t = |phi: f64| {
        (std::f64::consts::FRAC_PI_4 - phi / 2.).tan()
            / ((1. - eccentricity * phi.sin()) / (1. + eccentricity * phi.sin())).powf(eccentricity / 2.)
    };
    let m = |phi: f64| phi.cos() / (1. - eccentricity.powi(2) * phi.sin().powi(2)).sqrt();
    let [p0, p1, p2] = [
        40.166_666_666_666_7_f64,
        41.033_333_333_333_3,
        40.666_666_666_666_7,
    ]
    .map(f64::to_radians);
    let n = (m(p1) / m(p2)).ln() / (t(p1) / t(p2)).ln();
    let factor = m(p1) / (n * t(p1).powf(n));
    let rho0 = a * factor * t(p0).powf(n);
    let dx = easting - 300_000.;
    let dy = rho0 - northing;
    let rho = dx.hypot(dy);
    let theta = dx.atan2(dy);
    let target = (rho / (a * factor)).powf(1. / n);
    let mut phi = std::f64::consts::FRAC_PI_2 - 2. * target.atan();
    for _ in 0..20 {
        let next = std::f64::consts::FRAC_PI_2
            - 2. * (target
                * ((1. - eccentricity * phi.sin()) / (1. + eccentricity * phi.sin()))
                    .powf(eccentricity / 2.))
            .atan();
        if (phi - next).abs() < 1e-13 {
            return Ok((next.to_degrees(), -74. + (theta / n).to_degrees()));
        }
        phi = next;
    }
    Err("CityGML CRS inverse projection did not converge".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_origin_and_crs_identifiers() {
        let (lat, lon) = inverse_lcc(300000., 0.).unwrap();
        assert!((lat - 40.1666666666667).abs() < 1e-9);
        assert!((lon + 74.).abs() < 1e-9);
        for s in [
            "EPSG:2263",
            "urn:ogc:def:crs:EPSG::2263",
            "http://www.opengis.net/def/crs/EPSG/0/32118",
            "urn:ogc:def:crs,crs:EPSG::2263,crs:EPSG::5703",
        ] {
            assert!(Crs::parse(s).is_ok());
        }
        for s in [
            "EPSG:9999",
            "EPSG:2263junk",
            "https://attacker.invalid/2263",
            "urn:ogc:def:crs,crs:EPSG::2263,crs:EPSG::9999",
        ] {
            assert!(Crs::parse(s).is_err());
        }
    }
}
