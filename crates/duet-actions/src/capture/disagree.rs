//! `duet capture disagree`: the places two readings of one take disagree, found by kind, so the
//! owner can listen to them without listening through the take.
//!
//! The design is `docs/superpowers/specs/2026-11-20-reader-disagreement-design.md`, ruled
//! 2026-11-20 (A to D as recommended). The command reads two existing readings; it makes none.
//! The second is a second `duet capture melody --reader rules` run (A), so a capture run without
//! asking for this behaves exactly as before.
//!
//! - **What it reads:** the stem's own `.capture.duet`, named after its reader, and one
//!   `--reading NAME=ANALYSIS`, the other; or in a project, with no `-o` and no `--reading`, the
//!   take's two readings in `takes/TAKE/`, named by their labels.
//! - **What it finds:** the places of [`disagreements`]: stretches between the readings' seams
//!   whose keys differ, each note stated once as `split`, `key`, `only` or `other`. A stretch of
//!   the same keys is no place (B); only how many differ in a boundary by more than 30 ms, and by
//!   how much at most, is said.
//! - **What it writes:** a moments file, `STEM.disagree.moments` (in a project,
//!   `takes/TAKE/TAKE.disagree.moments`) or `--moments FILE`, new, in the format `duet capture
//!   review` reads (C: a place's kind is its label's first word). Each moment is its stretch with
//!   0.15 s either side, kept within the take, under a comment holding both readings' notes in
//!   the stretch. On standard output, each moment's line, then a summary. When nothing is found
//!   it says so and writes no file, since `review` refuses a moments file with no moment.
//! - **What it refuses, before anything is written:** a moments file that exists; two readings
//!   of different takes, or a reading that states no take, or whose notes are out of order or overlap; two readings on different tunings; a
//!   name given twice.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use duet_capture::{
    Analysis, Disagreements, Found, Origin, Place, PlaceKind, ReadNote, Reading, Side, Take,
    disagreements, in_order,
};
use schemars::JsonSchema;

use super::compare::{analysis, reading_of, svg::key_name, with_suffix, write_bytes_new};
use super::review::{named_reading, reader_name};

/// How far either side of a place's stretch its moment reaches, in seconds.
const WINDOW_PAD_SECS: f64 = 0.15;

/// How far apart, in seconds, two readings' boundaries of the same notes may be and still be
/// called the same: a stretch of the same keys with a boundary further apart is counted as one
/// that differs only in where notes start or end.
const SAME_BOUNDARY_SECS: f64 = 0.03;

/// What `duet capture disagree` is given.
#[derive(Debug, Args, JsonSchema)]
pub struct DisagreeArgs {
    /// The take's recording, as `duet capture melody` read it. In a project, a take's name too.
    input: PathBuf,
    /// The stem the take was captured to (`duet capture melody -o`): its .capture.duet is the
    /// first reading, named after its reader. Without it, the input's name without its
    /// extension, beside it; for a project's take with no --reading, its two readings in
    /// takes/TAKE/.
    #[arg(short, long, value_name = "STEM")]
    output: Option<PathBuf>,
    /// The other reading of the same take: NAME=ANALYSIS, such as
    /// rules=take-rules.capture.duet, made by `duet capture melody --reader rules -o
    /// take-rules`.
    #[arg(long = "reading", value_name = "NAME=ANALYSIS")]
    reading: Option<String>,
    /// The moments file to write, new. Without it, STEM.disagree.moments; for a project's take
    /// with no -o, takes/TAKE/TAKE.disagree.moments.
    #[arg(long, value_name = "FILE")]
    moments: Option<PathBuf>,
}

/// One of the two readings: its name, the analysis it is from, and that analysis.
struct Named {
    name: String,
    path: PathBuf,
    analysis: Analysis,
}

impl Named {
    fn reading(&self) -> &Reading {
        self.analysis
            .reading
            .as_ref()
            .expect("checked by reading_of when loaded")
    }

    fn notes(&self) -> &[ReadNote] {
        &self.reading().notes
    }
}

pub fn run(ctx: &crate::Context, args: DisagreeArgs) -> Result<()> {
    let input = crate::take::input(ctx, &args.input, args.output.is_none())?;
    // The two readings, as (name, analysis), and the moments file's default.
    let (named, default_moments) = match (&args.output, &args.reading, &input.take) {
        (None, None, Some(take)) => {
            let labels = take.readings();
            let [a, b] = labels.as_slice() else {
                let dir = Path::new(crate::take::TAKES_DIR).join(take.name.as_str());
                bail!(
                    "the take {} has {} in {}/{}: disagree compares two; with -o and --reading, \
                     name the two to compare",
                    take.name,
                    match labels.len() {
                        0 => "no reading".to_owned(),
                        1 => "one reading".to_owned(),
                        n => format!("{n} readings"),
                    },
                    dir.display(),
                    if labels.is_empty() {
                        String::new()
                    } else {
                        format!(", {}", labels.join(", "))
                    }
                );
            };
            (
                vec![
                    (a.clone(), with_suffix(&take.stem(a), ".capture.duet")),
                    (b.clone(), with_suffix(&take.stem(b), ".capture.duet")),
                ],
                take.dir().join(format!("{}.disagree.moments", take.name)),
            )
        }
        (output, reading, take) => {
            let stem = match (output, take) {
                (Some(stem), _) => stem.clone(),
                (None, Some(take)) => take.reading_stem()?,
                (None, None) => input.file.with_extension(""),
            };
            let Some(reading) = reading else {
                bail!(
                    "disagree compares two readings: give the other with --reading NAME=ANALYSIS, \
                     such as --reading rules={}, made by duet capture melody {} --reader rules \
                     -o {}",
                    with_suffix(&with_suffix(&stem, "-rules"), ".capture.duet").display(),
                    args.input.display(),
                    with_suffix(&stem, "-rules").display()
                );
            };
            let own = with_suffix(&stem, ".capture.duet");
            let own_name = reader_name(reading_of(&analysis(&own)?, &own)?);
            (
                vec![(own_name, own), named_reading(reading)?],
                with_suffix(&stem, ".disagree.moments"),
            )
        }
    };
    let moments = args.moments.clone().unwrap_or(default_moments);
    // Refused before anything is read, as every capture command refuses: nothing is overwritten.
    if moments.exists() {
        bail!(
            "{} exists; disagree writes a new moments file and never overwrites one, so move it \
             or name another with --moments",
            moments.display()
        );
    }
    let [a, b] = load(named)?;
    let take = one_take(&a, &b)?;
    one_tuning(&a, &b)?;

    let found = disagreements(a.notes(), b.notes());
    let rate = f64::from(a.analysis.rate);
    let same = (SAME_BOUNDARY_SECS * rate).round() as u64;
    let boundary_only: Vec<u64> = found
        .agreed
        .iter()
        .map(|x| x.boundary)
        .filter(|&d| d > same)
        .collect();
    let boundary = match boundary_only.iter().max() {
        None => "no stretch differs only in where notes start or end".to_owned(),
        Some(&most) => format!(
            "{} differ{} only in where notes start or end, by up to {:.2} s",
            count(boundary_only.len(), "stretch", "stretches"),
            if boundary_only.len() == 1 { "s" } else { "" },
            most as f64 / rate
        ),
    };
    if found.places.is_empty() {
        println!("nothing found: {boundary}; no moments file is written");
        return Ok(());
    }
    let take_secs = take.frames as f64 / f64::from(take.rate);
    let text = moments_text(&a, &b, &take, &found, take_secs);
    write_bytes_new(&moments, text.as_bytes())?;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        println!("{line}");
    }
    println!(
        "{}: {}; {boundary}",
        count(found.places.len(), "place", "places"),
        kinds(&found.places)
    );
    eprintln!("wrote {}", moments.display());
    Ok(())
}

/// The two readings, each loaded and checked to hold a reading, their names apart.
fn load(named: Vec<(String, PathBuf)>) -> Result<[Named; 2]> {
    let mut out = Vec::new();
    for (name, path) in named {
        let analysis = analysis(&path)?;
        if !in_order(&reading_of(&analysis, &path)?.notes) {
            bail!(
                "{}: its notes are not in order, or one overlaps the next, as no reader reads \
                 them; capture the melody again with duet capture melody",
                path.display()
            );
        }
        if let Some(first) = out.first().filter(|f: &&Named| f.name == name) {
            bail!(
                "two readings are named {name}, {} and {}: name the --reading apart from the \
                 stem's own, which is named after its reader",
                first.path.display(),
                path.display()
            );
        }
        out.push(Named {
            name,
            path,
            analysis,
        });
    }
    let [a, b]: [Named; 2] = out.try_into().ok().expect("two readings");
    if a.analysis.rate != b.analysis.rate {
        bail!(
            "{} counts frames at {} Hz and {} at {} Hz: they are not of one performance",
            a.path.display(),
            a.analysis.rate,
            b.path.display(),
            b.analysis.rate
        );
    }
    Ok([a, b])
}

/// The take both readings are of, refused when either states none or they differ.
fn one_take(a: &Named, b: &Named) -> Result<Take> {
    let stated = |x: &Named| {
        x.analysis.take.with_context(|| {
            format!(
                "{} states no take (it was written before capture-analysis 1.13), so it cannot \
                 be told to be a reading of the same take: capture it again with duet capture \
                 melody",
                x.path.display()
            )
        })
    };
    let (ta, tb) = (stated(a)?, stated(b)?);
    if ta.digest != tb.digest {
        bail!(
            "{} is a reading of take {} and {} of take {}: they are not readings of one take",
            a.path.display(),
            ta.digest,
            b.path.display(),
            tb.digest
        );
    }
    Ok(ta)
}

/// Refused unless both readings are on one tuning: a key is only comparable on one.
fn one_tuning(a: &Named, b: &Named) -> Result<()> {
    let (x, y) = (&a.reading().tuning, &b.reading().tuning);
    if x.a4.value() == y.a4.value() {
        return Ok(());
    }
    let said = |t: &duet_capture::Tuning| {
        format!(
            "A4 = {:.2} Hz ({})",
            t.a4_hz(),
            match t.a4 {
                Origin::Given(_) => "given",
                Origin::Estimated { .. } => "estimated",
            }
        )
    };
    bail!(
        "{} is read on {} and {} on {}: a key is only comparable on one tuning, so capture both \
         with one, such as --tuning {:.2}",
        a.path.display(),
        said(x),
        b.path.display(),
        said(y),
        x.a4_hz()
    )
}

/// `n` and the noun, singular or plural.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// How many places are of each kind, in the order of [`PlaceKind`]: `1 split, 1 key, 2 only`.
fn kinds(places: &[Place]) -> String {
    let mut by: BTreeMap<PlaceKind, usize> = BTreeMap::new();
    for place in places {
        *by.entry(place.kind()).or_default() += 1;
    }
    by.iter()
        .map(|(kind, n)| format!("{n} {}", kind.word()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Seconds to two places.
fn hundredths(t: f64) -> String {
    format!("{t:.2}")
}

/// A moment's end: its stretch's end and pad, to two places, but never past the take's end.
fn moment_end(t: f64, take_secs: f64) -> String {
    let floor = (take_secs * 100.0).floor() / 100.0;
    hundredths(((t * 100.0).round() / 100.0).min(floor))
}

/// The moments file: a header comment naming both readings and the take, then each place's
/// notes as a comment and its moment.
fn moments_text(
    a: &Named,
    b: &Named,
    take: &Take,
    found: &Disagreements,
    take_secs: f64,
) -> String {
    let rate = f64::from(a.analysis.rate);
    let mut text = format!(
        "# disagree: {} ({}) against {} ({}), take {}\n",
        a.name,
        a.path.display(),
        b.name,
        b.path.display(),
        take.digest
    );
    let side = |s: Side| match s {
        Side::A => a,
        Side::B => b,
    };
    let keys = |s: Side, notes: &[usize]| -> String {
        if notes.is_empty() {
            "none".to_owned()
        } else {
            notes
                .iter()
                .map(|&i| key_name(side(s).notes()[i].key))
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    for place in &found.places {
        let st = &place.stretch;
        let from = (st.start as f64 / rate - WINDOW_PAD_SECS).max(0.0);
        let to = st.end as f64 / rate + WINDOW_PAD_SECS;
        let label = place
            .found
            .iter()
            .map(|f| item(f, side(f.side), side(f.side.other()), rate))
            .collect::<Vec<_>>()
            .join("; ");
        text.push_str(&format!(
            "# {}: {} | {}: {}\n{} {} {label}\n",
            a.name,
            keys(Side::A, &st.a),
            b.name,
            keys(Side::B, &st.b),
            hundredths(from),
            moment_end(to, take_secs),
        ));
    }
    text
}

/// One stated note, as its kind's word and what it is: `split: hmm D#3 at 1.13 is rules D#3 E3,
/// its key among them`.
fn item(f: &Found, own: &Named, other: &Named, rate: f64) -> String {
    let note = &own.notes()[f.note];
    let at = format!(
        "{} {} at {}",
        own.name,
        key_name(note.key),
        hundredths(note.start as f64 / rate)
    );
    let with: Vec<String> = f
        .with
        .iter()
        .map(|&j| key_name(other.notes()[j].key))
        .collect();
    let word = f.kind.word();
    match f.kind {
        PlaceKind::Split => {
            let among = f.with.iter().any(|&j| other.notes()[j].key == note.key);
            format!(
                "{word}: {at} is {} {}, its key {}among them",
                other.name,
                with.join(" "),
                if among { "" } else { "not " }
            )
        }
        PlaceKind::Key => format!("{word}: {at}, {} {}", other.name, with.join(" ")),
        PlaceKind::Only => format!("{word}: {at}"),
        PlaceKind::Other if with.is_empty() => format!("{word}: {at}, with no {} note", other.name),
        PlaceKind::Other => format!("{word}: {at}, with {} {}", other.name, with.join(" ")),
    }
}
