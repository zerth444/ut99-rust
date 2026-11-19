mod font;
mod intro;
mod ladder;
pub(crate) mod smoke;

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    game::deathmatch::{Match, Mode, Rules},
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
    assault_ladder: ladder::Campaign,
    deathmatch_ladder: ladder::Campaign,
    ctf_ladder: ladder::Campaign,
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
    bar_ends: [Option<Texture>; 2],
    bar_hover: [Option<Texture>; 3],
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
        let installed = std::env::var_os("UT99_GAME_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        let reference_maps = maps.join("../../reference-client/Maps");
        let installed_maps = installed.as_ref().map(|path| path.join("Maps"));
        let match_maps = match_catalog(
            map_path,
            std::iter::once(maps)
                .chain(installed_maps.as_deref())
                .chain(std::iter::once(reference_maps.as_path())),
        );
        let names = map_names(&match_maps);
        let load_ladder = |mode| {
            std::iter::once(maps)
                .chain(installed_maps.as_deref())
                .chain(std::iter::once(reference_maps.as_path()))
                .find_map(|maps| {
                    let system = maps.join("../System");
                    ladder::load(&system, &names, mode)
                        .map_err(|error| optional(&system, error))
                        .ok()
                })
                .unwrap_or_default()
        };
        Self {
            intro,
            logo,
            match_maps,
            pending_arena: None,
            assault_ladder: load_ladder(Mode::Assault),
            deathmatch_ladder: load_ladder(Mode::Deathmatch),
            ctf_ladder: load_ladder(Mode::CaptureTheFlag),
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
        map_names(&self.match_maps)
    }

    pub fn load_match(&self, index: usize) -> io::Result<Option<Map>> {
        if index != 0 {
            return self.read_map(index).map(Some);
        }
        Ok(None)
    }

    fn read_map(&self, index: usize) -> io::Result<Map> {
        let path = self.match_maps.get(index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Selected map is unavailable")
        })?;
        let bytes = fs::read(path)?;
        let package = Package::read(&bytes)?;
        Map::load(&package, Some(path))
    }

    fn campaign(&self, mode: Mode) -> &ladder::Campaign {
        match mode {
            Mode::Assault => &self.assault_ladder,
            Mode::Deathmatch => &self.deathmatch_ladder,
            Mode::CaptureTheFlag => &self.ctf_ladder,
            _ => unreachable!("invalid tournament track"),
        }
    }

    pub fn load_ladder(&self, mode: Mode, stage: usize) -> io::Result<Map> {
        let index = self
            .campaign(mode)
            .stages
            .get(stage)
            .and_then(|entry| entry.installed)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "Tournament map is unavailable")
            })?;
        let map = self.read_map(index)?;
        if !map.supports_mode(mode)
            || (mode == Mode::Assault
                && stage == 0
                && map
                    .assault
                    .as_ref()
                    .is_none_or(|data| data.tutorial.is_none()))
            || (mode == Mode::Deathmatch && stage == 0 && map.dm_training.is_none())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Tournament map or tutorial assets are incomplete",
            ));
        }
        Ok(map)
    }

    pub fn ladder_names(&self, mode: Mode) -> Vec<(String, bool)> {
        self.campaign(mode)
            .stages
            .iter()
            .map(|stage| (stage.map.clone(), stage.installed.is_some()))
            .collect()
    }

    pub fn ladder_len(&self, mode: Mode) -> usize {
        self.campaign(mode).stages.len()
    }

    pub fn ladder_teams(&self, mode: Mode) -> Vec<String> {
        self.campaign(mode)
            .teams
            .iter()
            .map(|team| team.name.clone())
            .collect()
    }

    pub fn next_ladder_stage(&self, mode: Mode, stage: usize, game: &Match) -> Option<usize> {
        let next = stage + 1;
        (next < self.ladder_len(mode) && game.player_won_rated_match()).then_some(next)
    }

    pub fn start_match(
        &self,
        map: &Map,
        start: usize,
        action: Action,
        settings: &Settings,
    ) -> Match {
        match action {
            Action::StartMatch(_, mode) => Match::new(
                map,
                start,
                Rules {
                    mode,
                    ..settings.match_rules()
                },
            ),
            Action::StartLadder(mode, index) => {
                let campaign = self.campaign(mode);
                let entry = &campaign.stages[index];
                let roster = campaign.roster(mode, entry, settings.ladder_team(mode));
                Match::with_rated_roster(map, start, entry.rules(mode, settings), Some(roster))
            }
            _ => unreachable!("starting a match requires a launch action"),
        }
    }
}

fn map_names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn match_catalog<'a>(base: &Path, directories: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    let key = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
    };
    let base_key = key(base);
    let mut maps = BTreeMap::new();
    for directory in directories {
        for entry in fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if let Some(name) = key(&path)
                && ["dm-", "ctf-", "dom-", "as-"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                && name.ends_with(".unr")
                && base_key.as_deref() != Some(&name)
                && path.is_file()
            {
                maps.entry(name).or_insert(path);
            }
        }
    }
    std::iter::once(base.to_path_buf())
        .chain(maps.into_values())
        .collect()
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
                bar_hover: ["BarOutL", "BarOutM", "BarOutR"].map(bar),
                window: themed("ActiveFrame"),
                button: themed("Button"),
                panel: if gold {
                    menu("Area")
                } else {
                    themed("ClientArea")
                }
                .map(Texture::opaque),
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
    StartMatch(usize, Mode),
    StartLadder(Mode, usize),
    LeaveMatch,
    Quit,
    Changed,
}

impl Action {
    pub fn ladder_stage(self) -> Option<(Mode, usize)> {
        match self {
            Self::StartLadder(mode, stage) => Some((mode, stage)),
            _ => None,
        }
    }
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
            &["New Game", "Tournament", "Settings", "Quit Game"]
        }
    }
}

#[derive(Default)]
pub struct Menu {
    pub session: MenuSession,
    pub match_names: Vec<String>,
    pub ladder_names: Vec<(String, bool)>,
    pub deathmatch_ladder_names: Vec<(String, bool)>,
    pub ctf_ladder_names: Vec<(String, bool)>,
    pub ctf_ladder_teams: Vec<String>,
    ladder_index: usize,
    pub ladder_teams: Vec<String>,
    pub ladder_stage: Option<usize>,
    pub ladder_team: usize,
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
    mode: Mode,
    maps: Vec<usize>,
    focus: usize,
    values: [u32; 4],
}

impl MatchSetup {
    fn new(settings: &Settings) -> Self {
        Self {
            map: 0,
            mode: Mode::Deathmatch,
            maps: Vec::new(),
            focus: 0,
            values: [
                settings.bot_difficulty,
                settings.bot_count,
                settings.frag_limit,
                settings.match_minutes,
            ],
        }
    }

    fn read_maps(&mut self, names: &[String]) {
        self.maps = names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                name.to_ascii_lowercase()
                    .starts_with(self.mode.map_prefix())
                    .then_some(index)
            })
            .collect();
        self.map = 0;
    }

    fn change_mode(&mut self, names: &[String], step: i32) {
        let previous = Mode::ALL
            .iter()
            .position(|&mode| mode == self.mode)
            .unwrap() as i32;
        self.mode = Mode::ALL[(previous + step).rem_euclid(Mode::ALL.len() as i32) as usize];
        if self.mode == Mode::LastManStanding {
            self.values[2] = self.values[2].max(1);
        }
        self.read_maps(names);
    }

    fn launch(&self) -> Action {
        self.maps
            .get(self.map)
            .map_or(Action::None, |&index| Action::StartMatch(index, self.mode))
    }

    fn change(&mut self, field: usize, step: i32) {
        if self.mode == Mode::Assault && field >= 2 {
            return;
        }
        let (min, max) = self.bounds(field);
        self.values[field] = self.values[field]
            .saturating_add_signed(step)
            .clamp(min, max);
    }

    fn bounds(&self, field: usize) -> (u32, u32) {
        if self.mode == Mode::Assault && field >= 2 {
            return (self.values[field], self.values[field]);
        }
        let (min, max) = MATCH_BOUNDS[field];
        (
            if field == 2 && self.mode == Mode::LastManStanding {
                1
            } else {
                min
            },
            max,
        )
    }

    fn value(&self, field: usize) -> String {
        if self.mode == Mode::Assault && field >= 2 {
            return "Set by map".into();
        }
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
const FIELD_COLOR: [f32; 3] = [0.76, 0.79, 0.89];
const NAV_HEIGHT: i32 = 16;
const NAV_BUTTON_WIDTH: i32 = 88;
impl Menu {
    pub fn state(&self) -> MenuState {
        MenuState {
            window: self.preferences
                || self.quit.is_some()
                || self.match_setup.is_some()
                || self.ladder_stage.is_some(),
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
        self.nav_hover = Layout::nav_item(x, y, self.session);
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
                let mut setup = MatchSetup::new(settings);
                setup.read_maps(&self.match_names);
                self.match_setup = Some(setup);
                Action::None
            }
            (true, 1) => Action::LeaveMatch,
            (false, 1) => {
                self.select_ladder_checkpoint(settings);
                Action::None
            }
            (_, 2) => {
                self.preferences = true;
                self.selected = 0;
                Action::None
            }
            (_, 3) => {
                self.quit = Some(true);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn ladder_mode(&self) -> Mode {
        [Mode::Assault, Mode::Deathmatch, Mode::CaptureTheFlag][self.ladder_index]
    }

    fn current_ladder_names(&self) -> &[(String, bool)] {
        match self.ladder_mode() {
            Mode::Deathmatch => &self.deathmatch_ladder_names,
            Mode::CaptureTheFlag => &self.ctf_ladder_names,
            _ => &self.ladder_names,
        }
    }

    fn select_ladder_checkpoint(&mut self, settings: &Settings) {
        let progress = settings.ladder_progress(self.ladder_mode());
        self.ladder_stage = Some(progress.min(self.current_ladder_names().len().saturating_sub(1)));
    }

    fn current_ladder_teams(&self) -> &[String] {
        match self.ladder_mode() {
            Mode::Assault => &self.ladder_teams,
            Mode::CaptureTheFlag => &self.ctf_ladder_teams,
            _ => &[],
        }
    }

    fn switch_ladder(&mut self, settings: &Settings, direction: i32) {
        self.ladder_index = (self.ladder_index as i32 + direction).rem_euclid(3) as usize;
        self.ladder_team = settings
            .ladder_team(self.ladder_mode())
            .min(self.current_ladder_teams().len().saturating_sub(1));
        self.select_ladder_checkpoint(settings);
    }

    fn ladder_launch(&self, stage: usize, settings: &Settings) -> Action {
        let mode = self.ladder_mode();
        if stage <= settings.ladder_progress(mode)
            && self
                .current_ladder_names()
                .get(stage)
                .is_some_and(|(_, installed)| *installed)
        {
            return Action::StartLadder(mode, stage);
        }
        Action::None
    }

    fn shift_ladder_team(&mut self, settings: &Settings, direction: i32) {
        let count = self.current_ladder_teams().len() as i32;
        if settings.ladder_progress(self.ladder_mode()) == 0 && count > 1 {
            self.ladder_team = (self.ladder_team as i32 + direction).rem_euclid(count) as usize;
        }
    }

    pub fn key(&mut self, key: u16, settings: &mut Settings) -> Action {
        if let Some(stage) = self.ladder_stage {
            self.status.clear();
            match key {
                VK_ESCAPE => self.ladder_stage = None,
                VK_TAB => self.switch_ladder(settings, 1),
                VK_LEFT => self.shift_ladder_team(settings, -1),
                VK_RIGHT => self.shift_ladder_team(settings, 1),
                VK_UP => self.ladder_stage = Some(stage.saturating_sub(1)),
                VK_DOWN => {
                    self.ladder_stage =
                        Some((stage + 1).min(self.current_ladder_names().len().saturating_sub(1)))
                }
                VK_PRIOR => self.ladder_stage = Some(stage.saturating_sub(MAPS_PER_PAGE)),
                VK_NEXT => {
                    self.ladder_stage = Some(
                        (stage + MAPS_PER_PAGE)
                            .min(self.current_ladder_names().len().saturating_sub(1)),
                    )
                }
                VK_RETURN | VK_SPACE => return self.ladder_launch(stage, settings),
                _ => {}
            }
            return Action::None;
        }
        if let Some(setup) = &mut self.match_setup {
            let count = setup.maps.len();
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
                VK_LEFT | VK_RIGHT => {
                    setup.change_mode(&self.match_names, if key == VK_LEFT { -1 } else { 1 })
                }
                VK_PRIOR | VK_NEXT => setup.turn_page(key == VK_NEXT, count),
                VK_ESCAPE => self.match_setup = None,
                VK_RETURN | VK_SPACE if count > 0 => return setup.launch(),
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
        if let Some(stage) = self.ladder_stage {
            let [left, top, width, _] = ui.match_rect(size);
            let (x, y) = (x - left, y - top);
            self.status.clear();
            if ((width - 16..width - 2).contains(&x) && (0..16).contains(&y))
                || ((222..305).contains(&x) && (210..226).contains(&y))
            {
                self.ladder_stage = None;
            } else if (26..45).contains(&y) && (12..160).contains(&x) {
                self.switch_ladder(settings, if x < 34 { -1 } else { 1 });
            } else if (26..45).contains(&y) && (164..187).contains(&x) {
                self.shift_ladder_team(settings, -1);
            } else if (26..45).contains(&y) && (391..415).contains(&x) {
                self.shift_ladder_team(settings, 1);
            } else if (12..416).contains(&x) && (47..191).contains(&y) {
                let row = stage / MAPS_PER_PAGE * MAPS_PER_PAGE + ((y - 47) / 18) as usize;
                if row < self.current_ladder_names().len() {
                    self.ladder_stage = Some(row);
                }
            } else if (210..226).contains(&y) && (12..89).contains(&x) {
                return self.key(VK_PRIOR, settings);
            } else if (210..226).contains(&y) && (96..173).contains(&x) {
                return self.key(VK_NEXT, settings);
            } else if (313..416).contains(&x) && (210..226).contains(&y) {
                return self.ladder_launch(stage, settings);
            }
            return Action::None;
        }
        if let Some(setup) = &mut self.match_setup {
            let [left, top, width, _] = ui.match_rect(size);
            let (x, y) = (x - left, y - top);
            if (width - 16..width - 2).contains(&x) && (0..16).contains(&y) {
                self.match_setup = None;
                return Action::None;
            }
            let count = setup.maps.len();
            self.status.clear();
            if (26..49).contains(&y) {
                if (12..32).contains(&x) {
                    setup.change_mode(&self.match_names, -1);
                }
                if (187..209).contains(&x) {
                    setup.change_mode(&self.match_names, 1);
                }
                return Action::None;
            }
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
            if (210..226).contains(&y) {
                match x {
                    12..89 | 96..173 => setup.turn_page(x >= 96, count),
                    222..305 => self.match_setup = None,
                    313..416 if count > 0 => return setup.launch(),
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
            if let Some(index) = Layout::nav_item(x, y, self.session) {
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
        let [width, _] = ui.local(size[0], size[1]);
        ui.tile(
            frame,
            art.bar.as_ref(),
            [0, 0, width, NAV_HEIGHT],
            [0.24, 0.27, 0.31],
        );
        for (texture, x) in art.bar_ends.iter().zip([0, width - NAV_HEIGHT]) {
            if let Some(texture) = texture {
                ui.tile(
                    frame,
                    Some(texture),
                    [x, 0, NAV_HEIGHT, NAV_HEIGHT],
                    [1.0; 3],
                );
            }
        }
        let nav_active = !self.preferences
            && self.match_setup.is_none()
            && self.quit.is_none()
            && self.ladder_stage.is_none();
        for (item, caption) in self.session.entries().iter().enumerate() {
            let x = item as i32 * NAV_BUTTON_WIDTH;
            if nav_active && self.nav_hover.unwrap_or(self.nav_focus) == item {
                for (texture, (left, width)) in art.bar_hover.iter().zip([
                    (x, 1),
                    (x + 1, NAV_BUTTON_WIDTH - 2),
                    (x + NAV_BUTTON_WIDTH - 1, 1),
                ]) {
                    ui.tile(
                        frame,
                        texture.as_ref(),
                        [left, 0, width, NAV_HEIGHT],
                        [0.25, 0.38, 0.55],
                    );
                }
            }
            label(frame, x + 7, 1, caption);
        }
        if let Some(stage) = self.ladder_stage {
            let rect @ [x, y, _, _] = ui.match_rect(size);
            ui.window(frame, art, rect);
            label(frame, x + 5, y + 1, "Tournament");
            let mode = self.ladder_mode();
            let progress = settings.ladder_progress(mode);
            let names = self.current_ladder_names();
            ui.tile(frame, None, [x + 12, y + 26, 148, 18], FIELD_COLOR);
            label(frame, x + 16, y + 29, "<");
            label(frame, x + 30, y + 29, mode.label());
            label(frame, x + 148, y + 29, ">");
            let teams = self.current_ladder_teams();
            if let Some(team) = teams.get(self.ladder_team) {
                ui.tile(frame, None, [x + 164, y + 26, 251, 18], FIELD_COLOR);
                label(frame, x + 192, y + 29, &format!("Team  {team}"));
                if progress == 0 && teams.len() > 1 {
                    label(frame, x + 171, y + 29, "<");
                    label(frame, x + 398, y + 29, ">");
                }
            }
            ui.tile(frame, None, [x + 12, y + 47, 404, 144], FIELD_COLOR);
            let page = stage / MAPS_PER_PAGE * MAPS_PER_PAGE;
            for (row, (map, installed)) in names.iter().enumerate().skip(page).take(MAPS_PER_PAGE) {
                let top = y + 47 + 18 * (row - page) as i32;
                if row == stage {
                    let selected = [x + 12, top, 404, 18];
                    ui.tile(frame, art.highlight.as_ref(), selected, [0.52, 0.64, 0.81]);
                    ui.solid_outline(frame, selected);
                }
                label(frame, x + 18, top + 2, map);
                let status = if !installed {
                    "Map unavailable"
                } else if row < progress {
                    "Complete"
                } else if row > progress {
                    "Locked"
                } else {
                    "Ready"
                };
                label(frame, x + 318, top + 2, status);
            }
            for (left, width, caption, enabled) in [
                (12, 77, "Previous", page > 0),
                (96, 77, "Next", page + MAPS_PER_PAGE < names.len()),
                (222, 83, "Back", true),
                (
                    313,
                    103,
                    "Start match",
                    self.ladder_launch(stage, settings) != Action::None,
                ),
            ] {
                if enabled {
                    ui.button(frame, art, [x + left, y + 210, width, 16]);
                    label(frame, x + left + 8, y + 211, caption);
                }
            }
            if names.is_empty() {
                label(frame, x + 12, y + 235, "Tournament data unavailable");
            } else if !self.status.is_empty() {
                label(frame, x + 12, y + 235, &self.status);
            }
            return;
        }
        if let Some(setup) = &self.match_setup {
            let rect @ [x, y, _, _] = ui.match_rect(size);
            ui.window(frame, art, rect);
            label(frame, x + 5, y + 1, "Local match");
            ui.tile(frame, None, [x + 12, y + 27, 197, 19], FIELD_COLOR);
            label(frame, x + 16, y + 29, "<");
            label(frame, x + 37, y + 29, setup.mode.label());
            label(frame, x + 193, y + 29, ">");
            label(frame, x + 222, y + 31, "Match rules");
            ui.tile(frame, None, [x + 12, y + 52, 197, 144], FIELD_COLOR);
            let page = setup.page();
            let count = setup.maps.len();
            for (row, name) in setup
                .maps
                .iter()
                .skip(page)
                .take(MAPS_PER_PAGE)
                .map(|&index| &self.match_names[index])
                .enumerate()
            {
                let top = y + 52 + 18 * row as i32;
                if page + row == setup.map {
                    ui.tile(
                        frame,
                        art.highlight.as_ref(),
                        [x + 12, top, 197, 18],
                        [0.52, 0.64, 0.81],
                    );
                    if setup.focus == 0 {
                        ui.solid_outline(frame, [x + 12, top, 197, 18]);
                    }
                }
                let name: String = name.chars().take(23).collect();
                label(frame, x + 18, top + 2, &name);
            }
            for (field, name) in MATCH_FIELDS.into_iter().enumerate() {
                let top = y + 52 + field as i32 * 36;
                label(
                    frame,
                    x + 222,
                    top,
                    if field == 2 {
                        setup.mode.goal_label()
                    } else {
                        name
                    },
                );
                let control = [x + 222, top + 12, 194, 18];
                ui.tile(frame, None, control, FIELD_COLOR);
                if setup.focus == field + 1 {
                    ui.solid_outline(frame, control);
                }
                label(frame, x + 228, top + 14, &setup.value(field));
                let (min, max) = setup.bounds(field);
                if setup.values[field] > min {
                    label(frame, x + 340, top + 14, "<");
                }
                if setup.values[field] < max {
                    label(frame, x + 400, top + 14, ">");
                }
            }
            for (left, width, caption, visible) in [
                (12, 77, "Previous", page > 0),
                (96, 77, "Next", page + MAPS_PER_PAGE < count),
                (222, 83, "Cancel", true),
                (313, 103, "Start match", count > 0),
            ] {
                if visible {
                    ui.button(frame, art, [x + left, y + 210, width, 16]);
                    label(frame, x + left + 8, y + 211, caption);
                }
            }
            if count == 0 {
                label(frame, x + 12, y + 235, "No maps installed for this mode");
            } else if !self.status.is_empty() {
                label(frame, x + 12, y + 235, &self.status);
            }
            return;
        }
        if !self.preferences {
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
                ui.solid_outline(frame, [x + 12, row_y + 2, 122, 18]);
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
                    FIELD_COLOR,
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
            ui.tile(frame, None, [x + 279, row_y + 2, 130, 18], FIELD_COLOR);
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
    fn nav_item(x: i32, y: i32, session: MenuSession) -> Option<usize> {
        if x < 0 || !(0..NAV_HEIGHT).contains(&y) {
            return None;
        }
        let item = (x / NAV_BUTTON_WIDTH) as usize;
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

    fn button(&self, frame: &mut Frame, art: &SkinArt, rect @ [x, y, w, h]: [i32; 4]) {
        if let Some(button) = &art.button {
            for (piece, source) in [
                ([x, y, 4, h], [0.0, 0.0, 4.0, 16.0]),
                ([x + 4, y, w - 8, h], [4.0, 0.0, 40.0, 16.0]),
                ([x + w - 4, y, 4, h], [44.0, 0.0, 4.0, 16.0]),
            ] {
                self.segment(frame, button, piece, source);
            }
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

pub(crate) fn training_keys(settings: &Settings) -> [String; 7] {
    [0, 1, 2, 3, 4, 6, 7].map(|index| key_name(settings.bindings[index]))
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
            menu.click(size, 12 + item * NAV_BUTTON_WIDTH, 8, VK_LBUTTON, settings)
        };

        assert!(menu.key(VK_RETURN, &mut settings) == Action::None);
        assert!(menu.match_setup.is_some());
        assert!(menu.key(VK_ESCAPE, &mut settings) == Action::None);
        assert!(menu.match_setup.is_none());
        assert!(click(&mut menu, 0, &mut settings) == Action::None);
        assert!(menu.match_setup.is_some());
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartMatch(0, Mode::Deathmatch));
        menu.key(VK_ESCAPE, &mut settings);

        assert!(click(&mut menu, 2, &mut settings) == Action::None);
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
        assert!(menu.activate(2, &settings) == Action::None);
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
    fn assault_tournament_selects_the_checkpoint_and_blocks_locked_or_missing_maps() {
        let mut settings = Settings::default();
        let mut menu = Menu::default();
        menu.ladder_names = vec![
            ("AS-Tutorial".into(), true),
            ("AS-Frigate".into(), true),
            ("AS-HiSpeed".into(), false),
        ];
        menu.ladder_teams = vec!["First team".into(), "Second team".into()];
        assert!(menu.activate(1, &settings) == Action::None);
        menu.key(VK_RIGHT, &mut settings);
        assert_eq!(menu.ladder_team, 1);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::Assault, 0));
        menu.key(VK_DOWN, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::None);
        menu.key(VK_ESCAPE, &mut settings);
        assert!(settings.record_ladder_win(Mode::Assault, 0));
        assert!(!settings.record_ladder_win(Mode::Assault, 0));
        assert!(!settings.record_ladder_win(Mode::Assault, 2));
        menu.activate(1, &settings);
        menu.key(VK_LEFT, &mut settings);
        assert_eq!(menu.ladder_team, 1);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::Assault, 1));
        assert!(settings.record_ladder_win(Mode::Assault, 1));
        menu.key(VK_DOWN, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::None);
    }

    #[test]
    fn tournament_tabs_use_separate_checkpoints() {
        let mut settings = Settings {
            assault_progress: 1,
            ..Settings::default()
        };
        let mut menu = Menu::default();
        menu.ladder_names = vec![("AS-Tutorial".into(), true), ("AS-Frigate".into(), true)];
        menu.deathmatch_ladder_names = vec![
            ("DM-Tutorial".into(), true),
            ("DM-Oblivion".into(), true),
            ("DM-Stalwart".into(), true),
        ];
        menu.ctf_ladder_names = vec![("CTF-Niven".into(), true), ("CTF-Face".into(), true)];
        menu.activate(1, &settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::Assault, 1));
        menu.key(VK_TAB, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::Deathmatch, 0));
        menu.key(VK_DOWN, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::None);
        assert!(settings.record_ladder_win(Mode::Deathmatch, 0));
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::Deathmatch, 1));
        menu.key(VK_TAB, &mut settings);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::CaptureTheFlag, 0));
        assert!(settings.record_ladder_win(Mode::CaptureTheFlag, 0));
        menu.key(VK_TAB, &mut settings);
        assert_eq!(settings.assault_progress, 1);
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartLadder(Mode::Assault, 1));
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
        assert!(menu.key(VK_RETURN, &mut settings) == Action::StartMatch(1, Mode::Deathmatch));
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
            menu.click(size, x + 340, y + 216, VK_LBUTTON, &mut settings)
                == Action::StartMatch(1, Mode::Deathmatch)
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
    #[ignore = "requires the local GOTY and reference-client map installations"]
    fn installed_deathmatch_catalog_loads_and_simulates_each_arena() {
        use crate::game::deathmatch::{Command, Match, Rules};

        let bundled = Path::new("local/goty/Maps");
        let reference = Path::new("local/reference-client/Maps");
        let base = bundled.join("DM-Deck16][.unr");
        assert!(base.is_file() && reference.is_dir());
        let catalog = match_catalog(&base, [bundled, reference]);
        assert_eq!(catalog[0], base);
        assert!(catalog.len() >= 34);
        assert!(catalog.contains(&bundled.join("DM-Liandri.unr")));
        assert!(!catalog.contains(&reference.join("DM-Liandri.unr")));

        for path in catalog.into_iter().filter(|path| {
            path.file_stem().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("dm-")
            })
        }) {
            let data = fs::read(&path).unwrap();
            let package = Package::read(&data).unwrap();
            let mut map = Map::load(&package, Some(&path))
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                map.faces.iter().any(|face| face.texture.is_some()),
                "{}",
                path.display()
            );
            if matches!(
                path.file_stem().and_then(|name| name.to_str()),
                Some("DM-ArcaneTemple" | "DM-Morpheus" | "DM-SpaceNoxx")
            ) {
                assert!(
                    map.faces.iter().all(|face| face.texture.is_some()),
                    "{} has missing materials",
                    path.display()
                );
            }
            assert!(
                map.hud.ready() && !map.arsenal.is_empty(),
                "{}",
                path.display()
            );
            let mut game = Match::new(
                &map,
                0,
                Rules {
                    bots: 1,
                    ..Rules::default()
                },
            );
            for _ in 0..180 {
                game.step(&mut map, Command::default(), 1.0 / 120.0);
            }
            assert!(game.time >= 1.4 && !game.finished);
            assert!(
                game.fighters.iter().all(|fighter| {
                    let crate::map::Vec3(x, y, z) = fighter.walker.position;
                    x.is_finite() && y.is_finite() && z.is_finite()
                }),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn mode_picker_uses_map_indices_from_the_matching_family() {
        let mut setup = MatchSetup::new(&Settings::default());
        let names = [
            "DM-A".into(),
            "DOM-A".into(),
            "CTF-A".into(),
            "CTF-B".into(),
            "AS-A".into(),
        ];
        setup.read_maps(&names);
        assert!(setup.launch() == Action::StartMatch(0, Mode::Deathmatch));
        setup.change_mode(&names, 1);
        assert!(setup.launch() == Action::StartMatch(0, Mode::TeamDeathmatch));
        setup.change_mode(&names, 1);
        setup.map = 1;
        assert!(setup.launch() == Action::StartMatch(3, Mode::CaptureTheFlag));
        setup.change_mode(&names, 1);
        assert!(setup.launch() == Action::StartMatch(1, Mode::Domination));
        setup.change_mode(&names, 1);
        assert!(setup.launch() == Action::StartMatch(0, Mode::LastManStanding));
        setup.change_mode(&names, 1);
        assert!(setup.launch() == Action::StartMatch(4, Mode::Assault));
    }

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn installed_assault_maps_load_and_start_both_sides() {
        use crate::game::deathmatch::{Command, Match, Rules};

        let reference = Path::new("local/reference-client/Maps");
        let maps = match_catalog(&reference.join("DM-Deck16][.unr"), [reference]);
        let mut count = 0;
        for path in maps.iter().filter(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with(Mode::Assault.map_prefix())
            })
        }) {
            count += 1;
            let bytes = fs::read(path).unwrap();
            let package = Package::read(&bytes).unwrap();
            let mut map = Map::load(&package, Some(path))
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let data = map.assault.as_ref().unwrap();
            assert!(
                !data.briefing.is_empty(),
                "{} has no objective briefing",
                path.display()
            );
            assert!(
                data.briefing
                    .iter()
                    .all(|page| !page.description.trim().is_empty()),
                "{} has empty objective text",
                path.display()
            );
            assert!(
                data.briefing.iter().any(|page| page.image.is_some()),
                "{} has no briefing images",
                path.display()
            );
            let page = data
                .briefing
                .iter()
                .position(|page| page.image.is_some())
                .unwrap();
            let mut briefing_frame = crate::render::Frame::new();
            crate::hud::Hud::new().draw_assault_briefing(&mut briefing_frame, &map, page);
            let colors: std::collections::HashSet<_> =
                briefing_frame.pixels.iter().copied().collect();
            assert!(
                colors.len() > 32,
                "{} briefing image was not drawn",
                path.display()
            );
            for turret in &data.turrets {
                assert!(
                    !turret.activate.frames.is_empty() && turret.activate.duration() < 3.0,
                    "{} activation {:?}",
                    path.display(),
                    turret.activate.duration()
                );
                assert!(
                    turret.fire.iter().all(|motion| !motion.frames.is_empty()),
                    "{} firing frames",
                    path.display()
                );
                assert!(
                    turret.base.iter().any(|face| face.texture.is_some())
                        && turret.audio[0].is_some()
                        && turret.audio[1].is_some(),
                    "{} mesh and sounds",
                    path.display()
                );
                if !turret.minigun {
                    assert!(
                        turret
                            .projectile
                            .as_ref()
                            .is_some_and(|shot| shot.speed > 0.0 && !shot.sprites.is_empty()),
                        "{} projectile",
                        path.display()
                    );
                }
            }
            assert!(
                data.forts.len() >= 3 && data.seconds >= 240,
                "{}",
                path.display()
            );
            assert!(
                map.hud.ready() && map.faces.iter().any(|face| face.texture.is_some()),
                "{}",
                path.display()
            );
            if path.file_stem().is_some_and(|name| name == "AS-Rook") {
                assert!(
                    map.faces.iter().all(|face| face.texture.is_some()),
                    "AS-Rook is missing BSP materials"
                );
            }
            let mut game = Match::new(
                &map,
                0,
                Rules {
                    mode: Mode::Assault,
                    bots: 3,
                    ..Rules::default()
                },
            );
            let first = game.fighters[0].walker.position;
            for _ in 0..60 {
                game.step(&mut map, Command::default(), 1.0 / 120.0);
            }
            assert_eq!(game.assault_status(&map).map(|state| state.half), Some(1));
            assert!(
                game.fighters.iter().all(|fighter| fighter.health > 0),
                "{}",
                path.display()
            );
            game.time = f32::from(map.assault.as_ref().unwrap().seconds);
            game.step(&mut map, Command::default(), 1.0 / 120.0);
            if game.training_assault() {
                assert!(game.finished);
                continue;
            }
            assert_eq!(game.assault_status(&map).map(|state| state.half), Some(2));
            let second = game.fighters[0].walker.position;
            assert!(
                (second - first).dot(second - first) > 17.0_f32.powi(2),
                "same spawn after side switch on {}",
                path.display()
            );
        }
        assert!(count >= 8);
    }

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn installed_objective_catalog_loads_and_simulates_each_arena() {
        use crate::game::deathmatch::{Command, Match, Rules};

        let reference = Path::new("local/reference-client/Maps");
        let catalog = match_catalog(&reference.join("DM-Deck16][.unr"), [reference]);
        for (mode, minimum) in [(Mode::CaptureTheFlag, 20), (Mode::Domination, 11)] {
            let paths = catalog.iter().filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .to_ascii_lowercase()
                        .starts_with(mode.map_prefix())
                })
            });
            let mut count = 0;
            for path in paths {
                count += 1;
                let data = fs::read(path).unwrap();
                let package = Package::read(&data).unwrap();
                let mut map = Map::load(&package, Some(path))
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                assert!(
                    map.supports_mode(mode),
                    "{} has no objectives",
                    path.display()
                );
                assert!(
                    map.faces.iter().any(|face| face.texture.is_some()),
                    "{} has no textured faces",
                    path.display()
                );
                assert!(
                    map.hud.ready() && !map.arsenal.is_empty(),
                    "{} lacks HUD or weapons",
                    path.display()
                );
                let mut game = Match::new(
                    &map,
                    0,
                    Rules {
                        mode,
                        bots: 1,
                        ..Rules::default()
                    },
                );
                for _ in 0..60 {
                    game.step(&mut map, Command::default(), 1.0 / 120.0);
                }
                assert!(
                    !game.finished && game.fighters.iter().all(|fighter| fighter.health > 0),
                    "{} failed to start",
                    path.display()
                );
            }
            assert!(count >= minimum, "{} has only {count} maps", mode.label());
        }
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
        assert!(front.art.skins[0].bar_ends.iter().all(Option::is_some));
        assert!(front.art.skins[0].bar_hover.iter().all(Option::is_some));
        let panel = front.art.skins[0].panel.as_ref().unwrap().image(0.0);
        assert_eq!(panel.sample(1.5, 0.5, true), panel.sample(1.5, 0.5, false));
        assert!(panel.bgra().iter().all(|pixel| pixel >> 24 == 255));
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
