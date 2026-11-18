use std::{collections::HashMap, fs, io, path::Path, sync::Arc};

use crate::map::Vec3;
use crate::package::{Export, Package, invalid};
use crate::texture::{export_path, import_path};

const MIX_RATE: u32 = 44_100;
pub const CONTACT_SOUND_RADIUS: f32 = 1200.0;
pub const BLAST_SOUND_RADIUS: f32 = 2800.0;

pub struct Wave {
    samples: Vec<f32>,
    channels: usize,
    rate: u32,
}

#[derive(Default)]
pub struct SoundEvents(Vec<SoundEvent>);

struct SoundEvent {
    wave: Arc<Wave>,
    gain: f32,
    source: Option<SoundSource>,
}

#[derive(Clone, Copy)]
struct SoundSource {
    position: Vec3,
    radius: f32,
}

impl SoundSource {
    fn gains(self, listener: Vec3, right: Vec3, gain: f32) -> [f32; 2] {
        let offset = self.position - listener;
        let distance = offset.dot(offset).sqrt();
        if distance >= self.radius {
            return [0.0; 2];
        }
        let pan = (offset.dot(right) / distance.max(1.0)).clamp(-1.0, 1.0);
        let gain = gain * (1.0 - distance / self.radius).powi(2);
        [
            gain * (0.5 - 0.5 * pan).sqrt(),
            gain * (0.5 + 0.5 * pan).sqrt(),
        ]
    }
}

impl SoundEvents {
    pub(crate) fn append(&mut self, mut more: Self) {
        self.0.append(&mut more.0);
    }

    pub fn push(&mut self, wave: Arc<Wave>) {
        self.0.push(SoundEvent {
            wave,
            gain: 1.0,
            source: None,
        });
    }

    pub fn extend(&mut self, waves: impl IntoIterator<Item = Arc<Wave>>) {
        waves.into_iter().for_each(|wave| self.push(wave));
    }

    pub fn extend_at(
        &mut self,
        waves: impl IntoIterator<Item = Arc<Wave>>,
        position: Vec3,
        radius: f32,
        gain: f32,
    ) {
        debug_assert!(radius.is_finite() && radius > 0.0);
        self.0.extend(waves.into_iter().map(|wave| SoundEvent {
            wave,
            gain,
            source: Some(SoundSource { position, radius }),
        }));
    }
}

#[cfg(test)]
impl SoundEvents {
    pub fn waves(&self) -> impl DoubleEndedIterator<Item = &Arc<Wave>> {
        self.0.iter().map(|event| &event.wave)
    }

    pub fn position_of(&self, wave: &Arc<Wave>) -> Option<Vec3> {
        self.0
            .iter()
            .find(|event| Arc::ptr_eq(&event.wave, wave))?
            .source
            .map(|source| source.position)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

pub struct Emitter {
    position: Vec3,
    volume: f32,
    radius: f32,
    speed: f32,
    wave: Arc<Wave>,
}

impl Emitter {
    fn mix(
        &self,
        output: &mut [[f32; 2]],
        cursor: &mut f64,
        listener: Vec3,
        right: Vec3,
        muted: bool,
    ) {
        let [left, right] = SoundSource {
            position: self.position,
            radius: self.radius,
        }
        .gains(listener, right, self.volume * 0.45);
        let step = f64::from(self.wave.rate) * f64::from(self.speed) / f64::from(MIX_RATE);
        let count = self.wave.frames();
        if muted || (left == 0.0 && right == 0.0) {
            *cursor = (*cursor + step * output.len() as f64) % count as f64;
            return;
        }
        for frame in output {
            let [a, b] = self.wave.sample(*cursor, count, true);
            let sample = (a + b) * 0.5;
            frame[0] += sample * left;
            frame[1] += sample * right;
            *cursor = (*cursor + step) % count as f64;
        }
    }
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
        let mut header = crate::package::Reader::at(bytes, 4)?;
        let length = header.u32()? as usize;
        let payload = header.take(length)?;
        let mut reader = crate::package::Reader::at(payload, 4)?;
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
        if !reader.remaining.is_empty() {
            return Err(invalid("Incomplete WAV chunk header"));
        }
        let format = format.ok_or_else(|| invalid("Missing WAV format"))?;
        let mut format = crate::package::Reader::at(format, 0)?;
        let encoding = format.u16()?;
        let channels = usize::from(format.u16()?);
        let rate = format.u32()?;
        format.u32()?; // Average bytes per second is redundant for PCM.
        let block_align = usize::from(format.u16()?);
        let bits = format.u16()?;
        if encoding != 1 || !(1..=2).contains(&channels) || rate == 0 || !matches!(bits, 8 | 16) {
            return Err(invalid("Unsupported WAV encoding"));
        }
        let bytes = usize::from(bits / 8);
        let samples = samples.ok_or_else(|| invalid("Missing WAV samples"))?;
        if block_align != bytes * channels || !samples.len().is_multiple_of(block_align) {
            return Err(invalid("Invalid WAV frame alignment"));
        }
        let samples: Vec<_> = samples
            .chunks_exact(bytes)
            .map(|sample| {
                if bytes == 1 {
                    (f32::from(sample[0]) - 128.0) / 128.0
                } else {
                    f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0
                }
            })
            .collect();
        if samples.is_empty() {
            return Err(invalid("Empty WAV sound"));
        }
        Ok(Self {
            samples,
            channels,
            rate,
        })
    }

    fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    fn sample(&self, cursor: f64, frames: usize, looping: bool) -> [f32; 2] {
        let current = cursor as usize;
        let next = if current + 1 < frames {
            current + 1
        } else if looping {
            0
        } else {
            current
        };
        let fraction = cursor.fract() as f32;
        let current = current * self.channels;
        let next = next * self.channels;
        let interpolate = |channel: usize| {
            let a = self.samples[current + channel];
            let b = self.samples[next + channel];
            a + (b - a) * fraction
        };
        let left = interpolate(0);
        [
            left,
            if self.channels == 1 {
                left
            } else {
                interpolate(1)
            },
        ]
    }
}

pub struct Mixer {
    cursors: Vec<f64>,
    mix: Vec<[f32; 2]>,
    voices: Vec<Voice>,
    weapon_loop: Option<(Arc<Wave>, f64)>,
    world_loops: HashMap<LoopId, MovingEmitter>,
}

struct MovingEmitter {
    emitter: Emitter,
    cursor: f64,
    active: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum LoopId {
    Fighter(usize),
    Rocket(u64),
}

pub(crate) struct WorldLoop<'a> {
    pub id: LoopId,
    pub wave: &'a Arc<Wave>,
    pub position: Vec3,
    pub radius: f32,
    pub gain: f32,
}

struct Voice {
    event: SoundEvent,
    cursor: f64,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            cursors: Vec::new(),
            mix: Vec::new(),
            voices: Vec::new(),
            weapon_loop: None,
            world_loops: HashMap::new(),
        }
    }

    fn set_world_loops<'a>(&mut self, loops: impl IntoIterator<Item = WorldLoop<'a>>) {
        for voice in self.world_loops.values_mut() {
            voice.active = false;
        }
        for sound in loops {
            let voice = self
                .world_loops
                .entry(sound.id)
                .or_insert_with(|| MovingEmitter {
                    emitter: Emitter {
                        wave: Arc::clone(sound.wave),
                        position: sound.position,
                        radius: sound.radius,
                        volume: sound.gain,
                        speed: 1.0,
                    },
                    cursor: 0.0,
                    active: true,
                });
            if !Arc::ptr_eq(&voice.emitter.wave, sound.wave) {
                voice.emitter.wave = Arc::clone(sound.wave);
                voice.cursor = 0.0;
            }
            voice.emitter.volume = sound.gain;
            voice.emitter.position = sound.position;
            voice.emitter.radius = sound.radius;
            voice.active = true;
        }
        self.world_loops.retain(|_, voice| voice.active);
    }

    fn play(&mut self, event: SoundEvent) {
        if !event.gain.is_finite() || event.gain <= 0.0 {
            return;
        }
        if self.voices.len() == 24 {
            if let Some(index) = self
                .voices
                .iter()
                .position(|voice| voice.event.source.is_some())
            {
                self.voices.remove(index);
            } else if event.source.is_none() {
                self.voices.remove(0);
            } else {
                return;
            }
        }
        self.voices.push(Voice { event, cursor: 0.0 });
    }

    fn mix_effects(
        &mut self,
        frames: usize,
        emitters: &[Emitter],
        listener: Vec3,
        right: Vec3,
        muted: bool,
    ) {
        self.cursors.resize(emitters.len(), 0.0);
        self.mix.resize(frames, [0.0; 2]);
        self.mix.fill([0.0; 2]);
        for (emitter, cursor) in emitters.iter().zip(&mut self.cursors) {
            emitter.mix(&mut self.mix, cursor, listener, right, muted);
        }
        if let Some((wave, cursor)) = &mut self.weapon_loop {
            let step = f64::from(wave.rate) / f64::from(MIX_RATE);
            let count = wave.frames();
            if muted {
                *cursor = (*cursor + step * frames as f64) % count as f64;
            } else {
                for frame in &mut self.mix {
                    let [left, right] = wave.sample(*cursor, count, true);
                    frame[0] += left * 0.45;
                    frame[1] += right * 0.45;
                    *cursor = (*cursor + step) % count as f64;
                }
            }
        }
        for voice in self.world_loops.values_mut() {
            voice
                .emitter
                .mix(&mut self.mix, &mut voice.cursor, listener, right, muted);
        }
        self.voices.retain_mut(|Voice { event, cursor }| {
            let wave = &event.wave;
            let count = wave.frames();
            let step = f64::from(wave.rate) / f64::from(MIX_RATE);
            let gain = event.gain * 0.6;
            let [left, right] = event
                .source
                .map_or([gain; 2], |source| source.gains(listener, right, gain));
            if muted || (left == 0.0 && right == 0.0) {
                *cursor += step * frames as f64;
                return (*cursor as usize) < count;
            }
            for frame in &mut self.mix {
                if (*cursor as usize) >= count {
                    return false;
                }
                let [a, b] = wave.sample(*cursor, count, false);
                if event.source.is_some() {
                    let sample = (a + b) * 0.5;
                    frame[0] += sample * left;
                    frame[1] += sample * right;
                } else {
                    frame[0] += a * left;
                    frame[1] += b * right;
                }
                *cursor += step;
            }
            (*cursor as usize) < count
        });
    }

    fn write_pcm(&self, output: &mut [i16], effects_gain: f32, music: &[i16], music_gain: f32) {
        debug_assert_eq!(output.len(), self.mix.len() * 2);
        for (index, sample) in output.iter_mut().enumerate() {
            let effects = self.mix[index / 2][index % 2] * effects_gain;
            let music =
                f32::from(music.get(index).copied().unwrap_or(0)) * (0.25 / 32768.0) * music_gain;
            *sample = ((effects + music).clamp(-1.0, 1.0) * 32767.0) as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(frames: usize) -> Arc<Wave> {
        Arc::new(Wave {
            samples: vec![1.0; frames],
            channels: 1,
            rate: MIX_RATE,
        })
    }

    fn play(mixer: &mut Mixer, wave: &Arc<Wave>, gain: f32) {
        mixer.play(SoundEvent {
            wave: wave.clone(),
            gain,
            source: None,
        });
    }

    #[test]
    fn voice_gain_is_applied_per_playback_and_survives_buffer_boundaries() {
        let mut mixer = Mixer::new();
        let wave = wave(3);
        play(&mut mixer, &wave, 0.25);
        play(&mut mixer, &wave, 0.5);

        let mut next_frame = |muted| {
            let mut frame = [0; 2];
            mixer.mix_effects(1, &[], Vec3::default(), Vec3::default(), muted);
            mixer.write_pcm(&mut frame, 1.0, &[], 0.0);
            frame
        };
        let frame = next_frame(false);
        let expected = (0.45_f32 * 32767.0) as i16;
        assert!((frame[0] - expected).abs() <= 1);
        assert_eq!(frame[0], frame[1]);
        assert_eq!(next_frame(false), frame);
        assert_eq!(next_frame(true), [0; 2]);
        assert_eq!(next_frame(false), [0; 2]);
    }

    #[test]
    fn effects_and_music_share_one_output_clamp() {
        let mut mixer = Mixer::new();
        let wave = wave(2);
        play(&mut mixer, &wave, 1.0);
        play(&mut mixer, &wave, 1.0);
        mixer.mix_effects(2, &[], Vec3::default(), Vec3::default(), false);
        let mut output = [0; 4];
        mixer.write_pcm(&mut output, 0.5, &[], 0.0);
        assert_eq!(output, [(0.6_f32 * 32767.0) as i16; 4]);
        mixer.write_pcm(&mut output, 1.0, &[i16::MIN, i16::MAX], 1.0);
        let left = (0.95_f32 * 32767.0) as i16;
        assert!((output[0] - left).abs() <= 1);
        assert_eq!(output[1..], [i16::MAX; 3]);
    }

    #[test]
    fn world_voice_tracks_listener_and_expires_outside_its_radius() {
        let mut mixer = Mixer::new();
        let listener = Vec3::default();
        let right = Vec3(1.0, 0.0, 0.0);
        let mut events = SoundEvents::default();
        events.extend_at(Some(wave(3)), Vec3(10.0, 0.0, 0.0), 20.0, 1.0);
        mixer.play(events.0.pop().unwrap());

        mixer.mix_effects(1, &[], listener, right, false);
        assert_eq!(mixer.mix[0], [0.0, 0.15]);
        mixer.mix_effects(1, &[], listener, right * -1.0, false);
        assert_eq!(mixer.mix[0], [0.15, 0.0]);
        mixer.mix_effects(1, &[], Vec3(-30.0, 0.0, 0.0), right, false);
        assert_eq!(mixer.mix[0], [0.0; 2]);
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn moving_loop_preserves_phase_across_pan_updates_and_stops_when_removed() {
        let mut mixer = Mixer::new();
        let wave = Arc::new(Wave {
            samples: vec![1.0, -0.5],
            channels: 1,
            rate: MIX_RATE,
        });
        let id = LoopId::Rocket(1);
        let sound = |x| WorldLoop {
            id,
            wave: &wave,
            position: Vec3(x, 0.0, 0.0),
            radius: 20.0,
            gain: 1.0,
        };
        let right = Vec3(1.0, 0.0, 0.0);
        mixer.set_world_loops([sound(10.0)]);
        mixer.mix_effects(1, &[], Vec3::default(), right, false);
        assert_eq!(mixer.mix[0], [0.0, 0.1125]);
        mixer.set_world_loops([sound(-10.0)]);
        mixer.mix_effects(1, &[], Vec3::default(), right, false);
        assert_eq!(mixer.mix[0], [-0.05625, 0.0]);
        mixer.set_world_loops(std::iter::empty());
        mixer.mix_effects(2, &[], Vec3::default(), right, false);
        assert_eq!(mixer.mix, [[0.0, 0.0]; 2]);
    }

    #[test]
    fn stereo_samples_keep_their_channels_for_local_playback() {
        let wave = Arc::new(Wave {
            samples: vec![1.0, 0.0, 0.0, -1.0],
            channels: 2,
            rate: MIX_RATE / 2,
        });
        let listener = Vec3::default();
        let right = Vec3(1.0, 0.0, 0.0);
        let mut mixer = Mixer::new();
        play(&mut mixer, &wave, 1.0);
        mixer.mix_effects(3, &[], listener, right, false);
        assert_eq!(mixer.mix, [[0.6, 0.0], [0.3, -0.3], [0.0, -0.6]]);

        let mut mixer = Mixer::new();
        let mut events = SoundEvents::default();
        events.extend_at(Some(wave), Vec3(10.0, 0.0, 0.0), 20.0, 1.0);
        mixer.play(events.0.pop().unwrap());
        mixer.mix_effects(3, &[], listener, right, false);
        assert_eq!(mixer.mix, [[0.0, 0.075], [0.0, 0.0], [0.0, -0.075]]);
    }

    #[test]
    fn world_voice_cannot_evict_a_full_queue_of_local_sounds() {
        let mut mixer = Mixer::new();
        let wave = wave(1);
        for _ in 0..24 {
            play(&mut mixer, &wave, 1.0);
        }
        let mut events = SoundEvents::default();
        events.extend_at(Some(wave), Vec3::default(), 20.0, 1.0);
        mixer.play(events.0.pop().unwrap());
        assert_eq!(mixer.voices.len(), 24);
        assert!(
            mixer
                .voices
                .iter()
                .all(|voice| voice.event.source.is_none())
        );
    }

    #[test]
    fn wav_decoder_handles_padding_and_rejects_truncated_pcm_frames() {
        fn pcm_wave(samples: &[u8]) -> Vec<u8> {
            let mut wav = b"RIFF\0\0\0\0WAVEJUNK\x01\0\0\0\xFF\0fmt \x10\0\0\0".to_vec();
            wav.extend([1, 0, 2, 0]); // Stereo PCM.
            wav.extend(MIX_RATE.to_le_bytes());
            wav.extend((MIX_RATE * 4).to_le_bytes());
            wav.extend([4, 0, 16, 0]);
            wav.extend(b"data");
            wav.extend((samples.len() as u32).to_le_bytes());
            wav.extend(samples);
            let size = (wav.len() - 8) as u32;
            wav[4..8].copy_from_slice(&size.to_le_bytes());
            wav
        }
        let full = pcm_wave(&[0, 0x40, 0, 0x40]);
        let decoded = Wave::read(&full).unwrap();
        assert_eq!(decoded.rate, MIX_RATE);
        assert_eq!(decoded.frames(), 1);
        assert_eq!(decoded.sample(0.0, 1, false), [0.5; 2]);
        assert!(Wave::read(&full[..full.len() - 1]).is_err());
        assert!(Wave::read(&pcm_wave(&[0, 0x40, 0, 0x40, 0xFF, 0xFF])).is_err());
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
        effects_gain: f32,
        music_gain: f32,
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
                effects_gain: 1.0,
                music_gain: 1.0,
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

        pub fn set_volume(&mut self, effects: u32, music: u32) {
            self.effects_gain = effects as f32 / 100.0;
            self.music_gain = music as f32 / 100.0;
        }

        pub fn set_track(&mut self, track: Option<&crate::music::Track>) {
            unsafe { waveOutReset(self.device) };
            for buffer in &mut self.buffers {
                buffer.queued = false;
            }
            self.mixer = Mixer::new();
            self.music = track.and_then(crate::music::player::Music::open);
        }

        pub fn play(&mut self, wave: Arc<Wave>) {
            self.play_with_gain(wave, 1.0);
        }

        pub fn play_with_gain(&mut self, wave: Arc<Wave>, gain: f32) {
            self.mixer.play(SoundEvent {
                wave,
                gain,
                source: None,
            });
        }

        pub fn play_events(&mut self, events: SoundEvents) {
            for event in events.0 {
                self.mixer.play(event);
            }
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

        pub(crate) fn set_world_loops<'a>(
            &mut self,
            loops: impl IntoIterator<Item = WorldLoop<'a>>,
        ) {
            self.mixer.set_world_loops(loops);
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
                    self.mixer.mix_effects(
                        buffer.samples.len() / 2,
                        emitters,
                        listener,
                        right,
                        muted,
                    );
                    let music_count = if !muted
                        && !music_muted
                        && let Some(music) = &mut self.music
                    {
                        music.render(MIX_RATE, &mut self.music_samples)
                    } else {
                        0
                    };
                    self.mixer.write_pcm(
                        &mut *buffer.samples,
                        self.effects_gain,
                        &self.music_samples[..music_count],
                        self.music_gain,
                    );
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
