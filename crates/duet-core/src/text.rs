//! The one-note-per-line text format.
//!
//! ```text
//! # notefmt v1: positions are bar.beat.tick, lengths are quarter notes
//! title "Sketch in D"
//! tpq 480
//! tempo 92
//! time 4/4
//! key D minor
//! tempo 96 at 9.1.0
//! end 13.1.0
//!
//! [track "Piano" ch1 prog0]
//! 1.1.0     D4    1      80
//! 1.2.0     F4    0.5    72
//! 1.2.240   A4    0.5    75
//! 1.3.0     cc64  127
//! 2.1.0     bend  -512
//! 3.1.0     prog  5
//! ```
//!
//! Header lines come before the first track; one after a track is an error. A note line is position, pitch, length in
//! quarter notes (a decimal or a fraction like `1/3`) and velocity. Lines starting with
//! `#` are comments, and so is anything after a `#` that starts a word.

use std::fmt::Write as _;

use crate::error::{Error, Result};
use crate::model::{Control, KeyChange, Note, Song, TempoChange, TimeChange, Track};
use crate::pitch::Pitch;
use crate::time::{Key, Position, TimeSig, parse_beats};

/// The words that start a header line. A header line inside a track is an error rather
/// than a song-wide change, so where a tempo, meter or key change sits is never in doubt.
const HEADER_KEYWORDS: [&str; 6] = ["title", "tpq", "tempo", "time", "key", "end"];

const HEADER_COMMENT: &str = "# notefmt v1: positions are bar.beat.tick, lengths are quarter notes";

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

enum Line<'a> {
    Note(&'a Note),
    Control(&'a Control),
}

impl Song {
    /// Writes the song in the text format.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = self.write_text(&mut out);
        out
    }

    /// The song's header as notefmt header lines: title, tpq, tempo, time and key, then
    /// their changes, then the end when the song names one. Other formats that share the notefmt header (the drum grid) write it
    /// with this, so both agree on what each line means.
    pub fn header_text(&self) -> String {
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = self.write_header(&mut out);
        out
    }

    /// Applies one notefmt header line, such as `tempo 96 at 9.1.0`, to this song.
    ///
    /// Returns `Ok(false)` and changes nothing when the line's first word is not a header
    /// keyword (title, tpq, tempo, time, key or end), so a format that extends the header can
    /// read its own lines. Fails when the line is a header line but malformed.
    pub fn apply_header_line(&mut self, line: &str) -> Result<bool> {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("title") {
            let (title, _) = parse_quoted(rest.trim())?;
            self.title = Some(title);
            return Ok(true);
        }
        let words: Vec<&str> = line
            .split_whitespace()
            .take_while(|w| !w.starts_with('#'))
            .collect();
        let [kw, rest @ ..] = words.as_slice() else {
            return Ok(false);
        };
        if !HEADER_KEYWORDS.contains(kw) {
            return Ok(false);
        }
        let (value, at) = match rest.iter().position(|w| *w == "at") {
            Some(i) => {
                if i + 2 != rest.len() {
                    return Err(Error::invalid(format!(
                        "'{kw} ... at' needs one position after 'at'"
                    )));
                }
                (rest[..i].join(" "), Some(rest[i + 1].parse::<Position>()?))
            }
            None => (rest.join(" "), None),
        };
        match (*kw, at) {
            ("tpq", None) => {
                self.ticks_per_quarter = value
                    .parse()
                    .ok()
                    .filter(|t| (1..=0x7FFF).contains(t))
                    .ok_or_else(|| {
                        Error::invalid(format!("'{value}' is not ticks per quarter (1 to 32767)"))
                    })?;
            }
            ("tempo", at) => {
                let bpm: f64 = value
                    .parse()
                    .ok()
                    .filter(|b: &f64| b.is_finite() && *b > 0.0)
                    .ok_or_else(|| Error::invalid(format!("'{value}' is not a tempo in bpm")))?;
                match at {
                    None => self.tempo_bpm = bpm,
                    Some(at) => self.tempo_changes.push(TempoChange { at, bpm }),
                }
            }
            ("time", at) => {
                let time_sig: TimeSig = value.parse()?;
                match at {
                    None => self.time_sig = time_sig,
                    Some(at) => self.time_changes.push(TimeChange { at, time_sig }),
                }
            }
            ("end", None) => self.end = Some(value.parse()?),
            ("key", at) => {
                let key: Key = value.parse()?;
                match at {
                    None => self.key = Some(key),
                    Some(at) => self.key_changes.push(KeyChange { at, key }),
                }
            }
            (kw, _) => {
                return Err(Error::invalid(format!(
                    "'{kw} ... at' is not a header line"
                )));
            }
        }
        Ok(true)
    }

    fn write_text(&self, out: &mut String) -> std::fmt::Result {
        writeln!(out, "{HEADER_COMMENT}")?;
        self.write_header(out)?;
        self.write_tracks(out)
    }

    fn write_header(&self, out: &mut String) -> std::fmt::Result {
        if let Some(title) = &self.title {
            writeln!(out, "title {}", quote(title))?;
        }
        writeln!(out, "tpq {}", self.ticks_per_quarter)?;
        writeln!(out, "tempo {}", self.tempo_bpm)?;
        writeln!(out, "time {}", self.time_sig)?;
        if let Some(key) = self.key {
            writeln!(out, "key {key}")?;
        }
        for c in &self.time_changes {
            writeln!(out, "time {} at {}", c.time_sig, c.at)?;
        }
        for c in &self.key_changes {
            writeln!(out, "key {} at {}", c.key, c.at)?;
        }
        for c in &self.tempo_changes {
            writeln!(out, "tempo {} at {}", c.bpm, c.at)?;
        }
        if let Some(end) = self.end {
            writeln!(out, "end {end}")?;
        }
        Ok(())
    }

    fn write_tracks(&self, out: &mut String) -> std::fmt::Result {
        for track in &self.tracks {
            writeln!(out)?;
            write!(out, "[track {} ch{}", quote(&track.name), track.channel)?;
            if let Some(program) = track.program {
                write!(out, " prog{program}")?;
            }
            writeln!(out, "]")?;

            // Merge notes and controls by position, controls first at the same position.
            let mut lines: Vec<(Position, u8, usize, Line)> = Vec::new();
            for (i, c) in track.controls.iter().enumerate() {
                lines.push((c.at(), 0, i, Line::Control(c)));
            }
            for (i, n) in track.notes.iter().enumerate() {
                lines.push((n.at, 1, i, Line::Note(n)));
            }
            lines.sort_by_key(|(at, class, i, _)| (*at, *class, *i));

            for (at, _, _, line) in lines {
                let at = at.to_string();
                match line {
                    Line::Note(n) => {
                        let pitch = n.pitch.to_string();
                        let beats = n.beats.to_string();
                        writeln!(out, "{at:<9} {pitch:<5} {beats:<6} {}", n.vel)?;
                    }
                    Line::Control(Control::Cc {
                        controller, value, ..
                    }) => {
                        let cc = format!("cc{controller}");
                        writeln!(out, "{at:<9} {cc:<5} {value}")?;
                    }
                    Line::Control(Control::Bend { value, .. }) => {
                        writeln!(out, "{at:<9} {:<5} {value}", "bend")?;
                    }
                    Line::Control(Control::Program { program, .. }) => {
                        writeln!(out, "{at:<9} {:<5} {program}", "prog")?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Parses the text format, then [normalizes](Song::normalize) the song, so lengths
    /// come back rounded to whole ticks and positions in canonical form.
    pub fn from_text(text: &str) -> Result<Song> {
        let mut song = Song::default();
        let mut track: Option<Track> = None;

        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let err = |msg: String| Error::parse(line_no, msg);
            let ctx = |e: Error| match e {
                Error::Invalid(msg) => Error::parse(line_no, msg),
                other => other,
            };
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(inner) = line.strip_prefix('[') {
                if let Some(done) = track.take() {
                    song.tracks.push(done);
                }
                track = Some(parse_track_header(inner).map_err(ctx)?);
                continue;
            }

            let words: Vec<&str> = line
                .split_whitespace()
                .take_while(|w| !w.starts_with('#'))
                .collect();

            match track.as_mut() {
                None => {
                    if !song.apply_header_line(line).map_err(ctx)? {
                        return Err(err(format!(
                            "'{}' is not a header line; expected title, tpq, tempo, time, key, end or a [track]",
                            words[0]
                        )));
                    }
                }
                Some(_) if words.first().is_some_and(|w| HEADER_KEYWORDS.contains(w)) => {
                    return Err(err(format!(
                        "'{}' is a header line, and header lines go before the first [track]",
                        words[0]
                    )));
                }
                Some(track) => match words.as_slice() {
                    [at, what, rest @ ..] => {
                        let at: Position = at.parse().map_err(ctx)?;
                        let one = |name: &str| -> Result<&str> {
                            match rest {
                                [v] => Ok(*v),
                                _ => Err(err(format!("'{name}' takes one value"))),
                            }
                        };
                        if let Some(num) = what.strip_prefix("cc") {
                            let controller = parse_u7(num, "controller").map_err(ctx)?;
                            let value = parse_u7(one(what)?, "controller value").map_err(ctx)?;
                            track.controls.push(Control::Cc {
                                at,
                                controller,
                                value,
                            });
                        } else if *what == "bend" {
                            let value: i16 = one("bend")?
                                .trim_start_matches('+')
                                .parse()
                                .ok()
                                .filter(|v| (-8192..=8191).contains(v))
                                .ok_or_else(|| err("pitch bend is -8192 to 8191".into()))?;
                            track.controls.push(Control::Bend { at, value });
                        } else if *what == "prog" {
                            let program = parse_u7(one("prog")?, "program").map_err(ctx)?;
                            track.controls.push(Control::Program { at, program });
                        } else {
                            let pitch: Pitch = what.parse().map_err(ctx)?;
                            let [beats, vel] = rest else {
                                return Err(err(
                                    "a note is position, pitch, length and velocity".into()
                                ));
                            };
                            let beats = parse_beats(beats).map_err(ctx)?;
                            let vel = parse_u7(vel, "velocity").map_err(ctx)?;
                            if vel == 0 {
                                return Err(err("velocity must be 1 to 127".into()));
                            }
                            track.notes.push(Note {
                                at,
                                pitch,
                                beats,
                                vel,
                            });
                        }
                    }
                    _ => {
                        return Err(err(
                            "expected a note like '1.1.0 D4 1 80', or a cc, bend or prog line"
                                .into(),
                        ));
                    }
                },
            }
        }

        if let Some(done) = track.take() {
            song.tracks.push(done);
        }
        song.normalize()?;
        Ok(song)
    }
}

fn parse_u7(s: &str, what: &str) -> Result<u8> {
    s.parse::<u8>()
        .ok()
        .filter(|v| *v <= 127)
        .ok_or_else(|| Error::invalid(format!("'{s}' is not a {what} from 0 to 127")))
}

/// Parses a quoted string at the start of `s`, returning it and what follows.
fn parse_quoted(s: &str) -> Result<(String, &str)> {
    let rest = s
        .strip_prefix('"')
        .ok_or_else(|| Error::invalid("expected a quoted name"))?;
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Ok((out, &rest[i + 1..])),
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, other)) => out.push(other),
                None => break,
            },
            _ => out.push(c),
        }
    }
    Err(Error::invalid("a quoted name is missing its closing quote"))
}

/// Parses a track line, `[track "Name" ch1 prog0]`, from just after its `[`. The name is
/// read first, so a name may hold `]`; anything after the closing `]` is ignored.
fn parse_track_header(line: &str) -> Result<Track> {
    let rest = line
        .trim_start()
        .strip_prefix("track")
        .ok_or_else(|| Error::invalid("a track line looks like [track \"Piano\" ch1 prog0]"))?
        .trim_start();
    let (name, rest) = parse_quoted(rest)?;
    let (rest, _) = rest
        .split_once(']')
        .ok_or_else(|| Error::invalid("a track line needs a closing ]"))?;
    let mut track = Track::new(name, 1);
    for word in rest.split_whitespace() {
        if let Some(ch) = word.strip_prefix("ch") {
            track.channel = ch
                .parse()
                .ok()
                .filter(|c| (1..=16).contains(c))
                .ok_or_else(|| Error::invalid(format!("'{word}': channels are ch1 to ch16")))?;
        } else if let Some(p) = word.strip_prefix("prog") {
            track.program = Some(parse_u7(p, "program")?);
        } else {
            return Err(Error::invalid(format!(
                "'{word}' is not ch<n> or prog<n> in a track line"
            )));
        }
    }
    Ok(track)
}
