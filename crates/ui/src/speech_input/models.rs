//! Opt-in model acquisition: immutable URLs, resumable partials, and mandatory content hashes.
#[cfg(feature = "desktop-audio")]
use super::pipeline::AudioClip;
use super::pipeline::Recognized;
#[cfg(feature = "whisper")]
use super::pipeline::Recognizer;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
#[derive(Deserialize)]
struct Manifest {
    version: u32,
    models: Vec<Model>,
}
#[derive(Deserialize)]
struct Model {
    id: String,
    files: Vec<Artifact>,
}
#[derive(Deserialize)]
struct Artifact {
    name: String,
    url: String,
    sha256: String,
    bytes: u64,
}
fn manifest(id: &str) -> Result<Model, String> {
    let manifest: Manifest = serde_json::from_str(include_str!("../../../../assets/speech/models.json"))
        .map_err(|e| e.to_string())?;
    if manifest.version != 1 {
        return Err("Unsupported speech model manifest.".into());
    }
    let model = manifest
        .models
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("Choose tiny.en or base.en in Preferences.")?;
    let required = ["config.json", "tokenizer.json", "model.safetensors"];
    if model.files.len() != 3
        || required
            .iter()
            .any(|name| !model.files.iter().any(|f| f.name == *name))
    {
        return Err("Speech model downloads are unavailable in this build. Verified model files are needed from the maintainer.".into());
    }
    for f in &model.files {
        let prefix = format!("https://huggingface.co/openai/whisper-{id}/resolve/");
        let revision = f.url.strip_prefix(&prefix).and_then(|v| v.split('/').next());
        if revision.is_none_or(|r| r.len() != 40 || !r.bytes().all(|b| b.is_ascii_hexdigit()))
            || !f.url.ends_with(&format!("/{}", f.name))
            || f.sha256.len() != 64
            || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !(1..=400_000_000).contains(&f.bytes)
        {
            return Err("Speech model manifest has an invalid pin; no download was started.".into());
        }
    }
    Ok(model)
}
pub(super) fn directory(id: &str) -> Result<PathBuf, String> {
    if !matches!(id, "tiny.en" | "base.en") {
        return Err("Unknown speech model.".into());
    }
    flightcraft_control::navigation_data_directory()
        .map(|p| p.join("speech").join(id))
        .ok_or("User data directory unavailable.".into())
}
#[cfg(feature = "whisper")]
pub(super) fn verify(id: &str) -> Result<PathBuf, String> {
    let root = std::env::var_os("FLIGHTCRAFT_WHISPER_DIR")
        .map(|root| PathBuf::from(root).join(id))
        .map_or_else(|| directory(id), Ok)?;
    for artifact in manifest(id)?.files {
        verify_file(&root.join(&artifact.name), &artifact)?;
    }
    Ok(root)
}
fn verify_file(path: &Path, artifact: &Artifact) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|_| {
        "Speech model is not installed. Open Preferences and choose Download / resume selected model."
            .to_owned()
    })?;
    if file.metadata().map_err(|e| e.to_string())?.len() != artifact.bytes {
        return Err("Speech model size is incorrect; download it again in Preferences.".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(buffer.get(..n).ok_or("Model read failed.")?);
    }
    if format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err("Speech model checksum mismatch. Download it again in Preferences.".into());
    }
    Ok(())
}
enum Work {
    #[cfg(feature = "whisper")]
    Prepare(String),
    Download(String),
    #[cfg(feature = "desktop-audio")]
    Recognize(AudioClip, bool, u64, String, String),
}
type Completion = (u64, bool, Result<Recognized, String>);
pub struct Worker {
    requests: mpsc::SyncSender<Work>,
    results: mpsc::Receiver<Completion>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}
impl Worker {
    pub fn new() -> Result<Self, String> {
        let (requests, receiver) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(2);
        let done = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let d = done.clone();
        let t = total.clone();
        std::thread::Builder::new().name("speech-input".into()).spawn(move || {
            #[cfg(feature = "whisper")]
            let mut recognizer: Option<(String, super::whisper::Whisper)> = None;
            while let Ok(work) = receiver.recv() {
                match work {
                    #[cfg(feature = "whisper")]
                    Work::Prepare(id) => {
                        if recognizer.as_ref().is_none_or(|(old, _)| old != &id) {
                            match super::whisper::Whisper::load(&id) {
                                Ok(model) => recognizer = Some((id, model)),
                                Err(error) => { let _ = sender.send((u64::MAX, false, Err(error))); }
                            }
                        }
                    }
                    Work::Download(id) => {
                        let result = download(&id, &d, &t).map(|()| Recognized { text: String::new(), confidence: 0., latency_ms: 0, alternatives: Vec::new() });
                        // Download status is a host notification, never pilot speech.
                        let _ = sender.send((u64::MAX, false, result));
                    }
                    #[cfg(feature = "desktop-audio")]
                    Work::Recognize(clip, intercom, epoch, id, context) => {
                        #[cfg(feature = "whisper")]
                        let result = (|| {
                            if recognizer.as_ref().is_none_or(|(old, _)| old != &id) { recognizer = Some((id.clone(), super::whisper::Whisper::load(&id)?)); }
                            let model = &mut recognizer.as_mut().ok_or("Speech recognizer unavailable.")?.1;
                            model.set_context(&context)?;
                            model.recognize(&clip)
                        })();
                        #[cfg(not(feature = "whisper"))]
                        let result = { let _ = (clip, id, context); Err("This build has no Whisper backend. Build desktop with the whisper feature; typing remains available.".into()) };
                        let _ = sender.send((epoch, intercom, result));
                    }
                }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self {
            requests,
            results,
            done,
            total,
        })
    }
    #[cfg(feature = "whisper")]
    pub fn prepare(&self, id: &str) {
        // Preparation is opportunistic; an occupied queue still recognizes on release.
        let _ = self.requests.try_send(Work::Prepare(id.into()));
    }
    pub fn download(&self, id: &str) -> Result<(), String> {
        self.requests
            .try_send(Work::Download(id.into()))
            .map_err(|_| "Speech worker busy; try again after recognition finishes.".into())
    }
    #[cfg(feature = "desktop-audio")]
    pub fn recognize(
        &self,
        clip: AudioClip,
        intercom: bool,
        epoch: u64,
        id: &str,
        context: String,
    ) -> Result<(), String> {
        self.requests
            .try_send(Work::Recognize(clip, intercom, epoch, id.into(), context))
            .map_err(|_| "Recognition busy. Wait for the previous call, then retry.".into())
    }
    pub fn poll(&self) -> Option<Completion> {
        self.results.try_recv().ok()
    }
    pub fn progress(&self) -> (u64, u64) {
        (
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}
fn download(id: &str, done: &AtomicU64, total: &AtomicU64) -> Result<(), String> {
    let model = manifest(id)?;
    let root = directory(id)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    total.store(model.files.iter().map(|f| f.bytes).sum(), Ordering::Relaxed);
    done.store(0, Ordering::Relaxed);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .into();
    for artifact in model.files {
        let path = root.join(&artifact.name);
        if verify_file(&path, &artifact).is_ok() {
            done.fetch_add(artifact.bytes, Ordering::Relaxed);
            continue;
        }
        let partial = root.join(format!("{}.partial", artifact.name));
        let offset = fs::metadata(&partial).map_or(0, |m| m.len());
        let offset = if offset <= artifact.bytes {
            offset
        } else {
            fs::remove_file(&partial).map_err(|e| e.to_string())?;
            0
        };
        if offset < artifact.bytes {
            let response = agent.get(&artifact.url).header("Range", &format!("bytes={offset}-")).call()
                .map_err(|e| format!("Unable to download speech model. You may be offline. Partial progress is saved; retry in Preferences. {e}"))?;
            let resumed = response.status().as_u16() == 206;
            if resumed
                && response
                    .headers()
                    .get("Content-Range")
                    .and_then(|v| v.to_str().ok())
                    .is_none_or(|v| {
                        !v.starts_with(&format!("bytes {offset}-"))
                            || !v.ends_with(&format!("/{}", artifact.bytes))
                    })
            {
                return Err("Speech download returned an invalid resume range. Retry later.".into());
            }
            let start = if resumed { offset } else { 0 };
            done.fetch_add(start, Ordering::Relaxed);
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(resumed)
                .truncate(!resumed)
                .open(&partial)
                .map_err(|e| e.to_string())?;
            let mut reader = response.into_body().into_reader();
            let mut count = start;
            let mut buffer = [0u8; 65536];
            loop {
                let n = reader
                    .read(&mut buffer)
                    .map_err(|_| "Speech model download interrupted. Retry to resume.".to_owned())?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > artifact.bytes {
                    return Err("Speech model download exceeds its pinned size.".into());
                }
                file.write_all(buffer.get(..n).ok_or("Model download read failed.")?)
                    .map_err(|e| e.to_string())?;
                done.fetch_add(n as u64, Ordering::Relaxed);
            }
            file.sync_all().map_err(|e| e.to_string())?;
        } else {
            done.fetch_add(offset, Ordering::Relaxed);
        }
        if fs::metadata(&partial).map_err(|e| e.to_string())?.len() < artifact.bytes {
            return Err("Speech model download ended early. Retry in Preferences to resume.".into());
        }
        if let Err(e) = verify_file(&partial, &artifact) {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
        // A previously corrupt destination is not usable. Windows rename does
        // not replace it, so remove it before the atomic installation step.
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        fs::rename(partial, path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_or_absent_model_never_passes_verification() {
        let path = std::env::temp_dir().join(format!("fc-speech-integrity-{}", std::process::id()));
        let artifact = Artifact {
            name: "model.safetensors".into(),
            url: String::new(),
            sha256: format!("{:x}", Sha256::digest(b"good")),
            bytes: 4,
        };
        assert!(verify_file(&path, &artifact).is_err());
        assert!(fs::write(&path, b"evil").is_ok());
        assert!(verify_file(&path, &artifact).is_err());
        let _ = fs::remove_file(path);
    }
}
