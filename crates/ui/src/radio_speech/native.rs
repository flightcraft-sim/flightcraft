use super::dsp::{RadioParams, blocked_garble, radio_effect};
use super::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use flightcraft_control::{Gender, VoiceProfile};
use std::{
    collections::VecDeque,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
struct Request {
    text: String,
    voice: VoiceProfile,
    com: bool,
    morse: Option<String>,
    /// Two carriers keyed together: a heterodyne squeal of this many seconds instead of speech.
    garble: Option<f32>,
    /// Seconds the simulation allotted the call; speech that runs long is re-rendered faster to fit.
    slot_s: f32,
    distance: f32,
    seed: u32,
    volume: f32,
    epoch: u64,
}
type Rendered = (u64, Result<Vec<f32>, String>);
pub(super) struct Output {
    _stream: cpal::Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    requests: mpsc::SyncSender<Request>,
    rendered: mpsc::Receiver<Rendered>,
    epoch: u64,
}
impl Output {
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No speech output device; captions remain available")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate.0;
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let stream = match format {
            cpal::SampleFormat::F32 => stream::<f32>(&device, &config, queue.clone()),
            cpal::SampleFormat::I16 => stream::<i16>(&device, &config, queue.clone()),
            cpal::SampleFormat::U16 => stream::<u16>(&device, &config, queue.clone()),
            _ => return Err("Unsupported speech sample format".into()),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        let (requests, recv) = mpsc::sync_channel::<Request>(8);
        let (send, rendered) = mpsc::sync_channel::<Rendered>(8);
        std::thread::Builder::new()
            .name("radio-speech".into())
            .spawn(move || {
                while let Ok(request) = recv.recv() {
                    let samples = render(&request, rate);
                    if send.send((request.epoch, samples)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            queue,
            requests,
            rendered,
            epoch: 0,
        })
    }
    pub fn say(
        &mut self,
        event: &flightcraft_control::Transmission,
        volume: f32,
        voice: VoiceProfile,
        distance: f32,
    ) -> Result<(), String> {
        self.request(Request {
            text: event.text.clone(),
            voice,
            com: event.channel == Channel::Com,
            morse: if let Meaning::Ident { morse, .. } = &event.meaning {
                Some(morse.clone())
            } else {
                None
            },
            garble: None,
            slot_s: event.duration_s as f32,
            distance,
            seed: event.id as u32,
            volume,
            epoch: self.epoch,
        })
    }
    /// The squeal of two transmissions on one frequency.
    pub fn garble(
        &mut self,
        event: &flightcraft_control::Transmission,
        seconds: f32,
        volume: f32,
    ) -> Result<(), String> {
        self.request(Request {
            text: String::new(),
            voice: flightcraft_control::pilot_voice(0, 0),
            com: true,
            morse: None,
            garble: Some(seconds),
            slot_s: seconds,
            distance: 0.0,
            seed: event.id as u32,
            volume,
            epoch: self.epoch,
        })
    }
    fn request(&mut self, request: Request) -> Result<(), String> {
        self.requests
            .try_send(request)
            .map_err(|_| "Speech queue full; captions contain every call".into())
    }
    pub fn stop(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if let Ok(mut queue) = self.queue.lock() {
            queue.clear();
        }
    }
    pub fn poll(&mut self, active: bool, error: &mut Option<String>) {
        while let Ok((epoch, result)) = self.rendered.try_recv() {
            if !active || epoch != self.epoch {
                continue;
            }
            match result {
                Ok(samples) => {
                    if let Ok(mut queue) = self.queue.lock() {
                        mix_in(&mut queue, &samples);
                    }
                }
                Err(message) => *error = Some(message),
            }
        }
    }
}
/// Adds `samples` to what is already queued, starting now. Calls are voiced as they go on the air, so
/// two receivers on different frequencies overlap in time just as they would in the cockpit.
fn mix_in(queue: &mut VecDeque<f32>, samples: &[f32]) {
    for (i, sample) in samples.iter().enumerate().take(48000 * 90) {
        if let Some(slot) = queue.get_mut(i) {
            *slot = (*slot + sample).clamp(-1.0, 1.0);
        } else {
            queue.push_back(*sample);
        }
    }
}
fn stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels).max(1);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                if let Ok(mut queue) = queue.lock() {
                    for frame in output.chunks_mut(channels) {
                        let sample = queue.pop_front().unwrap_or(0.0);
                        for value in frame {
                            *value = T::from_sample(sample);
                        }
                    }
                } else {
                    for value in output {
                        *value = T::from_sample(0.0);
                    }
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| e.to_string())
}
/// Seconds of speech that fit a call's slot after the squelch break and tail.
fn speech_budget(slot_s: f32) -> f32 {
    (slot_s - (dsp::SQUELCH_BREAK_S + dsp::SQUELCH_TAIL_S)).max(0.8)
}
fn render(request: &Request, rate: u32) -> Result<Vec<f32>, String> {
    let mut samples = if let Some(seconds) = request.garble {
        blocked_garble(seconds, rate, request.seed)
    } else if let Some(code) = &request.morse {
        let mut tones = morse_samples(code, rate);
        if request.com {
            radio_effect(&mut tones, rate, &params(request));
        }
        tones
    } else {
        let mut voice = request.voice;
        let mut resampled = resample(synthesize(&request.text, &voice)?, rate);
        // The simulation held the frequency for the estimated speech time; never run past it.
        let budget = speech_budget(request.slot_s);
        let seconds = resampled.len() as f32 / rate as f32;
        if seconds > budget {
            // A little extra, since engines round their rate.
            voice.wpm = (f32::from(voice.wpm) * (seconds / budget * 1.03).min(1.8)).min(320.0) as u16;
            if let Ok(faster) = synthesize(&request.text, &voice) {
                resampled = resample(faster, rate);
            }
        }
        // Audio never runs past the carrier: whatever is left over is faded out at the budget.
        fit_to(&mut resampled, (budget * rate as f32) as usize, rate);
        if request.com {
            radio_effect(&mut resampled, rate, &params(request));
        }
        resampled
    };
    for sample in &mut samples {
        *sample *= request.volume;
    }
    Ok(samples)
}
/// Truncates `samples` to `limit` with a 20 ms fade-out.
fn fit_to(samples: &mut Vec<f32>, limit: usize, rate: u32) {
    if samples.len() <= limit {
        return;
    }
    samples.truncate(limit);
    let fade = ((rate / 50) as usize).min(limit).max(1);
    for i in 0..fade {
        let at = limit - fade + i;
        if let Some(s) = samples.get_mut(at) {
            *s *= 1.0 - i as f32 / fade as f32;
        }
    }
}
fn params(request: &Request) -> RadioParams {
    RadioParams {
        radio: request.voice.radio,
        distance: request.distance,
        seed: request.seed,
    }
}
fn resample((pcm, source_rate): (Vec<f32>, u32), rate: u32) -> Vec<f32> {
    let len = ((pcm.len() as u64 * u64::from(rate)) / u64::from(source_rate.max(1))).min(u64::from(rate) * 60)
        as usize;
    (0..len)
        .map(|i| {
            let position = i as f64 * f64::from(source_rate) / f64::from(rate);
            let at = position as usize;
            let fraction = position.fract() as f32;
            pcm.get(at).copied().unwrap_or(0.0) * (1.0 - fraction)
                + pcm.get(at + 1).copied().unwrap_or(0.0) * fraction
        })
        .collect()
}
fn synthesize(text: &str, voice: &VoiceProfile) -> Result<(Vec<f32>, u32), String> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let number = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("flightcraft-speech-{}-{number}", std::process::id()));
    std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let text_path = directory.join("utterance.txt");
    let wave = directory.join("utterance.wav");
    let result = (|| {
        std::fs::write(&text_path, text).map_err(|e| e.to_string())?;
        let mut command = platform_command(&text_path, &wave, voice)?;
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_| "Local speech engine unavailable; install espeak-ng or espeak on Linux. Captions remain available.".to_owned())?;
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err("Platform speech synthesis failed; captions remain available".into());
                }
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Speech synthesis timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        use std::io::Read;
        let file = std::fs::File::open(&wave).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Speech wave exceeds 4 MiB".into());
        }
        decode_wave(&bytes)
    })();
    let _ = std::fs::remove_dir_all(directory);
    result
}
#[cfg(target_os = "linux")]
fn platform_command(
    text: &std::path::Path,
    wave: &std::path::Path,
    voice: &VoiceProfile,
) -> Result<Command, String> {
    let available = |name: &str| {
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join(name).is_file()))
    };
    let mut command = Command::new(if available("espeak-ng") {
        "espeak-ng"
    } else {
        "espeak"
    });
    command
        .arg("-f")
        .arg(text)
        .arg("-w")
        .arg(wave)
        .arg("-v")
        .arg(espeak_voice(voice))
        .arg("-p")
        .arg(voice.pitch.min(99).to_string())
        .arg("-s")
        .arg(voice.wpm.to_string());
    Ok(command)
}
/// espeak's male (m1-m7) and female (f1-f5) timbre variants.
#[cfg(target_os = "linux")]
fn espeak_voice(voice: &VoiceProfile) -> String {
    match voice.gender {
        Gender::Male => format!("en-us+m{}", voice.variant.clamp(1, 7)),
        Gender::Female => format!("en-us+f{}", voice.variant.clamp(1, 5)),
    }
}
#[cfg(target_os = "macos")]
fn platform_command(
    text: &std::path::Path,
    wave: &std::path::Path,
    voice: &VoiceProfile,
) -> Result<Command, String> {
    // Stock macOS voices, filtered to the ones installed (`say -v ?`); with none, the default voice.
    const MALE: [&str; 5] = ["Alex", "Daniel", "Fred", "Tom", "Aaron"];
    const FEMALE: [&str; 5] = ["Samantha", "Karen", "Victoria", "Moira", "Allison"];
    static INSTALLED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let installed = INSTALLED.get_or_init(|| {
        Command::new("say")
            .args(["-v", "?"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    });
    let names: Vec<&str> = (if voice.gender == Gender::Male {
        &MALE
    } else {
        &FEMALE
    })
    .iter()
    .copied()
    .filter(|n| installed.lines().any(|l| l.split_whitespace().next() == Some(*n)))
    .collect();
    let mut command = Command::new("say");
    if !names.is_empty() {
        command
            .arg("-v")
            .arg(names[usize::from(voice.variant) % names.len()]);
    }
    command
        .arg("-f")
        .arg(text)
        .arg("-o")
        .arg(wave)
        .arg("--file-format=WAVE")
        .arg("--data-format=LEI16@22050")
        .arg("-r")
        .arg(voice.wpm.to_string());
    Ok(command)
}
#[cfg(target_os = "windows")]
fn platform_command(
    text: &std::path::Path,
    wave: &std::path::Path,
    voice: &VoiceProfile,
) -> Result<Command, String> {
    // User text is read from a file, never interpolated into executable PowerShell. Installed voices of
    // the wanted gender are chosen by timbre number; rate maps 160 wpm to 0 and 12 wpm to one step.
    let mut command = Command::new("powershell");
    command.arg("-NoProfile").arg("-NonInteractive").arg("-Command").arg("Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $all=$s.GetInstalledVoices(); $g=$all | Where-Object { $_.VoiceInfo.Gender -eq $env:FC_SPEECH_GENDER }; if($g.Count -eq 0){$g=$all}; if($g.Count -gt 0){$s.SelectVoice($g[[int]$env:FC_SPEECH_VARIANT % $g.Count].VoiceInfo.Name)}; $s.Rate=[int]$env:FC_SPEECH_RATE; $s.SetOutputToWaveFile($env:FC_SPEECH_WAVE); $s.Speak([IO.File]::ReadAllText($env:FC_SPEECH_TEXT)); $s.Dispose()")
        .env("FC_SPEECH_TEXT", text)
        .env("FC_SPEECH_WAVE", wave)
        .env("FC_SPEECH_GENDER", if voice.gender == Gender::Male { "Male" } else { "Female" })
        .env("FC_SPEECH_VARIANT", voice.variant.to_string())
        .env("FC_SPEECH_RATE", ((i32::from(voice.wpm) - 160) / 12).clamp(-10, 10).to_string());
    Ok(command)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_command(_: &std::path::Path, _: &std::path::Path, _: &VoiceProfile) -> Result<Command, String> {
    Err("No platform speech adapter".into())
}
fn decode_wave(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let u16_at = |at| {
        bytes
            .get(at..at + 2)
            .and_then(|s| <[u8; 2]>::try_from(s).ok())
            .map(u16::from_le_bytes)
    };
    let u32_at = |at| {
        bytes
            .get(at..at + 4)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            .map(u32::from_le_bytes)
    };
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("Speech engine did not produce a WAV file".into());
    }
    let mut offset: usize = 12;
    let mut format = None;
    let mut data = None;
    while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
        let length = u32_at(offset + 4).ok_or("Invalid wave chunk")? as usize;
        let begin = offset + 8;
        let end = begin.checked_add(length).ok_or("Wave chunk overflow")?;
        if header.get(..4) == Some(b"fmt ") {
            format = Some((
                u16_at(begin),
                u16_at(begin + 2),
                u32_at(begin + 4),
                u16_at(begin + 14),
            ));
        } else if header.get(..4) == Some(b"data") {
            data = bytes.get(begin..end);
        }
        offset = end.checked_add(length % 2).ok_or("Wave padding overflow")?;
    }
    let Some((Some(1), Some(channels), Some(rate), Some(16))) = format else {
        return Err("Speech requires 16-bit PCM wave".into());
    };
    if channels == 0 || channels > 8 || !(8000..=192000).contains(&rate) {
        return Err("Unsupported speech wave format".into());
    }
    let pcm = data
        .ok_or("Missing speech wave data")?
        .chunks_exact(usize::from(channels) * 2)
        .map(|frame| {
            frame
                .as_chunks::<2>()
                .0
                .iter()
                .copied()
                .map(|v| f32::from(i16::from_le_bytes(v)) / 32768.0)
                .sum::<f32>()
                / f32::from(channels)
        })
        .collect();
    Ok((pcm, rate))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn have_espeak() -> bool {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|p| p.join("espeak-ng").is_file() || p.join("espeak").is_file())
        })
    }
    /// Offline: two different voices say the same call through the engine and the radio. Skipped where no
    /// local speech tool is installed.
    #[cfg(target_os = "linux")]
    #[test]
    fn voices_render_different_audio_that_fits_the_call_slot() {
        if !have_espeak() {
            return;
        }
        let text = "Cessna four two seven kilo papa, descend and maintain three thousand.";
        let rate = 22050;
        let request = |voice: VoiceProfile| Request {
            text: text.into(),
            voice,
            com: true,
            morse: None,
            garble: None,
            slot_s: voice.duration_s(text) as f32,
            distance: 0.3,
            seed: 5,
            volume: 1.0,
            epoch: 0,
        };
        let controller = flightcraft_control::controller_voice(0);
        let pilot = flightcraft_control::pilot_voice(2, 7);
        let a = render(&request(controller), rate).unwrap();
        let b = render(&request(pilot), rate).unwrap();
        assert_ne!(a, b);
        assert!(a.iter().chain(&b).all(|s| s.is_finite() && s.abs() <= 1.0));
        for (samples, voice) in [(&a, controller), (&b, pilot)] {
            let seconds = samples.len() as f64 / f64::from(rate);
            // Speech plus squelch break and tail stays within the carrier time the simulation allotted:
            // the audio starts with the transmission, so it must also end with it.
            assert!(
                seconds <= voice.duration_s(text),
                "{seconds} vs {}",
                voice.duration_s(text)
            );
            assert!(seconds > 1.0);
        }
        // Blocked: a squeal, whatever the text.
        let mut squeal = request(controller);
        squeal.garble = Some(1.0);
        assert!(render(&squeal, rate).unwrap().len() >= rate as usize);
    }
    #[test]
    fn audio_longer_than_the_carrier_is_cut_to_it() {
        let rate = 22050;
        let mut long = vec![0.5_f32; rate as usize * 3];
        fit_to(&mut long, rate as usize * 2, rate);
        assert_eq!(long.len(), rate as usize * 2);
        assert!(long.last().is_some_and(|v| v.abs() < 0.01));
        let mut short = vec![0.5_f32; 100];
        fit_to(&mut short, 1000, rate);
        assert_eq!(short.len(), 100);
    }
    #[test]
    fn overlapping_receivers_are_mixed_not_queued() {
        let mut queue: VecDeque<f32> = VecDeque::from(vec![0.25; 4]);
        mix_in(&mut queue, &[0.25; 6]);
        assert_eq!(queue.len(), 6);
        assert!(queue.iter().take(4).all(|v| (*v - 0.5).abs() < 1e-6));
        assert!(queue.iter().skip(4).all(|v| (*v - 0.25).abs() < 1e-6));
    }
    #[test]
    fn hostile_wave_is_an_error() {
        for input in [vec![], vec![0; 50], b"RIFFxxxxWAVEdata\xff\xff\xff\xff".to_vec()] {
            assert!(decode_wave(&input).is_err());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod preview {
    use super::*;
    fn wave(samples: &[f32], rate: u32) -> Vec<u8> {
        let bytes = samples.len() as u32 * 2;
        let mut output = b"RIFF".to_vec();
        output.extend((36 + bytes).to_le_bytes());
        output.extend(b"WAVEfmt ");
        output.extend(16_u32.to_le_bytes());
        output.extend(1_u16.to_le_bytes());
        output.extend(1_u16.to_le_bytes());
        output.extend(rate.to_le_bytes());
        output.extend((rate * 2).to_le_bytes());
        output.extend(2_u16.to_le_bytes());
        output.extend(16_u16.to_le_bytes());
        output.extend(b"data");
        output.extend(bytes.to_le_bytes());
        for sample in samples {
            output.extend(((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        output
    }
    fn preview_params(voice: &VoiceProfile, distance: f32) -> RadioParams {
        RadioParams {
            radio: voice.radio,
            distance,
            seed: 1,
        }
    }
    #[test]
    #[ignore = "requires a platform speech tool and writes audio previews"]
    fn render_radio_audio_preview() {
        let (mut pcm, rate) = synthesize(
            "Blairstown traffic, Cherokee eight one Foxtrot, downwind runway two five, Blairstown.",
            &flightcraft_control::pilot_voice(6, 0),
        )
        .unwrap();
        assert!(pcm.iter().any(|s| s.abs() > 0.01));
        radio_effect(
            &mut pcm,
            rate,
            &preview_params(&flightcraft_control::pilot_voice(6, 0), 0.2),
        );
        let encoded = wave(&pcm, rate);
        let (decoded, decoded_rate) = decode_wave(&encoded).unwrap();
        assert_eq!(decoded_rate, rate);
        assert_eq!(decoded.len(), pcm.len());
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/radio");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("traffic-radio.wav"), encoded).unwrap();
        if let Ok(report) = std::fs::read_to_string(directory.join("awos-ifr.txt"))
            && let Some(text) = report.lines().nth(1)
        {
            let voice = flightcraft_control::controller_voice(6);
            let (mut pcm, rate) = synthesize(text, &voice).unwrap();
            radio_effect(&mut pcm, rate, &preview_params(&voice, 0.1));
            std::fs::write(directory.join("awos-ifr.wav"), wave(&pcm, rate)).unwrap();
        }
        std::fs::write(
            directory.join("nav-ident.wav"),
            wave(&morse_samples("... -... .---", 22050), 22050),
        )
        .unwrap();
    }
    #[test]
    #[ignore = "requires a local platform speech engine; writes crew voice previews"]
    fn render_crew_audio_previews() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/crew");
        std::fs::create_dir_all(&directory).unwrap();
        let record: flightcraft_control::FlightRecord =
            serde_json::from_slice(&std::fs::read(directory.join("water-gap-events.json")).unwrap()).unwrap();
        let intercom = record
            .events
            .iter()
            .find(|e| e.speaker == Speaker::Crew && e.text.starts_with("Climbing at"))
            .unwrap()
            .text
            .clone();
        let radio = record
            .events
            .iter()
            .find(|e| {
                e.speaker == Speaker::Pilot && e.channel == Channel::Com && e.reception == Reception::Heard
            })
            .unwrap()
            .text
            .clone();
        assert!(radio.contains("Sling Three Seven One Foxtrot Charlie"));
        let crew = flightcraft_control::speaker_voice(&Speaker::Crew, &Meaning::Text).unwrap();
        for (name, text, voice, radio) in [
            ("intercom", intercom.as_str(), crew, false),
            (
                "pilot-ctaf",
                radio.as_str(),
                flightcraft_control::pilot_voice(0, 0),
                true,
            ),
        ] {
            let (mut pcm, rate) = synthesize(text, &voice).unwrap();
            assert!(pcm.iter().any(|s| s.abs() > 0.01));
            if radio {
                radio_effect(&mut pcm, rate, &preview_params(&voice, 0.2));
            }
            std::fs::write(directory.join(format!("{name}.wav")), wave(&pcm, rate)).unwrap();
        }
    }
}
