mod font;
mod intro;
pub(crate) mod smoke;

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

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
    match_maps: Vec<PathBuf>,
    pub pending_arena: Option<Map>,
    art: MenuArt,
}

#[derive(Default)]
struct MenuArt {
    desktop: Option<[Texture; 12]>,
    skins: [SkinArt; GUI_SKINS.len()],
    checkbox: [Option<Texture>; 2],
    sounds: [Option<Arc<Wave>>; 3],
    fonts: [Option<MenuFont>; FONT_SIZES.len()],
}

#[derive(Default)]
struct SkinArt {
    bar: Option<Texture>,
    window: Option<Texture>,
    button: Option<Texture>,
    panel: Option<Texture>,
    dropdown: Option<Texture>,
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
        let mut match_maps: Vec<_> = fs::read_dir(maps)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("unr"))
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.to_ascii_lowercase().starts_with("dm-"))
                    && path.file_name() != map_path.file_name()
            })
            .collect();
        match_maps.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
        });
        match_maps.insert(0, map_path.to_path_buf());
        Self {
            intro,
            logo,
            match_maps,
            pending_arena: None,
            art: MenuArt::load(
                &maps.join("../System/UMenu.u"),
                &maps.join("../Textures/UWindowFonts.utx"),
            ),
        }
    }

    pub fn play_menu_sound(&self, audio: &mut crate::sound::device::Audio, sound: MenuSound) {
        if let Some(sound) = &self.art.sounds[sound as usize] {
            audio.play(Arc::clone(sound));
        }
    }

    pub fn has_desktop(&self) -> bool {
        self.art.desktop.is_some()
    }

    pub fn match_names(&self) -> Vec<String> {
        self.match_maps
            .iter()
            .filter_map(|path| path.file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }

    pub fn load_match(&self, index: usize) -> io::Result<Option<Map>> {
        if index != 0 {
            let path = self.match_maps.get(index).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "Selected map is unavailable")
            })?;
            let bytes = fs::read(path)?;
            let package = Package::read(&bytes)?;
            return Ok(Some(Map::load(&package, Some(path))?));
        }
        Ok(None)
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
            let menu = |part: &str| {
                let prefix = if gold { "B" } else { prefix };
                texture(&format!("{prefix}Menu{part}"))
            };
            SkinArt {
                bar: if gold {
                    texture("BarTile")
                } else {
                    themed("BarTile")
                },
                window: themed("ActiveFrame"),
                button: themed("Button"),
                panel: if gold {
                    menu("Area")
                } else {
                    themed("ClientArea")
                },
                dropdown: menu("Area"),
                highlight: menu("HM"),
            }
        });
        Self {
            desktop,
            skins,
            checkbox: [texture("ChkUnchecked"), texture("ChkChecked")],
            fonts: MenuFont::load_sizes(font_path),
            sounds: ["LittleSelect", "WindowOpen", "WindowClose"].map(|name| {
                export(name, "Sound")
                    .and_then(|item| Wave::from_export(&package, item).ok())
                    .map(Arc::new)
            }),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Resume,
    Deathmatch(usize),
    LeaveMatch,
    Quit,
    Changed,
}

pub enum MenuSound {
    Select,
    Open,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MenuState {
    window: bool,
    focus: usize,
}

impl MenuState {
    pub fn sound_to(self, next: Self, action: Action) -> Option<MenuSound> {
        if self.window != next.window {
            Some(if next.window {
                MenuSound::Open
            } else {
                MenuSound::Close
            })
        } else if self.focus != next.focus || action == Action::Changed {
            Some(MenuSound::Select)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum MenuSession {
    #[default]
    Main,
    Practice,
    Match,
}

impl MenuSession {
    pub fn paused(self) -> bool {
        self != Self::Main
    }

    fn entries(self) -> &'static [&'static str] {
        if self.paused() {
            &["Resume", "Main Menu", "Settings", "Quit Game"]
        } else {
            &["New Game", "Settings", "Quit Game"]
        }
    }
}

#[derive(Default)]
pub struct Menu {
    pub session: MenuSession,
    pub match_names: Vec<String>,
    match_setup: Option<MatchSetup>,
    preferences: bool,
    tab: usize,
    selected: usize,
    nav_focus: usize,
    nav_hover: Option<usize>,
    waiting: Option<usize>,
    quit: Option<bool>,
    choice: Option<(Choice, usize)>,
    panel_offset: [i32; 2],
    drag_offset: Option<[i32; 2]>,
    pub dpi: u32,
    pub status: String,
}

const MAPS_PER_PAGE: usize = 8;
const MATCH_FIELDS: [&str; 4] = ["Difficulty", "Bot count", "Frag limit", "Time limit"];
const MATCH_BOUNDS: [(u32, u32); 4] = [(1, 3), (0, 8), (0, 100), (0, 60)];

struct MatchSetup {
    map: usize,
    focus: usize,
    values: [u32; 4],
}

impl MatchSetup {
    fn new(settings: &Settings) -> Self {
        Self {
            map: 0,
            focus: 0,
            values: [
                settings.bot_difficulty,
                settings.bot_count,
                settings.frag_limit,
                settings.match_minutes,
            ],
        }
    }

    fn change(&mut self, field: usize, step: i32) {
        let (min, max) = MATCH_BOUNDS[field];
        self.values[field] = self.values[field]
            .saturating_add_signed(step)
            .clamp(min, max);
    }

    fn value(&self, field: usize) -> String {
        let value = self.values[field];
        match field {
            0 => crate::game::deathmatch::Difficulty::from_level(value)
                .label()
                .into(),
            2 | 3 if value == 0 => "Off".into(),
            3 => format!("{value} min"),
            _ => value.to_string(),
        }
    }

    fn page(&self) -> usize {
        self.map / MAPS_PER_PAGE * MAPS_PER_PAGE
    }

    fn turn_page(&mut self, next: bool, count: usize) {
        self.map = if next && self.page() + MAPS_PER_PAGE < count {
            self.page() + MAPS_PER_PAGE
        } else if !next && self.page() > 0 {
            self.page() - MAPS_PER_PAGE
        } else {
            self.map
        };
        self.focus = 0;
    }
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
const HUD: [&str; 2] = ["HUD Scale", "Context Help"];
const GAME: [&str; 7] = [
    "Dodging",
    "Game Speed",
    "View Bob",
    "Bot Difficulty",
    "Bot Count",
    "Frag Limit",
    "Time Limit",
];
const MENU_BUTTON_HEIGHT: i32 = 32;
impl Menu {
    pub fn state(&self) -> MenuState {
        MenuState {
            window: self.preferences || self.quit.is_some() || self.match_setup.is_some(),
            focus: self.nav_focus,
        }
    }

    pub fn commit_match_settings(&mut self, settings: &mut Settings) -> bool {
        let Some(setup) = self.match_setup.take() else {
            return false;
        };
        if setup.values == MatchSetup::new(settings).values {
            return false;
        }
        [
            settings.bot_difficulty,
            settings.bot_count,
            settings.frag_limit,
            settings.match_minutes,
        ] = setup.values;
        true
    }

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
        self.nav_hover = layout.nav_item(size, x, y, self.session);
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

    fn activate(&mut self, item: usize, settings: &Settings) -> Action {
        self.status.clear();
        match (self.session.paused(), item) {
            (true, 0) => Action::Resume,
            (false, 0) => {
                self.match_setup = Some(MatchSetup::new(settings));
                Action::None
            }
            (true, 1) => Action::LeaveMatch,
            (true, 2) | (false, 1) => {
                self.preferences = true;
                self.selected = 0;
                Action::None
            }
            (true, 3) | (false, 2) => {
                self.quit = Some(true);
                Action::None
            }
            _ => Action::None,
        }
    }
    pub fn key(&mut self, key: u16, settings: &mut Settings) -> Action {
        if let Some(setup) = &mut self.match_setup {
            let count = self.match_names.len();
            match key {
                VK_UP if setup.focus == 0 && count > 0 => {
                    setup.map = (setup.map + count - 1) % count
                }
                VK_DOWN if setup.focus == 0 && count > 0 => setup.map = (setup.map + 1) % count,
                VK_UP if setup.focus > 0 => setup.focus = setup.focus.saturating_sub(1),
                VK_DOWN if setup.focus > 0 => {
                    setup.focus = (setup.focus + 1).min(MATCH_FIELDS.len())
                }
                VK_TAB => setup.focus = (setup.focus + 1) % (MATCH_FIELDS.len() + 1),
                VK_LEFT | VK_RIGHT if setup.focus > 0 => {
                    setup.change(setup.focus - 1, if key == VK_LEFT { -1 } else { 1 })
                }
                VK_PRIOR | VK_NEXT => setup.turn_page(key == VK_NEXT, count),
                VK_ESCAPE => self.match_setup = None,
                VK_RETURN | VK_SPACE if count > 0 => return Action::Deathmatch(setup.map),
                _ => {}
            }
            self.status.clear();
            return Action::None;
        }
        if let Some(yes) = &mut self.quit {
            match key {
                VK_LEFT | VK_RIGHT | VK_TAB => *yes = !*yes,
                VK_ESCAPE | 0x4e => self.quit = None,
                VK_RETURN | VK_SPACE | 0x59 => {
                    if key == 0x59 || *yes {
                        return Action::Quit;
                    }
                    self.quit = None;
                }
                _ => {}
            }
            return Action::None;
        }
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
        let count = self.session.entries().len();
        match key {
            VK_UP | VK_LEFT => self.nav_focus = (self.nav_focus + count - 1) % count,
            VK_DOWN | VK_RIGHT | VK_TAB => self.nav_focus = (self.nav_focus + 1) % count,
            VK_RETURN | VK_SPACE => return self.activate(self.nav_focus, settings),
            VK_ESCAPE if self.session.paused() => return Action::Resume,
            0x4d if self.session.paused() => return Action::LeaveMatch,
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
        if let Some(setup) = &mut self.match_setup {
            let [left, top, ..] = ui.match_rect(size);
            let (x, y) = (x - left, y - top);
            let count = self.match_names.len();
            self.status.clear();
            if (12..209).contains(&x) && (52..196).contains(&y) {
                let index = setup.page() + ((y - 52) / 18) as usize;
                if index < count {
                    setup.map = index;
                    setup.focus = 0;
                }
            }
            if (222..416).contains(&x) && (52..196).contains(&y) {
                let field = ((y - 52) / 36) as usize;
                setup.focus = field + 1;
                if (5..29).contains(&((y - 52) % 36)) {
                    match x {
                        333..357 => setup.change(field, -1),
                        392..416 => setup.change(field, 1),
                        _ => {}
                    }
                }
            }
            if (207..229).contains(&y) {
                match x {
                    12..89 | 96..173 => setup.turn_page(x >= 96, count),
                    222..305 => self.match_setup = None,
                    313..416 if count > 0 => return Action::Deathmatch(setup.map),
                    _ => {}
                }
            }
            return Action::None;
        }
        if self.quit.is_some() {
            let [left, top, ..] = ui.quit_rect(size);
            if (top + 57..top + 75).contains(&y) {
                if (left + 75..left + 125).contains(&x) {
                    return Action::Quit;
                }
                if (left + 145..left + 195).contains(&x) {
                    self.quit = None;
                }
            }
            if (left + 249..left + 270).contains(&x) && (top..top + 16).contains(&y) {
                self.quit = None;
            }
            return Action::None;
        }
        if !self.preferences {
            if let Some(index) = ui.nav_item(size, x, y, self.session) {
                self.nav_focus = index;
                return self.activate(index, settings);
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
                            (5, 1) => {
                                Some((&mut settings.game_speed, 50 + (amount * 30 + 50) / 100 * 5))
                            }
                            (5, 2) => Some((&mut settings.view_bob, (amount * 8 + 50) / 100)),
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
            (4, 1) => settings.context_help = !settings.context_help,
            (5, 0) => settings.dodging = !settings.dodging,
            (5, 1) => adjust(&mut settings.game_speed, 5, 50, 200),
            (5, 2) => adjust(&mut settings.view_bob, 1, 0, 8),
            (5, 3) => adjust(&mut settings.bot_difficulty, 1, 1, 3),
            (5, 4) => adjust(&mut settings.bot_count, 1, 0, 8),
            (5, 5) => adjust(&mut settings.frag_limit, 1, 0, 100),
            (5, 6) => adjust(&mut settings.match_minutes, 1, 0, 60),
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
            .clamp(1, FONT_SIZES.len() - 1);
        let menu_font = resources.fonts[font_index]
            .as_ref()
            .map(|font| (font, 1.0 + font_index as f32 * 0.25))
            .or_else(|| resources.fonts[1].as_ref().map(|font| (font, 1.25)));
        let art = &resources.skins[settings.gui_skin];
        let art = if art.window.is_some() {
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
        if let Some(setup) = &self.match_setup {
            let rect @ [x, y, width, _] = ui.match_rect(size);
            let text = |frame: &mut Frame, left: i32, top: i32, size: f32, value: &str, color| {
                font.text(
                    frame,
                    ((x + left) as f32 * ui.scale).round() as i32,
                    ((y + top) as f32 * ui.scale).round() as i32,
                    size * ui.scale,
                    value,
                    color,
                );
            };
            let fill = |frame: &mut Frame, [left, top, w, h]: [i32; 4], color| {
                ui.tile(frame, None, [x + left, y + top, w, h], color);
            };
            let ink = [0.91, 0.93, 0.96];
            let dim = [0.64, 0.70, 0.75];
            let highlight = [0.23, 0.40, 0.43];
            let control = [0.17, 0.23, 0.27];
            let enabled = |on| if on { ink } else { [0.46, 0.49, 0.52] };
            let button = |frame: &mut Frame, rect: [i32; 4], value: &str, primary, on| {
                fill(frame, rect, if primary && on { highlight } else { control });
                text(frame, rect[0] + 8, rect[1] + 4, 0.64, value, enabled(on));
            };
            ui.tile(frame, None, rect, [0.09, 0.12, 0.15]);
            fill(frame, [0, 0, width, 2], highlight);
            text(frame, 12, 9, 0.97, "Local Deathmatch", ink);
            text(frame, 12, 33, 0.69, "Maps", dim);
            text(frame, 222, 33, 0.69, "Match rules", dim);
            fill(frame, [12, 52, 197, 144], [0.07, 0.09, 0.11]);
            let page = setup.page();
            for (row, name) in self
                .match_names
                .iter()
                .skip(page)
                .take(MAPS_PER_PAGE)
                .enumerate()
            {
                let top = 52 + 18 * row as i32;
                if page + row == setup.map {
                    fill(
                        frame,
                        [12, top, 197, 18],
                        if setup.focus == 0 { highlight } else { control },
                    );
                }
                let name: String = name.chars().take(23).collect();
                text(frame, 18, top + 2, 0.66, &name, ink);
            }
            for (field, name) in MATCH_FIELDS.into_iter().enumerate() {
                let y = 52 + field as i32 * 36;
                if setup.focus == field + 1 {
                    fill(frame, [222, y, 194, 34], highlight);
                }
                text(frame, 230, y + 1, 0.61, name, dim);
                text(frame, 230, y + 15, 0.72, &setup.value(field), ink);
                let (min, max) = MATCH_BOUNDS[field];
                button(
                    frame,
                    [333, y + 5, 24, 24],
                    "-",
                    false,
                    setup.values[field] > min,
                );
                button(
                    frame,
                    [392, y + 5, 24, 24],
                    "+",
                    false,
                    setup.values[field] < max,
                );
            }
            let count = self.match_names.len();
            button(frame, [12, 207, 77, 22], "Previous", false, page > 0);
            button(
                frame,
                [96, 207, 77, 22],
                "Next",
                false,
                page + MAPS_PER_PAGE < count,
            );
            button(frame, [222, 207, 83, 22], "Cancel", false, true);
            button(frame, [313, 207, 103, 22], "Start match", true, count > 0);
            let status = if count == 0 {
                "No installed Deathmatch maps"
            } else {
                &self.status
            };
            text(frame, 12, 236, 0.63, status, [1.0, 0.70, 0.56]);
            return;
        }
        if !self.preferences {
            let rect @ [x, y, width, _] = ui.nav_rect(size, self.session);
            let text = |frame: &mut Frame, xx, yy, scale, label: &str, color| {
                font.text(
                    frame,
                    (xx as f32 * ui.scale).round() as i32,
                    (yy as f32 * ui.scale).round() as i32,
                    scale * ui.scale,
                    label,
                    color,
                );
            };
            let title = match self.session {
                MenuSession::Main => "Main Menu",
                MenuSession::Practice => "Practice paused",
                MenuSession::Match => "Match paused",
            };
            ui.tile(frame, None, rect, [0.08, 0.10, 0.13]);
            ui.tile(frame, None, [x, y, width, 2], [0.28, 0.56, 0.58]);
            text(frame, x + 14, y + 9, 1.1, title, [0.94, 0.97, 1.0]);
            for (item, label) in self.session.entries().iter().enumerate() {
                let row = y + 42 + item as i32 * MENU_BUTTON_HEIGHT;
                let selected = self.nav_hover.unwrap_or(self.nav_focus) == item;
                ui.tile(
                    frame,
                    None,
                    [x + 10, row, width - 20, 29],
                    if selected {
                        [0.22, 0.40, 0.44]
                    } else {
                        [0.13, 0.18, 0.22]
                    },
                );
                text(frame, x + 18, row + 4, 0.91, label, [0.94, 0.97, 1.0]);
            }
            if settings.context_help && self.session.paused() {
                text(
                    frame,
                    x + 14,
                    y + 176,
                    0.72,
                    "Esc to resume    M for main menu",
                    [0.70, 0.76, 0.81],
                );
            }
            let label =
                |frame: &mut Frame, x, y, label| ui.label(frame, font, menu_font, x, y, label);
            if let Some(yes) = self.quit {
                let rect = ui.quit_rect(size);
                let [x, y, ..] = rect;
                ui.window(frame, art, rect);
                label(frame, x + 5, y + 1, "Confirm Quit");
                label(frame, x + 38, y + 31, "Are you sure you want to Quit?");
                for (index, caption) in ["Yes", "No"].into_iter().enumerate() {
                    let left = x + 75 + index as i32 * 70;
                    ui.button(frame, art, [left, y + 57, 50, 18]);
                    let selected = yes == (index == 0);
                    if selected {
                        ui.solid_outline(frame, [left + 3, y + 59, 44, 14]);
                    }
                    label(frame, left + 15, y + 58, caption);
                }
            }
            return;
        }
        let [x, y] = ui.panel_position(size, self.panel_offset);
        ui.window(frame, art, [x, y, 420, 338]);
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
            4 => vec![
                format!("{}%", settings.hud_scale),
                enabled(settings.context_help),
            ],
            _ => vec![
                enabled(settings.dodging),
                format!("{}%", settings.game_speed),
                settings.view_bob.to_string(),
                crate::game::deathmatch::Difficulty::from_level(settings.bot_difficulty)
                    .label()
                    .into(),
                settings.bot_count.to_string(),
                MatchSetup::new(settings).value(2),
                MatchSetup::new(settings).value(3),
            ],
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
                (4, 1) => Some(settings.context_help),
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
                (5, 1) => Some((settings.game_speed - 50) as f32 / 150.0),
                (5, 2) => Some(settings.view_bob as f32 / 8.0),
                _ => None,
            };
            if let Some(fraction) = slider {
                if self.tab == 5 {
                    label(frame, x + 201, row_y + 4, &value);
                }
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
        ui.button(frame, art, [x + 368, y + 300, 48, 16]);
        label(frame, x + 376, y + 301, "Close");
        if !self.status.is_empty() {
            label(frame, x + 12, y + 301, &self.status);
        }
        if let Some((choice, selected)) = self.choice {
            let left = x + CHOICE_LEFT;
            let top = y + row_y(self.selected) + 22;
            ui.tile(
                frame,
                art.dropdown.as_ref(),
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
    fn nav_rect(&self, size: [i32; 2], session: MenuSession) -> [i32; 4] {
        let [w, h] = self.local(size[0], size[1]);
        let height = if session.paused() { 204 } else { 148 };
        [(w - 272) / 2, (h - height) / 2, 272, height]
    }

    fn nav_item(&self, size: [i32; 2], x: i32, y: i32, session: MenuSession) -> Option<usize> {
        let [left, top, width, _] = self.nav_rect(size, session);
        let row = y - top - 42;
        if !(left + 10..left + width - 10).contains(&x) || row < 0 || row % MENU_BUTTON_HEIGHT >= 29
        {
            return None;
        }
        let item = (row / MENU_BUTTON_HEIGHT) as usize;
        (item < session.entries().len()).then_some(item)
    }
    fn match_rect(&self, size: [i32; 2]) -> [i32; 4] {
        let [w, h] = self.local(size[0], size[1]);
        [(w - 428) / 2, (h - 258) / 2, 428, 258]
    }
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

    fn quit_rect(&self, [width, height]: [i32; 2]) -> [i32; 4] {
        let [width, height] = self.local(width, height);
        [width / 2 - 135, height / 2 - 42, 270, 84]
    }

    fn window(&self, frame: &mut Frame, art: &SkinArt, [x, y, w, h]: [i32; 4]) {
        self.tile(frame, None, [x, y, w, h], [0.62, 0.64, 0.75]);
        self.tile(
            frame,
            art.panel.as_ref(),
            [x + 2, y + 16, w - 4, h - 19],
            [0.62, 0.64, 0.75],
        );
        if let Some(texture) = &art.window {
            for (rect, source) in [
                ([x, y, 2, 16], [0.0, 0.0, 2.0, 16.0]),
                ([x + 2, y, w - 4, 16], [32.0, 0.0, 1.0, 16.0]),
                ([x + w - 2, y, 2, 16], [126.0, 0.0, 2.0, 16.0]),
                ([x + w - 14, y + 3, 11, 11], [4.0, 32.0, 11.0, 11.0]),
                ([x, y + 16, 2, h - 19], [0.0, 32.0, 2.0, 1.0]),
                ([x + w - 2, y + 16, 2, h - 19], [126.0, 32.0, 2.0, 1.0]),
                ([x, y + h - 3, w, 3], [32.0, 125.0, 1.0, 3.0]),
            ] {
                self.segment(frame, texture, rect, source);
            }
        }
    }

    fn button(&self, frame: &mut Frame, art: &SkinArt, rect: [i32; 4]) {
        if let Some(button) = &art.button {
            self.segment(frame, button, rect, [0.0, 0.0, 48.0, 16.0]);
        } else {
            self.tile(frame, None, rect, [0.71, 0.75, 0.86]);
        }
    }

    fn solid_outline(&self, frame: &mut Frame, [x, y, w, h]: [i32; 4]) {
        for rect in [
            [x, y, w, 1],
            [x, y + h - 1, w, 1],
            [x, y + 1, 1, h - 2],
            [x + w - 1, y + 1, 1, h - 2],
        ] {
            self.tile(frame, None, rect, [0.16, 0.21, 0.30]);
        }
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
                (self.scale / native_scale).max(self.scale.min(1.0)),
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
    fn new_game_opens_setup_and_pause_has_a_direct_return_to_menu() {
        let mut settings = Settings::default();
        let mut menu = Menu::default();
        menu.match_names = vec!["DM-Deck16][".into()];
        let size = [900, 600];
        let click = |menu: &mut Menu, item, settings: &mut Settings| {
            let [x, y, ..] = Layout::new(size, menu.dpi).nav_rect(size, menu.session);
            menu.click(
                size,
                x + 23,
                y + 50 + item * MENU_BUTTON_HEIGHT,
                VK_LBUTTON,
                settings,
            )
        };

        assert!(menu.key(VK_RETURN, &mut settings) == Action::None);
        assert!(menu.match_setup.is_some());
        assert!(menu.key(VK_ESCAPE, &mut settings) == Action::None);
        assert!(menu.match_setup.is_none());
        assert!(click(&mut menu, 0, &mut settings) == Action::None);
        assert!(menu.match_setup.is_some());
        assert!(menu.key(VK_RETURN, &mut settings) == Action::Deathmatch(0));
        menu.key(VK_ESCAPE, &mut settings);

        assert!(click(&mut menu, 1, &mut settings) == Action::None);
        assert!(menu.preferences);
        menu.key(VK_TAB, &mut settings);
        assert!(menu.key(VK_LEFT, &mut settings) == Action::Changed);
        assert_eq!(settings.music_volume, 90);
        menu.key(VK_TAB, &mut settings);
        menu.key(VK_RETURN, &mut settings);
        assert!(menu.key(b'I' as u16, &mut settings) == Action::Changed);
        assert_eq!(settings.bindings[0], b'I' as u16);
        menu.key(VK_ESCAPE, &mut settings);

        for session in [MenuSession::Match, MenuSession::Practice] {
            menu.session = session;
            assert!(click(&mut menu, 0, &mut settings) == Action::Resume);
            assert!(click(&mut menu, 1, &mut settings) == Action::LeaveMatch);
            assert!(menu.key(b'M' as u16, &mut settings) == Action::LeaveMatch);
            assert!(menu.key(VK_ESCAPE, &mut settings) == Action::Resume);
        }
        assert!(click(&mut menu, 3, &mut settings) == Action::None);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::Quit);
    }

    #[test]
    fn preference_controls_still_save_tab_values_and_choose_resolution() {
        let mut settings = Settings::default();
        let mut menu = Menu::default();
        let size = [900, 600];
        let [x, y] = Layout::new(size, menu.dpi).panel;
        assert!(menu.activate(1, &settings) == Action::None);
        let resolution_y = y + 16 + row_y(1);
        let combo_x = x + CHOICE_LEFT + 10;
        menu.click(size, combo_x, resolution_y + 8, VK_LBUTTON, &mut settings);
        menu.key(VK_DOWN, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::Changed);
        assert_eq!(settings.resolution, 3);
        menu.tab = 5;
        menu.selected = 3;
        assert!(menu.key(VK_RIGHT, &mut settings) == Action::Changed);
        assert_eq!(settings.bot_difficulty, 3);
        menu.key(VK_ESCAPE, &mut settings);
        assert!(!menu.preferences);
    }
    #[test]
    fn deathmatch_setup_applies_draft_rules_only_after_successful_launch() {
        let mut settings = Settings::default();
        let mut menu = Menu::default();
        menu.match_names = vec!["DM-Deck16][".into(), "DM-Liandri".into()];
        assert!(menu.key(VK_RETURN, &mut settings) == Action::None);
        assert!(menu.match_setup.is_some());
        menu.key(VK_DOWN, &mut settings);
        menu.key(VK_TAB, &mut settings);
        menu.key(VK_RIGHT, &mut settings);
        assert_eq!(settings.bot_difficulty, 2);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::Deathmatch(1));
        assert!(menu.key(VK_ESCAPE, &mut settings) == Action::None);
        assert!(menu.match_setup.is_none());
        assert_eq!(settings.bot_difficulty, 2);

        menu.activate(0, &settings);
        let size = [900, 600];
        let [x, y, ..] = Layout::new(size, menu.dpi).match_rect(size);
        assert!(menu.click(size, x + 25, y + 74, VK_LBUTTON, &mut settings) == Action::None);
        assert!(menu.click(size, x + 400, y + 98, VK_LBUTTON, &mut settings) == Action::None);
        assert_eq!(settings.bot_count, 3);
        assert!(
            menu.click(size, x + 340, y + 216, VK_LBUTTON, &mut settings) == Action::Deathmatch(1)
        );
        assert!(menu.commit_match_settings(&mut settings));
        assert_eq!(settings.match_rules().bots, 4);
        assert!(menu.match_setup.is_none());

        menu.activate(0, &settings);
        let setup = menu.match_setup.as_mut().unwrap();
        for field in 1..=3 {
            setup.change(field, -100);
        }
        assert_eq!(setup.value(1), "0");
        assert_eq!(setup.value(2), "Off");
        assert_eq!(setup.value(3), "Off");
        assert!(menu.commit_match_settings(&mut settings));
        let rules = settings.match_rules();
        assert_eq!((rules.bots, rules.frag_limit, rules.seconds), (0, 0, 0));
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
        assert!(!intro.map.animated_lightmaps.is_empty());
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
        assert!(front.art.sounds.iter().all(Option::is_some));
        let mut heard = 0;
        let mut moved = false;
        let mut saw_smoke = false;
        let mut saw_moving_flare = false;
        let mut saw_light_fade = false;
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
            if !saw_light_fade
                && intro
                    .map
                    .scene_lights
                    .iter()
                    .any(|light| matches!(light.gain(intro.time), Some(1..=254)))
            {
                let mut render = Frame::hardware(&intro.map);
                render.draw_cinematic(
                    &intro.map,
                    &shot.camera,
                    intro.time,
                    shot.fov,
                    shot.roll,
                    (shot.flash, shot.fog),
                );
                let commands = render.commands.as_mut().unwrap();
                assert!(!commands.lightmap_updates.is_empty());
                commands.lightmap_updates.clear();
                render.draw_cinematic(
                    &intro.map,
                    &shot.camera,
                    intro.time,
                    shot.fov,
                    shot.roll,
                    (shot.flash, shot.fog),
                );
                assert!(
                    render
                        .commands
                        .as_ref()
                        .unwrap()
                        .lightmap_updates
                        .is_empty()
                );
                saw_light_fade = true;
            }
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
        assert!(saw_light_fade);
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
