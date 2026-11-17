mod font;
mod intro;
pub(crate) mod smoke;

use std::{fs, io, path::Path, sync::Arc};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    hud::Hud,
    map::Map,
    package::Package,
    render::{Camera, Frame},
    settings::{BINDINGS, GUI_SKINS, RESOLUTIONS, Settings},
    sound::Wave,
    texture::Texture,
};
use font::{FONT_SIZES, MenuFont};
pub use intro::Intro;

pub struct Frontend {
    pub intro: Option<Intro>,
    pub logo: Option<(Map, Camera)>,
    art: MenuArt,
}

#[derive(Default)]
struct MenuArt {
    desktop: Option<[Texture; 12]>,
    skins: [SkinArt; GUI_SKINS.len()],
    checkbox: [Option<Texture>; 2],
    click: Option<Arc<Wave>>,
    fonts: [Option<MenuFont>; FONT_SIZES.len()],
}

#[derive(Default)]
struct SkinArt {
    bar: Option<Texture>,
    bar_ends: [Option<Texture>; 2],
    bar_item: [[Option<Texture>; 3]; 2],
    window: Option<Texture>,
    button: Option<Texture>,
    panel: Option<Texture>,
    pulldown: [Option<Texture>; 9],
    highlight: Option<Texture>,
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
        let skins = GUI_SKINS.map(|(_, prefix)| {
            let themed = |name: &str| texture(&format!("{prefix}{name}"));
            let gold = prefix == "Gold";
            let bar = |name: &str| if gold { texture(name) } else { themed(name) };
            let menu = |part: &str| {
                let prefix = if gold { "B" } else { prefix };
                texture(&format!("{prefix}Menu{part}"))
            };
            SkinArt {
                bar: bar("BarTile"),
                bar_ends: [bar("BarL"), bar("BarWin")],
                bar_item: ["In", "Out"]
                    .map(|state| ["L", "M", "R"].map(|part| bar(&format!("Bar{state}{part}")))),
                window: themed("ActiveFrame"),
                button: themed("Button"),
                panel: if gold {
                    menu("Area")
                } else {
                    themed("ClientArea")
                },
                pulldown: ["TL", "T", "TR", "L", "Area", "R", "BL", "B", "BR"].map(menu),
                highlight: menu("HM"),
            }
        });
        Self {
            desktop,
            skins,
            checkbox: [texture("ChkUnchecked"), texture("ChkChecked")],
            fonts: MenuFont::load_sizes(font_path),
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
    options_open: bool,
    choice: Option<(Choice, usize)>,
    panel_offset: [i32; 2],
    drag_offset: Option<[i32; 2]>,
    pub dpi: u32,
    pub status: String,
}

#[derive(Clone, Copy)]
enum Choice {
    Resolution,
    Skin,
}

impl Choice {
    fn for_row(tab: usize, row: usize) -> Option<Self> {
        match (tab, row) {
            (0, 1) => Some(Self::Resolution),
            (0, 7) => Some(Self::Skin),
            _ => None,
        }
    }

    fn slot(self, settings: &mut Settings) -> &mut usize {
        match self {
            Self::Resolution => &mut settings.resolution,
            Self::Skin => &mut settings.gui_skin,
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Resolution => RESOLUTIONS.len(),
            Self::Skin => GUI_SKINS.len(),
        }
    }

    fn label(self, index: usize) -> String {
        match self {
            Self::Resolution => {
                let (w, h) = RESOLUTIONS[index];
                format!("{w}x{h}")
            }
            Self::Skin => GUI_SKINS[index].0.into(),
        }
    }

    fn at(self, x: i32, y: i32, row: usize) -> Option<usize> {
        let y = y - 16 - row_y(row) - 22;
        ((CHOICE_LEFT..CHOICE_LEFT + CHOICE_WIDTH).contains(&x)
            && (0..self.len() as i32 * CHOICE_HEIGHT).contains(&y))
        .then(|| (y / CHOICE_HEIGHT) as usize)
    }
}

const ROW_START: i32 = 42;
const ROW_HEIGHT: i32 = 26;
const CHOICE_LEFT: i32 = 279;
const CHOICE_WIDTH: i32 = 130;
const CHOICE_HEIGHT: i32 = 16;

fn row_y(index: usize) -> i32 {
    ROW_START + index as i32 * ROW_HEIGHT
}

const TAB_WIDTH: i32 = 68;
const TABS: [&str; 6] = ["Video", "Audio", "Controls", "Input", "HUD", "Game"];
const VIDEO: [&str; 8] = [
    "Display Mode",
    "Resolution",
    "Vertical Sync",
    "Field of View",
    "Show FPS",
    "Play Intro",
    "Show Decals",
    "GUI Skin",
];
const AUDIO: [&str; 2] = ["Music Volume", "Sound Volume"];
const CONTROLS: [&str; 8] = [
    "Move Forward",
    "Move Backward",
    "Strafe Left",
    "Strafe Right",
    "Jump / Swim Up",
    "Walk",
    "Fire",
    "Alt Fire",
];
const INPUT: [&str; 2] = ["Mouse Sensitivity", "Invert Mouse"];
const HUD: [&str; 1] = ["HUD Scale"];
const GAME: [&str; 1] = ["Dodging"];
const TOP_ITEMS: [i32; 4] = [0, 56, 112, 148];
const OPTIONS_RECT: [i32; 4] = [56, 16, 120, 24];

impl Menu {
    pub fn pointer(&mut self, size: [i32; 2], x: i32, y: i32) {
        let layout = Layout::new(size, self.dpi);
        let [x, y] = layout.local(x, y);
        if let Some([left, top]) = self.drag_offset {
            let [center_x, center_y] = layout.panel;
            self.panel_offset = [x - left - center_x, y - top - center_y];
        }
        let [left, top] = layout.panel_position(size, self.panel_offset);
        if let Some((choice, highlight)) = &mut self.choice
            && let Some(index) = choice.at(x - left, y - top, self.selected)
        {
            *highlight = index;
        }
        self.hover = Self::top_item([x, y]);
    }

    pub fn start_drag(&mut self, size: [i32; 2], x: i32, y: i32) -> bool {
        if !self.preferences || self.choice.is_some() || self.waiting.is_some() {
            return false;
        }
        let layout = Layout::new(size, self.dpi);
        let [x, y] = layout.local(x, y);
        let [left, top] = layout.panel_position(size, self.panel_offset);
        let offset = [x - left, y - top];
        if (0..400).contains(&offset[0]) && (0..16).contains(&offset[1]) {
            self.drag_offset = Some(offset);
            return true;
        }
        false
    }

    pub fn end_drag(&mut self) -> bool {
        self.drag_offset.take().is_some()
    }

    fn top_item([x, y]: [i32; 2]) -> Option<usize> {
        if (0..16).contains(&y) {
            TOP_ITEMS
                .windows(2)
                .position(|edges| (edges[0]..edges[1]).contains(&x))
        } else {
            None
        }
    }

    fn rows(&self) -> &[&str] {
        match self.tab {
            0 => &VIDEO,
            1 => &AUDIO,
            2 => &CONTROLS,
            3 => &INPUT,
            4 => &HUD,
            _ => &GAME,
        }
    }

    fn main(&mut self, index: usize) -> Action {
        self.options_open = false;
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
        if let Some((choice, selected)) = &mut self.choice {
            match key {
                VK_UP => *selected = (*selected + choice.len() - 1) % choice.len(),
                VK_DOWN => *selected = (*selected + 1) % choice.len(),
                VK_ESCAPE => self.choice = None,
                VK_RETURN | VK_SPACE => {
                    *choice.slot(settings) = *selected;
                    self.choice = None;
                    return Action::Changed;
                }
                _ => {}
            }
            return Action::None;
        }
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
                VK_RETURN | VK_SPACE
                    if let Some(choice) = Choice::for_row(self.tab, self.selected) =>
                {
                    self.choice = Some((choice, *choice.slot(settings)));
                }
                VK_RIGHT | VK_RETURN | VK_SPACE => return self.adjust(1, settings),
                _ => {}
            }
            return Action::None;
        }
        if self.options_open && key == VK_ESCAPE {
            self.options_open = false;
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
        let ui = Layout::new(size, self.dpi);
        let [x, y] = ui.local(x, y);
        if !self.preferences {
            let was_open = std::mem::take(&mut self.options_open);
            let [left, top, width, height] = OPTIONS_RECT;
            if was_open && (left..left + width).contains(&x) && (top..top + height).contains(&y) {
                return self.main(1);
            }
            if let Some(index) = Self::top_item([x, y]) {
                self.selected = index;
                if index == 1 {
                    self.options_open = !was_open;
                    return Action::None;
                }
                return self.main(self.selected);
            }
        } else {
            let [left, top] = ui.panel_position(size, self.panel_offset);
            let [x, y] = [x - left, y - top];
            if let Some((choice, _)) = self.choice.take() {
                if let Some(index) = choice.at(x, y, self.selected) {
                    *choice.slot(settings) = index;
                    return Action::Changed;
                }
                return Action::None;
            }
            if ((0..16).contains(&y) && (400..420).contains(&x))
                || ((316..332).contains(&y) && (368..416).contains(&x))
            {
                self.preferences = false;
                self.selected = 1;
                return Action::None;
            }
            let y = y - 16;
            if (2..19).contains(&y) && (3..3 + TAB_WIDTH * TABS.len() as i32).contains(&x) {
                self.tab = ((x - 3) / TAB_WIDTH) as usize;
                self.selected = 0;
            } else if (0..420).contains(&x) && y >= ROW_START {
                let row = ((y - ROW_START) / ROW_HEIGHT) as usize;
                if row < self.rows().len() {
                    self.selected = row;
                    if let Some(choice) = Choice::for_row(self.tab, row) {
                        if (CHOICE_LEFT..CHOICE_LEFT + CHOICE_WIDTH).contains(&x) {
                            self.choice = Some((choice, *choice.slot(settings)));
                        }
                        return Action::None;
                    }
                    if (279..409).contains(&x) {
                        let amount = ((x - 279) as u32 * 100 / 129).min(100);
                        let slider = match (self.tab, row) {
                            (1, 0) => Some((&mut settings.music_volume, amount)),
                            (1, 1) => Some((&mut settings.effects_volume, amount)),
                            (3, 0) => Some((&mut settings.sensitivity, 10 + amount * 240 / 100)),
                            _ => None,
                        };
                        if let Some((value, amount)) = slider {
                            *value = amount;
                            return Action::Changed;
                        }
                    }
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
            (0, 4) => settings.show_fps = !settings.show_fps,
            (0, 5) => settings.intro = !settings.intro,
            (0, 6) => settings.show_decals = !settings.show_decals,
            (0, 7) => {
                settings.gui_skin =
                    (settings.gui_skin as i32 + step).rem_euclid(GUI_SKINS.len() as i32) as usize;
            }
            (1, 0) => adjust(&mut settings.music_volume, 10, 0, 100),
            (1, 1) => adjust(&mut settings.effects_volume, 10, 0, 100),
            (2, row) => {
                self.waiting = Some(row);
                self.status = format!("PRESS A KEY FOR {}  /  ESC CANCELS", BINDINGS[row].0);
                return Action::None;
            }
            (3, 0) => adjust(&mut settings.sensitivity, 10, 10, 250),
            (3, 1) => settings.invert_mouse = !settings.invert_mouse,
            (4, 0) => adjust(&mut settings.hud_scale, 10, 50, 150),
            (5, 0) => settings.dodging = !settings.dodging,
            _ => return Action::None,
        }
        Action::Changed
    }

    pub fn draw(&self, frontend: &Frontend, frame: &mut Frame, font: &Hud, settings: &Settings) {
        let size = [frame.width as i32, frame.height as i32];
        let ui = Layout::new(size, self.dpi);
        let resources = &frontend.art;
        let font_index = ((ui.scale * 4.0).round() as usize)
            .saturating_sub(4)
            .min(FONT_SIZES.len() - 1);
        let menu_font = resources.fonts[font_index]
            .as_ref()
            .map(|font| (font, 1.0 + font_index as f32 * 0.25))
            .or_else(|| resources.fonts[1].as_ref().map(|font| (font, 1.25)));
        let art = &resources.skins[settings.gui_skin];
        let art = if art.bar.is_some() {
            art
        } else {
            &resources.skins[0]
        };
        if let Some(tiles) = &resources.desktop {
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
        let label =
            |frame: &mut Frame, x, y, text: &str| ui.label(frame, font, menu_font, x, y, text);
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
            let x = TOP_ITEMS[index];
            let item_width = TOP_ITEMS[index + 1] - x;
            if (self.selected == index || self.hover == Some(index)) && !self.preferences {
                let pressed = index == 1 && self.options_open;
                let parts = &art.bar_item[usize::from(!pressed)];
                for (texture, (left, width)) in
                    parts
                        .iter()
                        .zip([(x, 1), (x + 1, item_width - 2), (x + item_width - 1, 1)])
                {
                    ui.tile(
                        frame,
                        texture.as_ref(),
                        [left, 0, width, 16],
                        [0.25, 0.38, 0.55],
                    );
                }
            }
            ui.label(frame, font, menu_font, x + 6, 1, label);
        }
        if frontend.logo.is_none() && !frontend.has_desktop() {
            label(frame, (width - 160) / 2, 65, "UNREAL TOURNAMENT");
        }
        if !self.preferences {
            if self.options_open {
                let [x, y, w, h] = OPTIONS_RECT;
                let columns = [(x, 4), (x + 4, w - 8), (x + w - 4, 4)];
                let rows = [(y, 4), (y + 4, h - 8), (y + h - 4, 4)];
                for (row, (top, height)) in rows.into_iter().enumerate() {
                    for (col, (left, width)) in columns.into_iter().enumerate() {
                        ui.tile(
                            frame,
                            art.pulldown[row * 3 + col].as_ref(),
                            [left, top, width, height],
                            [0.65, 0.68, 0.79],
                        );
                    }
                }
                label(frame, x + 10, y + 5, "Preferences");
            }
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
        let [x, y] = ui.panel_position(size, self.panel_offset);
        ui.tile(frame, None, [x, y, 420, 338], [0.62, 0.64, 0.75]);
        ui.tile(
            frame,
            art.panel.as_ref(),
            [x + 2, y + 16, 416, 319],
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
        for (index, label) in TABS.into_iter().enumerate() {
            let tx = x + 3 + index as i32 * TAB_WIDTH;
            let selected = index == self.tab;
            let (left, right, height) = if selected {
                (4.0, 55.0, 17)
            } else {
                (57.0, 109.0, 15)
            };
            if let Some(atlas) = &art.window {
                for (rect, source) in [
                    ([tx, y + 2, 3, height], [left, 80.0, 3.0, height as f32]),
                    (
                        [tx + 3, y + 2, TAB_WIDTH - 5, height],
                        [left + 3.0, 80.0, 1.0, height as f32],
                    ),
                    (
                        [tx + TAB_WIDTH - 2, y + 2, 2, height],
                        [right, 80.0, 2.0, height as f32],
                    ),
                ] {
                    ui.segment(frame, atlas, rect, source);
                }
            } else {
                ui.tile(
                    frame,
                    art.bar.as_ref(),
                    [tx, y + 2, TAB_WIDTH, height],
                    [0.25, 0.33, 0.42],
                );
            }
            ui.label(frame, font, menu_font, tx + 12, y + 4, label);
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
                        "Borderless"
                    } else {
                        "Windowed"
                    }
                    .into(),
                    format!("{w}x{h}"),
                    enabled(settings.vsync),
                    settings.field_of_view.to_string(),
                    enabled(settings.show_fps),
                    enabled(settings.intro),
                    enabled(settings.show_decals),
                    GUI_SKINS[settings.gui_skin].0.into(),
                ]
            }
            1 => vec![
                format!("{}%", settings.music_volume),
                format!("{}%", settings.effects_volume),
            ],
            2 => settings.bindings.map(key_name).into(),
            3 => vec![
                format!("{}%", settings.sensitivity),
                enabled(settings.invert_mouse),
            ],
            4 => vec![format!("{}%", settings.hud_scale)],
            _ => vec![enabled(settings.dodging)],
        };
        for (index, (caption, value)) in self.rows().iter().zip(values).enumerate() {
            let row_y = y + row_y(index);
            if self.selected == index {
                ui.tile(
                    frame,
                    art.highlight.as_ref(),
                    [x + 8, row_y + 3, 3, 15],
                    [0.21, 0.33, 0.50],
                );
            }
            label(frame, x + 15, row_y + 4, caption);
            let checkbox = match (self.tab, index) {
                (0, 2) => Some(settings.vsync),
                (0, 4) => Some(settings.show_fps),
                (0, 5) => Some(settings.intro),
                (0, 6) => Some(settings.show_decals),
                (3, 1) => Some(settings.invert_mouse),
                (5, 0) => Some(settings.dodging),
                _ => None,
            };
            if let Some(checked) = checkbox {
                ui.tile(
                    frame,
                    resources.checkbox[usize::from(checked)].as_ref(),
                    [x + 395, row_y + 4, 12, 12],
                    if checked {
                        [0.12, 0.25, 0.5]
                    } else {
                        [0.84; 3]
                    },
                );
                continue;
            }
            let slider = match (self.tab, index) {
                (1, 0) => Some(settings.music_volume as f32 / 100.0),
                (1, 1) => Some(settings.effects_volume as f32 / 100.0),
                (3, 0) => Some((settings.sensitivity - 10) as f32 / 240.0),
                _ => None,
            };
            if let Some(fraction) = slider {
                ui.tile(
                    frame,
                    None,
                    [x + 279, row_y + 10, 130, 3],
                    [0.18, 0.22, 0.37],
                );
                ui.tile(
                    frame,
                    art.highlight.as_ref(),
                    [x + 279 + (120.0 * fraction) as i32, row_y + 4, 10, 14],
                    [0.65, 0.72, 0.85],
                );
                continue;
            }
            if Choice::for_row(self.tab, index).is_some() {
                let left = x + CHOICE_LEFT;
                ui.tile(
                    frame,
                    None,
                    [left, row_y + 2, CHOICE_WIDTH, 18],
                    [0.76, 0.79, 0.89],
                );
                if let Some(atlas) = &art.window {
                    ui.segment(
                        frame,
                        atlas,
                        [left + CHOICE_WIDTH - 16, row_y + 5, 12, 12],
                        [20.0, 60.0, 12.0, 12.0],
                    );
                } else {
                    label(frame, left + CHOICE_WIDTH - 14, row_y + 4, "v");
                }
                label(frame, left + 5, row_y + 4, &value);
                continue;
            }
            ui.tile(
                frame,
                None,
                [x + 279, row_y + 2, 130, 18],
                [0.76, 0.79, 0.89],
            );
            if self.tab == 0 || self.tab == 4 {
                ui.label(frame, font, menu_font, x + 291, row_y + 4, "<");
                ui.label(frame, font, menu_font, x + 401, row_y + 4, ">");
            }
            ui.label(frame, font, menu_font, x + 314, row_y + 4, &value);
        }
        if let Some(button) = &art.button {
            ui.segment(
                frame,
                button,
                [x + 368, y + 300, 48, 16],
                [0.0, 0.0, 48.0, 16.0],
            );
        } else {
            ui.tile(frame, None, [x + 368, y + 300, 48, 16], [0.71, 0.75, 0.86]);
        }
        label(frame, x + 376, y + 301, "Close");
        if !self.status.is_empty() {
            label(frame, x + 12, y + 301, &self.status);
        }
        if let Some((choice, selected)) = self.choice {
            let left = x + CHOICE_LEFT;
            let top = y + row_y(self.selected) + 22;
            ui.tile(
                frame,
                art.pulldown[4].as_ref(),
                [left, top, CHOICE_WIDTH, choice.len() as i32 * CHOICE_HEIGHT],
                [0.65, 0.68, 0.79],
            );
            for index in 0..choice.len() {
                let top = top + index as i32 * CHOICE_HEIGHT;
                if index == selected {
                    ui.tile(
                        frame,
                        art.highlight.as_ref(),
                        [left, top, CHOICE_WIDTH, CHOICE_HEIGHT],
                        [0.25, 0.38, 0.55],
                    );
                }
                label(frame, left + 5, top + 1, &choice.label(index));
            }
        }
    }
}

struct Layout {
    scale: f32,
    panel: [i32; 2],
}

impl Layout {
    fn new([width, height]: [i32; 2], dpi: u32) -> Self {
        let desktop_scale = (dpi / 24).clamp(4, 12) as f32 * 0.25;
        let fit = (width as f32 / 440.0).min(height as f32 / 370.0);
        let scale = desktop_scale.min(fit).max(0.4);
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

    fn panel_position(&self, [width, height]: [i32; 2], [x, y]: [i32; 2]) -> [i32; 2] {
        let [width, height] = self.local(width, height);
        [
            (self.panel[0] + x).clamp(0, (width - 420).max(0)),
            (self.panel[1] + y).clamp(16, (height - 338).max(16)),
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

    fn segment(&self, frame: &mut Frame, texture: &Texture, rect: [i32; 4], source: [f32; 4]) {
        frame.tile(
            texture.image(0.0),
            rect.map(|v| (v as f32 * self.scale).round() as i32),
            source,
            [1.0; 3],
        );
    }

    fn label(
        &self,
        frame: &mut Frame,
        font: &Hud,
        menu_font: Option<(&MenuFont, f32)>,
        x: i32,
        y: i32,
        label: &str,
    ) {
        if let Some((font, native_scale)) = menu_font {
            font.draw(
                frame,
                x as f32 * self.scale,
                y as f32 * self.scale,
                self.scale / native_scale,
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
        menu.key(VK_RETURN, &mut settings);
        assert!(menu.key(b'I' as u16, &mut settings) == Action::Changed);
        assert_eq!(settings.bindings[0], b'I' as u16);
        menu.key(VK_ESCAPE, &mut settings);
        assert!(!menu.preferences);
        let size = [900, 600];
        menu.click(size, 104, 8, VK_LBUTTON, &mut settings);
        assert!(menu.options_open);
        menu.click(size, 104, 25, VK_LBUTTON, &mut settings);
        assert!(menu.preferences);
        let [x, y] = Layout::new(size, menu.dpi).panel;
        menu.click(size, x + 90, y + 22, VK_LBUTTON, &mut settings);
        assert!(menu.click(size, x + 408, y + 60, VK_LBUTTON, &mut settings) == Action::Changed);
        assert_eq!(settings.music_volume, 100);
        menu.click(size, x + 380, y + 321, VK_LBUTTON, &mut settings);
        assert!(!menu.preferences);
        menu.tab = 0;
        menu.click(size, 104, 8, VK_LBUTTON, &mut settings);
        menu.click(size, 104, 25, VK_LBUTTON, &mut settings);
        let combo_x = x + CHOICE_LEFT + 10;
        let resolution_y = y + 16 + row_y(1);
        let settings_before = settings.resolution;
        menu.click(size, combo_x, resolution_y + 8, VK_LBUTTON, &mut settings);
        menu.key(VK_DOWN, &mut settings);
        menu.key(VK_ESCAPE, &mut settings);
        assert_eq!(settings.resolution, settings_before);
        menu.click(size, combo_x, resolution_y + 8, VK_LBUTTON, &mut settings);
        assert!(
            menu.click(
                size,
                combo_x,
                resolution_y + 22 + 5 * CHOICE_HEIGHT,
                VK_LBUTTON,
                &mut settings
            ) == Action::Changed
        );
        assert_eq!(settings.resolution, 5);
        let skin_y = y + 16 + row_y(7);
        menu.click(size, combo_x, skin_y + 8, VK_LBUTTON, &mut settings);
        menu.key(VK_DOWN, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::Changed);
        assert_eq!(settings.gui_skin, 1);
        menu.key(VK_ESCAPE, &mut settings);
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
        assert!(!intro.map.scene_lights.is_empty());
        assert!(!intro.map.coronas.is_empty());
        assert!(
            intro
                .map
                .movers
                .iter()
                .any(|mover| mover.attach_tag.is_some() && mover.faces.is_empty())
        );
        let bytes = fs::read(path.with_file_name("CityIntro.unr")).unwrap();
        let package = Package::read(&bytes).unwrap();
        for class in ["Wreck1", "Bin2", "Brick", "Pylon"] {
            let tag = package.names.iter().position(|name| name == class).unwrap();
            let prop = intro
                .map
                .props
                .iter()
                .find(|prop| prop.tag == Some(tag))
                .unwrap();
            assert!(
                prop.faces.iter().all(|face| face.texture.is_some()),
                "{class}"
            );
        }
        assert!(
            front
                .art
                .skins
                .iter()
                .all(|skin| skin.bar.is_some() && skin.panel.is_some())
        );
        assert!(front.has_desktop() && front.art.fonts[0].is_some());
        assert!(front.art.click.is_some());
        let mut heard = 0;
        let mut moved = false;
        let mut saw_smoke = false;
        let mut saw_moving_flare = false;
        let initial_flare = intro
            .map
            .smoke
            .sprites(0.0, &intro.map.movers, &intro.map.ballistics.smoke)
            .find(|sprite| sprite.3 == 1.0)
            .expect("CityIntro car flare")
            .0;
        let first = intro.shot().unwrap().camera.position;
        while let Some((shot, voices)) = intro.update(0.025) {
            assert!(intro.time < 240.0);
            heard += voices.len();
            let delta = shot.camera.position - first;
            moved |= delta.dot(delta) > 10_000.0;
            let sprites =
                intro
                    .map
                    .smoke
                    .sprites(intro.time, &intro.map.movers, &intro.map.ballistics.smoke);
            for (position, _, _, glow) in sprites {
                if glow < 1.0 {
                    saw_smoke = true;
                } else {
                    let delta = position - initial_flare;
                    saw_moving_flare |= delta.dot(delta) > 1.0;
                    break;
                }
            }
        }
        assert!(intro.time > 45.0);
        assert!(saw_smoke && saw_moving_flare);
        assert!(moved && heard > 0);
        assert!(intro.map.props.iter().any(|prop| {
            prop.cinematic
                .as_ref()
                .is_some_and(|clip| clip.started.is_some())
        }));
        let mut frame = Frame::new();
        frame.draw_cinematic(
            logo,
            camera,
            1.0,
            90.0,
            0.0,
            (1.0, crate::map::Vec3::default()),
        );
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
