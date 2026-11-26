use super::{GlyphAtlas, Hud, glyph_index};
use crate::{
    game::deathmatch::{AssaultStatus, Fighter, Match, Mode},
    map::Map,
    render::Frame,
};

const TEXT: [f32; 3] = [0.96, 0.95, 0.91];
const MUTED: [f32; 3] = [0.76, 0.81, 0.84];
const ACCENT: [f32; 3] = [1.0, 0.82, 0.45];
const TEAM: [[f32; 3]; 2] = [[1.0, 0.55, 0.49], [0.49, 0.79, 1.0]];
const LINE: [f32; 3] = [0.34, 0.42, 0.47];
const HEAD: f32 = 90.0;
const TEAM_HEAD: f32 = 47.0;
const COLUMNS: f32 = 26.0;
const ROW: f32 = 28.0;
const INSET: f32 = 16.0;
const GAP: f32 = 14.0;

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
        let stacked = teams.is_some() && (aspect < 1.25 || frame.width < 850);
        let side = teams.is_none() && game.finished && aspect >= 1.5 && frame.width >= 760;
        let pane_width = if teams.is_some() { 490.0 } else { 590.0 };
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
        let table_height = |count: usize| COLUMNS + ROW * count.max(1) as f32;
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
                frame.solid(board.rect(x, y + TEAM_HEAD - 4.0, pane_width, 2.0), color);
                board.label(
                    frame,
                    [x + 11.0, y + 2.0],
                    pane_width - 110.0,
                    1.10,
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
                        [x + 11.0, y + 24.0],
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
                    board.right(
                        frame,
                        [x + pane_width - 11.0, y + 3.0],
                        1.46,
                        &scores[team].to_string(),
                        TEXT,
                    );
                }
                scoreboard_rows(frame, game, &board, [x, y + TEAM_HEAD, pane_width], players);
                if stacked {
                    y += TEAM_HEAD + table_height(players.len()) + GAP;
                }
            }
        } else {
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

struct Column {
    label: &'static str,
    value: fn(&Match, usize, &Fighter) -> i32,
}

const SCORE: Column = Column {
    label: "Score",
    value: |_, _, f| i32::from(f.score),
};
const FRAGS: Column = Column {
    label: "Frags",
    value: |_, _, f| i32::from(f.score),
};
const KILLS: Column = Column {
    label: "Kills",
    value: |_, _, f| i32::from(f.stats.kills),
};
const DEATHS: Column = Column {
    label: "Deaths",
    value: |_, _, f| i32::from(f.deaths),
};
const LIVES: Column = Column {
    label: "Lives",
    value: |game, id, _| i32::from(game.lives_left(id).unwrap_or(0)),
};
const CAPS: Column = Column {
    label: "Caps",
    value: |_, _, f| i32::from(f.stats.captures),
};
const RETURNS: Column = Column {
    label: "Returns",
    value: |_, _, f| i32::from(f.stats.returns),
};
const TAKEN: Column = Column {
    label: "Taken",
    value: |_, _, f| i32::from(f.stats.points_taken),
};

fn scoreboard_rows(
    frame: &mut Frame,
    game: &Match,
    board: &Board<'_>,
    pane: [f32; 3],
    players: &[usize],
) {
    let [x, y, width] = pane;
    let columns = match game.mode() {
        Mode::CaptureTheFlag => &[SCORE, CAPS, RETURNS, KILLS, DEATHS][..],
        Mode::Domination => &[SCORE, TAKEN, KILLS, DEATHS],
        Mode::LastManStanding => &[LIVES, FRAGS, DEATHS],
        _ => &[FRAGS, DEATHS],
    };
    let cell = if columns.len() >= 4 { 66.0 } else { 68.0 };
    let column_x = |index: usize| x + width - 11.0 - (columns.len() - 1 - index) as f32 * cell;
    let name_width = column_x(0) - cell - (x + 38.0);
    frame.solid(board.rect(x, y + COLUMNS - 1.0, width, 1.0), LINE);
    board.label(frame, [x + 10.0, y + 3.0], 25.0, 0.74, "#", MUTED);
    board.label(
        frame,
        [x + 38.0, y + 3.0],
        name_width,
        0.74,
        "PLAYER",
        MUTED,
    );
    for (i, column) in columns.iter().enumerate() {
        board.right(frame, [column_x(i), y + 3.0], 0.77, column.label, MUTED);
    }
    if players.is_empty() {
        board.label(
            frame,
            [x + 38.0, y + COLUMNS + 5.0],
            name_width,
            0.85,
            "No players",
            MUTED,
        );
    }
    for (rank, &id) in players.iter().enumerate() {
        let fighter = &game.fighters[id];
        let top = y + COLUMNS + rank as f32 * ROW;
        let color = if id == 0 { ACCENT } else { TEXT };
        if id == 0 {
            frame.shade(board.rect(x, top, width, ROW), ACCENT, 0.14);
            frame.solid(board.rect(x, top, 2.0, ROW), ACCENT);
        }
        let status = if game.fighter_carries_flag(id) {
            Some(("FLAG", TEAM[1 - fighter.team.unwrap_or(0)]))
        } else if game.lives_left(id) == Some(0) {
            Some(("OUT", MUTED))
        } else if !game.finished && fighter.health == 0 {
            Some(("DOWN", MUTED))
        } else {
            None
        };
        board.label(
            frame,
            [x + 10.0, top + 4.0],
            25.0,
            0.82,
            &(rank + 1).to_string(),
            MUTED,
        );
        let name = if id == 0 {
            "You".to_owned()
        } else {
            game.fighter_label(id)
        };
        let name_room =
            name_width - status.map_or(0.0, |(text, _)| board.measure(text, 0.68) + 8.0);
        board.label(frame, [x + 38.0, top + 3.0], name_room, 0.95, &name, color);
        if let Some((text, tint)) = status {
            let left = x + 38.0 + board.measure(&name, 0.95).min(name_room) + 7.0;
            board.label(
                frame,
                [left, top + 6.0],
                name_width - name_room,
                0.68,
                text,
                tint,
            );
        }
        for (i, column) in columns.iter().enumerate() {
            let value = (column.value)(game, id, fighter);
            board.right(
                frame,
                [column_x(i), top + 3.0],
                0.94,
                &value.to_string(),
                if i == 0 { color } else { MUTED },
            );
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
