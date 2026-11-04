//! Skin pack directories: bounded reads, no symlinks or escapes, bad skins skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::*;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fc-skins-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_skin(root: &std::path::Path, id: &str, font: Option<&[u8]>) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    let assets = if font.is_some() {
        r#","font":"fonts/a.ttf","assets":[{"path":"fonts/a.ttf","kind":"font","licence":"OFL-1.1"}]"#
    } else {
        ""
    };
    std::fs::write(
        dir.join("skin.json"),
        format!(
            r#"{{"kind":"skin_pack","format_version":1,"id":"{id}","version":"1","display_name":"Demo",
            "licence":"Apache-2.0","display_model":"landscape_touch_efis"{assets}}}"#
        ),
    )
    .unwrap();
    if let Some(font) = font {
        std::fs::write(dir.join("fonts/a.ttf"), font).unwrap();
    }
    dir
}

#[test]
fn a_directory_of_skins_loads_and_bad_ones_are_reported_and_skipped() {
    let root = scratch("load");
    write_skin(&root, "good-skin", Some(b"fontbytes"));
    write_skin(&root, "plain-skin", None);
    // Wrong id for its directory is still a valid manifest; a broken one is not.
    let broken = root.join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("skin.json"), "{not json").unwrap();
    let mut s = Session::new().unwrap();
    let errors = s.load_skins_directory(&root);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("broken"));
    assert_eq!(s.skins().count(), 2);
    assert_eq!(s.skin("good-skin").unwrap().font(), Some(&b"fontbytes"[..]));
    assert!(s.skin("plain-skin").unwrap().font().is_none());
    // A missing directory is not an error.
    assert!(s.load_skins_directory(&root.join("absent")).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

fn write_instrument_skin(root: &std::path::Path, id: &str, instruments: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("skin.json"),
        format!(
            r#"{{"kind":"skin_pack","format_version":1,"id":"{id}","version":"1","display_name":"Demo",
            "licence":"Apache-2.0","display_model":"landscape_touch_efis","instruments":{instruments}}}"#
        ),
    )
    .unwrap();
}

#[test]
fn instrument_palettes_load_and_a_bad_palette_skips_only_that_skin() {
    let root = scratch("instruments");
    write_instrument_skin(
        &root,
        "painted",
        r##"{"palette":{"magenta":"#ff33ff","cyan":"#00e6e6","tape_fill":"#202830","tape_text":"#f4f4f4"},
            "typography":{"tape_text":1.2}}"##,
    );
    // Magenta that is really green: the meaning is lost, so the whole skin is refused.
    write_instrument_skin(&root, "swapped", r##"{"palette":{"magenta":"#33ff33"}}"##);
    write_instrument_skin(&root, "oversized", r##"{"typography":{"box_text":4.0}}"##);
    write_skin(&root, "plain-skin", None);
    let mut s = Session::new().unwrap();
    let errors = s.load_skins_directory(&root);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("swapped")));
    assert!(errors.iter().any(|e| e.contains("oversized")));
    let painted = s.skin("painted").unwrap();
    let palette = &painted.manifest.instruments.palette;
    assert_eq!(palette.magenta.as_ref().unwrap().0, "#ff33ff");
    assert!(palette.green.is_none(), "unset fields keep the built-in colour");
    assert_eq!(painted.manifest.instruments.typography.tape_text, Some(1.2));
    // A skin without an instruments section changes nothing about the instrument faces.
    let plain = &s.skin("plain-skin").unwrap().manifest.instruments;
    assert_eq!(plain, &Default::default());
    assert!(s.skin("swapped").is_none() && s.skin("oversized").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn symlinked_assets_and_directories_are_refused() {
    let root = scratch("links");
    let dir = write_skin(&root, "linked", Some(b"x"));
    std::fs::remove_file(dir.join("fonts/a.ttf")).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", dir.join("fonts/a.ttf")).unwrap();
    assert!(read_skin(&dir).is_err(), "symlinked font");
    let real = write_skin(&root, "real", Some(b"x"));
    let link = root.join("alias");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(read_skin(&link).is_err(), "symlinked skin directory");
    assert!(read_skin(&real).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn oversize_manifests_and_fonts_are_refused() {
    let root = scratch("big");
    let dir = write_skin(&root, "big-font", Some(&vec![0u8; MAX_SKIN_FONT_BYTES + 1]));
    assert!(read_skin(&dir).is_err());
    let dir = write_skin(&root, "big-manifest", None);
    std::fs::write(dir.join("skin.json"), " ".repeat(MAX_SKIN_MANIFEST_BYTES + 1)).unwrap();
    assert!(read_skin(&dir).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

/// With FLIGHTCRAFT_SKINS_DIR set to a checkout of the skins repository, every skin in it loads.
#[test]
fn skins_in_a_local_checkout_load_when_one_is_provided() {
    let Some(dir) = std::env::var_os("FLIGHTCRAFT_SKINS_DIR") else {
        return;
    };
    let mut s = Session::new().unwrap();
    let errors = s.load_skins_directory(std::path::Path::new(&dir));
    assert!(errors.is_empty(), "{errors:?}");
    assert!(s.skins().count() > 0);
    for skin in s.skins() {
        assert!(skin.font().is_some_and(|f| !f.is_empty()), "{}", skin.manifest.id);
    }
}
