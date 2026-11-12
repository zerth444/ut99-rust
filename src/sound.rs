use std::{collections::HashMap, fs, io, path::Path, sync::Arc};

use crate::map::Vec3;
use crate::package::{Export, Package, invalid};
use crate::texture::{export_path, import_path};

const MIX_RATE: u32 = 44_100;

pub struct Wave {
    samples: Vec<f32>,
    rate: u32,
}

pub struct Emitter {
    position: Vec3,
    volume: f32,
    radius: f32,
    speed: f32,
    wave: Arc<Wave>,
}

pub fn load(
    package: &Package<'_>,
    actors: &[&Export],
    map_path: &Path,
) -> io::Result<Vec<Emitter>> {
    let Some(folder) = map_path.parent() else {
        return Ok(Vec::new());
    };
    let files: HashMap<_, _> = [folder.to_path_buf(), folder.join("../Sounds")]
        .iter()
        .filter_map(|folder| fs::read_dir(folder).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            path.extension()?
                .to_str()?
                .eq_ignore_ascii_case("uax")
                .then_some(())?;
            Some((path.file_stem()?.to_str()?.to_ascii_lowercase(), path))
        })
        .collect();
    let mut data = HashMap::new();
    let mut waves = HashMap::new();
    let mut emitters = Vec::new();
    for actor in actors {
        let mut props = package.properties(actor)?;
        let mut position = Vec3::default();
        let mut reference = 0;
        let mut volume = 128;
        let mut radius = 32;
        let mut pitch = 64;
        while let Some((name, mut value)) = props.next()? {
            match name {
                "Location" => position = Vec3::read(&mut value)?,
                "AmbientSound" => reference = value.compact()?,
                "SoundVolume" => volume = value.byte()?,
                "SoundRadius" => radius = value.byte()?,
                "SoundPitch" => pitch = value.byte()?,
                _ => {}
            }
        }
        if reference == 0 || volume == 0 {
            continue;
        }
        let wave = waves.entry(reference).or_insert_with(|| {
            if reference > 0 {
                return Wave::from_export(package, package.exports.get(reference as usize - 1)?)
                    .ok()
                    .map(Arc::new);
            }
            let (name, object) = import_path(package, reference)?;
            let bytes = data
                .entry(name.clone())
                .or_insert_with(|| files.get(&name).and_then(|path| fs::read(path).ok()));
            let sounds = Package::read(bytes.as_ref()?).ok()?;
            let sound = sounds.exports.iter().find(|item| {
                sounds.object_name(item.class) == Some("Sound")
                    && export_path(&sounds, item).as_ref() == Some(&object)
            })?;
            Wave::from_export(&sounds, sound).ok().map(Arc::new)
        });
        if let Some(wave) = wave {
            emitters.push(Emitter {
                position,
                volume: f32::from(volume) / 255.0,
                radius: (f32::from(radius) + 1.0) * 25.0,
                speed: (f32::from(pitch) / 64.0).max(0.05),
                wave: Arc::clone(wave),
            });
        }
    }
    Ok(emitters)
}

impl Wave {
    pub(crate) fn from_export(package: &Package<'_>, export: &Export) -> io::Result<Self> {
        let mut props = package.properties(export)?;
        while props.next()?.is_some() {}
        let mut native = props.reader;
        if !package.name(native.index()?)?.eq_ignore_ascii_case("wav") {
            return Err(invalid("Expected a WAV sound"));
        }
        if package.version >= 63 {
            native.u32()?;
        }
        let length = native.count(1)?;
        Self::read(native.take(length)?)
    }

    fn read(bytes: &[u8]) -> io::Result<Self> {
        if !bytes.starts_with(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
            return Err(invalid("Invalid RIFF/WAVE sound"));
        }
        let mut reader = crate::package::Reader::at(bytes, 12)?;
        let mut format = None;
        let mut samples = None;
        while reader.remaining.len() >= 8 {
            let id = reader.take(4)?;
            let length = reader.u32()? as usize;
            let data = reader.take(length)?;
            if id == b"fmt " {
                format = Some(data);
            }
            if id == b"data" {
                samples = Some(data);
            }
            if !length.is_multiple_of(2) && !reader.remaining.is_empty() {
                reader.take(1)?;
            }
        }
        let format = format.ok_or_else(|| invalid("Missing WAV format"))?;
        let mut format = crate::package::Reader::at(format, 0)?;
        let encoding = format.u16()?;
        let channels = usize::from(format.u16()?);
        let rate = format.u32()?;
        format.take(6)?;
        let bits = format.u16()?;
        if encoding != 1 || !(1..=2).contains(&channels) || rate == 0 || !matches!(bits, 8 | 16) {
            return Err(invalid("Unsupported WAV encoding"));
        }
        let bytes = usize::from(bits / 8);
        let samples: Vec<_> = samples
            .ok_or_else(|| invalid("Missing WAV samples"))?
            .chunks_exact(bytes * channels)
            .map(|frame| {
                frame
                    .chunks_exact(bytes)
                    .map(|sample| {
                        if bytes == 1 {
                            (f32::from(sample[0]) - 128.0) / 128.0
                        } else {
                            f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0
                        }
                    })
                    .sum::<f32>()
                    / channels as f32
            })
            .collect();
        if samples.is_empty() {
            return Err(invalid("Empty WAV sound"));
        }
        Ok(Self { samples, rate })
    }
}

pub struct Mixer {
    cursors: Vec<f64>,
    mix: Vec<[f32; 2]>,
    voices: Vec<(Arc<Wave>, f64)>,
    weapon_loop: Option<(Arc<Wave>, f64)>,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            cursors: Vec::new(),
            mix: Vec::new(),
            voices: Vec::new(),
            weapon_loop: None,
        }
    }

    pub fn fill(
        &mut self,
        output: &mut [i16],
        emitters: &[Emitter],
        listener: Vec3,
        right: Vec3,
        muted: bool,
    ) {
        self.cursors.resize(emitters.len(), 0.0);
        self.mix.resize(output.len() / 2, [0.0; 2]);
        self.mix.fill([0.0; 2]);
        for (emitter, cursor) in emitters.iter().zip(&mut self.cursors) {
            let offset = emitter.position - listener;
            let distance = offset.dot(offset).sqrt();
            let step =
                f64::from(emitter.wave.rate) * f64::from(emitter.speed) / f64::from(MIX_RATE);
            let count = emitter.wave.samples.len();
            if muted || distance >= emitter.radius {
                *cursor = (*cursor + step * self.mix.len() as f64) % count as f64;
                continue;
            }
            let pan = (offset.dot(right) / distance.max(1.0)).clamp(-1.0, 1.0);
            let gain = emitter.volume * (1.0 - distance / emitter.radius).powi(2) * 0.45;
            let left = gain * (0.5 - 0.5 * pan).sqrt();
            let right = gain * (0.5 + 0.5 * pan).sqrt();
            for frame in &mut self.mix {
                let index = *cursor as usize;
                let a = emitter.wave.samples[index];
                let b = emitter.wave.samples[(index + 1) % count];
                let sample = a + (b - a) * cursor.fract() as f32;
                frame[0] += sample * left;
                frame[1] += sample * right;
                *cursor = (*cursor + step) % count as f64;
            }
        }
        if let Some((wave, cursor)) = &mut self.weapon_loop {
            let step = f64::from(wave.rate) / f64::from(MIX_RATE);
            let count = wave.samples.len();
            for frame in &mut self.mix {
                let index = *cursor as usize;
                let a = wave.samples[index];
                let b = wave.samples[(index + 1) % count];
                let sample = (a + (b - a) * cursor.fract() as f32) * 0.45;
                if !muted {
                    frame[0] += sample;
                    frame[1] += sample;
                }
                *cursor = (*cursor + step) % count as f64;
            }
        }
        self.voices.retain_mut(|(wave, cursor)| {
            let step = f64::from(wave.rate) / f64::from(MIX_RATE);
            for frame in &mut self.mix {
                let index = *cursor as usize;
                let Some(&a) = wave.samples.get(index) else {
                    return false;
                };
                let b = wave.samples.get(index + 1).copied().unwrap_or(a);
                let sample = (a + (b - a) * cursor.fract() as f32) * 0.6;
                if !muted {
                    frame[0] += sample;
                    frame[1] += sample;
                }
                *cursor += step;
            }
            true
        });
        for (sample, mixed) in output.iter_mut().zip(self.mix.iter().flatten()) {
            *sample = (mixed.clamp(-1.0, 1.0) * 32767.0) as i16;
        }
    }
}

#[cfg(windows)]
pub mod device {
    use std::ptr::null_mut;

    use super::*;
    use windows_sys::Win32::Media::Audio::*;

    const SAMPLES: usize = 2048;
    const HEADER_SIZE: u32 = size_of::<WAVEHDR>() as u32;

    struct Buffer {
        samples: Box<[i16; SAMPLES]>,
        header: Box<WAVEHDR>,
        queued: bool,
    }

    pub struct Audio {
        device: HWAVEOUT,
        buffers: Vec<Buffer>,
        mixer: Mixer,
        music: Option<crate::music::player::Music>,
        music_samples: [i16; SAMPLES],
    }

    impl Audio {
        pub fn open(track: Option<&crate::music::Track>) -> Option<Self> {
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 2,
                nSamplesPerSec: MIX_RATE,
                nAvgBytesPerSec: MIX_RATE * 4,
                nBlockAlign: 4,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let mut device = null_mut();
            if unsafe { waveOutOpen(&mut device, WAVE_MAPPER, &format, 0, 0, CALLBACK_NULL) } != 0 {
                return None;
            }
            let mut audio = Self {
                device,
                buffers: Vec::new(),
                mixer: Mixer::new(),
                music: track.and_then(crate::music::player::Music::open),
                music_samples: [0; SAMPLES],
            };
            for _ in 0..3 {
                let mut samples = Box::new([0; SAMPLES]);
                let mut header = Box::new(WAVEHDR {
                    lpData: samples.as_mut_ptr().cast(),
                    dwBufferLength: (SAMPLES * size_of::<i16>()) as u32,
                    ..Default::default()
                });
                // Both allocations stay fixed until waveOutReset and UnprepareHeader.
                if unsafe { waveOutPrepareHeader(device, &mut *header, HEADER_SIZE) } != 0 {
                    return None;
                }
                audio.buffers.push(Buffer {
                    samples,
                    header,
                    queued: false,
                });
            }
            Some(audio)
        }

        pub fn music_playing(&self) -> bool {
            self.music.is_some()
        }

        pub fn play(&mut self, wave: Arc<Wave>) {
            if self.mixer.voices.len() == 24 {
                self.mixer.voices.remove(0);
            }
            self.mixer.voices.push((wave, 0.0));
        }

        pub fn set_weapon_loop(&mut self, wave: Option<&Arc<Wave>>) {
            if self
                .mixer
                .weapon_loop
                .as_ref()
                .map(|(wave, _)| Arc::as_ptr(wave))
                != wave.map(Arc::as_ptr)
            {
                self.mixer.weapon_loop = wave.map(|wave| (Arc::clone(wave), 0.0));
            }
        }

        pub fn update(
            &mut self,
            emitters: &[Emitter],
            listener: Vec3,
            right: Vec3,
            muted: bool,
            music_muted: bool,
        ) {
            for buffer in &mut self.buffers {
                let header = &raw mut *buffer.header;
                let flags = unsafe { std::ptr::read_volatile(&raw const (*header).dwFlags) };
                if !buffer.queued || flags & WHDR_DONE != 0 {
                    self.mixer
                        .fill(&mut *buffer.samples, emitters, listener, right, muted);
                    if !muted
                        && !music_muted
                        && let Some(music) = &mut self.music
                    {
                        let count = music.render(MIX_RATE, &mut self.music_samples);
                        for (output, &music) in
                            buffer.samples.iter_mut().zip(&self.music_samples[..count])
                        {
                            *output = output.saturating_add((f32::from(music) * 0.25) as i16);
                        }
                    }
                    buffer.queued = unsafe { waveOutWrite(self.device, header, HEADER_SIZE) } == 0;
                }
            }
        }
    }

    impl Drop for Audio {
        fn drop(&mut self) {
            unsafe {
                waveOutReset(self.device);
                for buffer in &mut self.buffers {
                    waveOutUnprepareHeader(self.device, &mut *buffer.header, HEADER_SIZE);
                }
                waveOutClose(self.device);
            }
        }
    }
}
