//! Local-only visual comparisons; repository artifacts are restricted to ignored `out/`.
use image::{GrayImage, Luma, Rgba, RgbaImage};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let (mut pack, mut reference, mut out) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = PathBuf::from(args.next().ok_or("expected flag value")?);
        match flag.as_str() {"--pack" if pack.is_none()=>pack=Some(value),"--reference" if reference.is_none()=>reference=Some(value),"--out" if out.is_none()=>out=Some(value),_=>return Err("usage: cargo xtask panel-compare --pack <dir> --reference <local image> --out <ignored-out-or-external dir>".into())}
    }
    compare(
        &pack.ok_or("missing --pack")?,
        &reference.ok_or("missing --reference")?,
        &out.ok_or("missing --out")?,
    )
}
/// Resolve existing ancestors before creating anything, including symlinks and `..`.
fn resolved_destination(path: &Path) -> Result<PathBuf, String> {
    if path.components().count() > 256 {
        return Err("output path exceeds depth limit".into());
    }
    if path.exists() {
        return path.canonicalize().map_err(|e| e.to_string());
    }
    let name = path.file_name().ok_or("invalid output path")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(resolved_destination(parent)?.join(name))
}
/// The git checkout holding the pack, so comparison images (which contain the reference) never land in it.
fn pack_repository(pack: &Path) -> Option<PathBuf> {
    let pack = pack.canonicalize().ok()?;
    pack.ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf)
}
/// Outputs may go under the engine's ignored `out/` or anywhere outside both the engine and the pack
/// repositories. Unrelated checkouts above the output (a scratch folder under a dotfiles repo) are fine.
fn check_output(path: &Path, root: &Path, pack_repo: Option<&Path>) -> Result<PathBuf, String> {
    let output = resolved_destination(path)?;
    if output.starts_with(root.join("out")) {
        return Ok(output);
    }
    if output.starts_with(root) || pack_repo.is_some_and(|r| output.starts_with(r)) {
        return Err(
            "--out must be outside the engine and pack repositories, or under the engine's ignored out/; comparison images are local only"
                .into(),
        );
    }
    Ok(output)
}
#[derive(Serialize)]
struct Alignment {
    installation: String,
    rect_px: [u32; 4],
    /// Symmetric edge proximity within four pixels, 0..1; appearance also affects this score.
    edge_alignment: f64,
    reference_edge_pixels: u64,
    render_edge_pixels: u64,
}
fn edges(image: &RgbaImage) -> GrayImage {
    let gray = image::DynamicImage::ImageRgba8(image.clone()).to_luma8();
    let mut result = GrayImage::new(gray.width(), gray.height());
    for (x, y, p) in result.enumerate_pixels_mut() {
        if x == 0 || y == 0 || x + 1 >= gray.width() || y + 1 >= gray.height() {
            continue;
        }
        let get =
            |dx: i32, dy: i32| i32::from(gray.get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32).0[0]);
        let gx = -get(-1, -1) + get(1, -1) - 2 * get(-1, 0) + 2 * get(1, 0) - get(-1, 1) + get(1, 1);
        let gy = -get(-1, -1) - 2 * get(0, -1) - get(1, -1) + get(-1, 1) + 2 * get(0, 1) + get(1, 1);
        *p = Luma([if gx.abs() + gy.abs() > 160 { 255 } else { 0 }]);
    }
    result
}
fn alignment(a: &GrayImage, b: &GrayImage, [x, y, w, h]: [u32; 4]) -> (f64, u64, u64) {
    let mut matched = 0_u64;
    let mut counts = [0_u64; 2];
    for (i, (src, dst)) in [(a, b), (b, a)].into_iter().enumerate() {
        for sy in y..y.saturating_add(h).min(src.height()) {
            for sx in x..x.saturating_add(w).min(src.width()) {
                if src.get_pixel(sx, sy).0[0] == 0 {
                    continue;
                }
                if let Some(count) = counts.get_mut(i) {
                    *count += 1;
                }
                let near = (sy.saturating_sub(4).max(y)
                    ..=sy
                        .saturating_add(4)
                        .min(y.saturating_add(h).saturating_sub(1))
                        .min(dst.height().saturating_sub(1)))
                    .any(|dy| {
                        (sx.saturating_sub(4).max(x)
                            ..=sx
                                .saturating_add(4)
                                .min(x.saturating_add(w).saturating_sub(1))
                                .min(dst.width().saturating_sub(1)))
                            .any(|dx| dst.get_pixel(dx, dy).0[0] > 0)
                    });
                matched += u64::from(near);
            }
        }
    }
    let [ac, bc] = counts;
    let total = ac + bc;
    (
        if total == 0 {
            0.
        } else {
            matched as f64 / total as f64
        },
        ac,
        bc,
    )
}
fn compare(pack: &Path, reference: &Path, out: &Path) -> Result<(), String> {
    let root = super::root()?.canonicalize().map_err(|e| e.to_string())?;
    let pack_repo = pack_repository(pack);
    let out = check_output(out, &root, pack_repo.as_deref())?;
    let reference_path = reference.canonicalize().map_err(|e| e.to_string())?;
    for name in [
        "render.png",
        "side-by-side.png",
        "overlay.png",
        "edge-difference.png",
        "alignment.json",
    ] {
        let child = out.join(name);
        let resolved = check_output(&child, &root, pack_repo.as_deref())?;
        if resolved == reference_path
            || std::fs::symlink_metadata(&child).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err("output must not overwrite the reference or follow file symlinks".into());
        }
    }
    // Decode with limits before allocating the derived images.
    let mut reader = image::ImageReader::open(reference)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let reference = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    if u64::from(reference.width()) * u64::from(reference.height()) > 16_777_216 {
        return Err("reference exceeds 16 megapixels".into());
    }
    let mut session = flightcraft_control::Session::new().map_err(|e| e.to_string())?;
    let pack = flightcraft_control::read_pack(pack)?;
    // Pick the first declared aircraft selection; manifests and loaded definitions are bounded.
    let aircraft = pack
        .files
        .values()
        .filter_map(|text| flightcraft_aero::LayerFile::from_json(text).ok())
        .find_map(|d| {
            if let flightcraft_aero::LayerFile::Aircraft(a) = d {
                Some(a.identity.id.clone())
            } else {
                None
            }
        })
        .ok_or("pack contains no aircraft selection")?;
    session.install_pack(&pack).map_err(|e| e.to_string())?;
    session
        .execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":aircraft},"reset":true}),
        )
        .map_err(|e| e.to_string())?;
    let render = flightcraft_ui::panel_preview::render_equipment_panel(
        &mut session,
        reference.width(),
        reference.height(),
    )?;
    let configuration = session.aircraft_configuration().ok_or("no configuration")?;
    let panel = configuration
        .panel
        .equipment
        .as_ref()
        .ok_or("no equipment panel")?;
    let [pw, ph] = panel.size;
    let scale = (f64::from(render.width()) / pw).min(f64::from(render.height()) / ph);
    let (left, top) = (
        (f64::from(render.width()) - pw * scale) * 0.5,
        (f64::from(render.height()) - ph * scale) * 0.5,
    );
    let (a, b) = (edges(&reference), edges(&render));
    let scores: Vec<_> = panel
        .elements
        .iter()
        .map(|e| {
            let [x, y] = e.position;
            let [w, h] = e.size;
            let rect = [
                (left + x * scale).round() as u32,
                (top + y * scale).round() as u32,
                (w * scale).round() as u32,
                (h * scale).round() as u32,
            ];
            let (score, ac, bc) = alignment(&a, &b, rect);
            Alignment {
                installation: e.id.clone(),
                rect_px: rect,
                edge_alignment: score,
                reference_edge_pixels: ac,
                render_edge_pixels: bc,
            }
        })
        .collect();
    let mut side = RgbaImage::new(render.width() * 2, render.height());
    image::imageops::replace(&mut side, &reference, 0, 0);
    image::imageops::replace(&mut side, &render, i64::from(render.width()), 0);
    let mut overlay = render.clone();
    let mut difference = render.clone();
    for (x, y, p) in overlay.enumerate_pixels_mut() {
        let r = reference.get_pixel(x, y);
        for (channel, other) in p.0.iter_mut().zip(r.0) {
            *channel = ((u16::from(*channel) + u16::from(other)) / 2) as u8;
        }
        let av = a.get_pixel(x, y).0[0];
        let bv = b.get_pixel(x, y).0[0];
        if let Some(d) = difference.get_pixel_mut_checked(x, y) {
            *d = Rgba([av, bv, av.abs_diff(bv), 255]);
        }
    }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (name, image) in [
        ("render.png", render),
        ("side-by-side.png", side),
        ("overlay.png", overlay),
        ("edge-difference.png", difference),
    ] {
        image.save(out.join(name)).map_err(|e| e.to_string())?;
    }
    super::download::write_json(&out.join("alignment.json"), &scores)?;
    println!("Panel comparison written to {}", out.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn outputs_use_ignored_out_and_reject_source_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        assert!(check_output(&root.join("out/new-comparison"), &root, None).is_ok());
        assert!(check_output(&root.join("docs/new-comparison"), &root, None).is_err());
        assert!(check_output(&root.join("out/../new-comparison"), &root, None).is_err());
    }
    #[test]
    fn pack_repository_is_refused_but_unrelated_checkouts_are_not() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let temp = std::env::temp_dir().join(format!("flightcraft-panel-repos-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let temp = temp.canonicalize().unwrap();
        let pack_repo = temp.join("pack");
        let scratch = temp.join("scratch");
        for dir in [
            pack_repo.join(".git"),
            scratch.join(".git"),
            scratch.join("compare"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        assert!(check_output(&pack_repo.join("previews/cmp"), &root, Some(&pack_repo)).is_err());
        assert!(check_output(&scratch.join("compare"), &root, Some(&pack_repo)).is_ok());
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn edge_score_detects_displacement() {
        let mut a = GrayImage::new(60, 40);
        let mut b = a.clone();
        for y in 10..30 {
            a.put_pixel(10, y, Luma([255]));
            b.put_pixel(25, y, Luma([255]));
        }
        assert_eq!(alignment(&a, &a, [0, 0, 60, 40]).0, 1.);
        assert_eq!(alignment(&a, &b, [0, 0, 60, 40]).0, 0.);
    }
    #[cfg(unix)]
    #[test]
    fn output_symlinks_cannot_bypass_repository_boundary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let temp = std::env::temp_dir().join(format!("flightcraft-panel-boundary-{}", std::process::id()));
        std::fs::create_dir(&temp).unwrap();
        let link = temp.join("repository");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(check_output(&link.join("new-panel-comparison"), &root, None).is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }
}
