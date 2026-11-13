mod font;
mod intro;

use std::{fs, io, path::Path, sync::Arc};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    hud::Hud,
    map::Map,
    package::Package,
    render::{Camera, Frame},
    settings::{BINDINGS, RESOLUTIONS, Settings},
    sound::Wave,
    texture::Texture,
};
use font::MenuFont;
pub use intro::Intro;

pub struct Frontend {
    pub intro: Option<Intro>,
    pub logo: Option<(Map, Camera)>,
    art: MenuArt,
}

#[derive(Default)]
struct MenuArt {
    desktop: Option<[Texture; 12]>,
    bar: Option<Texture>,
    bar_ends: [Option<Texture>; 2],
    window: Option<Texture>,
    panel: Option<Texture>,
    highlight: Option<Texture>,
    click: Option<Arc<Wave>>,
    font: Option<MenuFont>,
}

impl Frontend {
    pub fn load(map_path: &Path, play_intro: bool) -> Self {
        let maps = map_path.parent().unwrap_or_else(|| Path::new("."));
        let optional = |path: &Path, error: io::Error| {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!("{}: {error}", path.display());
            }
        };
        let city = maps.join("CityIntro.unr");
        let intro = play_intro
            .then(|| {
                Intro::load(&city)
                    .map_err(|error| optional(&city, error))
                    .ok()
            })
            .flatten();
        let logo_path = maps.join("UT-Logo-Map.unr");
        let load_logo = || -> io::Result<_> {
            let bytes = fs::read(&logo_path)?;
            let package = Package::read(&bytes)?;
            let map = Map::load(&package, Some(&logo_path))?;
            let camera = intro::logo_camera(&package, &map)?;
            Ok((map, camera))
        };
        let logo = load_logo()
            .map_err(|error| optional(&logo_path, error))
            .ok();
        Self {
            intro,
            logo,
            art: MenuArt::load(
                &maps.join("../System/UMenu.u"),
                &maps.join("../Textures/UWindowFonts.utx"),
            ),
        }
    }

    pub fn play_menu_sound(&self, audio: &mut crate::sound::device::Audio) {
        if let Some(sound) = &self.art.click {
            audio.play(Arc::clone(sound));
        }
    }

    pub fn has_desktop(&self) -> bool {
        self.art.desktop.is_some()
    }
}

impl MenuArt {
    fn load(path: &Path, font_path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        let Ok(package) = Package::read(&bytes) else {
            return Self::default();
        };
        let export = |name: &str, class: &str| {
            package.exports.iter().find(|item| {
                package.names[item.name] == name && package.object_name(item.class) == Some(class)
            })
        };
        let texture = |name: &str| Texture::read(&package, export(name, "Texture")?).ok();
        let desktop = (0..12)
            .map(|index| texture(&format!("Bg{}{}", index % 4 + 1, index / 4 + 1)))
            .collect::<Option<Vec<_>>>()
            .and_then(|tiles| tiles.try_into().ok());
        Self {
            desktop,
            bar: texture("BlueBarTile"),
            bar_ends: [texture("BlueBarL"), texture("BlueBarWin")],
            window: texture("BlueActiveFrame"),
            panel: texture("BlueClientArea"),
            highlight: texture("BlueMenuHM"),
            font: MenuFont::load(font_path).ok(),
            click: export("LittleSelect", "Sound")
                .and_then(|item| Wave::from_export(&package, item).ok())
                .map(Arc::new),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Play,
    Quit,
    Changed,
}

#[derive(Default)]
pub struct Menu {
    pub resume: bool,
    preferences: bool,
    tab: usize,
    selected: usize,
    hover: Option<usize>,
    waiting: Option<usize>,
    pub status: String,
}

const TABS: [&str; 3] = ["VIDEO", "AUDIO", "CONTROLS"];
const VIDEO: [&str; 7] = [
    "DISPLAY MODE",
    "WINDOW SIZE",
    "VERTICAL SYNC",
    "FIELD OF VIEW",
    "HUD SCALE",
    "SHOW FPS",
    "PLAY INTRO",
];
const AUDIO: [&str; 2] = ["MUSIC VOLUME", "SOUND VOLUME"];
const CONTROLS: [&str; 10] = [
    "MOUSE SENSITIVITY",
    "INVERT MOUSE",
    "FORWARD",
    "BACKWARD",
    "STRAFE LEFT",
    "STRAFE RIGHT",
    "JUMP / SWIM UP",
    "WALK",
    "FIRE",
    "ALT FIRE",
];

impl Menu {
    pub fn pointer(&mut self, size: [i32; 2], x: i32, y: i32) {
        self.hover = Self::top_item(Layout::new(size).local(x, y));
    }

    fn top_item([x, y]: [i32; 2]) -> Option<usize> {
        ((0..16).contains(&y) && (16..232).contains(&x)).then(|| ((x - 16) / 72) as usize)
    }

    fn rows(&self) -> &[&str] {
        match self.tab {
            0 => &VIDEO,
            1 => &AUDIO,
            _ => &CONTROLS,
        }
    }

    fn main(&mut self, index: usize) -> Action {
        match index {
            0 => Action::Play,
            1 => {
                self.preferences = true;
                self.selected = 0;
                Action::None
            }
            _ => Action::Quit,
        }
    }

    pub fn key(&mut self, key: u16, settings: &mut Settings) -> Action {
        if let Some(index) = self.waiting.take() {
            self.status.clear();
            if key == VK_ESCAPE {
                return Action::None;
            }
            if settings.bind(index, key) {
                return Action::Changed;
            }
            self.status = "KEY IS RESERVED".into();
            return Action::None;
        }
        if self.preferences {
            match key {
                VK_ESCAPE => {
                    self.preferences = false;
                    self.selected = 1;
                }
                VK_TAB => {
                    self.tab = (self.tab + 1) % TABS.len();
                    self.selected = 0;
                }
                VK_UP => {
                    self.selected = (self.selected + self.rows().len() - 1) % self.rows().len()
                }
                VK_DOWN => self.selected = (self.selected + 1) % self.rows().len(),
                VK_LEFT => return self.adjust(-1, settings),
                VK_RIGHT | VK_RETURN | VK_SPACE => return self.adjust(1, settings),
                _ => {}
            }
            return Action::None;
        }
        match key {
            VK_LEFT | VK_UP => self.selected = (self.selected + 2) % 3,
            VK_RIGHT | VK_DOWN | VK_TAB => self.selected = (self.selected + 1) % 3,
            VK_RETURN | VK_SPACE => return self.main(self.selected),
            VK_ESCAPE if self.resume => return Action::Play,
            _ => {}
        }
        Action::None
    }

    pub fn click(
        &mut self,
        size: [i32; 2],
        x: i32,
        y: i32,
        button: u16,
        settings: &mut Settings,
    ) -> Action {
        if self.waiting.is_some() {
            return self.key(button, settings);
        }
        if button != VK_LBUTTON {
            return Action::None;
        }
        let ui = Layout::new(size);
        let [x, y] = ui.local(x, y);
        if !self.preferences {
            if let Some(index) = Self::top_item([x, y]) {
                self.selected = index;
                return self.main(self.selected);
            }
        } else {
            let [x, y] = [x - ui.panel[0], y - ui.panel[1]];
            if (0..16).contains(&y) && (400..420).contains(&x) {
                self.preferences = false;
                self.selected = 1;
                return Action::None;
            }
            let y = y - 16;
            if (0..31).contains(&y) && (0..420).contains(&x) {
                if x >= 336 {
                    self.preferences = false;
                    self.selected = 1;
                } else {
                    self.tab = (x / 112) as usize;
                    self.selected = 0;
                }
            } else if (0..420).contains(&x) && y >= 42 {
                let row = ((y - 42) / 26) as usize;
                if row < self.rows().len() {
                    self.selected = row;
                    return self.adjust(if (282..313).contains(&x) { -1 } else { 1 }, settings);
                }
            }
        }
        Action::None
    }

    fn adjust(&mut self, step: i32, settings: &mut Settings) -> Action {
        self.status.clear();
        let adjust = |value: &mut u32, by: i32, min: i32, max: i32| {
            *value = (*value as i32 + step * by).clamp(min, max) as u32;
        };
        match (self.tab, self.selected) {
            (0, 0) => settings.fullscreen = !settings.fullscreen,
            (0, 1) => {
                settings.resolution = (settings.resolution as i32 + step)
                    .rem_euclid(RESOLUTIONS.len() as i32)
                    as usize
            }
            (0, 2) => settings.vsync = !settings.vsync,
            (0, 3) => adjust(&mut settings.field_of_view, 5, 75, 120),
            (0, 4) => adjust(&mut settings.hud_scale, 10, 50, 150),
            (0, 5) => settings.show_fps = !settings.show_fps,
            (0, 6) => settings.intro = !settings.intro,
            (1, 0) => adjust(&mut settings.music_volume, 10, 0, 100),
            (1, 1) => adjust(&mut settings.effects_volume, 10, 0, 100),
            (2, 0) => adjust(&mut settings.sensitivity, 10, 10, 250),
            (2, 1) => settings.invert_mouse = !settings.invert_mouse,
            (2, row) => {
                self.waiting = Some(row - 2);
                self.status = format!("PRESS A KEY FOR {}  /  ESC CANCELS", BINDINGS[row - 2].0);
                return Action::None;
            }
            _ => return Action::None,
        }
        Action::Changed
    }

    pub fn draw(&self, frontend: &Frontend, frame: &mut Frame, font: &Hud, settings: &Settings) {
        let ui = Layout::new([frame.width as i32, frame.height as i32]);
        let art = &frontend.art;
        if let Some(tiles) = &art.desktop {
            let size = (frame.width as f32 / 4.0).min(frame.height as f32 / 3.0);
            let left = (frame.width as f32 - 4.0 * size) * 0.5;
            let top = (frame.height as f32 - 3.0 * size) * 0.5;
            for (index, tile) in tiles.iter().enumerate() {
                let x = left + (index % 4) as f32 * size;
                let y = top + (index / 4) as f32 * size;
                let (w, h) = tile.size();
                frame.tile(
                    tile.image(0.0),
                    [
                        x.round() as i32,
                        y.round() as i32,
                        ((x + size).round() - x.round()) as i32,
                        ((y + size).round() - y.round()) as i32,
                    ],
                    [0.0, 0.0, w, h],
                    [1.0; 3],
                );
            }
        }
        let label = |frame: &mut Frame, x, y, text: &str| {
            ui.label(frame, font, art.font.as_ref(), x, y, text)
        };
        let width = (frame.width as f32 / ui.scale).ceil() as i32;
        ui.tile(
            frame,
            art.bar.as_ref(),
            [0, 0, width, 16],
            [0.24, 0.27, 0.31],
        );
        for (index, texture) in art.bar_ends.iter().enumerate() {
            if let Some(texture) = texture {
                ui.tile(
                    frame,
                    Some(texture),
                    [if index == 0 { 0 } else { width - 16 }, 0, 16, 16],
                    [0.24, 0.27, 0.31],
                );
            }
        }
        for (index, label) in [
            if self.resume { "Resume" } else { "Play" },
            "Options",
            "Exit",
        ]
        .into_iter()
        .enumerate()
        {
            let x = 16 + index as i32 * 72;
            if (self.selected == index || self.hover == Some(index)) && !self.preferences {
                ui.tile(
                    frame,
                    art.highlight.as_ref(),
                    [x, 0, 72, 16],
                    [0.25, 0.38, 0.55],
                );
            }
            ui.label(frame, font, art.font.as_ref(), x + 7, 1, label);
        }
        if frontend.logo.is_none() && !frontend.has_desktop() {
            label(frame, (width - 160) / 2, 65, "UNREAL TOURNAMENT");
        }
        if !self.preferences {
            if let Some(index) = self.hover {
                let bottom = (frame.height as f32 / ui.scale) as i32 - 16;
                ui.tile(
                    frame,
                    art.bar.as_ref(),
                    [0, bottom, width, 16],
                    [0.24, 0.27, 0.31],
                );
                label(
                    frame,
                    7,
                    bottom + 1,
                    [
                        if self.resume {
                            "Return to the match."
                        } else {
                            "Start a practice session on Deck16][."
                        },
                        "Change video, audio and controls.",
                        "Exit Unreal Tournament.",
                    ][index],
                );
            }
            return;
        }
        let [x, y] = ui.panel;
        ui.tile(
            frame,
            art.panel.as_ref(),
            [x, y, 420, 338],
            [0.18, 0.20, 0.22],
        );
        if let Some(texture) = &art.window {
            for (rect, source) in [
                ([x, y, 2, 16], [0.0, 0.0, 2.0, 16.0]),
                ([x + 2, y, 416, 16], [32.0, 0.0, 1.0, 16.0]),
                ([x + 418, y, 2, 16], [126.0, 0.0, 2.0, 16.0]),
                ([x + 406, y + 3, 11, 11], [4.0, 32.0, 11.0, 11.0]),
                ([x, y + 16, 2, 319], [0.0, 32.0, 2.0, 1.0]),
                ([x + 418, y + 16, 2, 319], [126.0, 32.0, 2.0, 1.0]),
                ([x, y + 335, 420, 3], [32.0, 125.0, 1.0, 3.0]),
            ] {
                frame.tile(
                    texture.image(0.0),
                    rect.map(|value| (value as f32 * ui.scale).round() as i32),
                    source,
                    [1.0; 3],
                );
            }
        }
        label(frame, x + 5, y + 1, "Preferences");
        let y = y + 16;
        for (index, label) in TABS.into_iter().chain(["BACK"]).enumerate() {
            let tx = x + index as i32 * 112;
            let selected = index == self.tab;
            ui.tile(
                frame,
                if selected {
                    art.highlight.as_ref()
                } else {
                    art.bar.as_ref()
                },
                [tx, y, if index == 3 { 84 } else { 112 }, 31],
                [0.25, 0.33, 0.42],
            );
            ui.label(frame, font, art.font.as_ref(), tx + 12, y + 7, label);
        }
        let enabled = |enabled| {
            if enabled {
                "ON".to_string()
            } else {
                "OFF".to_string()
            }
        };
        let values: Vec<String> = match self.tab {
            0 => {
                let (w, h) = RESOLUTIONS[settings.resolution];
                vec![
                    if settings.fullscreen {
                        "BORDERLESS"
                    } else {
                        "WINDOWED"
                    }
                    .into(),
                    format!("{w}x{h}"),
                    enabled(settings.vsync),
                    settings.field_of_view.to_string(),
                    format!("{}%", settings.hud_scale),
                    enabled(settings.show_fps),
                    enabled(settings.intro),
                ]
            }
            1 => vec![
                format!("{}%", settings.music_volume),
                format!("{}%", settings.effects_volume),
            ],
            _ => [
                format!("{}%", settings.sensitivity),
                enabled(settings.invert_mouse),
            ]
            .into_iter()
            .chain(settings.bindings.map(key_name))
            .collect(),
        };
        for (index, (label, value)) in self.rows().iter().zip(values).enumerate() {
            let row_y = y + 42 + index as i32 * 26;
            if self.selected == index {
                ui.tile(
                    frame,
                    art.highlight.as_ref(),
                    [x + 4, row_y, 412, 24],
                    [0.21, 0.33, 0.50],
                );
            }
            ui.label(frame, font, art.font.as_ref(), x + 15, row_y + 4, label);
            let arrows = self.tab < 2 || index < 2;
            if arrows {
                ui.label(frame, font, art.font.as_ref(), x + 291, row_y + 4, "<");
                ui.label(frame, font, art.font.as_ref(), x + 401, row_y + 4, ">");
            }
            ui.label(frame, font, art.font.as_ref(), x + 314, row_y + 4, &value);
        }
        if !self.status.is_empty() {
            label(frame, x + 12, y + 303, &self.status);
        }
    }
}

struct Layout {
    scale: f32,
    panel: [i32; 2],
}

impl Layout {
    fn new([width, height]: [i32; 2]) -> Self {
        let scale = (width as f32 / 900.0)
            .min(height as f32 / 460.0)
            .clamp(0.4, 2.5);
        Self {
            scale,
            panel: [
                (width as f32 / scale / 2.0) as i32 - 210,
                (height as f32 / scale / 2.0) as i32 - 161,
            ],
        }
    }

    fn local(&self, x: i32, y: i32) -> [i32; 2] {
        [
            (x as f32 / self.scale) as i32,
            (y as f32 / self.scale) as i32,
        ]
    }

    fn tile(&self, frame: &mut Frame, texture: Option<&Texture>, rect: [i32; 4], color: [f32; 3]) {
        let rect = rect.map(|v| (v as f32 * self.scale).round() as i32);
        if let Some(texture) = texture {
            let (w, h) = texture.size();
            frame.tile(texture.image(0.0), rect, [0.0, 0.0, w, h], [1.0; 3]);
        } else {
            frame.solid(rect, color);
        }
    }

    fn label(
        &self,
        frame: &mut Frame,
        font: &Hud,
        menu_font: Option<&MenuFont>,
        x: i32,
        y: i32,
        label: &str,
    ) {
        if let Some(font) = menu_font {
            font.draw(
                frame,
                x as f32 * self.scale,
                y as f32 * self.scale,
                self.scale,
                label,
            );
            return;
        }
        font.text(
            frame,
            (x as f32 * self.scale) as i32,
            (y as f32 * self.scale) as i32,
            self.scale * 0.68,
            label,
            [0.93, 0.95, 1.0],
        );
    }
}

fn key_name(key: u16) -> String {
    match key {
        VK_LBUTTON => "MOUSE 1".into(),
        VK_RBUTTON => "MOUSE 2".into(),
        VK_MBUTTON => "MOUSE 3".into(),
        VK_XBUTTON1 => "MOUSE 4".into(),
        VK_XBUTTON2 => "MOUSE 5".into(),
        VK_SPACE => "SPACE".into(),
        VK_SHIFT => "SHIFT".into(),
        VK_CONTROL => "CTRL".into(),
        65..=90 => char::from(key as u8).to_string(),
        _ => format!("KEY {key}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_is_the_direct_action_and_preferences_return_to_it() {
        let mut settings = Settings::default();
        let mut menu = Menu::default();
        assert!(menu.key(VK_RETURN, &mut settings) == Action::Play);
        menu.key(VK_RIGHT, &mut settings);
        menu.key(VK_RETURN, &mut settings);
        assert!(menu.preferences);
        menu.key(VK_TAB, &mut settings);
        assert!(menu.key(VK_LEFT, &mut settings) == Action::Changed);
        assert_eq!(settings.music_volume, 90);
        menu.key(VK_TAB, &mut settings);
        menu.key(VK_DOWN, &mut settings);
        menu.key(VK_DOWN, &mut settings);
        menu.key(VK_RETURN, &mut settings);
        assert!(menu.key(b'I' as u16, &mut settings) == Action::Changed);
        assert_eq!(settings.bindings[0], b'I' as u16);
        menu.key(VK_ESCAPE, &mut settings);
        assert!(!menu.preferences);
        assert!(menu.click([900, 600], 40, 13, VK_LBUTTON, &mut settings) == Action::Play);
    }

    #[test]
    #[ignore = "requires the original startup and menu packages"]
    fn goty_city_intro_plays_its_camera_path_and_narration() {
        let path = Path::new("local/goty/Maps/DM-Deck16][.unr");
        let mut front = Frontend::load(path, true);
        let mut intro = front.intro.take().expect("CityIntro");
        let (logo, camera) = front.logo.as_ref().expect("UT-Logo-Map");
        assert!(intro.map.music.is_some() && logo.music.is_some());
        assert!(front.art.bar.is_some() && front.art.panel.is_some());
        assert!(front.has_desktop() && front.art.font.is_some());
        assert!(front.art.click.is_some());
        let mut heard = 0;
        let mut moved = false;
        let first = intro.shot().unwrap().camera.position;
        while let Some((shot, voices)) = intro.update(0.025) {
            assert!(intro.time < 240.0);
            heard += voices.len();
            let delta = shot.camera.position - first;
            moved |= delta.dot(delta) > 10_000.0;
        }
        assert!(intro.time > 45.0);
        assert!(moved && heard > 0);
        assert!(intro.map.props.iter().any(|prop| {
            prop.cinematic
                .as_ref()
                .is_some_and(|clip| clip.started.is_some())
        }));
        let mut frame = Frame::new();
        frame.draw_cinematic(logo, camera, 1.0, 90.0, 0.0, 1.0);
        assert!(
            frame
                .pixels
                .iter()
                .filter(|pixel| **pixel > 0x202020)
                .count()
                > 500
        );
    }
}
