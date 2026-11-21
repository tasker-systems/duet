//! The rules for placing a take on a song's bars, with no device and no project in them: an
//! estimate from capture's analysis, what a capture may replace, what an estimate says against
//! the song, and finding the analysis and the songs that go with a take (the placing-takes
//! design, "Capture of a linked take").
//!
//! - **The estimate is the measured tempo,** `60 × rate / beat_period_frames`, never the rounded
//!   `tempo_bpm`: the placement keeps what the analysis placed notes by.
//! - **Capture replaces an unplaced or `estimated` placement and keeps `click` and `set`:** a
//!   placement the owner or the click gave is never overwritten by a guess.
//! - **Disagreements are stated, never acted on:** tempos more than [`TOLERANCE_BPM`] apart, a
//!   tempo near half or double the song's, a meter, a key, and the tempo a `set` placement gave.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use duet_capture::{Analysis, Origin, TakeDigest, Timing};
use duet_core::Position;
use duet_schemas::project::Name;
use duet_schemas::song::{By, Composition, Placed};
use duet_schemas::{Document, Song};

use crate::capture::statement::number;

/// How far apart two tempos may be, in quarter notes per minute, and not disagree.
pub const TOLERANCE_BPM: f64 = 0.5;

/// The take's tempo as the analysis measured it, in quarter notes per minute:
/// `60 × rate / beat_period_frames`. `rate` is the rate the analysis's frames count at.
#[must_use]
pub fn measured_bpm(timing: &Timing, rate: u32) -> f64 {
    60.0 * f64::from(rate) / timing.beat_period_frames
}

/// The placement an analysis gives a take: its own start is bar 1, its downbeat falls at
/// `downbeat_frame / rate` seconds in, and its tempo is the measured one.
#[must_use]
pub fn estimate(analysis: &Analysis) -> Placed {
    let rate = f64::from(analysis.rate);
    Placed::estimated(
        Position::bar(1),
        *analysis.timing.downbeat_frame.value() as f64 / rate,
        measured_bpm(&analysis.timing, analysis.rate),
    )
}

/// What a capture does with a link's placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Replace {
    /// Write this placement.
    Place(Placed),
    /// Leave the placement, which this kind of placing made.
    Keep(By),
}

/// What a capture's `estimate` does to a link's `current` placement: it places an unplaced link
/// and replaces an `estimated` one, and keeps a `click` or `set` one.
#[must_use]
pub fn by_capture(current: Option<&Placed>, estimate: Placed) -> Replace {
    match current {
        Some(placed) if placed.by != By::Estimated => Replace::Keep(placed.by),
        _ => Replace::Place(estimate),
    }
}

/// A placement in a line: its kind, then its tempo when it has one, its place and its seconds.
pub(crate) fn describe(placed: &Placed) -> String {
    let mut text = format!(
        "{}: bar {} at {:.3} s",
        placed.by, placed.bar, placed.at_secs
    );
    if let Some(bpm) = placed.bpm {
        text.push_str(&format!(", {} bpm", number(bpm)));
    }
    if let Some(ppm) = placed.clock_ppm {
        text.push_str(&format!(", clock {ppm:+} ppm"));
    }
    text
}

/// States a new placement and, when the link had one, what moved:
/// `placed the take wayfaring on wayfaring-stranger's bars, estimated: bar 1.1.0 at 1.057 s,
/// 88.6 bpm (was 88.2 bpm, bar 1.1.0 at 1.061 s)`. The earlier placement's kind is named only
/// when it is not the new one's.
#[must_use]
pub fn placed_said(song: &Name, take: &Name, before: Option<&Placed>, after: &Placed) -> String {
    let mut line = format!(
        "placed the take {take} on {song}'s bars, {}",
        describe(after)
    );
    if let Some(before) = before {
        let kind = if before.by == after.by {
            String::new()
        } else {
            format!("{}, ", before.by)
        };
        let tempo = before
            .bpm
            .map_or_else(String::new, |bpm| format!("{} bpm, ", number(bpm)));
        line.push_str(&format!(
            " (was {kind}{tempo}bar {} at {:.3} s)",
            before.bar, before.at_secs
        ));
    }
    line
}

/// `given` or `estimated`, from the value's own origin.
const fn origin_word<T>(origin: &Origin<T>) -> &'static str {
    match origin {
        Origin::Given(_) => "given",
        Origin::Estimated { .. } => "estimated",
    }
}

/// Where the analysis and the song disagree, one line each: the tempo (or that it is half or
/// double the song's), the meter, the key, and, for a `set` placement, the tempo it was set at.
/// Each names whether the analysis's value was `estimated` or `given`. Nothing is said for a
/// tempo within [`TOLERANCE_BPM`], or for a value the analysis lacks (a rhythm has no key), or
/// for a place the song's own maps do not answer for.
#[must_use]
pub fn disagreements(
    song: &Name,
    composition: &Composition,
    placed: &Placed,
    analysis: &Analysis,
) -> Vec<String> {
    let mut lines = Vec::new();
    let at = placed.bar;
    let bar = at.bar;
    let timing = &analysis.timing;
    let bpm = measured_bpm(timing, analysis.rate);
    let said = number(bpm);
    let word = origin_word(&timing.tempo_bpm);

    if let Ok(held) = composition.tempo_at(at) {
        let held_said = number(held);
        if (bpm - held).abs() > TOLERANCE_BPM {
            let kin = if (bpm - held / 2.0).abs() <= TOLERANCE_BPM {
                Some("half")
            } else if (bpm - held * 2.0).abs() <= TOLERANCE_BPM {
                Some("double")
            } else {
                None
            };
            lines.push(match kin {
                Some(kin) => {
                    format!("tempo {word} {said} bpm, {kin} {song}'s {held_said} at bar {bar}")
                }
                None => format!("tempo {word} {said} bpm; {song} is {held_said} at bar {bar}"),
            });
        }
    }
    if let (By::Set, Some(set)) = (placed.by, placed.bpm)
        && (bpm - set).abs() > TOLERANCE_BPM
    {
        lines.push(format!(
            "tempo {word} {said} bpm; the placement is set at {} bpm",
            number(set)
        ));
    }
    if let Ok(held) = composition.meter_at(at) {
        let meter = *timing.meter.value();
        if meter != held {
            lines.push(format!(
                "meter {} {meter}; {song} is {held} at bar {bar}",
                origin_word(&timing.meter)
            ));
        }
    }
    if let (Some(estimate), Some(held)) = (&analysis.key, composition.key_at(at)) {
        let key = *estimate.key.value();
        if key != held {
            lines.push(format!(
                "key {} {key}; the song is {held} at bar {bar}",
                origin_word(&estimate.key)
            ));
        }
    }
    lines
}

/// Of the `*.capture.duet` files in `take_dir` whose analysis is of the take with this digest,
/// the one modified most recently, with its path. A file that does not read, or that states no
/// take, is skipped. A `take_dir` that does not exist holds none.
///
/// # Errors
///
/// When `take_dir` exists and cannot be listed.
pub fn latest_analysis(
    take_dir: &Path,
    digest: &TakeDigest,
) -> Result<Option<(PathBuf, Analysis)>> {
    let entries = match fs::read_dir(take_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| take_dir.display().to_string()),
    };
    let mut latest: Option<(SystemTime, PathBuf, Analysis)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_analysis = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".capture.duet"));
        if !is_analysis {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = Document::<Analysis>::read(&text) else {
            continue;
        };
        if doc.body.take.as_ref().map(|t| &t.digest) != Some(digest) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let newer = latest
            .as_ref()
            .is_none_or(|(at, held, _)| (modified, &path) > (*at, held));
        if newer {
            latest = Some((modified, path, doc.body));
        }
    }
    Ok(latest.map(|(_, path, analysis)| (path, analysis)))
}

/// The songs under `root`'s `songs/` that link the take with this digest, each by its name and
/// its folder (`songs/<song>`), sorted by name. A folder with no `song.duet`, or whose name is
/// no song's, is no song and is passed over.
///
/// # Errors
///
/// When `songs/` cannot be listed, or a song does not read; the message names the song.
pub fn songs_linking(root: &Path, digest: &TakeDigest) -> Result<Vec<(Name, PathBuf)>> {
    let songs = root.join("songs");
    let entries = match fs::read_dir(&songs) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| songs.display().to_string()),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| songs.display().to_string())?;
        let folder = entry.path();
        let file = folder.join("song.duet");
        if !folder.is_dir() || !file.is_file() {
            continue;
        }
        let Ok(name) = entry.file_name().to_string_lossy().parse::<Name>() else {
            continue;
        };
        let text = fs::read_to_string(&file)
            .with_context(|| format!("the song {name} ({})", file.display()))?;
        let doc = Document::<Song>::read(&text)
            .with_context(|| format!("the song {name} ({}) does not read", file.display()))?;
        if doc.body.linking(digest).is_some() {
            out.push((name, folder));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fresh_id;
    use crate::io::tool;
    use duet_schemas::song::TakeLink;
    use std::time::Duration;

    const RATE: u32 = 11_025;
    const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    fn name(text: &str) -> Name {
        text.parse().expect("a name")
    }

    fn digest(text: &str) -> TakeDigest {
        text.parse().expect("a digest")
    }

    /// An analysis, built as a document would write it. `tempo` is the measured tempo; the
    /// stated one is `stated`. A `key` of none is a rhythm.
    fn analysis(
        tempo: f64,
        stated: serde_json::Value,
        meter: serde_json::Value,
        key: Option<&str>,
        take: &str,
    ) -> Analysis {
        let mut doc = serde_json::json!({
            "rate": RATE,
            "take": { "digest": take, "rate": 44_100, "channels": 1, "frames": 441_000 },
            "events": [],
            "timing": {
                "tempo_bpm": stated,
                "beat_period_frames": 60.0 * f64::from(RATE) / tempo,
                "meter": meter,
                "downbeat_frame": { "estimated": { "value": 11_653, "confidence": 0.8 } },
            },
            "beats": [],
            "offsets": [],
        });
        if let Some(key) = key {
            doc["key"] = serde_json::json!({
                "key": { "estimated": { "value": key, "confidence": 0.7 } },
                "alternative": "C major",
                "histogram": vec![0.0_f64; 12],
                "scores": [],
            });
        }
        serde_json::from_value(doc).expect("an analysis")
    }

    fn est(value: f64) -> serde_json::Value {
        serde_json::json!({ "estimated": { "value": value, "confidence": 0.9 } })
    }

    fn four_four() -> serde_json::Value {
        serde_json::json!({ "estimated": { "value": "4/4", "confidence": 0.9 } })
    }

    fn composition(bpm: f64, meter: &str, key: Option<&str>) -> Composition {
        Composition::new(
            bpm,
            meter.parse().expect("a meter"),
            key.map(|k| k.parse().expect("a key")),
        )
    }

    fn said(analysis: &Analysis, composition: &Composition, placed: &Placed) -> Vec<String> {
        disagreements(&name("wayfaring-stranger"), composition, placed, analysis)
    }

    fn estimated_at(bpm: f64) -> Placed {
        Placed::estimated(Position::bar(1), 1.0, bpm)
    }

    #[test]
    fn the_estimate_is_the_measured_tempo_not_the_rounded_one() {
        let a = analysis(92.4, est(92.0), four_four(), None, DIGEST);
        let placed = estimate(&a);
        assert_eq!(placed.by, By::Estimated);
        assert_eq!(placed.bar, Position::bar(1));
        assert!((placed.bpm.expect("a tempo") - 92.4).abs() < 1e-9);
        assert!((placed.at_secs - 11_653.0 / f64::from(RATE)).abs() < 1e-12);
    }

    #[test]
    fn capture_replaces_an_unplaced_or_estimated_placement_and_keeps_click_and_set() {
        let new = Placed::estimated(Position::bar(1), 2.0, 90.0);
        assert_eq!(by_capture(None, new), Replace::Place(new));
        let old = estimated_at(88.0);
        assert_eq!(by_capture(Some(&old), new), Replace::Place(new));
        let click = Placed::click(Position::bar(1), 1.0, None);
        assert_eq!(by_capture(Some(&click), new), Replace::Keep(By::Click));
        let set = Placed::set(Position::bar(1), 1.0, 90.0);
        assert_eq!(by_capture(Some(&set), new), Replace::Keep(By::Set));
    }

    #[test]
    fn a_tempo_more_than_half_a_bpm_from_the_songs_is_stated_and_one_within_it_is_not() {
        let song = composition(92.0, "4/4", None);
        let far = analysis(92.6, est(93.0), four_four(), None, DIGEST);
        assert_eq!(
            said(&far, &song, &estimated_at(92.6)),
            ["tempo estimated 92.6 bpm; wayfaring-stranger is 92 at bar 1"]
        );
        let near = analysis(92.4, est(92.0), four_four(), None, DIGEST);
        assert!(said(&near, &song, &estimated_at(92.4)).is_empty());
    }

    #[test]
    fn a_tempo_near_half_or_double_is_said_to_be() {
        let song = composition(92.0, "4/4", None);
        let half = analysis(46.2, est(46.0), four_four(), None, DIGEST);
        assert_eq!(
            said(&half, &song, &estimated_at(46.2)),
            ["tempo estimated 46.2 bpm, half wayfaring-stranger's 92 at bar 1"]
        );
        let double = analysis(184.0, est(184.0), four_four(), None, DIGEST);
        assert_eq!(
            said(&double, &song, &estimated_at(184.0)),
            ["tempo estimated 184 bpm, double wayfaring-stranger's 92 at bar 1"]
        );
    }

    #[test]
    fn a_meter_and_a_key_that_differ_are_stated_and_a_rhythm_states_no_key() {
        let song = composition(92.0, "3/4", Some("G minor"));
        let melody = analysis(92.0, est(92.0), four_four(), Some("A minor"), DIGEST);
        assert_eq!(
            said(&melody, &song, &estimated_at(92.0)),
            [
                "meter estimated 4/4; wayfaring-stranger is 3/4 at bar 1",
                "key estimated A minor; the song is G minor at bar 1",
            ]
        );
        let rhythm = analysis(92.0, est(92.0), four_four(), None, DIGEST);
        assert_eq!(
            said(&rhythm, &song, &estimated_at(92.0)),
            ["meter estimated 4/4; wayfaring-stranger is 3/4 at bar 1"]
        );
        // The same meter and key say nothing.
        let same = composition(92.0, "4/4", Some("A minor"));
        assert!(said(&melody, &same, &estimated_at(92.0)).is_empty());
    }

    #[test]
    fn a_given_tempo_is_stated_as_given() {
        let song = composition(92.0, "4/4", None);
        let a = analysis(
            100.0,
            serde_json::json!({ "given": 100.0 }),
            four_four(),
            None,
            DIGEST,
        );
        assert_eq!(
            said(&a, &song, &estimated_at(100.0)),
            ["tempo given 100 bpm; wayfaring-stranger is 92 at bar 1"]
        );
    }

    #[test]
    fn a_set_placement_also_states_the_set_tempo_it_disagrees_with() {
        let song = composition(88.6, "4/4", None);
        let a = analysis(88.6, est(89.0), four_four(), None, DIGEST);
        let set = Placed::set(Position::bar(1), 1.0, 90.0);
        assert_eq!(
            said(&a, &song, &set),
            ["tempo estimated 88.6 bpm; the placement is set at 90 bpm"]
        );
        let within = Placed::set(Position::bar(1), 1.0, 88.9);
        assert!(said(&a, &song, &within).is_empty());
    }

    #[test]
    fn a_placement_is_said_with_what_it_moved_from() {
        let before = Placed::estimated(Position::bar(1), 1.061, 88.2);
        let after = Placed::estimated(Position::bar(1), 1.057, 88.6);
        assert_eq!(
            placed_said(
                &name("wayfaring-stranger"),
                &name("wayfaring"),
                Some(&before),
                &after
            ),
            "placed the take wayfaring on wayfaring-stranger's bars, estimated: bar 1.1.0 at \
             1.057 s, 88.6 bpm (was 88.2 bpm, bar 1.1.0 at 1.061 s)"
        );
        assert_eq!(
            placed_said(
                &name("wayfaring-stranger"),
                &name("wayfaring"),
                None,
                &after
            ),
            "placed the take wayfaring on wayfaring-stranger's bars, estimated: bar 1.1.0 at \
             1.057 s, 88.6 bpm"
        );
    }

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("duet-placing-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn write_analysis(dir: &Path, file: &str, a: &Analysis) {
        let text = Document::new(tool(), a.clone()).write().expect("writes");
        fs::write(dir.join(file), text).expect("writes");
    }

    #[test]
    fn the_latest_analysis_of_the_take_is_read_and_another_takes_is_not() {
        let dir = scratch("latest");
        let older = analysis(90.0, est(90.0), four_four(), None, DIGEST);
        let newer = analysis(91.0, est(91.0), four_four(), None, DIGEST);
        let other = analysis(70.0, est(70.0), four_four(), None, OTHER);
        write_analysis(&dir, "t.rules.capture.duet", &older);
        std::thread::sleep(Duration::from_millis(50));
        write_analysis(&dir, "t.basic-pitch.capture.duet", &newer);
        std::thread::sleep(Duration::from_millis(50));
        // Newest of all, but of another take; and one that does not read.
        write_analysis(&dir, "t.other.capture.duet", &other);
        fs::write(dir.join("t.broken.capture.duet"), "not a document").expect("writes");

        let (path, found) = latest_analysis(&dir, &digest(DIGEST))
            .expect("reads")
            .expect("one");
        assert!(path.ends_with("t.basic-pitch.capture.duet"));
        assert_eq!(found, newer);
        let (path, _) = latest_analysis(&dir, &digest(OTHER))
            .expect("reads")
            .expect("one");
        assert!(path.ends_with("t.other.capture.duet"));
        assert!(
            latest_analysis(&dir, &digest(&format!("sha256:{}", "3".repeat(64))))
                .expect("reads")
                .is_none()
        );
        assert!(
            latest_analysis(&dir.join("absent"), &digest(DIGEST))
                .expect("reads")
                .is_none()
        );
    }

    fn write_song(root: &Path, folder: &str, links: &[(&str, &str)]) {
        let mut song = Song::new(
            fresh_id().expect("an id"),
            "A song",
            composition(92.0, "4/4", None),
        );
        for (take, digest_text) in links {
            song.takes
                .push(TakeLink::new(name(take), digest(digest_text)));
        }
        let dir = root.join("songs").join(folder);
        fs::create_dir_all(&dir).expect("a song folder");
        fs::write(
            dir.join("song.duet"),
            Document::new(tool(), song).write().expect("writes"),
        )
        .expect("writes");
    }

    #[test]
    fn the_songs_linking_a_take_are_found_by_digest_and_sorted_by_name() {
        let root = scratch("songs");
        write_song(&root, "wayfaring-stranger", &[("a", DIGEST)]);
        write_song(&root, "amazing-grace", &[("b", DIGEST), ("c", OTHER)]);
        write_song(&root, "unrelated", &[("c", OTHER)]);
        fs::create_dir_all(root.join("songs").join("empty")).expect("a folder");
        let found = songs_linking(&root, &digest(DIGEST)).expect("reads");
        assert_eq!(
            found,
            [
                (name("amazing-grace"), root.join("songs/amazing-grace")),
                (
                    name("wayfaring-stranger"),
                    root.join("songs/wayfaring-stranger")
                ),
            ]
        );
        assert!(
            songs_linking(&scratch("none"), &digest(DIGEST))
                .expect("reads")
                .is_empty()
        );

        fs::write(root.join("songs/unrelated/song.duet"), "not a song").expect("writes");
        let err = songs_linking(&root, &digest(DIGEST)).expect_err("a song that does not read");
        assert!(format!("{err:#}").contains("unrelated"), "{err:#}");
    }
}
