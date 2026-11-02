//! Durable debrief archive. Loading history never changes live flight dynamics.
use super::*;
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightSummary {
    pub id: String,
    pub created_unix_ms: u64,
    pub aircraft: String,
    pub registration: String,
    pub duration_s: f64,
    pub events: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedFlight {
    pub summary: FlightSummary,
    pub record: FlightRecord,
}
impl SavedFlight {
    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() > 128 * 1024 * 1024 {
            return Err("flight archive exceeds 128 MiB".into());
        }
        let saved: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if saved.summary.events != saved.record.events.len()
            || !valid_id(&saved.summary.id)
            || !saved.summary.duration_s.is_finite()
            || saved.summary.duration_s < 0.
            || saved.summary.aircraft.len() > 256
            || saved.summary.registration.len() > 12
        {
            return Err("invalid flight metadata".into());
        }
        FlightRecord::from_json(&serde_json::to_string(&saved.record).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(saved)
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct ArchiveStatus {
    pub busy: bool,
    pub pending_operations: usize,
    pub error: Option<String>,
    pub flights: Vec<FlightSummary>,
    pub loaded: Option<SavedFlight>,
    pub last_saved: Option<String>,
}
#[derive(Debug, Default)]
pub(crate) struct FlightArchive {
    pub(super) state: Arc<Mutex<ArchiveStatus>>,
    current: Option<(u64, String, u64)>,
    pub(crate) autosave_enabled: bool,
    last_signature: Option<(u64, u64, usize, usize)>,
}
fn clock_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now().max(0.) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64)
    }
}
impl Session {
    pub fn flight_archive_status(&self) -> ArchiveStatus {
        self.flight_archive
            .state
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| ArchiveStatus {
                error: Some("flight archive state unavailable".into()),
                ..Default::default()
            })
    }
    /// Called by the application on a wall-time cadence. Headless callers save explicitly.
    pub fn autosave_flight(&mut self) {
        self.flight_archive.autosave_enabled = true;
        let signature = (
            self.flight_generation(),
            self.sim().state().time_s.to_bits(),
            self.flight_record().events.len(),
            self.flight_record().inputs.len(),
        );
        if self.flight_archive.last_signature == Some(signature) || self.flight_archive_status().busy {
            return;
        }
        match self.save_flight() {
            Ok(_) => self.flight_archive.last_signature = Some(signature),
            Err(error) => {
                if let Ok(mut state) = self.flight_archive.state.lock() {
                    state.error = Some(error.to_string());
                }
            }
        }
    }
    pub(crate) fn save_flight(&mut self) -> Result<ArchiveStatus, CommandError> {
        let generation = self.flight_generation();
        let (id, created) = match &self.flight_archive.current {
            Some((g, id, created)) if *g == generation => (id.clone(), *created),
            _ => {
                let created = clock_ms();
                // Time, process/tab nonce, and generation separate independently opened flights.
                let id = format!("flight-{created}-{generation}-{}", flight_nonce());
                self.flight_archive.current = Some((generation, id.clone(), created));
                (id, created)
            }
        };
        let saved = SavedFlight {
            summary: FlightSummary {
                id,
                created_unix_ms: created,
                aircraft: self.sim().aircraft().identity.id.clone(),
                registration: self.pilot_callsign().registration,
                duration_s: self.sim().state().time_s,
                events: self.flight_record().events.len(),
            },
            record: self.flight_record().clone(),
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            desktop::save(&desktop::directory()?, &saved).map_err(CommandError::Failed)?;
            let mut state = self
                .flight_archive
                .state
                .lock()
                .map_err(|_| invalid("flight.save", "archive unavailable"))?;
            state.last_saved = Some(saved.summary.id.clone());
            state.error = None;
        }
        #[cfg(target_arch = "wasm32")]
        browser::save(self.flight_archive.state.clone(), saved).map_err(CommandError::Failed)?;
        Ok(self.flight_archive_status())
    }
}
fn flight_nonce() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        u64::from(std::process::id()) * 1_000_000
            + SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Math::random() * 9_007_199_254_740_991.) as u64
    }
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightSave {}
impl Command for FlightSave {
    const NAME: &'static str = "flight.save";
    const DESCRIPTION: &'static str =
        "Save the current flight to durable local storage; poll flight.records for browser completion.";
    type Output = ArchiveStatus;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        s.save_flight()
    }
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightRecords {
    #[serde(default)]
    pub refresh: bool,
}
impl Command for FlightRecords {
    const NAME: &'static str = "flight.records";
    const DESCRIPTION: &'static str =
        "List saved flights and poll archive operations; refresh starts a storage listing.";
    type Output = ArchiveStatus;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.refresh && !s.flight_archive_status().busy {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let flights = desktop::list(&desktop::directory()?).map_err(CommandError::Failed)?;
                let mut state = s
                    .flight_archive
                    .state
                    .lock()
                    .map_err(|_| invalid(Self::NAME, "archive unavailable"))?;
                state.flights = flights;
                state.error = None;
            }
            #[cfg(target_arch = "wasm32")]
            browser::list(s.flight_archive.state.clone()).map_err(CommandError::Failed)?;
        }
        Ok(s.flight_archive_status())
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightLoad {
    pub id: String,
}
impl Command for FlightLoad {
    const NAME: &'static str = "flight.load";
    const DESCRIPTION: &'static str = "Load a saved flight for debrief without changing live simulation; browser completion is asynchronous.";
    type Output = ArchiveStatus;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if !valid_id(&self.id) || s.flight_archive_status().busy {
            return Err(invalid(Self::NAME, "invalid id or archive busy"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let saved = desktop::load(&desktop::directory()?, &self.id).map_err(CommandError::Failed)?;
            let mut state = s
                .flight_archive
                .state
                .lock()
                .map_err(|_| invalid(Self::NAME, "archive unavailable"))?;
            state.loaded = Some(saved);
            state.error = None;
        }
        #[cfg(target_arch = "wasm32")]
        browser::load(s.flight_archive.state.clone(), self.id).map_err(CommandError::Failed)?;
        Ok(s.flight_archive_status())
    }
}
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlightExport {
    #[serde(default)]
    pub id: Option<String>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct FlightJson {
    pub filename: String,
    pub json: String,
}
impl Command for FlightExport {
    const NAME: &'static str = "flight.export";
    const DESCRIPTION: &'static str =
        "Export current flight record JSON, or an already loaded saved flight, without dialogs.";
    type Output = FlightJson;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let (filename, json) = if let Some(id) = self.id {
            let state = s.flight_archive_status();
            let saved = state
                .loaded
                .filter(|r| r.summary.id == id)
                .ok_or_else(|| invalid(Self::NAME, "load this saved flight first and poll until complete"))?;
            (format!("{id}.json"), serde_json::to_string_pretty(&saved))
        } else {
            (
                "current-flight.json".into(),
                serde_json::to_string_pretty(s.flight_record()),
            )
        };
        Ok(FlightJson {
            filename,
            json: json.map_err(|e| invalid(Self::NAME, e.to_string()))?,
        })
    }
}
