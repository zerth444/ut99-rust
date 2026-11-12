use std::{fs, io, path::Path};

pub const RESOLUTIONS: [(u32, u32); 6] = [
    (800, 600),
    (1024, 768),
    (1120, 680),
    (1280, 720),
    (1600, 900),
    (1920, 1080),
];

pub const BINDINGS: [(&str, u16); 8] = [
    ("FORWARD", b'W' as u16),
    ("BACKWARD", b'S' as u16),
    ("STRAFE LEFT", b'A' as u16),
    ("STRAFE RIGHT", b'D' as u16),
    ("JUMP / SWIM UP", 0x20),
    ("WALK", 0x10),
    ("FIRE", 0x01),
    ("ALT FIRE", 0x02),
];

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub fullscreen: bool,
    pub resolution: usize,
    pub vsync: bool,
    pub intro: bool,
    pub show_fps: bool,
    pub field_of_view: u32,
    pub hud_scale: u32,
    pub music_volume: u32,
    pub effects_volume: u32,
    pub sensitivity: u32,
    pub invert_mouse: bool,
    pub bindings: [u16; BINDINGS.len()],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen: false,
            resolution: 2,
            vsync: true,
            intro: true,
            show_fps: false,
            field_of_view: 90,
            hud_scale: 100,
            music_volume: 100,
            effects_volume: 100,
            sensitivity: 100,
            invert_mouse: false,
            bindings: BINDINGS.map(|(_, key)| key),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        Ok(Self::parse(&text))
    }

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (name, value) in text.lines().filter_map(|line| line.split_once('=')) {
            let Ok(value) = value.trim().parse::<u32>() else {
                continue;
            };
            match name.trim() {
                "fullscreen" => settings.fullscreen = value != 0,
                "resolution" => settings.resolution = (value as usize).min(RESOLUTIONS.len() - 1),
                "vsync" => settings.vsync = value != 0,
                "intro" => settings.intro = value != 0,
                "show_fps" => settings.show_fps = value != 0,
                "fov" => settings.field_of_view = value.clamp(75, 120),
                "hud_scale" => settings.hud_scale = value.clamp(50, 150),
                "music" => settings.music_volume = value.min(100),
                "effects" => settings.effects_volume = value.min(100),
                "sensitivity" => settings.sensitivity = value.clamp(10, 250),
                "invert_mouse" => settings.invert_mouse = value != 0,
                name if name.starts_with("bind") => {
                    if let Ok(index) = name[4..].parse::<usize>()
                        && let Ok(key) = u16::try_from(value)
                    {
                        settings.bind(index, key);
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn bind(&mut self, index: usize, key: u16) -> bool {
        let mouse = matches!(key, 1 | 2 | 4 | 5 | 6);
        if index >= self.bindings.len()
            || !(1..255).contains(&key)
            || matches!(key, 0x09 | 0x1b | 0x25..=0x28 | 0x30..=0x39 | 0x70..=0x7b)
            || (mouse && index < 6)
        {
            return false;
        }
        if let Some(other) = self.bindings.iter().position(|&bound| bound == key) {
            if self.bindings[index] < 7 && other < 6 {
                return other == index;
            }
            self.bindings[other] = self.bindings[index];
        }
        self.bindings[index] = key;
        true
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = format!(
            "fullscreen={}\nresolution={}\nvsync={}\nintro={}\nshow_fps={}\nfov={}\nhud_scale={}\nmusic={}\neffects={}\nsensitivity={}\ninvert_mouse={}\n",
            u8::from(self.fullscreen),
            self.resolution,
            u8::from(self.vsync),
            u8::from(self.intro),
            u8::from(self.show_fps),
            self.field_of_view,
            self.hud_scale,
            self.music_volume,
            self.effects_volume,
            self.sensitivity,
            u8::from(self.invert_mouse),
        );
        for (index, key) in self.bindings.iter().enumerate() {
            use std::fmt::Write;
            let _ = writeln!(text, "bind{index}={key}");
        }
        let pending = path.with_extension("ini.tmp");
        fs::write(&pending, text)?;
        fs::rename(pending, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_bounds_invalid_values_and_keeps_escape_available() {
        let config = Settings::parse(
            "resolution=999\nmusic=300\nsensitivity=0\nfov=0\nbind0=27\nbind6=69\n",
        );
        assert_eq!(config.resolution, RESOLUTIONS.len() - 1);
        assert_eq!(
            (
                config.music_volume,
                config.sensitivity,
                config.field_of_view
            ),
            (100, 10, 75)
        );
        assert_eq!(config.bindings[0], b'W' as u16);
        assert_eq!(config.bindings[6], b'E' as u16);
    }

    #[test]
    fn writes_and_replaces_a_local_preferences_file() {
        let name = format!("ut99-preferences-{}.ini", std::process::id());
        let path = std::env::temp_dir().join(name);
        let mut settings = Settings::default();
        settings.save(&path).unwrap();
        settings.music_volume = 20;
        settings.fullscreen = true;
        settings.intro = false;
        settings.bind(0, b'I' as u16);
        settings.bind(6, 2);
        settings.save(&path).unwrap();
        let saved = Settings::load(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(settings, saved);
    }
}
