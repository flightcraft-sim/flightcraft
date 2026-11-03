use super::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
struct Request {
    text: String,
    role: usize,
    com: bool,
    morse: Option<String>,
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
    pub fn say(&mut self, event: &flightcraft_control::Transmission, volume: f32) -> Result<(), String> {
        self.requests
            .try_send(Request {
                text: event.text.clone(),
                role: role(&event.speaker),
                com: event.channel == Channel::Com,
                morse: if let Meaning::Ident { morse, .. } = &event.meaning {
                    Some(morse.clone())
                } else {
                    None
                },
                volume,
                epoch: self.epoch,
            })
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
                    if let Ok(mut queue) = self.queue.lock()
                        && queue.len() < 48000 * 90
                    {
                        queue.extend(samples);
                    }
                }
                Err(message) => *error = Some(message),
            }
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
fn render(request: &Request, rate: u32) -> Result<Vec<f32>, String> {
    let mut samples = if let Some(code) = &request.morse {
        morse_samples(code, rate)
    } else {
        let (pcm, source_rate) = synthesize(&request.text, request.role)?;
        let len = ((pcm.len() as u64 * u64::from(rate)) / u64::from(source_rate.max(1)))
            .min(u64::from(rate) * 60) as usize;
        (0..len)
            .map(|i| {
                let position = i as f64 * f64::from(source_rate) / f64::from(rate);
                let at = position as usize;
                let fraction = position.fract() as f32;
                pcm.get(at).copied().unwrap_or(0.0) * (1.0 - fraction)
                    + pcm.get(at + 1).copied().unwrap_or(0.0) * fraction
            })
            .collect()
    };
    if request.com {
        radio_effect(&mut samples, rate);
    }
    for sample in &mut samples {
        *sample *= request.volume;
    }
    Ok(samples)
}
fn synthesize(text: &str, voice: usize) -> Result<(Vec<f32>, u32), String> {
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
fn platform_command(text: &std::path::Path, wave: &std::path::Path, voice: usize) -> Result<Command, String> {
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
        .arg(format!(
            "en-us+{}{}",
            if voice.is_multiple_of(2) { "m" } else { "f" },
            voice % 4 + 1
        ))
        .arg("-s")
        .arg((150 + voice % 5 * 9).to_string());
    Ok(command)
}
#[cfg(target_os = "macos")]
fn platform_command(text: &std::path::Path, wave: &std::path::Path, voice: usize) -> Result<Command, String> {
    let mut command = Command::new("say");
    command
        .arg("-f")
        .arg(text)
        .arg("-o")
        .arg(wave)
        .arg("--file-format=WAVE")
        .arg("--data-format=LEI16@22050")
        .arg("-r")
        .arg((155 + voice % 5 * 12).to_string());
    Ok(command)
}
#[cfg(target_os = "windows")]
fn platform_command(text: &std::path::Path, wave: &std::path::Path, voice: usize) -> Result<Command, String> {
    // User text is read from a file, never interpolated into executable PowerShell.
    let mut command = Command::new("powershell");
    command.arg("-NoProfile").arg("-NonInteractive").arg("-Command").arg("Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $v=$s.GetInstalledVoices(); if($v.Count -gt 0){$s.SelectVoice($v[[int]$env:FC_SPEECH_VOICE % $v.Count].VoiceInfo.Name)}; $s.SetOutputToWaveFile($env:FC_SPEECH_WAVE); $s.Speak([IO.File]::ReadAllText($env:FC_SPEECH_TEXT)); $s.Dispose()")
        .env("FC_SPEECH_TEXT", text).env("FC_SPEECH_WAVE", wave).env("FC_SPEECH_VOICE", voice.to_string());
    Ok(command)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_command(_: &std::path::Path, _: &std::path::Path, _: usize) -> Result<Command, String> {
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
                .chunks_exact(2)
                .filter_map(|s| <[u8; 2]>::try_from(s).ok())
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
    #[test]
    #[ignore = "requires a platform speech tool and writes audio previews"]
    fn render_radio_audio_preview() {
        let (mut pcm, rate) = synthesize(
            "Blairstown traffic, Cherokee eight one Foxtrot, downwind runway two five, Blairstown.",
            6,
        )
        .unwrap();
        assert!(pcm.iter().any(|s| s.abs() > 0.01));
        radio_effect(&mut pcm, rate);
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
            let (mut pcm, rate) = synthesize(text, 5).unwrap();
            radio_effect(&mut pcm, rate);
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
        assert!(radio.contains("Sling One Two Three Four Bravo"));
        for (name, text, voice, radio) in [
            ("intercom", intercom.as_str(), 10, false),
            ("pilot-ctaf", radio.as_str(), 0, true),
        ] {
            let (mut pcm, rate) = synthesize(text, voice).unwrap();
            assert!(pcm.iter().any(|s| s.abs() > 0.01));
            if radio {
                radio_effect(&mut pcm, rate);
            }
            std::fs::write(directory.join(format!("{name}.wav")), wave(&pcm, rate)).unwrap();
        }
    }
}
