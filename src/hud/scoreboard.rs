use super::{GlyphAtlas, Hud, glyph_index};
use crate::{
    game::deathmatch::{AssaultStatus, Match, Mode},
    map::Map,
    render::Frame,
};

const TEXT: [f32; 3] = [0.96, 0.95, 0.91];
const MUTED: [f32; 3] = [0.76, 0.81, 0.84];
const ACCENT: [f32; 3] = [1.0, 0.82, 0.45];
const TEAM: [[f32; 3]; 2] = [[1.0, 0.55, 0.49], [0.49, 0.79, 1.0]];
const LINE: [f32; 3] = [0.34, 0.42, 0.47];
const HEAD: f32 = 90.0;
const TEAM_HEAD: f32 = 62.0;
const ROW: f32 = 42.0;
const INSET: f32 = 16.0;
const GAP: f32 = 36.0;

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
        let stacked = teams.is_some() && (aspect < 1.35 || frame.width < 720);
        let side = teams.is_none() && game.finished && aspect >= 1.5 && frame.width >= 760;
        let pane_width = if teams.is_some() { 352.0 } else { 500.0 };
        let width = 2.0 * INSET
            + pane_width
                * if teams.is_some() && !stacked {
                    2.0
                } else {
                    1.0
                }
            + if teams.is_some() && !stacked {
                GAP
            } else {
                0.0
            };
        let table_height = |count: usize| ROW * count.max(1) as f32;
        let tables = match &teams {
            Some([a, b]) if stacked => {
                2.0 * TEAM_HEAD + table_height(a.len()) + table_height(b.len()) + GAP
            }
            Some([a, b]) => TEAM_HEAD + table_height(a.len().max(b.len())),
            None => table_height(standings.len()),
        };
        let point_strip = mode == Mode::Domination && !map.control_points.is_empty();
        let footer = HEAD + tables + if point_strip { 47.0 } else { 12.0 };
        let height = footer + 32.0;
        let available_width = frame.width as f32 * if side { 0.46 } else { 1.0 } - 24.0;
        let scale = (available_width / width)
            .min((frame.height as f32 - 28.0) / height)
            .clamp(0.1, 1.35);
        let board = Board {
            glyphs: self.scoreboard_glyphs.as_ref().or(self.glyphs.as_ref()),
            origin: [
                if side {
                    14.0
                } else {
                    (frame.width as f32 - width * scale) * 0.5
                },
                (frame.height as f32 - height * scale) * if side { 0.5 } else { 0.31 },
            ],
            scale,
        };
        let assault = game.assault_status(map);

        let shade = if side {
            board.rect(0.0, 0.0, width, height)
        } else {
            [0, 0, frame.width as i32, frame.height as i32]
        };
        frame.shade(shade, [0.0; 3], 0.55);
        frame.solid(
            board.rect(INSET, HEAD - 5.0, width - 2.0 * INSET, 1.0),
            LINE,
        );
        let training = game.training_assault() || game.training_ctf() || game.training_deathmatch();
        let mode_name = if training {
            format!("{} TRAINING", mode.label().to_ascii_uppercase())
        } else {
            mode.label().to_ascii_uppercase()
        };
        board.label(frame, [INSET, 8.0], width - 148.0, 0.76, &mode_name, ACCENT);
        let title = result_title(game, team_scores, training).unwrap_or_else(|| {
            if map.title.is_empty() {
                "Standings".to_owned()
            } else {
                map.title.clone()
            }
        });
        board.label(frame, [INSET, 26.0], width - 148.0, 1.48, &title, TEXT);
        let (limit, seconds) = game.limits();
        let status_line = if let Some(state) = assault.as_ref() {
            let round = if state.tutorial {
                "Exercise".to_owned()
            } else {
                format!("Round {}/2", state.half)
            };
            let progress = format!(
                "{round}   |   {}/{} objectives",
                state.completed, state.total
            );
            if let Some(name) = state.objective.filter(|_| !game.finished) {
                format!("{progress}   |   {name}")
            } else if game.finished {
                format!("{}   |   {progress}", map.title)
            } else {
                progress
            }
        } else if game.finished {
            if game.overtime {
                format!("{}   |   Decided in sudden death", map.title)
            } else {
                map.author.as_ref().map_or_else(
                    || map.title.clone(),
                    |author| format!("{}   |   {author}", map.title),
                )
            }
        } else if game.overtime {
            "Sudden death".to_owned()
        } else if limit > 0 {
            format!("{}  {}", mode.goal_label(), limit)
        } else {
            "Unlimited score".to_owned()
        };
        board.label(
            frame,
            [INSET, 63.0],
            width - 148.0,
            0.82,
            &status_line,
            MUTED,
        );
        let (time, clock_label) =
            if let Some(time) = assault.as_ref().and_then(|state| state.attack_time) {
                (time, "Attack time")
            } else if game.finished {
                (game.time, "Elapsed")
            } else if assault.is_some() || seconds > 0 {
                (
                    f32::from(game.remaining()),
                    if assault.is_some() {
                        "Round left"
                    } else {
                        "Time left"
                    },
                )
            } else {
                (game.time, "Elapsed")
            };
        board.right(frame, [width - INSET, 16.0], 1.39, &clock(time), TEXT);
        board.right(frame, [width - INSET, 47.0], 0.78, clock_label, MUTED);

        if let Some((teams, scores)) = teams.zip(team_scores) {
            let mut y = HEAD;
            for (team, players) in teams.iter().enumerate() {
                let x = INSET
                    + if stacked {
                        0.0
                    } else {
                        team as f32 * (pane_width + GAP)
                    };
                let color = TEAM[team];
                frame.shade(board.rect(x, y, pane_width, TEAM_HEAD - 9.0), color, 0.13);
                frame.solid(board.rect(x, y, pane_width, 2.0), color);
                board.label(
                    frame,
                    [x + 11.0, y + 5.0],
                    pane_width - 130.0,
                    1.18,
                    game.team_label(team),
                    color,
                );

                if let Some(status) = assault.as_ref() {
                    let attacking = team + 1 == status.half;
                    let description = match (attacking, game.finished) {
                        (true, false) => "Attacking",
                        (false, false) => "Defending",
                        (true, true) => "Attacked",
                        (false, true) => "Defended",
                    };
                    let total = assault_total(status, attacking, game.finished);
                    board.label(
                        frame,
                        [x + 11.0, y + 27.0],
                        pane_width - 110.0,
                        0.80,
                        description,
                        MUTED,
                    );
                    board.right(frame, [x + pane_width - 11.0, y + 8.0], 1.05, &total, TEXT);
                } else {
                    let description = match game.flag_status().map(|flags| flags[team]) {
                        Some("You have it") => "Flag carried by you".to_owned(),
                        Some(flag) => format!("Flag {}", flag.to_ascii_lowercase()),
                        None if mode == Mode::Domination => "Control points".to_owned(),
                        None => "Team frags".to_owned(),
                    };
                    board.label(
                        frame,
                        [x + 11.0, y + 24.0],
                        pane_width - 110.0,
                        0.80,
                        &description,
                        MUTED,
                    );
                    let right = x + pane_width - 13.0;
                    if !map.hud.draw_score_number(
                        frame,
                        board.point(right, y + 6.0),
                        board.scale * 0.62,
                        scores[team],
                        TEXT,
                    ) {
                        board.right(
                            frame,
                            [right, y + 6.0],
                            1.48,
                            &scores[team].to_string(),
                            color,
                        );
                    }
                }
                board.right(
                    frame,
                    [x + pane_width - 12.0, y + 45.0],
                    0.71,
                    score_label(mode),
                    MUTED,
                );
                scoreboard_rows(frame, game, &board, [x, y + TEAM_HEAD, pane_width], players);
                if stacked {
                    y += TEAM_HEAD + table_height(players.len()) + GAP;
                }
            }
        } else {
            board.right(
                frame,
                [width - INSET - 12.0, 67.0],
                0.73,
                score_label(mode),
                MUTED,
            );
            scoreboard_rows(frame, game, &board, [INSET, HEAD, pane_width], &standings);
        }

        if point_strip {
            let slot = (width - INSET * 2.0) / map.control_points.len() as f32;
            for (i, (point, owner)) in map
                .control_points
                .iter()
                .zip(game.control_owners())
                .enumerate()
            {
                let x = INSET + i as f32 * slot;
                let tint = owner.map_or(MUTED, |team| TEAM[team]);
                frame.solid(board.rect(x, footer - 27.0, slot - 8.0, 2.0), tint);
                board.label(
                    frame,
                    [x, footer - 24.0],
                    slot - 8.0,
                    0.79,
                    &point.name,
                    tint,
                );
            }
        }
        frame.solid(board.rect(INSET, footer, width - 2.0 * INSET, 1.0), LINE);
        if game.finished {
            board.label(
                frame,
                [INSET, footer + 5.0],
                width - 2.0 * INSET,
                0.84,
                help,
                MUTED,
            );
        } else if !game.tutorial_forces_scoreboard() {
            board.label(
                frame,
                [INSET, footer + 5.0],
                width - 2.0 * INSET,
                0.84,
                "Release Tab to return to the match",
                MUTED,
            );
        }
    }
}

fn score_label(mode: Mode) -> &'static str {
    match mode {
        Mode::CaptureTheFlag | Mode::Domination => "PLAYER SCORE",
        Mode::LastManStanding => "LIVES",
        _ => "FRAGS",
    }
}

fn clock(seconds: f32) -> String {
    let seconds = seconds as u32;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn attack_result(time: Option<f32>) -> String {
    time.map_or_else(
        || "Time expired".to_owned(),
        |time| format!("Captured in {}", clock(time)),
    )
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
        format!("{} left", status.total.saturating_sub(status.completed))
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

fn scoreboard_rows(
    frame: &mut Frame,
    game: &Match,
    board: &Board<'_>,
    pane: [f32; 3],
    players: &[usize],
) {
    let [x, y, width] = pane;
    if players.is_empty() {
        board.label(
            frame,
            [x + 11.0, y + 8.0],
            width - 22.0,
            0.94,
            "No players",
            MUTED,
        );
    }

    for (index, &id) in players.iter().enumerate() {
        let fighter = &game.fighters[id];
        let top = y + index as f32 * ROW;
        let lives = game.lives_left(id);
        let eliminated = lives == Some(0);
        let color = if id == 0 {
            ACCENT
        } else if eliminated {
            MUTED
        } else {
            TEXT
        };
        if id == 0 {
            frame.shade(board.rect(x, top, width, ROW - 4.0), ACCENT, 0.11);
            frame.solid(board.rect(x, top + 4.0, 2.0, ROW - 12.0), ACCENT);
        }

        let name = if id == 0 {
            "You".to_owned()
        } else {
            game.fighter_label(id)
        };
        board.label(
            frame,
            [x + 11.0, top],
            width
                - if game.fighter_carries_flag(id) {
                    151.0
                } else {
                    116.0
                },
            1.12,
            &name,
            color,
        );
        let mut record = match game.mode() {
            Mode::CaptureTheFlag => format!(
                "{} caps  {} ret  {} kills  {} deaths",
                fighter.stats.captures, fighter.stats.returns, fighter.stats.kills, fighter.deaths
            ),
            Mode::Domination => format!(
                "{} taken  {} kills  {} deaths",
                fighter.stats.points_taken, fighter.stats.kills, fighter.deaths
            ),
            Mode::LastManStanding => format!("{} frags   {} deaths", fighter.score, fighter.deaths),
            _ => format!("{} deaths", fighter.deaths),
        };
        if eliminated {
            record.push_str("   /   Eliminated");
        } else if fighter.health == 0 && !game.finished {
            record.push_str("   /   Respawning");
        }
        board.label(
            frame,
            [x + 11.0, top + 21.0],
            width - 23.0,
            0.76,
            &record,
            MUTED,
        );

        let right = x + width - 12.0;
        let value = lives.map_or_else(|| fighter.score.to_string(), |lives| lives.to_string());
        board.right(frame, [right, top - 2.0], 1.25, &value, color);
        if game.fighter_carries_flag(id) {
            let flag_tint = TEAM[1 - fighter.team.unwrap_or(0)];
            board.right(frame, [right - 63.0, top + 4.0], 0.72, "FLAG", flag_tint);
        }
    }
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
}
