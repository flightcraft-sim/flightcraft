//! Bounded one-meter DSM/CHM blocks shared by canopy and roof reconstruction.
use super::{dem::Dems, lidar::Point};
use flightcraft_world::scenery;
#[derive(Clone, Copy, Debug)]
pub(super) struct Cell {
    pub top: f32,
    pub ground: f32,
    pub count: u16,
    /// 1 = unclassified, 2 = vegetation, 4 = building, 8 = ground, 16 = water, 32 = roof interpolation.
    pub flags: u8,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            top: f32::NEG_INFINITY,
            ground: f32::NAN,
            count: 0,
            flags: 0,
        }
    }
}
impl Cell {
    pub fn height(self) -> f32 {
        self.top - self.ground
    }
}
pub(super) struct Grid {
    pub bounds: [f64; 4],
    pub step: f64,
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
}
impl Grid {
    pub fn new(bounds: [f64; 4]) -> Result<Self, String> {
        let [w, s, e, n] = bounds;
        if bounds.iter().any(|v| !v.is_finite() || v.abs() > 21_000_000.) || w >= e || s >= n {
            return Err("Invalid DSM block bounds".into());
        }
        let [lat, _] = scenery::inverse_mercator((w + e) / 2., (s + n) / 2.).map_err(|e| e.to_string())?;
        let step = 1. / libm::cos(lat.to_radians());
        let width = libm::ceil((e - w) / step) as usize;
        let height = libm::ceil((n - s) / step) as usize;
        let count = width
            .checked_mul(height)
            .filter(|n| *n > 0 && *n <= 4_194_304)
            .ok_or("DSM block exceeds 4 million cells; build the area in bounded blocks")?;
        Ok(Self {
            bounds,
            step,
            width,
            height,
            cells: vec![Cell::default(); count],
        })
    }
    pub fn insert(&mut self, p: Point) -> Result<(), String> {
        if p.withheld {
            return Ok(());
        }
        let flags = match p.classification {
            1 => 1,
            2 => 8,
            9 => 16,
            3..=5 => 2,
            6 => 4,
            _ => return Ok(()),
        };
        let [x, y, z] = p.xyz;
        if p.xyz.iter().any(|v| !v.is_finite()) {
            return Err("Nonfinite DSM point".into());
        }
        let [w, s, e, n] = self.bounds;
        if x < w || x >= e || y < s || y >= n {
            return Ok(());
        }
        let col = libm::floor((x - w) / self.step) as usize;
        let row = libm::floor((y - s) / self.step) as usize;
        if col >= self.width || row >= self.height {
            return Ok(());
        }
        let c = self
            .cells
            .get_mut(row * self.width + col)
            .ok_or("DSM cell outside block")?;
        let was_building = c.flags & 4 != 0;
        c.flags |= flags;
        if flags & 24 != 0 {
            return Ok(());
        }
        if flags & 4 != 0 {
            c.top = if was_building {
                c.top.max(z as f32)
            } else {
                z as f32
            };
        } else if !was_building {
            c.top = c.top.max(z as f32);
        }
        c.count = c.count.saturating_add(1);
        c.flags |= flags;
        Ok(())
    }
    pub fn coordinate(&self, index: usize) -> Result<[f64; 2], String> {
        if index >= self.cells.len() {
            return Err("DSM coordinate outside block".into());
        }
        let [w, s, _, _] = self.bounds;
        scenery::inverse_mercator(
            w + (index % self.width) as f64 * self.step + self.step / 2.,
            s + (index / self.width) as f64 * self.step + self.step / 2.,
        )
        .map_err(|e| e.to_string())
    }
    pub fn normalize(&mut self, dem: &mut Dems) -> Result<(), String> {
        for index in 0..self.cells.len() {
            if self.cells.get(index).is_some_and(|c| c.count > 0) {
                let [lat, lon] = self.coordinate(index)?;
                let ground = dem.height(lat, lon)? as f32;
                self.cells.get_mut(index).ok_or("DSM terrain index")?.ground = ground;
            }
        }
        Ok(())
    }
    pub fn at(&self, x: i32, y: i32) -> Option<(usize, Cell)> {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        let index = y as usize * self.width + x as usize;
        self.cells.get(index).copied().map(|c| (index, c))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classified_roof_points_take_precedence_over_overhanging_vegetation() {
        let mut g = Grid::new([0., 0., 10., 10.]).unwrap();
        for (classification, z) in [(5, 130.), (6, 115.), (5, 140.), (1, 145.)] {
            g.insert(Point {
                xyz: [1., 1., z],
                classification,
                withheld: false,
            })
            .unwrap();
        }
        let cell = g.cells.iter().find(|c| c.count > 0).unwrap();
        assert_eq!(cell.top, 115.);
        assert!(cell.flags & 4 != 0);
    }
    #[test]
    fn hostile_blocks_and_noise_cannot_make_objects() {
        for b in [[0., 0., f64::NAN, 1.], [0., 0., 1e6, 1e6], [1., 0., 0., 1.]] {
            assert!(Grid::new(b).is_err());
        }
        let mut g = Grid::new([0., 0., 10., 10.]).unwrap();
        for class in [2, 7, 9, 17, 18, 20] {
            g.insert(Point {
                xyz: [1., 1., 100.],
                classification: class,
                withheld: false,
            })
            .unwrap();
        }
        g.insert(Point {
            xyz: [1., 1., 100.],
            classification: 5,
            withheld: true,
        })
        .unwrap();
        assert!(g.cells.iter().all(|c| c.count == 0));
        g.insert(Point {
            xyz: [1., 1., 15.],
            classification: 1,
            withheld: false,
        })
        .unwrap();
        assert_eq!(g.cells.iter().filter(|c| c.count > 0).count(), 1);
        assert!(
            g.insert(Point {
                xyz: [f64::NAN, 1., 0.],
                classification: 1,
                withheld: false
            })
            .is_err()
        );
    }
}
