use super::{Hud, glyph_index};
use crate::{
    game::deathmatch::{Match, Mode},
    map::Map,
    render::Frame,
};

const TEXT: [f32; 3] = [0.96, 0.97, 0.98];
const MUTED: [f32; 3] = [0.70, 0.75, 0.81];
const GOLD: [f32; 3] = [1.0, 0.72, 0.33];
const TEAM_COLORS: [[f32; 3]; 2] = [[1.0, 0.52, 0.45], [0.48, 0.76, 1.0]];
const PANEL: [f32; 3] = [0.055, 0.071, 0.094];

impl Hud {
    pub(super) fn draw_scoreboard(&self, frame: &mut Frame, game: &Match, map: &Map, help: &str) {
        let standings = game.standings();
        let team_scores = game.team_scores();
        let teams = team_scores.map(|_| {
            [0, 1].map(|team| {
                standings
                    .iter()
                    .copied()
                    .filter(|&id| game.fighters[id].team == Some(team))
                    .collect::<Vec<_>>()
            })
        });
        let rows = teams
            .as_ref()
            .map_or(standings.len(), |[a, b]| a.len().max(b.len()).max(1));
        let table_top = 108.0;
        let rows_top = table_top + if teams.is_some() { 79.0 } else { 32.0 };
        let footer = rows_top + 32.0 * rows as f32 + 12.0;
        let leave_camera_visible = game.finished
            && teams.is_none()
            && frame.width >= 760
            && frame.width as f32 / frame.height.max(1) as f32 >= 1.5;
        let width = if teams.is_some() {
            860.0
        } else if leave_camera_visible {
            440.0
        } else {
            580.0
        };
        let height = footer + 34.0;
        let available_width = if leave_camera_visible {
            frame.width as f32 * 0.46
        } else {
            frame.width as f32 - 24.0
        };
        let scale = (available_width / width)
            .min((frame.height as f32 - 28.0) / height)
            .min(1.4)
            .max(0.1);
        let board = Board {
            hud: self,
            scale,
            origin: [
                if leave_camera_visible {
                    14.0
                } else {
                    (frame.width as f32 - width * scale) * 0.5
                },
                (frame.height as f32 - height * scale) * 0.5,
            ],
        };
        let assault = game.assault_status(map);
        let mode = game.mode();
        let (limit, seconds) = game.limits();
        frame.shade(board.rect(0.0, 0.0, width, height), PANEL, 0.94);
        frame.solid(board.rect(0.0, 0.0, width, 3.0), GOLD);
        frame.solid(
            board.rect(18.0, 98.0, width - 36.0, 1.0),
            [0.28, 0.34, 0.41],
        );

        let mode_label =
            if game.training_assault() || game.training_ctf() || game.training_deathmatch() {
                format!("{} training", mode.label())
            } else {
                mode.label().to_owned()
            };
        if let Some(font) = &map.score_font
            && font.width(&mode_label, 0.88) <= width - 160.0
        {
            let [x, y] = board.point(19.0, 12.0);
            font.draw_glyphs(frame, x, y, 0.88 * scale, &mode_label, Some(GOLD));
        } else {
            board.label(frame, [19.0, 10.0], width - 160.0, 0.82, &mode_label, GOLD);
        }
        let title = if game.finished {
            if game.training_assault() || game.training_ctf() || game.training_deathmatch() {
                if game.player_won_rated_match() {
                    "Training complete"
                } else {
                    "Training failed"
                }
                .to_owned()
            } else if let Some([red, blue]) = team_scores {
                match red.cmp(&blue) {
                    std::cmp::Ordering::Equal => "Match drawn".to_owned(),
                    outcome => format!("{} wins", game.team_label(usize::from(outcome.is_lt()))),
                }
            } else {
                match game.winner() {
                    Some(0) => "You won".to_owned(),
                    Some(winner) => format!("{} wins", game.fighter_label(winner)),
                    None => "Match drawn".to_owned(),
                }
            }
        } else if map.title.is_empty() {
            "Standings".to_owned()
        } else {
            map.title.clone()
        };
        board.label(frame, [19.0, 32.0], width - 160.0, 1.62, &title, TEXT);

        let (clock, clock_label) = if game.finished {
            (
                game.time as u32,
                if assault.is_some() {
                    "Round elapsed"
                } else {
                    "Elapsed"
                },
            )
        } else if assault.is_some() || seconds > 0 {
            (
                u32::from(game.remaining()),
                if assault.is_some() {
                    "Round time"
                } else {
                    "Time left"
                },
            )
        } else {
            (game.time as u32, "Elapsed")
        };
        board.right(
            frame,
            [width - 20.0, 25.0],
            1.42,
            &format!("{:02}:{:02}", clock / 60, clock % 60),
            TEXT,
        );
        board.right(frame, [width - 20.0, 56.0], 0.77, clock_label, MUTED);
        let context = if let Some(status) = &assault {
            let round = if status.tutorial {
                "Exercise".to_owned()
            } else {
                format!("Round {} of 2", status.half)
            };
            let progress = format!(
                "{round}  /  {} of {} objectives",
                status.completed, status.total
            );
            match status.objective.filter(|_| !game.finished) {
                Some(objective) => format!("{progress}  /  {objective}"),
                None => progress,
            }
        } else if game.overtime {
            if game.finished {
                "Decided in sudden death"
            } else {
                "Sudden death"
            }
            .to_owned()
        } else if game.finished {
            if seconds > 0 && game.remaining() == 0 {
                "Time expired".to_owned()
            } else if mode == Mode::LastManStanding {
                "Elimination".to_owned()
            } else {
                map.author.as_ref().map_or_else(
                    || map.title.clone(),
                    |author| format!("{}  /  {author}", map.title),
                )
            }
        } else if limit > 0 {
            format!("{}  /  {limit}", mode.goal_label())
        } else {
            "No score limit".to_owned()
        };
        board.label(frame, [19.0, 75.0], width - 38.0, 0.85, &context, MUTED);

        if let Some((teams, scores)) = teams.zip(team_scores) {
            let flags = game.flag_status();
            let controlled = (mode == Mode::Domination).then(|| {
                let mut count = [0; 2];
                for owner in game.control_owners().flatten() {
                    count[owner] += 1;
                }
                count
            });
            for (team, players) in teams.iter().enumerate() {
                let x = 18.0 + team as f32 * 418.0;
                let pane_width = 406.0;
                let tint = TEAM_COLORS[team];
                frame.shade(board.rect(x, table_top, pane_width, 47.0), tint, 0.16);
                frame.solid(board.rect(x, table_top, 3.0, 47.0), tint);
                board.label(
                    frame,
                    [x + 13.0, table_top + 2.0],
                    pane_width - 83.0,
                    1.12,
                    game.team_label(team),
                    tint,
                );
                let status = if let Some(status) = &assault {
                    let attack_team = status.half - 1;
                    match (team == attack_team, game.finished) {
                        (true, false) => "Attacking",
                        (false, false) => "Defending",
                        (true, true) => "Attacked",
                        (false, true) => "Defended",
                    }
                    .to_owned()
                } else if let Some(flags) = flags {
                    format!("Flag  /  {}", flags[team])
                } else if let Some(held) = controlled {
                    format!("Control points  /  {}", held[team])
                } else {
                    format!("{} players", players.len())
                };
                board.label(
                    frame,
                    [x + 13.0, table_top + 24.0],
                    pane_width - 82.0,
                    0.82,
                    &status,
                    TEXT,
                );
                if mode == Mode::Assault {
                    if game.finished {
                        let outcome = match scores[team].cmp(&scores[1 - team]) {
                            std::cmp::Ordering::Greater => "Win",
                            std::cmp::Ordering::Less => "Loss",
                            std::cmp::Ordering::Equal => "Draw",
                        };
                        board.right(
                            frame,
                            [x + pane_width - 13.0, table_top + 8.0],
                            1.16,
                            outcome,
                            TEXT,
                        );
                    }
                } else {
                    board.right(
                        frame,
                        [x + pane_width - 13.0, table_top + 1.0],
                        1.32,
                        &scores[team].to_string(),
                        TEXT,
                    );
                    let team_total = match mode {
                        Mode::CaptureTheFlag => "Captures",
                        Mode::Domination => "Points",
                        _ => "Frags",
                    };
                    board.right(
                        frame,
                        [x + pane_width - 13.0, table_top + 27.0],
                        0.72,
                        team_total,
                        MUTED,
                    );
                }
                self.scoreboard_rows(frame, game, &board, [x, rows_top, pane_width], players);
            }
        } else {
            self.scoreboard_rows(
                frame,
                game,
                &board,
                [18.0, rows_top, width - 36.0],
                &standings,
            );
        }
        frame.solid(
            board.rect(18.0, footer, width - 36.0, 1.0),
            [0.28, 0.34, 0.41],
        );
        let footer_label = if game.finished {
            help
        } else if game.tutorial_forces_scoreboard() {
            ""
        } else {
            "Release Tab to close"
        };
        board.label(
            frame,
            [19.0, footer + 7.0],
            width - 38.0,
            0.84,
            footer_label,
            MUTED,
        );
    }

    fn scoreboard_rows(
        &self,
        frame: &mut Frame,
        game: &Match,
        board: &Board<'_>,
        pane: [f32; 3],
        players: &[usize],
    ) {
        let [x, y, width] = pane;
        let lms = game.mode() == Mode::LastManStanding;
        let score = if matches!(game.mode(), Mode::CaptureTheFlag | Mode::Domination) {
            "Score"
        } else {
            "Frags"
        };
        let third = if lms { "Lives" } else { "Deaths" };
        let numeric = [x + width - 80.0, x + width - 13.0];
        let header = y - 32.0;
        frame.shade(board.rect(x, header, width, 32.0), [0.30, 0.36, 0.43], 0.48);
        board.label(
            frame,
            [x + 13.0, header + 6.0],
            width - 145.0,
            0.84,
            "Player",
            MUTED,
        );
        board.right(frame, [numeric[0], header + 6.0], 0.84, score, MUTED);
        board.right(frame, [numeric[1], header + 6.0], 0.84, third, MUTED);

        if players.is_empty() {
            board.label(
                frame,
                [x + 13.0, y + 5.0],
                width - 25.0,
                0.89,
                "No players",
                MUTED,
            );
        }
        for (row, &id) in players.iter().enumerate() {
            let top = y + row as f32 * 32.0;
            let fighter = &game.fighters[id];
            let flag = game.fighter_carries_flag(id);
            let tint = if id == 0 { GOLD } else { TEXT };
            if id == 0 {
                frame.shade(board.rect(x, top, width, 31.0), GOLD, 0.18);
                frame.solid(board.rect(x, top, 3.0, 31.0), GOLD);
            } else if row % 2 == 0 {
                frame.shade(board.rect(x, top, width, 31.0), [0.22, 0.30, 0.40], 0.36);
            }
            board.label(
                frame,
                [x + 13.0, top + 4.0],
                width - if flag { 192.0 } else { 145.0 },
                1.0,
                &game.fighter_label(id),
                tint,
            );
            if flag {
                let flag_color = TEAM_COLORS[1 - fighter.team.unwrap_or(0)];
                board.right(
                    frame,
                    [x + width - 144.0, top + 6.0],
                    0.78,
                    "FLAG",
                    flag_color,
                );
            }
            board.right(
                frame,
                [numeric[0], top + 4.0],
                1.0,
                &fighter.score.to_string(),
                tint,
            );
            let deaths_or_lives = game.lives_left(id).unwrap_or(fighter.deaths);
            let color = if lms && deaths_or_lives == 0 {
                MUTED
            } else {
                tint
            };
            board.right(
                frame,
                [numeric[1], top + 4.0],
                1.0,
                &deaths_or_lives.to_string(),
                color,
            );
        }
    }
}

struct Board<'a> {
    hud: &'a Hud,
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
        self.hud.glyphs.as_ref().map_or(0.0, |atlas| {
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
        self.hud.text(
            frame,
            x.round() as i32,
            y.round() as i32,
            size * self.scale,
            text,
            tint,
        );
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
