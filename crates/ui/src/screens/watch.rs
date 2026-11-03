//! Poll the directory, including atomic editor renames. Debounce and preserve last good files.
use flightcraft_control::{Session, screens::Layout};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
struct File {
    seen: Vec<u8>,
    applied: Vec<u8>,
    pending: Option<(Vec<u8>, Instant)>,
}
pub struct Folder {
    root: PathBuf,
    files: BTreeMap<u64, File>,
    last: Instant,
}
impl Folder {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            files: BTreeMap::new(),
            last: Instant::now() - Duration::from_secs(1),
        })
    }
    pub fn poll(&mut self, s: &mut Session, error: &mut Option<String>) {
        if self.last.elapsed() < Duration::from_millis(150) {
            return;
        }
        self.last = Instant::now();
        for window in s.screens().windows.clone() {
            let path = self.root.join(format!("screen-{}.json", window.id));
            let current = match serde_json::to_vec_pretty(&window.layout) {
                Ok(v) => v,
                Err(e) => {
                    *error = Some(e.to_string());
                    continue;
                }
            };
            let disk = std::fs::metadata(&path)
                .ok()
                .filter(|m| m.len() <= 1024 * 1024)
                .and_then(|_| std::fs::read(&path).ok());
            let file = self.files.entry(window.id).or_insert_with(|| File {
                seen: disk.clone().unwrap_or_else(|| current.clone()),
                applied: current.clone(),
                pending: disk
                    .clone()
                    .filter(|v| v != &current)
                    .map(|v| (v, Instant::now())),
            });
            if let Some(bytes) = disk {
                if bytes != file.seen && file.pending.as_ref().is_none_or(|(p, _)| p != &bytes) {
                    file.pending = Some((bytes.clone(), Instant::now()));
                }
                if let Some((pending, since)) = &file.pending
                    && since.elapsed() >= Duration::from_millis(250)
                {
                    let result = std::str::from_utf8(pending)
                        .map_err(|e| e.to_string())
                        .and_then(Layout::from_json)
                        .and_then(|layout| {
                            s.execute("layout.set", json!({"id":window.id,"layout":layout}))
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        });
                    let good = result.is_ok();
                    if let Err(e) = result {
                        *error = Some(format!("{}: {e}; last good layout retained", path.display()));
                    }
                    if good {
                        file.applied = s
                            .screens()
                            .windows
                            .iter()
                            .find(|w| w.id == window.id)
                            .and_then(|w| serde_json::to_vec_pretty(&w.layout).ok())
                            .unwrap_or_else(|| current.clone());
                    }
                    file.seen = pending.clone();
                    file.pending = None;
                    continue;
                }
                if file.pending.is_some() {
                    continue;
                }
                // An in-app edit differs from the last observed file. Save atomically.
                if current != file.applied
                    && let Err(e) = s.execute("layout.save", json!({"screen":window.id,"path":path}))
                {
                    *error = Some(e.to_string());
                } else if current != file.applied {
                    file.applied = current.clone();
                    file.seen = current;
                }
            } else if let Err(e) = s.execute("layout.save", json!({"screen":window.id,"path":path})) {
                *error = Some(e.to_string());
            }
        }
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn directory_rename_debounce_and_last_good() {
        let root = std::env::temp_dir().join(format!("flightcraft-layout-watch-{}", std::process::id()));
        let mut s = Session::new().unwrap();
        let mut folder = Folder::new(root.clone()).unwrap();
        let mut error = None;
        folder.poll(&mut s, &mut error);
        let original = s.screens().windows.first().unwrap().layout.clone();
        std::fs::write(root.join("next.json"), b"not json").unwrap();
        std::fs::rename(root.join("next.json"), root.join("screen-0.json")).unwrap();
        std::thread::sleep(Duration::from_millis(160));
        folder.poll(&mut s, &mut error);
        assert!(error.is_none());
        std::thread::sleep(Duration::from_millis(270));
        folder.poll(&mut s, &mut error);
        assert!(error.as_ref().unwrap().contains("last good"));
        assert_eq!(s.screens().windows.first().unwrap().layout.name, original.name);
        std::thread::sleep(Duration::from_millis(160));
        folder.poll(&mut s, &mut error);
        assert_eq!(
            std::fs::read_to_string(root.join("screen-0.json")).unwrap(),
            "not json"
        );
        let mut replacement = original;
        replacement.name = "Updated by editor rename".into();
        std::fs::write(root.join("next.json"), serde_json::to_vec(&replacement).unwrap()).unwrap();
        std::fs::rename(root.join("next.json"), root.join("screen-0.json")).unwrap();
        std::thread::sleep(Duration::from_millis(160));
        folder.poll(&mut s, &mut error);
        std::thread::sleep(Duration::from_millis(270));
        folder.poll(&mut s, &mut error);
        assert_eq!(s.screens().windows.first().unwrap().layout.name, replacement.name);
        std::fs::remove_dir_all(root).unwrap();
    }
}
