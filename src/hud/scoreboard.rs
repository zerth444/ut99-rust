use super::{GlyphAtlas, Hud, glyph_index};
use crate::{
    game::deathmatch::{AssaultStatus, Match, Mode},
    map::Map,
    render::Frame,
};

const TEXT: [f32; 3] = [0.96, 0.96, 0.93];
const MUTED: [f32; 3] = [0.72, 0.84, 0.90];
const SCORE: [f32; 3] = [0.57, 0.92, 0.96];
const LOCAL: [f32; 3] = [1.0, 0.85, 0.38];
const TEAM: [[f32; 3]; 2] = [[1.0, 0.47, 0.42], [0.40, 0.72, 1.0]];

pub(crate) const SCENE_DIM: f32 = 0.36;
const HEAD: f32 = 80.0;
const TEAM_HEAD: f32 = 44.0;
const FREE_ROW: f32 = 22.0;
const TEAM_ROW: f32 = 29.0;

impl Hud {
    pub(super) fn draw_scoreboard(&self, frame: &mut Frame, game: &Match, map: &Map, help: &str) {
        let mode = game.mode();
        let team_scores = game.team_scores();
        let standings = game.standings();
        let teams = team_scores.map(|_| {
            [0, 1].map(|team| {
                standings
                    .iter()
                    .copied()
                    .filter(|&id| game.fighters[id].team == Some(team))
                    .collect::<Vec<_>>()
            })
        });
        let aspect = frame.width as f32 / frame.height.max(1) as f32;
        let stacked = teams.is_some() && (aspect < 1.28 || frame.width < 660);
        let side = teams.is_none() && game.finished && aspect >= 1.5 && frame.width >= 760;
        let width = match (&teams, stacked, side) {
            (Some(_), true, _) => 480.0,
            (Some(_), false, _) => 936.0,
            (None, _, true) => 410.0,
            _ => 610.0,
        };
        let group_height = |count: usize| TEAM_HEAD + TEAM_ROW * count.max(1) as f32;
        let roster_height = match &teams {
            Some([a, b]) if stacked => group_height(a.len()) + group_height(b.len()) + 16.0,
            Some([a, b]) => group_height(a.len().max(b.len())),
            None => 27.0 + FREE_ROW * standings.len() as f32,
        };
        let point_strip = mode == Mode::Domination && !map.control_points.is_empty();
        let after_roster = HEAD + roster_height + if point_strip { 29.0 } else { 0.0 };
        let height = after_roster + 51.0;
        let available_width = (frame.width as f32 * if side { 0.45 } else { 0.93 }) - 16.0;
        let scale = (available_width / width)
            .min((frame.height as f32 - 28.0) / height)
            .clamp(0.1, 1.55);
        let board = Board {
            glyphs: self.scoreboard_glyphs.as_ref().or(self.glyphs.as_ref()),
            origin: [
                if side {
                    16.0
                } else {
                    (frame.width as f32 - width * scale) * 0.5
                },
                (frame.height as f32 - height * scale) * if side { 0.5 } else { 0.20 },
            ],
            scale,
        };
        let shade = if side {
            board.rect(0.0, 0.0, width, height)
        } else {
            [0, 0, frame.width as i32, frame.height as i32]
        };
        frame.shade(shade, [0.0; 3], SCENE_DIM);

        let assault = game.assault_status(map);
        let training = game.training_assault() || game.training_ctf() || game.training_deathmatch();
        let result = result_title(game, team_scores, training);
        let title = result.as_deref().unwrap_or(mode.label());
        board.center(
            frame,
            [width * 0.5, 4.0],
            width - 24.0,
            1.45,
            title,
            if result.is_some() { LOCAL } else { TEXT },
        );

        let (limit, seconds) = game.limits();
        let context = if let Some(state) = &assault {
            let round = if state.tutorial {
                "Exercise".to_owned()
            } else {
                format!("Round {}/2", state.half)
            };
            let progress = format!(
                "{round}   /   {}/{} objectives",
                state.completed, state.total
            );
            match state.objective.filter(|_| !game.finished) {
                Some(name) => format!("{progress}   /   {name}"),
                None => progress,
            }
        } else if training {
            "Training".to_owned()
        } else if game.overtime {
            "Sudden death".to_owned()
        } else if limit > 0 && !game.finished {
            format!("{}  {limit}", mode.goal_label())
        } else {
            String::new()
        };
        board.center(
            frame,
            [width * 0.5, 46.0],
            width - 24.0,
            1.00,
            &context,
            TEXT,
        );

        if let Some((teams, scores)) = teams.zip(team_scores) {
            let pane_width = if stacked { width - 28.0 } else { width * 0.42 };
            let flags = game.flag_status();
            let mut y = HEAD;
            for (team, players) in teams.iter().enumerate() {
                let x = if team == 0 || stacked {
                    14.0
                } else {
                    width - pane_width - 14.0
                };
                let color = TEAM[team];
                board.label(
                    frame,
                    [x, y],
                    pane_width - 128.0,
                    1.35,
                    game.team_label(team),
                    color,
                );
                let (status, total) = if let Some(state) = &assault {
                    let attacking = team + 1 == state.half;
                    let status = if attacking { "Attack" } else { "Defense" };
                    (
                        status.to_owned(),
                        assault_total(state, attacking, game.finished),
                    )
                } else {
                    let status = match flags.map(|flags| flags[team]) {
                        Some("You have it") => "Flag carried by you".to_owned(),
                        Some(flag) => format!("Flag {}", flag.to_ascii_lowercase()),
                        None => String::new(),
                    };
                    (status, scores[team].to_string())
                };
                board.label(
                    frame,
                    [x, y + 23.0],
                    pane_width - 128.0,
                    0.77,
                    &status,
                    MUTED,
                );
                board.right(
                    frame,
                    [x + pane_width, y + 1.0],
                    if assault.is_some() { 0.90 } else { 1.45 },
                    &total,
                    color,
                );
                draw_team(
                    frame,
                    game,
                    &board,
                    [x, y + TEAM_HEAD, pane_width],
                    players,
                    color,
                );
                if stacked {
                    y += group_height(players.len()) + 16.0;
                }
            }
        } else {
            draw_free_for_all(frame, game, &board, width, &standings);
        }
        if point_strip {
            let span = width - 28.0;
            let step = span / map.control_points.len() as f32;
            for (i, (point, owner)) in map
                .control_points
                .iter()
                .zip(game.control_owners())
                .enumerate()
            {
                let label = match owner {
                    Some(0) => format!("{}  /  Red", point.name),
                    Some(_) => format!("{}  /  Blue", point.name),
                    None => format!("{}  /  Neutral", point.name),
                };
                board.center(
                    frame,
                    [14.0 + step * (i as f32 + 0.5), after_roster - 22.0],
                    step - 8.0,
                    0.75,
                    &label,
                    owner.map_or(MUTED, |team| TEAM[team]),
                );
            }
        }

        let elapsed = clock(game.time);
        let timer = if let Some(time) = assault.as_ref().and_then(|state| state.attack_time) {
            format!("Attack time  {}", clock(time))
        } else if !game.finished && (seconds > 0 || assault.is_some()) {
            format!(
                "Elapsed  {elapsed}   /   Left  {}",
                clock(f32::from(game.remaining()))
            )
        } else {
            format!("Elapsed  {elapsed}")
        };
        let map_label = if game.finished {
            map.author.as_ref().map_or_else(
                || map.title.clone(),
                |author| format!("{}  /  {author}", map.title),
            )
        } else {
            map.title.clone()
        };
        let footer = if side {
            after_roster + 7.0
        } else {
            let bottom = (frame.height as f32 - board.origin[1] - 14.0) / board.scale;
            (bottom - 43.0).max(after_roster + 7.0)
        };
        board.label(
            frame,
            [14.0, footer],
            width - 44.0 - board.measure(&timer, 0.82),
            0.78,
            &map_label,
            MUTED,
        );
        board.right(frame, [width - 14.0, footer], 0.82, &timer, TEXT);
        if game.finished && !help.is_empty() {
            board.center(
                frame,
                [width * 0.5, footer + 21.0],
                width - 28.0,
                0.81,
                help,
                LOCAL,
            );
        } else if !game.finished && game.respawn_left().is_some() {
            board.center(
                frame,
                [width * 0.5, footer + 21.0],
                width - 28.0,
                0.80,
                "Waiting to respawn",
                MUTED,
            );
        }
    }
}

fn draw_free_for_all(
    frame: &mut Frame,
    game: &Match,
    board: &Board<'_>,
    width: f32,
    players: &[usize],
) {
    let lms = game.mode() == Mode::LastManStanding;
    let name_x = 18.0;
    let score_x = width * 0.77;
    let deaths_x = width - 16.0;
    let frag_x = width * 0.60;
    let name_width = width * if lms { 0.42 } else { 0.53 };
    board.label(frame, [name_x, HEAD], name_width, 0.88, "Player", TEXT);
    if lms {
        board.right(frame, [frag_x, HEAD], 0.88, "Frags", TEXT);
    }
    board.right(
        frame,
        [score_x, HEAD],
        0.88,
        if lms { "Lives" } else { "Frags" },
        TEXT,
    );
    board.right(frame, [deaths_x, HEAD], 0.88, "Deaths", TEXT);
    for (row, &id) in players.iter().enumerate() {
        let y = HEAD + 27.0 + row as f32 * FREE_ROW;
        let fighter = &game.fighters[id];
        let lives = game.lives_left(id);
        let color = if id == 0 { LOCAL } else { SCORE };
        if id == 0 {
            frame.solid(board.rect(name_x - 9.0, y + 6.0, 3.0, 8.0), LOCAL);
        }
        let name = match lives {
            Some(0) => format!("{}  (out)", game.fighter_label(id)),
            _ => game.fighter_label(id),
        };
        board.label(frame, [name_x, y], name_width, 0.90, &name, color);
        if lms {
            board.right(frame, [frag_x, y], 0.90, &fighter.score.to_string(), color);
        }
        let score = lives.map_or_else(|| fighter.score.to_string(), |left| left.to_string());
        board.right(frame, [score_x, y], 0.90, &score, color);
        board.right(
            frame,
            [deaths_x, y],
            0.90,
            &fighter.deaths.to_string(),
            color,
        );
    }
}

fn draw_team(
    frame: &mut Frame,
    game: &Match,
    board: &Board<'_>,
    pane: [f32; 3],
    players: &[usize],
    team_color: [f32; 3],
) {
    let [x, y, width] = pane;
    for (row, &id) in players.iter().enumerate() {
        let top = y + row as f32 * TEAM_ROW;
        let fighter = &game.fighters[id];
        let color = if id == 0 { LOCAL } else { TEXT };
        if id == 0 {
            frame.solid(board.rect(x - 7.0, top + 6.0, 3.0, 7.0), LOCAL);
        }
        let carrier = game.fighter_carries_flag(id);
        let name = game.fighter_label(id);
        board.label(
            frame,
            [x, top],
            width - if carrier { 101.0 } else { 56.0 },
            1.02,
            &name,
            color,
        );
        let right = x + width;
        board.right(
            frame,
            [right, top],
            1.03,
            &fighter.score.to_string(),
            if id == 0 { LOCAL } else { team_color },
        );
        if carrier {
            board.right(frame, [right - 56.0, top + 3.0], 0.72, "FLAG", MUTED);
        }
        let stats = &fighter.stats;
        let kill_count = if matches!(game.mode(), Mode::CaptureTheFlag | Mode::Domination) {
            stats.kills
        } else {
            0
        };
        let detail = [
            (stats.captures, "caps"),
            (stats.returns, "returns"),
            (stats.points_taken, "taken"),
            (kill_count, "kills"),
            (fighter.deaths, "deaths"),
        ]
        .into_iter()
        .filter(|&(count, _)| count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join("  /  ");
        board.label(frame, [x, top + 16.0], width - 42.0, 0.65, &detail, MUTED);
    }
}

fn clock(seconds: f32) -> String {
    let seconds = seconds as u32;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn attack_result(time: Option<f32>) -> String {
    time.map_or_else(|| "Time expired".to_owned(), |time| clock(time))
}

fn assault_total(status: &AssaultStatus<'_>, attacking: bool, finished: bool) -> String {
    if attacking {
        if finished || status.attack_time.is_some() {
            attack_result(status.attack_time)
        } else {
            format!("{}/{}", status.completed, status.total)
        }
    } else if let Some(time) = status.first_attack {
        attack_result(time)
    } else {
        String::new()
    }
}

fn result_title(game: &Match, team_scores: Option<[i16; 2]>, training: bool) -> Option<String> {
    if !game.finished {
        return None;
    }
    Some(if training {
        if game.player_won_rated_match() {
            "Training complete"
        } else {
            "Training failed"
        }
        .to_owned()
    } else if let Some([red, blue]) = team_scores {
        if red == blue {
            "Match drawn".to_owned()
        } else {
            format!("{} wins", game.team_label(usize::from(blue > red)))
        }
    } else {
        match game.winner() {
            Some(0) => "You won".to_owned(),
            Some(id) => format!("{} wins", game.fighter_label(id)),
            None => "Match drawn".to_owned(),
        }
    })
}

struct Board<'a> {
    glyphs: Option<&'a GlyphAtlas>,
    origin: [f32; 2],
    scale: f32,
}

impl Board<'_> {
    fn point(&self, x: f32, y: f32) -> [f32; 2] {
        [
            self.origin[0] + x * self.scale,
            self.origin[1] + y * self.scale,
        ]
    }

    fn rect(&self, x: f32, y: f32, width: f32, height: f32) -> [i32; 4] {
        let [left, top] = self.point(x, y).map(|value| value.round() as i32);
        let [right, bottom] = self
            .point(x + width, y + height)
            .map(|value| value.round() as i32);
        [left, top, (right - left).max(1), (bottom - top).max(1)]
    }

    fn measure(&self, text: &str, size: f32) -> f32 {
        self.glyphs.map_or(0.0, |atlas| {
            text.chars()
                .map(|ch| atlas.advance[glyph_index(ch)])
                .sum::<i32>() as f32
                * size
                * 0.5
        })
    }

    fn label(
        &self,
        frame: &mut Frame,
        [x, y]: [f32; 2],
        width: f32,
        size: f32,
        text: &str,
        tint: [f32; 3],
    ) {
        let fitted;
        let text = if self.measure(text, size) <= width {
            text
        } else {
            let mut remaining = width - self.measure("...", size);
            let end = text
                .char_indices()
                .find_map(|(index, ch)| {
                    remaining -= self.measure(ch.encode_utf8(&mut [0; 4]), size);
                    (remaining < 0.0).then_some(index)
                })
                .unwrap_or(text.len());
            fitted = format!("{}...", text[..end].trim_end());
            &fitted
        };
        let [x, y] = self.point(x, y);
        if let Some(glyphs) = self.glyphs {
            let (x, y) = (x.round() as i32, y.round() as i32);
            glyphs.draw(frame, x, y, size * self.scale, text, tint);
        }
    }

    fn right(
        &self,
        frame: &mut Frame,
        [right, y]: [f32; 2],
        size: f32,
        text: &str,
        tint: [f32; 3],
    ) {
        let width = self.measure(text, size);
        self.label(frame, [right - width, y], width, size, text, tint);
    }

    fn center(
        &self,
        frame: &mut Frame,
        [center, y]: [f32; 2],
        max_width: f32,
        size: f32,
        text: &str,
        tint: [f32; 3],
    ) {
        let width = self.measure(text, size).min(max_width);
        self.label(frame, [center - width * 0.5, y], width, size, text, tint);
    }
}
