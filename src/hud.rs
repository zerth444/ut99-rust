use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::{RECT, SIZE},
    Graphics::Gdi::*,
};

use crate::{
    game::{ArmorKind, Item, Powerup, Practice, Weapon, deathmatch::Match},
    map::Map,
    render::Frame,
    texture::TextureImage,
};

const CELL_W: usize = 42;
const CELL_H: usize = 44;
const COLUMNS: usize = 16;
const ROWS: usize = 6;

pub struct Hud {
    glyphs: Option<GlyphAtlas>,
}

struct GlyphAtlas {
    image: TextureImage,
    advance: [i32; 95],
}

impl Hud {
    pub fn new() -> Self {
        Self {
            glyphs: glyph_atlas(),
        }
    }

    pub fn draw_deathmatch(
        &self,
        frame: &mut Frame,
        game: &Match,
        map: &Map,
        scoreboard: bool,
        captured: bool,
        result_help: &str,
    ) {
        frame.begin_ui(false);
        if scoreboard || game.finished {
            self.draw_match_board(frame, game, map, result_help);
            if game.finished
                && let Some(subtitle) = game.dm_tutorial_subtitle(map)
            {
                let scale = (game.hud_scale * frame.width as f32 / 1280.0).clamp(0.55, 1.6);
                self.draw_tutorial_subtitle(frame, &subtitle, scale);
            }
            return;
        }
        let has_art = map.hud.ready();
        let has_guidance_art = map
            .arsenal
            .get(&Weapon::Redeemer)
            .is_some_and(|weapon| weapon.guided_hud[0].is_some());
        let scale = (game.hud_scale * frame.width as f32 / 1280.0).clamp(0.55, 1.6);
        let player = &game.fighters[0];
        let white = [0.92, 0.96, 1.0];
        let accent = [0.42, 0.87, 0.95];
        if let Some(status) = game.assault_status(map) {
            let seconds = game.remaining();
            let role = if status.attacking { "Attack" } else { "Defend" };
            let round = if status.tutorial {
                "Training".to_owned()
            } else {
                format!("Round {}/2", status.half)
            };
            self.centered(
                frame,
                (22.0 * scale) as i32,
                scale,
                &format!(
                    "{round}    {}/{} objectives    {:02}:{:02}",
                    status.completed,
                    status.total,
                    seconds / 60,
                    seconds % 60
                ),
                white,
            );
            if let Some((name, message)) = status.notice {
                self.centered(
                    frame,
                    (47.0 * scale) as i32,
                    scale,
                    &format!("{name} {message}"),
                    accent,
                );
            } else {
                let objective = status
                    .objective
                    .map_or_else(|| role.to_owned(), |name| format!("{role} {name}"));
                self.centered(frame, (47.0 * scale) as i32, scale, &objective, accent);
            }
        } else if let Some([red, blue]) = game.team_scores() {
            let team = if player.team == Some(0) {
                "Red"
            } else {
                "Blue"
            };
            self.centered(
                frame,
                (22.0 * scale) as i32,
                scale,
                &format!("Red {red}   -   {blue} Blue    |    Team {team}"),
                white,
            );
        } else if let Some(lives) = game.lives_left(0) {
            self.centered(
                frame,
                (22.0 * scale) as i32,
                scale,
                &format!("Lives {lives}"),
                white,
            );
        }
        if let Some(subtitle) = game.dm_tutorial_subtitle(map) {
            self.draw_tutorial_subtitle(frame, &subtitle, scale);
        } else if let Some(subtitle) = game.tutorial_subtitle(map) {
            self.draw_tutorial_subtitle(frame, subtitle, scale);
        }
        if let Some([red, blue]) = game.flag_status() {
            self.centered(
                frame,
                (45.0 * scale) as i32,
                0.83 * scale,
                &format!("Red flag: {red}    Blue flag: {blue}"),
                white,
            );
            if game.rated_capture_required() {
                self.centered(
                    frame,
                    (64.0 * scale) as i32,
                    0.83 * scale,
                    "Capture a flag yourself to advance",
                    accent,
                );
            }
        }
        if !has_art {
            self.text(
                frame,
                16,
                frame.height as i32 - (36.0 * scale) as i32,
                scale,
                &format!(
                    "HEALTH {}   ARMOR {}   {}   {}",
                    player.health,
                    player.armor(),
                    player.weapon.label(),
                    player
                        .ammo()
                        .map_or_else(|| "MELEE".to_owned(), |ammo| format!("AMMO {ammo}"))
                ),
                white,
            );
        }
        let mut active = Vec::new();
        if !has_art && player.boot_charges() > 0 {
            active.push(format!("BOOTS {}", player.boot_charges()));
        }
        if !has_art && player.amp_left(game.time) > 0.0 {
            active.push(format!("AMP {:.0}S", player.amp_left(game.time).ceil()));
        }
        if player.invis_left(game.time) > 0.0 {
            active.push(format!("CLOAK {:.0}S", player.invis_left(game.time).ceil()));
        }
        if !active.is_empty() {
            self.centered(
                frame,
                (frame.height * 3 / 4) as i32,
                scale,
                &active.join("   "),
                accent,
            );
        }
        if !has_art && let Some((units, progress)) = player.charge_status(map, game.time) {
            let width = (135.0 * scale) as i32;
            let x = (frame.width as i32 - width) / 2;
            let y = frame.height as i32 / 2 + (40.0 * scale) as i32;
            self.centered(
                frame,
                y - (21.0 * scale) as i32,
                scale,
                &format!("CHARGE {units}"),
                accent,
            );
            frame.solid([x, y, width, 4], [0.08, 0.13, 0.17]);
            frame.solid([x, y, (width as f32 * progress) as i32, 4], accent);
        }
        if !has_guidance_art && let Some(seconds) = game.guidance_left() {
            self.centered(
                frame,
                frame.height as i32 / 2 + (42.0 * scale) as i32,
                scale,
                &format!("GUIDANCE  {:.0}S", seconds.ceil()),
                accent,
            );
            self.centered(
                frame,
                frame.height as i32 / 2 + (65.0 * scale) as i32,
                scale,
                "PRESS FIRE TO DETONATE",
                white,
            );
        }
        if captured
            && player.health > 0
            && game.guidance_left().is_none()
            && let Some((health, armor, age)) = game.damage_notice()
        {
            let remaining = ((0.9 - age) / 0.25).clamp(0.0, 1.0);
            let text = if armor > 0 {
                format!("{health} HP + {armor} ARMOR")
            } else {
                format!("{health} DAMAGE")
            };
            self.centered(
                frame,
                frame.height as i32 / 2 - (58.0 * scale) as i32,
                scale,
                &text,
                [1.0, 0.78, 0.34].map(|channel| channel * remaining),
            );
        }
        if let Some(announcement) = game.announcement() {
            self.centered(
                frame,
                (84.0 * scale) as i32,
                1.2 * scale,
                announcement,
                [0.96, 0.83, 0.45],
            );
        }
        if game.lives_left(0) == Some(0) {
            self.centered(frame, frame.height as i32 / 2, scale, "Eliminated", white);
        } else if let Some(seconds) = game.respawn_left() {
            self.centered(
                frame,
                frame.height as i32 / 2,
                scale,
                &format!("Respawn in {seconds:.1}s"),
                white,
            );
        } else if !captured {
            self.centered(
                frame,
                frame.height as i32 / 2,
                scale,
                "CLICK TO CAPTURE MOUSE",
                white,
            );
        } else if let Some(item) = game.pickup_notice() {
            let name = match item {
                Item::Gun(weapon) => weapon.label().to_owned(),
                Item::Ammo(weapon, _) => format!("{} ammo", weapon.label()),
                Item::Health(..) => "Health".to_owned(),
                Item::Armor(ArmorKind::Belt) => "Shield Belt".to_owned(),
                Item::Armor(ArmorKind::Chest) => "Body Armor".to_owned(),
                Item::Armor(ArmorKind::Thigh) => "Thigh Pads".to_owned(),
                Item::Power(Powerup::Boots, _) => "Jump Boots".to_owned(),
                Item::Power(Powerup::Damage, _) => "Damage Amplifier".to_owned(),
                Item::Power(Powerup::Invisibility, _) => "Invisibility".to_owned(),
            };
            self.centered(
                frame,
                (111.0 * scale) as i32,
                scale,
                &format!("Picked up {name}"),
                white,
            );
        }
    }

    fn draw_match_board(&self, frame: &mut Frame, game: &Match, map: &Map, result_help: &str) {
        let standings = game.standings();
        let scale = (frame.width as f32 / 790.0)
            .min(frame.height as f32 / (191.0 + standings.len() as f32 * 27.0))
            .min(1.5);
        let px = |value: f32| (value * scale).round() as i32;
        let width = px(if game.finished { 365.0 } else { 530.0 });
        let height = px(164.0 + standings.len() as f32 * 27.0);
        let left = if game.finished {
            px(16.0)
        } else {
            (frame.width as i32 - width) / 2
        };
        let top = (frame.height as i32 - height) / 2;
        let white = [0.95, 0.96, 0.98];
        let dim = [0.76, 0.8, 0.85];
        let accent = [0.61, 0.84, 0.90];
        frame.shade([left, top, width, height], [0.045, 0.062, 0.078], 0.74);
        frame.solid([left, top, width, px(2.0).max(1)], [0.36, 0.58, 0.65]);
        let title = if game.finished && (game.training_assault() || game.training_deathmatch()) {
            if game.winner() == Some(0) {
                "Training complete".into()
            } else {
                "Training failed".into()
            }
        } else if game.finished {
            match game
                .winner()
                .map(|winner| (winner, game.fighters[winner].team))
            {
                Some((_, Some(team))) => format!("{} wins", game.team_label(team)),
                Some((0, None)) => "You won".to_owned(),
                Some((winner, None)) => format!("{} wins", game.fighter_label(winner)),
                None => "Match drawn".to_owned(),
            }
        } else if game.training_assault() {
            "Assault training".into()
        } else if game.training_deathmatch() {
            "Deathmatch training".into()
        } else {
            game.mode().label().to_owned()
        };
        self.text(
            frame,
            left + px(22.0),
            top + px(11.0),
            scale * 1.25,
            &title,
            white,
        );
        let (frag_limit, time_limit) = game.limits();
        let details = if let Some(status) = game.assault_status(map) {
            let seconds = game.remaining();
            let round = if status.tutorial {
                "Training".to_owned()
            } else {
                format!("Round {}/2", status.half)
            };
            format!(
                "{round}    {}/{} objectives    {:02}:{:02}",
                status.completed,
                status.total,
                seconds / 60,
                seconds % 60
            )
        } else if let Some([red, blue]) = game.team_scores() {
            let seconds = if game.finished || time_limit == 0 {
                game.time as u32
            } else {
                u32::from(game.remaining())
            };
            format!(
                "Red {red}   -   {blue} Blue    {:02}:{:02}",
                seconds / 60,
                seconds % 60
            )
        } else if game.finished {
            let elapsed = game.time as u32;
            format!("{:02}:{:02} played", elapsed / 60, elapsed % 60)
        } else if game.overtime {
            "Sudden death    Next clear lead wins".to_owned()
        } else {
            let (seconds, label) = if time_limit == 0 {
                (game.time as u32, "played")
            } else {
                (u32::from(game.remaining()), "remaining")
            };
            let frag = if frag_limit > 0 {
                frag_limit.to_string()
            } else {
                "Off".into()
            };
            format!(
                "{:02}:{:02} {label}    {} {frag}",
                seconds / 60,
                seconds % 60,
                game.mode().goal_label(),
            )
        };
        self.text(
            frame,
            left + px(22.0),
            top + px(41.0),
            scale * 0.89,
            &details,
            dim,
        );
        if let Some((allies, enemy)) = game.named_rated_teams() {
            self.text(
                frame,
                left + px(22.0),
                top + px(56.0),
                0.85 * scale,
                &format!("{allies} vs {enemy}"),
                dim,
            );
        }
        let columns = if game.finished {
            [22.0, 222.0, 289.0]
        } else {
            [22.0, 334.0, 435.0]
        };
        frame.shade(
            [left, top + px(73.0), width, px(24.0)],
            [0.22, 0.32, 0.36],
            0.58,
        );
        let last_column = if game.lives_left(0).is_some() {
            "Lives"
        } else {
            "Deaths"
        };
        let score_column = if game.flag_status().is_some() {
            "Score"
        } else {
            "Frags"
        };
        for (x, title) in columns
            .into_iter()
            .zip(["Player", score_column, last_column])
        {
            self.text(
                frame,
                left + px(x),
                top + px(77.0),
                scale * 0.88,
                title,
                dim,
            );
        }
        for (index, id) in standings.into_iter().enumerate() {
            let y = top + px(98.0 + index as f32 * 27.0);
            let fighter = &game.fighters[id];
            if id == 0 {
                frame.shade([left, y, width, px(26.0)], [0.18, 0.37, 0.42], 0.63);
            } else if index % 2 == 0 {
                frame.shade([left, y, width, px(26.0)], [0.20, 0.25, 0.31], 0.17);
            }
            for (x, value) in columns.into_iter().zip([
                game.fighter_label(id),
                fighter.score.to_string(),
                game.lives_left(id)
                    .map_or_else(|| fighter.deaths.to_string(), |lives| lives.to_string()),
            ]) {
                self.text(
                    frame,
                    left + px(x),
                    y + px(4.0),
                    scale,
                    &value,
                    match fighter.team {
                        Some(0) => [1.0, 0.48, 0.47],
                        Some(_) => [0.52, 0.73, 1.0],
                        None if id == 0 => accent,
                        None => white,
                    },
                );
            }
        }
        if !map.title.is_empty() {
            let caption = map.author.as_ref().map_or_else(
                || map.title.clone(),
                |author| format!("{}  /  {author}", map.title),
            );
            self.text(
                frame,
                left + px(22.0),
                top + height - px(53.0),
                scale * 0.9,
                &caption
                    .chars()
                    .take(if game.finished { 41 } else { 63 })
                    .collect::<String>(),
                dim,
            );
        }
        if game.finished {
            self.text(
                frame,
                left + px(19.0),
                top + height - px(27.0),
                scale * 0.8,
                result_help,
                white,
            );
        }
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        game: &Practice,
        captured: bool,
        has_art: bool,
        music: Option<&str>,
        stats: Option<&str>,
    ) {
        let scale = (frame.width as f32 / 1280.0).clamp(0.7, 2.0);
        let width = frame.width as i32;
        let height = frame.height as i32;
        if !has_art {
            let ammo = game
                .ammo()
                .map_or_else(|| "MELEE".to_string(), |ammo| ammo.to_string());
            self.text(
                frame,
                12,
                height - 30,
                scale,
                &format!(
                    "HEALTH {}  ARMOR {}  {}  {}",
                    game.health,
                    game.armor(),
                    game.weapon.label(),
                    ammo
                ),
                [0.8, 0.9, 1.0],
            );
        }
        if game.time < game.message_until {
            self.centered(
                frame,
                (50.0 * scale) as i32,
                scale,
                &game.message,
                [1.0, 0.79, 0.32],
            );
        }
        if game.scoped() {
            let magnification = game.base_fov / game.field_of_view();
            self.text(
                frame,
                width / 2 + width / 10,
                height / 2 + (height * 3 / 20),
                scale,
                &format!("X{magnification:.1}"),
                [0.20, 1.0, 0.20],
            );
        }
        if !captured && game.death.is_none() {
            let w = (350.0 * scale).round() as i32;
            let h = (54.0 * scale).round() as i32;
            frame.solid(
                [(width - w) / 2, (height - h) / 2, w, h],
                [0.07, 0.09, 0.12],
            );
            self.centered(
                frame,
                (height - h) / 2 + (7.0 * scale) as i32,
                scale,
                "CLICK TO CAPTURE MOUSE",
                [0.86, 0.94, 0.98],
            );
            self.centered(
                frame,
                height / 2,
                scale,
                "WASD / SPACE / LMB / RMB",
                [0.60, 0.77, 0.84],
            );
        }
        let mut y = (10.0 * scale) as i32;
        if let Some(stats) = stats {
            self.centered(frame, y, scale, stats, [0.72, 0.95, 0.78]);
            y += (CELL_H as f32 * 0.5 * scale) as i32;
        }
        if let Some(track) = music {
            self.centered(
                frame,
                y,
                scale,
                &format!("NOW PLAYING  {track}"),
                [0.77, 0.88, 0.93],
            );
        }
    }

    pub(crate) fn centered(
        &self,
        frame: &mut Frame,
        y: i32,
        scale: f32,
        text: &str,
        color: [f32; 3],
    ) {
        let width = self.glyphs.as_ref().map_or(0.0, |atlas| {
            text.chars()
                .map(|ch| atlas.advance[glyph_index(ch)])
                .sum::<i32>() as f32
                * scale
                * 0.5
        });
        self.text(
            frame,
            (frame.width as f32 * 0.5 - width * 0.5).round() as i32,
            y,
            scale,
            text,
            color,
        );
    }

    fn draw_tutorial_subtitle(&self, frame: &mut Frame, text: &str, scale: f32) {
        let width = (frame.width as f32 * 0.70).round() as i32;
        let lines = self.wrap_lines(text, width - 30, scale);
        if lines.is_empty() {
            return;
        }
        let stride = (19.0 * scale).round() as i32;
        let padding = (8.0 * scale).max(3.0) as i32;
        let height = stride * lines.len() as i32 + 2 * padding;
        let y = (frame.height as i32 * 3 / 4 - height).max(0);
        frame.shade(
            [(frame.width as i32 - width) / 2, y, width, height],
            [0.04, 0.06, 0.09],
            0.82,
        );
        for (index, line) in lines.iter().enumerate() {
            self.centered(
                frame,
                y + padding + stride * index as i32,
                scale,
                line,
                [0.96, 0.96, 0.92],
            );
        }
    }

    pub(crate) fn draw_assault_briefing(&self, frame: &mut Frame, map: &Map, index: usize) {
        let Some(data) = &map.assault else {
            return;
        };
        let Some(page) = data.briefing.get(index) else {
            return;
        };
        frame.begin_ui(false);
        let scale = (frame.height as f32 / 650.0).clamp(0.45, 1.7);
        let px = |size: f32| (size * scale).round() as i32;
        let width = px(670.0).min((frame.width as f32 * 0.88).round() as i32);
        let image = page.image.as_ref().map(|texture| {
            let (w, h) = texture.size();
            let ratio = ((width - px(54.0)) as f32 / w).min(px(275.0) as f32 / h);
            (
                texture.image(0.0),
                (w * ratio).round() as i32,
                (h * ratio).round() as i32,
            )
        });
        let lines = self.wrap_lines(&page.description, width - px(54.0), scale);
        let image_height = image.map_or(0, |(_, _, height)| height + px(14.0));
        let line_height = px(22.0);
        let height = px(102.0) + image_height + lines.len() as i32 * line_height;
        let left = (frame.width as i32 - width) / 2;
        let top = ((frame.height as i32 - height) / 2).max(0);
        frame.shade([left, top, width, height], [0.046, 0.070, 0.092], 0.94);
        frame.solid([left, top, width, px(2.0).max(1)], [0.31, 0.55, 0.62]);
        self.centered(
            frame,
            top + px(14.0),
            scale * 1.2,
            &map.title,
            [0.92, 0.97, 1.0],
        );
        if let Some((image, w, h)) = image {
            let (tw, th) = image.size();
            frame.tile(
                image,
                [(frame.width as i32 - w) / 2, top + px(51.0), w, h],
                [0.0, 0.0, tw, th],
                [1.0; 3],
            );
        }
        for (row, line) in lines.iter().enumerate() {
            self.centered(
                frame,
                top + px(51.0) + image_height + row as i32 * line_height,
                scale,
                line,
                [0.94, 0.95, 0.98],
            );
        }
        let hint = if data.briefing.len() > 1 {
            format!(
                "{} / {}     Left / Right     F3 Close",
                index + 1,
                data.briefing.len()
            )
        } else {
            "F3 Close".into()
        };
        self.centered(
            frame,
            top + height - px(29.0),
            scale * 0.83,
            &hint,
            [0.65, 0.80, 0.89],
        );
    }

    fn wrap_lines(&self, text: &str, width: i32, scale: f32) -> Vec<String> {
        let Some(glyphs) = &self.glyphs else {
            return Vec::new();
        };
        let steps = |word: &str| {
            word.chars()
                .map(|ch| glyphs.advance[glyph_index(ch)])
                .sum::<i32>()
        };
        let limit = (width as f32 * 2.0 / scale) as i32;
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut advance = 0;
        let space = steps(" ");
        for word in text.split_whitespace() {
            let word_width = steps(word);
            if !line.is_empty() && advance + space + word_width > limit {
                lines.push(std::mem::take(&mut line));
                advance = 0;
            }
            if !line.is_empty() {
                line.push(' ');
                advance += space;
            }
            line.push_str(word);
            advance += word_width;
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    pub(crate) fn text(
        &self,
        frame: &mut Frame,
        x: i32,
        y: i32,
        scale: f32,
        text: &str,
        color: [f32; 3],
    ) {
        let Some(glyphs) = &self.glyphs else {
            return;
        };
        let scale = scale * 0.5;
        let mut advance = 0;
        for ch in text.chars() {
            let index = glyph_index(ch);
            let left = x + (advance as f32 * scale).round() as i32;
            advance += glyphs.advance[index];
            if ch == ' ' {
                continue;
            }
            frame.translucent_tile(
                &glyphs.image,
                [
                    left,
                    y,
                    (CELL_W as f32 * scale).round() as i32,
                    (CELL_H as f32 * scale).round() as i32,
                ],
                [
                    (index % COLUMNS * CELL_W) as f32,
                    (index / COLUMNS * CELL_H) as f32,
                    CELL_W as f32,
                    CELL_H as f32,
                ],
                color,
            );
        }
    }
}

fn glyph_index(ch: char) -> usize {
    if (' '..='~').contains(&ch) {
        ch as usize - 32
    } else {
        usize::from(b'?' - 32)
    }
}

fn glyph_atlas() -> Option<GlyphAtlas> {
    let (width, height) = (CELL_W * COLUMNS, CELL_H * ROWS);
    let bitmap = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bits = null_mut();
        let dib = CreateDIBSection(dc, &bitmap, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous_bitmap = SelectObject(dc, dib);
        let face: Vec<_> = "Segoe UI\0".encode_utf16().collect();
        let font = CreateFontW(
            -36,
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.into(),
            OUT_DEFAULT_PRECIS.into(),
            CLIP_DEFAULT_PRECIS.into(),
            ANTIALIASED_QUALITY.into(),
            (FF_SWISS | VARIABLE_PITCH).into(),
            face.as_ptr(),
        );
        let previous_font = SelectObject(dc, font);
        std::ptr::write_bytes(bits.cast::<u32>(), 0, width * height);
        SetTextColor(dc, 0xffffff);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut advance = [0; 95];
        for (index, character) in (32u16..=126).enumerate() {
            let x = (index % COLUMNS * CELL_W) as i32;
            let y = (index / COLUMNS * CELL_H) as i32;
            let cell = RECT {
                left: x,
                top: y,
                right: x + CELL_W as i32,
                bottom: y + CELL_H as i32,
            };
            let mut size = SIZE::default();
            GetTextExtentPoint32W(dc, &character, 1, &mut size);
            advance[index] = size.cx.max(1);
            ExtTextOutW(
                dc,
                x,
                y,
                ETO_CLIPPED,
                &cell,
                &character,
                1,
                std::ptr::null(),
            );
        }
        GdiFlush();
        let pixels = std::slice::from_raw_parts(bits.cast::<u32>(), width * height)
            .iter()
            .map(|pixel| (pixel & 255) as u8)
            .collect();
        SelectObject(dc, previous_font);
        SelectObject(dc, previous_bitmap);
        DeleteObject(font);
        DeleteObject(dib);
        DeleteDC(dc);
        Some(GlyphAtlas {
            image: TextureImage::glyph_mask(width, height, pixels),
            advance,
        })
    }
}
