//! TUM acquisition is explicitly local-only. XML and ZIP bodies are streamed, never city-sized DOMs.
use super::{Operation, Request, city, city_crs::Crs, city_mesh, download, transport};
use crate::download::Source;
use glam::DMat4;
use quick_xml::{Reader, Writer, events::Event};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};
const URL: &str = "http://www.3dcitydb.net/3dcitydb/fileadmin/public/datasets/NYC/NYC_buildings_CityGML_LoD2/NYC_Buildings_LoD2_CityGML.zip";
const CREDIT: &str = "New York City LoD2 buildings: Chair of Geoinformatics, Technical University of Munich, derived from NYC Open Data. Dataset license unconfirmed; local user-fetched use only. Original FlightCraft procedural materials. For simulation only.";
const EXPANDED_LIMIT: u64 = 128_000_000_000;
const FRAGMENT_LIMIT: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CitySources {
    pub source: Source,
    pub documentation_url: String,
    pub licence: String,
    pub local_only: bool,
    pub redistribution_allowed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CityReport {
    pub region: String,
    pub area: String,
    pub manifest: std::path::PathBuf,
    pub buildings: usize,
    pub triangles: u64,
    pub tiles: usize,
    pub package_bytes: u64,
    pub source_crs: Vec<String>,
    pub local_only: bool,
    pub redistribution_allowed: bool,
    pub preview_only: bool,
    pub vertical_datum_verified: bool,
    pub runway_alignment_verified: bool,
    pub release_ready: bool,
}
/// A token-size budget bounds quick-xml allocation *before* a malicious token is buffered.
struct TokenBudget<R> {
    reader: R,
    left: usize,
}
impl<R: BufRead> Read for TokenBudget<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = {
            let input = self.fill_buf()?;
            let n = input.len().min(out.len());
            out.get_mut(..n)
                .ok_or_else(|| io::Error::other("Read bounds"))?
                .copy_from_slice(input.get(..n).ok_or_else(|| io::Error::other("Input bounds"))?);
            n
        };
        self.consume(n);
        Ok(n)
    }
}
impl<R: BufRead> BufRead for TokenBudget<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.left == 0 {
            return Err(io::Error::other("CityGML XML token exceeds 2 MiB"));
        }
        let input = self.reader.fill_buf()?;
        Ok(input.get(..input.len().min(self.left)).unwrap_or(&[]))
    }
    fn consume(&mut self, n: usize) {
        self.left = self.left.saturating_sub(n);
        self.reader.consume(n);
    }
}
#[derive(Default, Clone)]
struct Defaults {
    srs: Option<String>,
    dimension: Option<String>,
    units: Option<String>,
    axes: Option<String>,
}
fn xml_attr(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.ancestors()
        .find_map(|n| n.attribute(name).map(str::to_owned))
}
fn gml(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == name
        && matches!(
            node.tag_name().namespace(),
            Some("http://www.opengis.net/gml" | "http://www.opengis.net/gml/3.2")
        )
}
fn building_ns(node: Node<'_, '_>) -> bool {
    matches!(
        node.tag_name().namespace(),
        Some("http://www.opengis.net/citygml/building/1.0" | "http://www.opengis.net/citygml/building/2.0")
    )
}
fn lod2(node: Node<'_, '_>) -> bool {
    node.ancestors()
        .any(|n| n.is_element() && building_ns(n) && n.tag_name().name().starts_with("lod2"))
}
fn coordinates(
    node: Node<'_, '_>,
    defaults: &Defaults,
    inverse: DMat4,
    observed: &mut BTreeSet<String>,
) -> Result<Vec<glam::DVec3>, String> {
    let srs = xml_attr(node, "srsName")
        .or_else(|| defaults.srs.clone())
        .ok_or("CityGML coordinates have no srsName")?;
    let crs = Crs::parse(&srs)?;
    observed.insert(srs);
    let dimension = xml_attr(node, "srsDimension").or_else(|| defaults.dimension.clone());
    if dimension.as_deref().is_some_and(|s| s != "3") {
        return Err("CityGML LoD2 requires 3D coordinates".into());
    }
    let units = xml_attr(node, "uomLabels").or_else(|| defaults.units.clone());
    let axes = xml_attr(node, "axisLabels").or_else(|| defaults.axes.clone());
    let text = node.text().ok_or("CityGML coordinates are empty")?;
    let comma_form = gml(node, "coordinates");
    let numbers = text
        .split(|c: char| c.is_whitespace() || (comma_form && c == ','))
        .filter(|s| !s.is_empty());
    let mut values = Vec::new();
    for number in numbers {
        if values.len() >= 4096 * 3 {
            return Err("CityGML ring exceeds vertex budget".into());
        }
        values.push(number.parse::<f64>().map_err(|_| "Invalid CityGML number")?);
    }
    if values.is_empty() || values.len() % 3 != 0 || (gml(node, "pos") && values.len() != 3) {
        return Err("CityGML needs complete 3D position triples".into());
    }
    values
        .chunks_exact(3)
        .map(|p| crs.local([p[0], p[1], p[2]], units.as_deref(), axes.as_deref(), inverse))
        .collect()
}
fn parse_building(
    bytes: &[u8],
    defaults: &Defaults,
    inverse: DMat4,
    observed: &mut BTreeSet<String>,
) -> Result<Vec<city::Triangle>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "CityGML importer requires UTF-8")?;
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 200_000,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Invalid bounded CityGML building: {e}"))?;
    let building = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Building" && building_ns(*n))
        .ok_or("CityGML Building namespace is unsupported")?;
    let mut ids = BTreeSet::new();
    for n in building.descendants() {
        for attr in n.attributes() {
            if attr.name() == "id"
                && matches!(
                    attr.namespace(),
                    Some("http://www.opengis.net/gml" | "http://www.opengis.net/gml/3.2")
                )
                && !ids.insert(attr.value())
            {
                return Err("Duplicate CityGML geometry ID".into());
            }
        }
    }
    for n in building.descendants().filter(|n| lod2(*n)) {
        if let Some(href) = n.attribute(("http://www.w3.org/1999/xlink", "href"))
            && href.strip_prefix('#').is_none_or(|id| !ids.contains(id))
        {
            return Err(
                "CityGML has external or unresolved LoD2 geometry links; split input cannot omit surfaces"
                    .into(),
            );
        }
    }
    let mut triangles = Vec::new();
    for polygon in building.descendants().filter(|n| gml(*n, "Polygon") && lod2(*n)) {
        let mut rings = Vec::new();
        for boundary in polygon
            .children()
            .filter(|n| gml(*n, "exterior") || gml(*n, "interior"))
        {
            if gml(boundary, "exterior") && !rings.is_empty() {
                return Err("CityGML exterior must precede interior rings".into());
            }
            let linear = boundary
                .descendants()
                .find(|n| gml(*n, "LinearRing"))
                .ok_or("CityGML needs inline linear rings")?;
            let mut ring = Vec::new();
            for positions in linear
                .children()
                .filter(|n| gml(*n, "posList") || gml(*n, "pos") || gml(*n, "coordinates"))
            {
                ring.extend(coordinates(positions, defaults, inverse, observed)?);
                if ring.len() > 4096 {
                    return Err("CityGML ring exceeds vertex budget".into());
                }
            }
            if ring
                .first()
                .zip(ring.last())
                .is_some_and(|(a, b)| (*a - *b).length() < 0.001)
                && let Some(first) = ring.first().copied()
                && let Some(last) = ring.last_mut()
            {
                *last = first;
            }
            rings.push(ring);
        }
        triangles.extend(city_mesh::triangulate(&rings)?);
        if triangles.len() > 200_000 {
            return Err("CityGML building exceeds triangle budget".into());
        }
    }
    if triangles.is_empty() {
        return Err("CityGML Building has no supported inline LoD2 polygon surfaces".into());
    }
    Ok(triangles)
}
fn stream_xml(
    input: impl BufRead,
    mut consume: impl FnMut(&[u8], &Defaults) -> Result<(), String>,
) -> Result<(), String> {
    let mut reader = Reader::from_reader(TokenBudget {
        reader: input,
        left: 2 * 1024 * 1024,
    });
    let mut buf = Vec::new();
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut capture: Option<Writer<Vec<u8>>> = None;
    let mut building_depth = 0;
    let mut defaults = Defaults::default();
    let mut expanded = 0_u64;
    let mut buildings = 0;
    loop {
        reader.get_mut().left = 2 * 1024 * 1024;
        buf.clear();
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("CityGML XML stream: {e}"))?;
        expanded = expanded.saturating_add(event.as_ref().len() as u64);
        if expanded > EXPANDED_LIMIT {
            return Err("CityGML member exceeds expanded byte limit".into());
        }
        if let Event::Start(start) | Event::Empty(start) = &event {
            for attr in start.attributes() {
                let attr = attr.map_err(|e| e.to_string())?;
                if attr.key.as_ref() == b"srsName" {
                    Crs::parse(
                        &attr
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|e| e.to_string())?,
                    )?;
                }
            }
            if capture.is_none() && matches!(start.local_name().as_ref(), b"CityModel" | b"Envelope") {
                for attr in start.attributes() {
                    let attr = attr.map_err(|e| e.to_string())?;
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|e| e.to_string())?
                        .into_owned();
                    match attr.key.as_ref() {
                        b"srsName" => {
                            Crs::parse(&value)?;
                            defaults.srs = Some(value);
                        }
                        b"srsDimension" => defaults.dimension = Some(value),
                        b"uomLabels" => defaults.units = Some(value),
                        b"axisLabels" => defaults.axes = Some(value),
                        _ => (),
                    }
                }
            }
        }
        match &event {
            Event::DocType(_) => return Err("CityGML DTDs are not permitted".into()),
            Event::Decl(decl) => {
                if decl
                    .encoding()
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .is_some_and(|e| !e.eq_ignore_ascii_case(b"UTF-8"))
                {
                    return Err("CityGML importer requires UTF-8".into());
                }
            }
            Event::Start(start) => {
                if stack.len() >= 64 {
                    return Err("CityGML XML exceeds depth limit".into());
                }

                if capture.is_none() && start.local_name().as_ref() == b"Building" {
                    let mut writer = Writer::new(Vec::new());
                    for (raw, _) in &stack {
                        writer.get_mut().extend_from_slice(b"<");
                        writer.get_mut().extend_from_slice(raw);
                        writer.get_mut().extend_from_slice(b">");
                    }
                    capture = Some(writer);
                    building_depth = stack.len() + 1;
                }
                stack.push((start.to_vec(), start.name().as_ref().to_vec()));
            }
            Event::End(_) => {
                if stack.is_empty() {
                    return Err("Unbalanced CityGML XML".into());
                }
            }
            Event::Eof => {
                if capture.is_some() || !stack.is_empty() {
                    return Err("Truncated CityGML XML".into());
                }
                break;
            }
            _ => (),
        }
        if let Some(writer) = &mut capture {
            writer
                .write_event(event.clone().into_owned())
                .map_err(|e| e.to_string())?;
            if writer.get_ref().len() > FRAGMENT_LIMIT {
                return Err("CityGML building exceeds 16 MiB fragment budget".into());
            }
        }
        if matches!(event, Event::End(_)) {
            if stack.len() == building_depth
                && let Some(mut writer) = capture.take()
            {
                for (_, name) in stack.iter().take(stack.len().saturating_sub(1)).rev() {
                    writer.get_mut().extend_from_slice(b"</");
                    writer.get_mut().extend_from_slice(name);
                    writer.get_mut().extend_from_slice(b">");
                }
                consume(writer.get_ref(), &defaults)?;
                buildings += 1;
            }
            stack.pop();
        }
    }
    if buildings == 0 {
        return Err("CityGML member has no buildings".into());
    }
    Ok(())
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or("ZIP header truncated")?
            .try_into()
            .map_err(|_| "ZIP header")?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("ZIP header truncated")?
            .try_into()
            .map_err(|_| "ZIP header")?,
    ))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or("ZIP header truncated")?
            .try_into()
            .map_err(|_| "ZIP header")?,
    ))
}
/// Bound central-directory allocation, including ZIP64, before handing it to zip::ZipArchive.
fn zip_preflight(file: &mut File) -> Result<(), String> {
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let count = size.min(65557) as usize;
    file.seek(SeekFrom::End(-(count as i64)))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0; count];
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let offset = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|i| {
            tail.get(*i..*i + 4) == Some(b"PK\x05\x06")
                && u16_at(&tail, *i + 20).is_ok_and(|n| *i + 22 + usize::from(n) == tail.len())
        })
        .ok_or("ZIP end-of-directory missing")?;
    if u16_at(&tail, offset + 4)? != 0 || u16_at(&tail, offset + 6)? != 0 {
        return Err("Multidisk city ZIP is unsupported".into());
    }
    let mut entries = u64::from(u16_at(&tail, offset + 10)?);
    let mut bytes = u64::from(u32_at(&tail, offset + 12)?);
    let mut start = u64::from(u32_at(&tail, offset + 16)?);
    if entries == 65535 || bytes == u64::from(u32::MAX) || start == u64::from(u32::MAX) {
        let locator = tail
            .get(offset.checked_sub(20).ok_or("ZIP64 locator missing")?..offset)
            .ok_or("ZIP64 locator missing")?;
        if locator.get(..4) != Some(b"PK\x06\x07") || u32_at(locator, 4)? != 0 || u32_at(locator, 16)? != 1 {
            return Err("Unsupported ZIP64 locator".into());
        }
        let pointer = u64_at(locator, 8)?;
        if pointer.checked_add(56).is_none_or(|n| n > size) {
            return Err("ZIP64 directory pointer is outside archive".into());
        }
        file.seek(SeekFrom::Start(pointer)).map_err(|e| e.to_string())?;
        let mut header = [0; 56];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if header.get(..4) != Some(b"PK\x06\x06")
            || u64_at(&header, 4)? < 44
            || u32_at(&header, 16)? != 0
            || u32_at(&header, 20)? != 0
            || u64_at(&header, 24)? != u64_at(&header, 32)?
        {
            return Err("Unsupported ZIP64 directory".into());
        }
        entries = u64_at(&header, 32)?;
        bytes = u64_at(&header, 40)?;
        start = u64_at(&header, 48)?;
    }
    if entries == 0
        || entries > 100_000
        || bytes > 32 * 1024 * 1024
        || start.checked_add(bytes).is_none_or(|n| n > size)
    {
        return Err("City ZIP central directory exceeds allocation budget".into());
    }
    file.rewind().map_err(|e| e.to_string())?;
    Ok(())
}
struct Offline;
impl transport::Remote for Offline {
    fn max_object(&self) -> u64 {
        transport::MAX_CITY_ARCHIVE
    }
    fn allows(&self, url: &str) -> bool {
        url == URL
    }
    fn open(&mut self, _: &str, _: u64, _: Option<&str>) -> Result<transport::Response, String> {
        Err("Complete the TUM fetch before the offline build".into())
    }
}
fn build(request: &Request, out: &Path) -> Result<CityReport, String> {
    let sources: CitySources = download::json(&request.cache.join("citygml-sources.json"))?;
    if sources.source.url != URL || !sources.local_only || sources.redistribution_allowed {
        return Err("TUM source receipt or local-only policy is invalid".into());
    }
    let (path, receipt) = transport::fetch(
        &mut Offline,
        URL,
        &request.cache.join("objects"),
        transport::MAX_CITY_ARCHIVE,
    )?;
    if receipt.sha256 != sources.source.sha256 || receipt.bytes != sources.source.bytes {
        return Err("TUM archive differs from reviewed acquisition receipt".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    zip_preflight(&mut file)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut expanded = 0_u64;
    for i in 0..archive.len() {
        let member = archive.by_index(i).map_err(|e| e.to_string())?;
        expanded = expanded
            .checked_add(member.size())
            .ok_or("City ZIP size overflow")?;
        if expanded > EXPANDED_LIMIT {
            return Err("City ZIP exceeds expanded byte budget".into());
        }
    }
    let (_, inverse) = city_mesh::frame()?;
    let mut writer = city_mesh::Writer::new(out, 8_000_000_000)?;
    let mut observed = BTreeSet::new();
    let mut buildings = 0;
    let mut triangles = 0;
    for i in 0..archive.len() {
        let member = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = member.name().map_err(|e| e.to_string())?.to_ascii_lowercase();
        if member.is_dir() || !name.ends_with(".gml") && !name.ends_with(".xml") {
            continue;
        }
        stream_xml(
            BufReader::new(member.take(EXPANDED_LIMIT + 1)),
            |bytes, defaults| {
                let mesh = parse_building(bytes, defaults, inverse, &mut observed)?;
                triangles += mesh.len() as u64;
                writer.push(&mesh)?;
                buildings += 1;
                Ok(())
            },
        )?;
    }
    let extras = json!({"flightcraft":{"region":"new-york","area":"nyc-lod2","localOnly":true,"redistributionAllowed":false,"previewOnly":true,"verticalDatumVerified":false,"runwayAlignmentVerified":false,"lidarBuilt":false,"licence":"Unconfirmed; no explicit license on TUM project page"}});
    let (_, bytes, tiles) = writer.finish(CREDIT, extras)?;
    download::write_json(&out.join("citygml-sources.json"), &sources)?;
    let report = CityReport {
        region: "new-york".into(),
        area: "nyc-lod2".into(),
        manifest: request.out.join("tileset.json"),
        buildings,
        triangles,
        tiles,
        package_bytes: bytes,
        source_crs: observed.into_iter().collect(),
        local_only: true,
        redistribution_allowed: false,
        preview_only: true,
        vertical_datum_verified: false,
        runway_alignment_verified: false,
        release_ready: false,
    };
    download::write_json(&out.join("build-report.json"), &report)?;
    Ok(report)
}
pub(super) fn run(request: &Request) -> Result<(), String> {
    if request.sample.is_some() {
        return Err("CityGML buildings do not accept reference imagery".into());
    }
    match request.operation {
        Operation::Fetch => {
            if !request.allow_http {
                return Err("TUM serves this archive over plain HTTP. Explicitly pass --allow-http for the exact documented NYC prefix; output remains local-only".into());
            }
            let (_, source) = transport::fetch(
                &mut transport::Http::for_tum(true),
                URL,
                &request.cache.join("objects"),
                transport::MAX_CITY_ARCHIVE,
            )?;
            let sources = CitySources {
                source,
                documentation_url: "https://www.asg.ed.tum.de/gis/projekte/new-york-city-3d/".into(),
                licence: "No explicit license on project page; redistribution unconfirmed".into(),
                local_only: true,
                redistribution_allowed: false,
            };
            download::write_json(&request.cache.join("citygml-sources.json"), &sources)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&sources).map_err(|e| e.to_string())?
            );
        }
        Operation::Build => {
            if request.out.exists() {
                return Err("CityGML output must be a new directory".into());
            }
            let staging = request.out.with_extension("building");
            if let Some(parent) = staging.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
            match build(request, &staging) {
                Ok(report) => {
                    std::fs::rename(&staging, &request.out).map_err(|e| e.to_string())?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(staging);
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flightcraft_world::tiles3d;
    fn xml(srs: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><core:CityModel xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:xlink="http://www.w3.org/1999/xlink"><gml:boundedBy><gml:Envelope srsName="{srs}" srsDimension="3"/></gml:boundedBy><core:cityObjectMember><bldg:Building gml:id="test"><bldg:boundedBy><bldg:RoofSurface><bldg:lod2MultiSurface><gml:MultiSurface><gml:surfaceMember><gml:Polygon gml:id="roof"><gml:exterior><gml:LinearRing><gml:posList>40.7500 -74.0000 10 40.7500 -73.9999 10 40.7501 -73.99995 16 40.7500 -74.0000 10</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod2MultiSurface></bldg:RoofSurface></bldg:boundedBy></bldg:Building></core:cityObjectMember></core:CityModel>"#
        )
    }
    #[test]
    fn streaming_fragments_inherit_namespaces_and_envelope_crs_preserve_sloped_roof() {
        let input = xml("EPSG:4979");
        let (_, inverse) = city_mesh::frame().unwrap();
        let mut observed = BTreeSet::new();
        let mut count = 0;
        stream_xml(
            BufReader::with_capacity(7, io::Cursor::new(input)),
            |bytes, defaults| {
                let triangles = parse_building(bytes, defaults, inverse, &mut observed)?;
                let matrix = DMat4::from_cols_array(&city_mesh::frame()?.0)
                    * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
                let heights: Vec<_> = triangles
                    .iter()
                    .flatten()
                    .map(|p| tiles3d::geodetic(matrix.transform_point3(*p).to_array()).unwrap()[2])
                    .collect();
                assert!(heights.iter().any(|h| (*h - 16.).abs() < 0.001));
                assert!(heights.iter().any(|h| (*h - 10.).abs() < 0.001));
                count += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(observed.into_iter().collect::<Vec<_>>(), ["EPSG:4979"]);
    }
    #[test]
    fn xml_limits_unknown_crs_external_links_and_dtd_fail_closed() {
        assert!(stream_xml(io::Cursor::new(xml("EPSG:9999")), |_, _| Ok(())).is_err());
        assert!(
            stream_xml(
                io::Cursor::new("<!DOCTYPE x [<!ENTITY a 'bad'>]><x/>"),
                |_, _| Ok(())
            )
            .is_err()
        );
        let giant = format!("<x>{}</x>", "a".repeat(2 * 1024 * 1024 + 1));
        assert!(
            stream_xml(io::Cursor::new(giant), |_, _| Ok(()))
                .unwrap_err()
                .contains("token")
        );
        let input = xml("EPSG:4979").replace(
            "<gml:surfaceMember>",
            "<gml:surfaceMember xlink:href=\"https://outside.invalid/roof\">",
        );
        let (_, inverse) = city_mesh::frame().unwrap();
        let error = stream_xml(io::Cursor::new(input), |bytes, defaults| {
            parse_building(bytes, defaults, inverse, &mut BTreeSet::new()).map(|_| ())
        })
        .unwrap_err();
        assert!(error.contains("unresolved"));
    }
    #[test]
    fn zip_directory_preflight_bounds_allocations() {
        let path = std::env::temp_dir().join(format!("fc-city-zip-{}.zip", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("original.gml", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, xml("EPSG:4979").as_bytes()).unwrap();
        zip.finish().unwrap();
        zip_preflight(&mut File::open(&path).unwrap()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let len = bytes.len();
        bytes[len - 10..len - 6].copy_from_slice(&(100_000_000_u32).to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(zip_preflight(&mut File::open(&path).unwrap()).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
