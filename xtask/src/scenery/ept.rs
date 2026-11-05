//! Bounded selection of Entwine Point Tiles. Selection includes parent points: EPT is additive.
use serde::Deserialize;
use std::collections::BTreeMap;

pub const ROOT: &str = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NJ_NW6Co_2017";
pub const NYC_ROOT: &str = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NY_NewYorkCity";
pub const MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PAGE_NODES: usize = 100_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub depth: u8,
    pub x: u32,
    pub y: u32,
    pub z: u32,
}
impl Key {
    pub const ROOT: Self = Self {
        depth: 0,
        x: 0,
        y: 0,
        z: 0,
    };
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 40 {
            return Err("EPT node key exceeds limit".into());
        }
        let parts: Vec<_> = text.split('-').collect();
        let [d, x, y, z] = parts.as_slice() else {
            return Err("invalid EPT node key".into());
        };
        for part in &parts {
            if part.is_empty()
                || !part.bytes().all(|c| c.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return Err("noncanonical EPT node key".into());
            }
        }
        let depth: u8 = d.parse().map_err(|_| "invalid EPT depth")?;
        if depth > 24 {
            return Err("EPT depth exceeds limit".into());
        }
        let key = Self {
            depth,
            x: x.parse().map_err(|_| "EPT x")?,
            y: y.parse().map_err(|_| "EPT y")?,
            z: z.parse().map_err(|_| "EPT z")?,
        };
        let side = 1_u32 << depth;
        if key.x >= side || key.y >= side || key.z >= side {
            return Err("EPT node index outside depth".into());
        }
        Ok(key)
    }
    pub fn name(self) -> String {
        format!("{}-{}-{}-{}", self.depth, self.x, self.y, self.z)
    }
    fn descendant_of(self, parent: Self) -> bool {
        if self.depth < parent.depth {
            return false;
        }
        let shift = self.depth - parent.depth;
        self.x >> shift == parent.x && self.y >> shift == parent.y && self.z >> shift == parent.z
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub bounds: [f64; 6],
    pub bounds_conforming: [f64; 6],
    pub span: u32,
    pub points: u64,
    pub data_type: String,
    pub hierarchy_type: String,
    pub srs: Srs,
    pub schema: Vec<Dimension>,
}
#[derive(Debug, Deserialize)]
pub struct Srs {
    pub authority: String,
    pub horizontal: String,
    #[serde(default)]
    pub vertical: String,
    pub wkt: String,
}
#[derive(Debug, Deserialize)]
pub struct Dimension {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: u8,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub offset: Option<f64>,
}
struct Hierarchy(BTreeMap<String, i64>);
impl<'de> Deserialize<'de> for Hierarchy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Hierarchy;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a bounded, unique EPT node-count map")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut access: A) -> Result<Hierarchy, A::Error> {
                let mut result = BTreeMap::new();
                while let Some((key, count)) = access.next_entry::<String, i64>()? {
                    if result.len() >= MAX_PAGE_NODES || key.len() > 40 || result.insert(key, count).is_some()
                    {
                        return Err(serde::de::Error::custom(
                            "duplicate EPT key or page budget exceeded",
                        ));
                    }
                }
                Ok(Hierarchy(result))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
pub struct Page {
    pub data: Vec<(Key, u64)>,
    pub pages: Vec<Key>,
}
impl Metadata {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 1024 * 1024 {
            return Err("EPT metadata exceeds limit".into());
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        result.validate()?;
        Ok(result)
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        let [w, s, lo, e, n, hi] = self.bounds;
        let side = e - w;
        if self
            .bounds
            .iter()
            .chain(&self.bounds_conforming)
            .any(|v| !v.is_finite() || v.abs() > 50_000_000.)
            || side <= 0.
            || side > 40_100_000.
            || ((n - s) - side).abs() > side * 1e-8
            || ((hi - lo) - side).abs() > side * 1e-8
            || self.data_type != "laszip"
            || self.hierarchy_type != "json"
            || !self.span.is_power_of_two()
            || !(16..=2048).contains(&self.span)
            || self.points == 0
            || self.points > 1_000_000_000_000
            || self.srs.authority != "EPSG"
            || self.srs.horizontal != "3857"
            || self.srs.vertical.len() > 64
            || self.srs.wkt.len() > 65536
            || self.schema.len() > 128
        {
            return Err("unsupported or invalid EPT metadata".into());
        }
        for axis in 0..3 {
            let min = *self.bounds_conforming.get(axis).ok_or("EPT conforming minimum")?;
            let max = *self
                .bounds_conforming
                .get(axis + 3)
                .ok_or("EPT conforming maximum")?;
            if min > max
                || min < *self.bounds.get(axis).ok_or("EPT minimum")?
                || max > *self.bounds.get(axis + 3).ok_or("EPT maximum")?
            {
                return Err("EPT conforming bounds outside cube".into());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for d in &self.schema {
            if d.name.len() > 80
                || !seen.insert(d.name.as_str())
                || !matches!(d.kind.as_str(), "signed" | "unsigned" | "float")
                || !matches!(d.size, 1 | 2 | 4 | 8)
            {
                return Err("invalid EPT dimension".into());
            }
        }
        if ["X", "Y", "Z", "Classification"]
            .iter()
            .any(|name| !seen.contains(name))
        {
            return Err("EPT needs X, Y, Z, and Classification dimensions".into());
        }
        for name in ["X", "Y", "Z"] {
            let d = self
                .schema
                .iter()
                .find(|d| d.name == name)
                .ok_or("missing EPT coordinate")?;
            if d.scale.is_none_or(|v| !v.is_finite() || v <= 0.) || d.offset.is_none_or(|v| !v.is_finite()) {
                return Err("EPT coordinates need finite scale and offset, with positive scale".into());
            }
        }
        Ok(())
    }
    /// EPT node width / span bounds voxel spacing, rather than promising a uniform point count.
    pub fn target_depth(&self, query: [f64; 4], spacing_m: f64) -> Result<u8, String> {
        if !spacing_m.is_finite() || !(0.1..=10.).contains(&spacing_m) || query.iter().any(|v| !v.is_finite())
        {
            return Err("invalid EPT query or spacing".into());
        }
        let [w, s, e, n] = query;
        if w >= e || s >= n || !self.intersects(Key::ROOT, query) {
            return Err("query outside EPT coverage".into());
        }
        let [south, _] = flightcraft_world::scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
        let [north, _] = flightcraft_world::scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
        let equator_nearest = if south <= 0. && north >= 0. {
            0.
        } else {
            south.abs().min(north.abs())
        };
        let width = (self.bounds.get(3).ok_or("EPT east")? - self.bounds.first().ok_or("EPT west")?)
            * libm::cos(equator_nearest.to_radians())
            / f64::from(self.span);
        for depth in 0..=24_u8 {
            if width / f64::from(1_u32 << depth) <= spacing_m {
                return Ok(depth);
            }
        }
        Err("EPT spacing requires unsupported depth".into())
    }
    pub fn intersects(&self, key: Key, query: [f64; 4]) -> bool {
        if key.depth > 24 {
            return false;
        }
        let side = 1_u32 << key.depth;
        if key.x >= side || key.y >= side || key.z >= side {
            return false;
        }
        let [w, s, lo, e, _, _] = self.bounds;
        let width = (e - w) / f64::from(1_u32 << key.depth);
        let minx = w + f64::from(key.x) * width;
        let miny = s + f64::from(key.y) * width;
        let minz = lo + f64::from(key.z) * width;
        let [qw, qs, qe, qn] = query;
        let [cw, cs, cz, ce, cn, ch] = self.bounds_conforming;
        minx <= qe.min(ce)
            && minx + width >= qw.max(cw)
            && miny <= qn.min(cn)
            && miny + width >= qs.max(cs)
            && minz <= ch
            && minz + width >= cz
    }
    #[cfg(test)]
    pub fn select_page(&self, bytes: &[u8], root: Key, query: [f64; 4], depth: u8) -> Result<Page, String> {
        self.select_page_zone(bytes, root, query, depth, &|_| true)
    }
    /// Mercator rectangle of a node's cube footprint.
    pub fn node_rect(&self, key: Key) -> [f64; 4] {
        let [w, s, _, e, _, _] = self.bounds;
        let width = (e - w) / f64::from(1_u32 << key.depth);
        let x = w + f64::from(key.x) * width;
        let y = s + f64::from(key.y) * width;
        [x, y, x + width, y + width]
    }
    /// Like `select_page`, additionally requiring the node's footprint to satisfy `zone`.
    pub fn select_page_zone(
        &self,
        bytes: &[u8],
        root: Key,
        query: [f64; 4],
        depth: u8,
        zone: &dyn Fn([f64; 4]) -> bool,
    ) -> Result<Page, String> {
        if bytes.len() > MAX_PAGE_BYTES || depth > 24 {
            return Err("EPT hierarchy exceeds limit".into());
        }
        let Hierarchy(entries) = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if entries.len() > MAX_PAGE_NODES || !entries.contains_key(&root.name()) {
            return Err("invalid EPT hierarchy page root or budget".into());
        }
        let mut result = Page {
            data: Vec::new(),
            pages: Vec::new(),
        };
        for (name, count) in entries {
            let key = Key::parse(&name)?;
            if !key.descendant_of(root) || !(-1..=50_000_000).contains(&count) || (key == root && count == -1)
            {
                return Err("invalid EPT hierarchy descendant or count".into());
            }
            if key.depth > depth || !self.intersects(key, query) || !zone(self.node_rect(key)) {
                continue;
            }
            match count {
                -1 => result.pages.push(key),
                0 => {}
                _ => result.data.push((key, count as u64)),
            }
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> Metadata {
        let dims: Vec<_> = ["X", "Y", "Z", "Classification", "ReturnNumber", "NumberOfReturns"]
            .into_iter()
            .map(|name| serde_json::json!({"name":name,"type":"signed","size":4,"scale":0.001,"offset":0.}))
            .collect();
        Metadata::parse(&serde_json::to_vec(&serde_json::json!({"bounds":[0.,0.,0.,1024.,1024.,1024.],"boundsConforming":[0.,0.,100.,1024.,1024.,110.],"span":128,"points":30097024116_u64,"dataType":"laszip","hierarchyType":"json","srs":{"authority":"EPSG","horizontal":"3857","wkt":"Original synthetic EPT test fixture"},"schema":dims})).unwrap()).unwrap()
    }
    #[test]
    fn actual_new_jersey_schema_accepts_float_fields_and_requires_only_scaled_xyz_and_classification() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/nj-nw6co-2017-schema.json")).unwrap();
        let mut value = serde_json::json!({
            "bounds":[-8384951.,4915123.,-74944.,-8234569.,5065505.,75438.],
            "boundsConforming":[-8371954.,4915124.,-250.,-8247567.,5065503.,745.],
            "span":128,"points":30097024116_u64,"dataType":"laszip","hierarchyType":"json","version":"1.0.0",
            "srs":{"authority":"EPSG","horizontal":"3857","wkt":"Original test SRS label; schema and bounds supplied by flight lead"},
            "schema":schema,
        });
        let parse = |v: &serde_json::Value| Metadata::parse(&serde_json::to_vec(v).unwrap());
        let actual = parse(&value).unwrap();
        assert_eq!(actual.schema.len(), 15);
        assert_eq!(actual.schema[9].kind, "float");
        assert_eq!(actual.schema[12].size, 8);
        value["schema"]
            .as_array_mut()
            .unwrap()
            .retain(|d| ["X", "Y", "Z", "Classification"].contains(&d["name"].as_str().unwrap()));
        assert!(parse(&value).is_ok());
        for kind in ["signed", "unsigned", "float"] {
            for size in [1, 2, 4, 8] {
                let mut v = value.clone();
                v["schema"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({"name":"Ignored","type":kind,"size":size}));
                assert!(parse(&v).is_ok());
            }
        }
        for coordinate in 0..3 {
            for field in ["scale", "offset"] {
                let mut v = value.clone();
                v["schema"][coordinate].as_object_mut().unwrap().remove(field);
                assert!(parse(&v).is_err());
            }
        }
        let mut v = value.clone();
        v["schema"].as_array_mut().unwrap().pop();
        assert!(parse(&v).is_err());
        for (field, bad) in [
            ("type", serde_json::json!("float32")),
            ("size", serde_json::json!(3)),
            ("scale", serde_json::json!(0.)),
        ] {
            let mut v = value.clone();
            v["schema"][0][field] = bad;
            assert!(parse(&v).is_err());
        }
    }
    #[test]
    fn query_keeps_ancestor_points_and_follows_only_intersecting_pages() {
        let m = metadata();
        let query = [510., 510., 520., 520.];
        let p=m.select_page(br#"{"0-0-0-0":20,"1-0-0-0":3,"1-1-0-0":4,"1-0-1-0":5,"1-1-1-0":-1,"1-0-0-1":50,"3-7-7-0":7}"#,Key::ROOT,query,3).unwrap();
        assert_eq!(
            p.data.iter().map(|(k, _)| k.name()).collect::<Vec<_>>(),
            ["0-0-0-0", "1-0-0-0", "1-0-1-0", "1-1-0-0"]
        );
        assert_eq!(p.pages, [Key::parse("1-1-1-0").unwrap()]);
        assert_eq!(m.target_depth(query, 0.5).unwrap(), 4);
    }
    #[test]
    fn hostile_hierarchy_and_keys_fail_without_external_paths() {
        let m = metadata();
        for key in [
            "../ept-data",
            "25-0-0-0",
            "2-4-0-0",
            "01-0-0-0",
            "1-0-0--1",
            "1-0-0-0?token",
        ] {
            assert!(Key::parse(key).is_err());
        }
        for page in [
            r#"{"0-0-0-0":1,"0-0-0-0":2}"#,
            r#"{"0-0-0-0":-1}"#,
            r#"{"0-0-0-0":-2}"#,
            r#"{"0-0-0-0":1.5}"#,
            r#"{"0-0-0-0":50000001}"#,
            r#"{"1-1-0-0":1}"#,
        ] {
            assert!(
                m.select_page(page.as_bytes(), Key::ROOT, [1., 1., 2., 2.], 4)
                    .is_err()
            );
        }
        assert!(m.target_depth([1., 1., 2., 2.], f64::NAN).is_err());
        assert!(m.target_depth([2000., 2000., 3000., 3000.], 0.5).is_err());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataKind {
    Root,
    Hierarchy,
}
/// Metadata may be held in bounded memory; LAZ objects are streamed to the receipt cache.
pub trait Acquisition {
    fn metadata(&mut self, kind: MetadataKind, url: &str, limit: usize) -> Result<Vec<u8>, String>;
    fn points(&mut self, url: &str) -> Result<(), String>;
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SelectionSummary {
    pub depth: u8,
    pub node_count: usize,
    pub hierarchy_pages: usize,
    pub selected_points_before_clipping: u64,
}
pub fn fetch(
    root: &str,
    query: [f64; 4],
    spacing_m: f64,
    remote: &mut impl Acquisition,
) -> Result<SelectionSummary, String> {
    fetch_zone(
        root,
        query,
        &|_| true,
        spacing_m,
        ZoneBudget {
            points: 100_000_000,
            nodes: 100_000,
        },
        remote,
    )
}
/// Point and node ceilings for a selection. A corridor needs more than a one-airport test.
#[derive(Clone, Copy, Debug)]
pub struct ZoneBudget {
    pub points: u64,
    pub nodes: usize,
}
/// Selection restricted to an arbitrary footprint (`zone` receives node Mercator rectangles).
pub fn fetch_zone(
    root: &str,
    query: [f64; 4],
    zone: &dyn Fn([f64; 4]) -> bool,
    spacing_m: f64,
    budget: ZoneBudget,
    remote: &mut impl Acquisition,
) -> Result<SelectionSummary, String> {
    if !matches!(root, ROOT | NYC_ROOT) {
        return Err("EPT acquisition needs a reviewed USGS public survey root".into());
    }
    let bytes = remote.metadata(MetadataKind::Root, &format!("{root}/ept.json"), 1024 * 1024)?;
    let metadata = Metadata::parse(&bytes)?;
    let depth = metadata.target_depth(query, spacing_m)?;
    let mut pending = std::collections::BTreeSet::from([Key::ROOT]);
    let mut visited = std::collections::BTreeSet::new();
    let mut nodes = BTreeMap::<Key, u64>::new();
    let mut points = 0_u64;
    while let Some(key) = pending.pop_first() {
        if !visited.insert(key) {
            return Err("repeated EPT hierarchy page".into());
        }
        if visited.len() > 8192 {
            return Err("EPT hierarchy request budget exceeded; narrow the area".into());
        }
        let url = format!("{root}/ept-hierarchy/{}.json", key.name());
        let bytes = remote.metadata(MetadataKind::Hierarchy, &url, MAX_PAGE_BYTES)?;
        let page = metadata.select_page_zone(&bytes, key, query, depth, zone)?;
        for (key, count) in page.data {
            if let Some(previous) = nodes.insert(key, count) {
                if previous != count {
                    return Err("inconsistent repeated EPT point count".into());
                }
            } else {
                points = points.checked_add(count).ok_or("EPT point count overflow")?;
            }
            if nodes.len() > budget.nodes || points > budget.points {
                return Err("EPT point selection budget exceeded; narrow the area".into());
            }
        }
        pending.extend(page.pages);
        if pending.len() > 4096 {
            return Err("EPT hierarchy queue budget exceeded".into());
        }
    }
    if nodes.is_empty() {
        return Err("no EPT nodes intersect this area".into());
    }
    // Metadata planning completes before large objects are downloaded. Stable ordering makes
    // interrupted runs deterministic; cache receipts skip every completed object on retry.
    for key in nodes.keys() {
        remote.points(&format!("{root}/ept-data/{}.laz", key.name()))?;
    }
    Ok(SelectionSummary {
        depth,
        node_count: nodes.len(),
        hierarchy_pages: visited.len(),
        selected_points_before_clipping: points,
    })
}

#[cfg(test)]
mod acquisition_tests {
    use super::*;
    struct Recorded {
        survey_root: &'static str,
        root: Vec<u8>,
        pages: BTreeMap<String, Vec<u8>>,
        metadata: Vec<String>,
        data: Vec<String>,
    }
    impl Acquisition for Recorded {
        fn metadata(&mut self, _: MetadataKind, url: &str, limit: usize) -> Result<Vec<u8>, String> {
            self.metadata.push(url.into());
            let bytes = if url.ends_with("/ept.json") {
                self.root.clone()
            } else {
                self.pages.get(url).ok_or("unexpected hierarchy URL")?.clone()
            };
            if bytes.len() > limit {
                return Err("recorded metadata exceeds limit".into());
            }
            Ok(bytes)
        }
        fn points(&mut self, url: &str) -> Result<(), String> {
            self.data.push(url.into());
            Ok(())
        }
    }
    fn recorded() -> Recorded {
        let schema: Vec<_> = ["X", "Y", "Z", "Classification", "ReturnNumber", "NumberOfReturns"]
            .into_iter()
            .map(|name| serde_json::json!({"name":name,"type":"signed","size":4,"scale":0.001,"offset":0.}))
            .collect();
        let root=serde_json::to_vec(&serde_json::json!({"bounds":[0.,0.,0.,1024.,1024.,1024.],"boundsConforming":[0.,0.,100.,1024.,1024.,110.],"span":128,"points":30097024116_u64,"dataType":"laszip","hierarchyType":"json","srs":{"authority":"EPSG","horizontal":"3857","wkt":"Original synthetic EPT test fixture"},"schema":schema})).unwrap();
        let pages = BTreeMap::from([
            (
                format!("{ROOT}/ept-hierarchy/0-0-0-0.json"),
                br#"{"0-0-0-0":10,"1-0-0-0":-1,"1-1-0-0":-1,"1-0-1-0":-1,"1-1-1-0":-1,"1-0-0-1":-1}"#
                    .to_vec(),
            ),
            (
                format!("{ROOT}/ept-hierarchy/1-0-0-0.json"),
                br#"{"1-0-0-0":20,"2-1-1-0":30,"2-0-0-0":50}"#.to_vec(),
            ),
        ]);
        Recorded {
            survey_root: ROOT,
            root,
            pages,
            metadata: Vec::new(),
            data: Vec::new(),
        }
    }
    #[test]
    fn fetch_walks_only_intersecting_pages_and_keeps_parent_points() {
        let mut remote = recorded();
        let summary = fetch(ROOT, [300., 300., 310., 310.], 0.5, &mut remote).unwrap();
        assert_eq!(summary.depth, 4);
        assert_eq!(summary.node_count, 3);
        assert_eq!(summary.selected_points_before_clipping, 60);
        assert_eq!(remote.metadata.len(), 3);
        assert_eq!(
            remote.data,
            vec![
                format!("{ROOT}/ept-data/0-0-0-0.laz"),
                format!("{ROOT}/ept-data/1-0-0-0.laz"),
                format!("{ROOT}/ept-data/2-1-1-0.laz")
            ]
        );
    }
    #[test]
    fn nyc_survey_uses_the_same_bounded_selector_and_never_falls_back_to_new_jersey() {
        let mut remote = recorded();
        remote.survey_root = NYC_ROOT;
        remote.pages = remote
            .pages
            .into_iter()
            .map(|(url, bytes)| (url.replace(ROOT, NYC_ROOT), bytes))
            .collect();
        let mut metadata: serde_json::Value = serde_json::from_slice(&remote.root).unwrap();
        metadata["points"] = serde_json::json!(4_760_000_000_u64);
        remote.root = serde_json::to_vec(&metadata).unwrap();
        let summary = fetch(NYC_ROOT, [300., 300., 310., 310.], 0.5, &mut remote).unwrap();
        assert_eq!(summary.selected_points_before_clipping, 60);
        assert!(
            remote
                .metadata
                .iter()
                .chain(&remote.data)
                .all(|url| url.starts_with(NYC_ROOT))
        );
        let mut rejected = recorded();
        assert!(
            fetch(
                "https://example.invalid/ept",
                [300., 300., 310., 310.],
                0.5,
                &mut rejected
            )
            .is_err()
        );
        assert!(rejected.metadata.is_empty());
        assert!(rejected.data.is_empty());
    }
    #[test]
    fn malformed_split_page_stops_before_any_large_download() {
        let mut remote = recorded();
        remote.pages.insert(
            format!("{ROOT}/ept-hierarchy/1-0-0-0.json"),
            br#"{"1-0-0-0":-1}"#.to_vec(),
        );
        assert!(fetch(ROOT, [300., 300., 310., 310.], 0.5, &mut remote).is_err());
        assert!(remote.data.is_empty());
    }
}
