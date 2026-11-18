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
    ) {
        frame.begin_ui(false);
        if scoreboard || game.finished {
            self.draw_match_board(frame, game);
            return;
        }
        let has_art = map.hud.ready();
        let has_guidance_art = map
            .arsenal
            .get(&Weapon::Redeemer)
            .is_some_and(|weapon| weapon.guided_hud[0].is_some());
        let scale = (game.hud_scale * frame.width as f32 / 1280.0).clamp(0.55, 1.6);
        let player = &game.fighters[0];
        let seconds = if game.limits().1 == 0 {
            game.time as u32
        } else {
            u32::from(game.remaining())
        };
        let white = [0.92, 0.96, 1.0];
        let accent = [0.42, 0.87, 0.95];
        let clock = if game.overtime {
            "SUDDEN DEATH".to_owned()
        } else if has_art {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        } else {
            format!(
                "DEATHMATCH   {:02}:{:02}   FRAGS {}   {}",
                seconds / 60,
                seconds % 60,
                player.score,
                game.difficulty().label().to_ascii_uppercase()
            )
        };
        self.text(frame, 16, 12, scale, &clock, accent);
        for (line, message) in game.notices().enumerate() {
            self.text(
                frame,
                16,
                60 + (line as f32 * 22.0 * scale) as i32,
                scale,
                message,
                white,
            );
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
            self.text(
                frame,
                16,
                if has_art {
                    60 + (4.0 * 22.0 * scale) as i32
                } else {
                    frame.height as i32 - (61.0 * scale) as i32
                },
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
        if let Some(seconds) = game.respawn_left() {
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

    fn draw_match_board(&self, frame: &mut Frame, game: &Match) {
        let standings = game.standings();
        let scale = (frame.width as f32 / 790.0)
            .min(frame.height as f32 / (168.0 + standings.len() as f32 * 30.0))
            .min(1.5);
        let px = |value: f32| (value * scale).round() as i32;
        let width = px(if game.finished { 365.0 } else { 640.0 });
        let height = px(144.0 + standings.len() as f32 * 30.0);
        let left = if game.finished {
            px(16.0)
        } else {
            (frame.width as i32 - width) / 2
        };
        let top = (frame.height as i32 - height) / 2;
        let white = [0.94, 0.96, 0.98];
        let dim = [0.67, 0.72, 0.78];
        let accent = [0.81, 0.88, 0.98];
        frame.solid([left, top, width, height], [0.075, 0.09, 0.12]);
        frame.solid([left, top, width, px(76.0)], [0.11, 0.15, 0.21]);
        let score = game.fighters[standings[0]].score;
        let title = if game.finished {
            match game.winner() {
                Some(0) => "You won".to_owned(),
                Some(winner) => format!("{} wins", Match::label(winner)),
                None => "Match drawn".to_owned(),
            }
        } else {
            "Deathmatch".to_owned()
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
        let details = if game.finished {
            let reason = if game.overtime {
                "Sudden death"
            } else if frag_limit > 0 && score >= frag_limit {
                "Frag limit"
            } else {
                "Time limit"
            };
            let elapsed = game.time as u32;
            format!("{reason}    {:02}:{:02} played", elapsed / 60, elapsed % 60)
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
                "{:02}:{:02} {label}    Frag limit {frag}",
                seconds / 60,
                seconds % 60
            )
        };
        self.text(
            frame,
            left + px(22.0),
            top + px(47.0),
            scale * 0.89,
            &details,
            dim,
        );
        let columns = if game.finished {
            [19.0, 78.0, 234.0, 289.0]
        } else {
            [22.0, 100.0, 427.0, 528.0]
        };
        for (x, title) in columns
            .into_iter()
            .zip(["Rank", "Fighter", "Frags", "Deaths"])
        {
            self.text(
                frame,
                left + px(x),
                top + px(84.0),
                scale * 0.88,
                title,
                dim,
            );
        }
        let mut last_score = None;
        let mut rank = 0;
        for (index, id) in standings.into_iter().enumerate() {
            let y = top + px(107.0 + index as f32 * 30.0);
            let fighter = &game.fighters[id];
            if Some(fighter.score) != last_score {
                rank = index + 1;
                last_score = Some(fighter.score);
            }
            let tint = if id == 0 {
                [0.19, 0.27, 0.38]
            } else if index % 2 == 0 {
                [0.106, 0.13, 0.17]
            } else {
                [0.085, 0.11, 0.15]
            };
            frame.solid([left, y, width, px(30.0)], tint);
            if id == 0 {
                frame.solid([left, y, px(3.0).max(1), px(30.0)], accent);
            }
            for (x, value) in columns.into_iter().zip([
                rank.to_string(),
                if id == 0 {
                    "PLAYER (YOU)".into()
                } else {
                    Match::label(id)
                },
                fighter.score.to_string(),
                fighter.deaths.to_string(),
            ]) {
                self.text(frame, left + px(x), y + px(5.0), scale, &value, white);
            }
        }
        if game.finished {
            self.text(
                frame,
                left + px(19.0),
                top + height - px(27.0),
                scale * 0.8,
                "Enter for rematch    Esc for menu",
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
            self.text(frame, 12, y, scale, stats, [0.72, 0.95, 0.78]);
            y += (CELL_H as f32 * scale) as i32;
        }
        if let Some(track) = music {
            self.text(
                frame,
                12,
                y,
                scale,
                &format!("NOW PLAYING  {track}"),
                [0.77, 0.88, 0.93],
            );
        }
    }

    fn centered(&self, frame: &mut Frame, y: i32, scale: f32, text: &str, color: [f32; 3]) {
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
