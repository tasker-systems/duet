//! The comparison's plot, as SVG written by hand, with no dependency: the performance's pitch
//! over time, the reading's notes as bars at their keys on the take's tuning, the residual
//! shaded between them, and the onsets as ticks.
//!
//! Every element a reader or a test looks for has a class: `performance`, `note` (and
//! `between-keys` beside it), `residual`, `beside` (a second reading's notes), `onset` and
//! `key` (a key's gridline).

use std::fmt::Write as _;

use duet_capture::{PerformanceFrames, ReadNote, Reading};

use super::wav::key_cents;

/// Pixels a second of the take is drawn across, until the plot is as wide as [`MAX_WIDTH`].
const PX_PER_SEC: f64 = 100.0;
const MIN_WIDTH: f64 = 600.0;
const MAX_WIDTH: f64 = 30_000.0;
/// How tall the plot area is, at most and at least, in pixels.
const MIN_HEIGHT: f64 = 240.0;
const MAX_HEIGHT: f64 = 900.0;
/// Pixels a cent is drawn across, before the height is clamped.
const PX_PER_CENT: f64 = 0.6;
/// Margins about the plot area: room for key names on the left and seconds below.
const LEFT: f64 = 56.0;
const RIGHT: f64 = 16.0;
const TOP: f64 = 44.0;
const BOTTOM: f64 = 40.0;
/// How far, in cents, the plot reaches past the highest and lowest pitch it shows.
const MARGIN_CENTS: f64 = 60.0;
/// A note bar's height, in pixels.
const BAR: f64 = 6.0;

/// What a plot shows: the performance and its onsets over `from..to` (sample frames on the
/// take's timeline), and the reading, with a second reading beside it when one is compared.
pub struct Plot<'a> {
    pub title: &'a str,
    pub frames: &'a PerformanceFrames,
    pub from: u64,
    pub to: u64,
    pub reading: &'a Reading,
    pub beside: Option<&'a Reading>,
    pub onsets: &'a [u64],
}

/// A note shown in the window `from..to`: one that overlaps it.
pub fn shown(note: &ReadNote, from: u64, to: u64) -> bool {
    note.start < to && note.end > from
}

struct Scale {
    from: u64,
    rate: f64,
    px_per_sec: f64,
    hi: f64,
    px_per_cent: f64,
    width: f64,
    height: f64,
}

impl Scale {
    fn x(&self, frame: u64) -> f64 {
        LEFT + (frame as f64 - self.from as f64) / self.rate * self.px_per_sec
    }

    fn y(&self, cents: f64) -> f64 {
        TOP + (self.hi - cents) * self.px_per_cent
    }
}

/// The plot as an SVG document.
pub fn render(plot: &Plot) -> String {
    let frames = plot.frames;
    let rate = f64::from(frames.rate);
    let secs = (plot.to - plot.from) as f64 / rate;
    let width = (secs * PX_PER_SEC).clamp(MIN_WIDTH, MAX_WIDTH);

    // The window's frames, and the pitch range they and its notes span.
    let window: Vec<usize> = (0..frames.len())
        .filter(|&k| (plot.from..=plot.to).contains(&frames.frame(k)))
        .collect();
    let notes: Vec<&ReadNote> = plot
        .reading
        .notes
        .iter()
        .filter(|n| shown(n, plot.from, plot.to))
        .collect();
    let beside: Vec<&ReadNote> = plot
        .beside
        .map(|r| {
            r.notes
                .iter()
                .filter(|n| shown(n, plot.from, plot.to))
                .collect()
        })
        .unwrap_or_default();
    let pitches = window
        .iter()
        .map(|&k| f64::from(frames.cents[k]))
        .filter(|c| !c.is_nan())
        .chain(notes.iter().map(|n| key_cents(plot.reading, n.key)))
        .chain(
            plot.beside
                .into_iter()
                .flat_map(|r| beside.iter().map(move |n| key_cents(r, n.key))),
        );
    let (lo, hi) = pitches.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| {
        (lo.min(c), hi.max(c))
    });
    let (lo, hi) = if lo.is_finite() {
        (lo - MARGIN_CENTS, hi + MARGIN_CENTS)
    } else {
        (6000.0, 7200.0)
    };
    let height = ((hi - lo) * PX_PER_CENT).clamp(MIN_HEIGHT, MAX_HEIGHT);
    let s = Scale {
        from: plot.from,
        rate,
        px_per_sec: width / secs,
        hi,
        px_per_cent: height / (hi - lo),
        width,
        height,
    };
    let total_w = LEFT + width + RIGHT;
    let total_h = TOP + height + BOTTOM;

    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w:.0}" height="{total_h:.0}" viewBox="0 0 {total_w:.0} {total_h:.0}" font-family="sans-serif" font-size="11">"#
    );
    out.push_str(
        "<style>\
         .performance{fill:none;stroke:#1f5fa8;stroke-width:1.2}\
         .note{fill:#2e7d32}\
         .note.between-keys{fill:#e65100}\
         .residual{fill:#c62828;fill-opacity:0.25;stroke:none}\
         .beside{fill:none;stroke:#6a1b9a;stroke-width:1;stroke-dasharray:3 2}\
         .onset{stroke:#444;stroke-width:1}\
         .key{stroke:#e0e0e0;stroke-width:1}\
         .key.c{stroke:#bdbdbd}\
         .label{fill:#555}\
         </style>\n",
    );
    let _ = writeln!(
        out,
        r##"<rect x="0" y="0" width="{total_w:.0}" height="{total_h:.0}" fill="#ffffff"/>"##
    );
    let _ = writeln!(
        out,
        r#"<text x="{LEFT}" y="18" font-size="13">{}</text>"#,
        escape(plot.title)
    );
    let _ = writeln!(
        out,
        r#"<text x="{LEFT}" y="34" class="label">line: the performance · bars: the reading's keys on its tuning{} · shaded: the residual · orange: between keys · ticks: onsets</text>"#,
        if plot.beside.is_some() {
            " · dashed: the stem's own reading"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        r#"<defs><clipPath id="plot"><rect x="{LEFT}" y="{TOP}" width="{width:.1}" height="{height:.1}"/></clipPath></defs>"#
    );

    // Each key's gridline, named where there is room.
    let room = s.px_per_cent * 100.0 >= 12.0;
    let a4 = plot.reading.tuning.a4.value();
    let lowest = ((lo - a4) / 100.0).ceil() as i64 + 69;
    let highest = ((hi - a4) / 100.0).floor() as i64 + 69;
    for key in lowest.max(0)..=highest.min(127) {
        let key = key as u8;
        let y = s.y(key_cents(plot.reading, key));
        let c = key.is_multiple_of(12);
        let _ = writeln!(
            out,
            r#"<line class="key{}" x1="{LEFT}" x2="{:.1}" y1="{y:.1}" y2="{y:.1}"/>"#,
            if c { " c" } else { "" },
            LEFT + width
        );
        if room || c {
            let _ = writeln!(
                out,
                r#"<text class="label" x="{:.1}" y="{:.1}" text-anchor="end">{}</text>"#,
                LEFT - 4.0,
                y + 4.0,
                key_name(key)
            );
        }
    }

    // Seconds along the bottom.
    let step = if secs <= 30.0 {
        1.0
    } else if secs <= 150.0 {
        5.0
    } else {
        10.0
    };
    let start = plot.from as f64 / rate;
    let mut t = (start / step).ceil() * step;
    while t <= start + secs + 1e-9 {
        let x = s.x((t * rate).round() as u64);
        let _ = writeln!(
            out,
            r#"<text class="label" x="{x:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
            TOP + height + 30.0,
            seconds(t)
        );
        t += step;
    }

    out.push_str(r#"<g clip-path="url(#plot)">"#);
    out.push('\n');
    // The residual, between each note's bar and the pitch over its frames.
    for note in &notes {
        let bar = s.y(key_cents(plot.reading, note.key));
        let run = |out: &mut String, run: &[(f64, f64)]| {
            if run.len() < 2 {
                return;
            }
            let mut d = String::new();
            for (i, (x, y)) in run.iter().enumerate() {
                let _ = write!(d, "{}{x:.1} {y:.1} ", if i == 0 { 'M' } else { 'L' });
            }
            let _ = write!(
                d,
                "L{:.1} {bar:.1} L{:.1} {bar:.1} Z",
                run[run.len() - 1].0,
                run[0].0
            );
            let _ = writeln!(out, r#"<path class="residual" d="{d}"/>"#);
        };
        let mut points = Vec::new();
        for &k in &window {
            let at = frames.frame(k);
            if at < note.start || at >= note.end {
                continue;
            }
            let c = f64::from(frames.cents[k]);
            if c.is_nan() {
                run(&mut out, &points);
                points.clear();
            } else {
                points.push((s.x(at), s.y(c)));
            }
        }
        run(&mut out, &points);
    }
    // The performance's pitch, broken where a frame is unvoiced.
    let mut d = String::new();
    let mut pen = false;
    for &k in &window {
        let c = f64::from(frames.cents[k]);
        if c.is_nan() {
            pen = false;
            continue;
        }
        let _ = write!(
            d,
            "{}{:.1} {:.1} ",
            if pen { 'L' } else { 'M' },
            s.x(frames.frame(k)),
            s.y(c)
        );
        pen = true;
    }
    if !d.is_empty() {
        let _ = writeln!(out, r#"<path class="performance" d="{}"/>"#, d.trim_end());
    }
    // A second reading's notes, outlined.
    if let Some(other) = plot.beside {
        for note in &beside {
            let (x0, x1) = span(&s, note, plot);
            let y = s.y(key_cents(other, note.key));
            let _ = writeln!(
                out,
                r#"<rect class="beside" x="{x0:.1}" y="{:.1}" width="{:.1}" height="{:.1}"/>"#,
                y - BAR,
                x1 - x0,
                2.0 * BAR
            );
        }
    }
    // The reading's notes.
    for note in &notes {
        let (x0, x1) = span(&s, note, plot);
        let y = s.y(key_cents(plot.reading, note.key));
        let _ = writeln!(
            out,
            r#"<rect class="note{}" x="{x0:.1}" y="{:.1}" width="{:.1}" height="{BAR}"><title>{} {:+.1} cents{}</title></rect>"#,
            if note.between_keys {
                " between-keys"
            } else {
                ""
            },
            y - BAR / 2.0,
            (x1 - x0).max(0.5),
            key_name(note.key),
            note.centre,
            if note.between_keys {
                ", between keys"
            } else {
                ""
            }
        );
    }
    out.push_str("</g>\n");

    // The onsets, as ticks under the plot.
    for &at in plot.onsets {
        if !(plot.from..plot.to).contains(&at) {
            continue;
        }
        let x = s.x(at);
        let _ = writeln!(
            out,
            r#"<line class="onset" x1="{x:.1}" x2="{x:.1}" y1="{:.1}" y2="{:.1}"/>"#,
            TOP + height,
            TOP + height + 10.0
        );
    }
    let _ = writeln!(
        out,
        r##"<rect x="{LEFT}" y="{TOP}" width="{:.1}" height="{:.1}" fill="none" stroke="#9e9e9e"/>"##,
        s.width, s.height
    );
    out.push_str("</svg>\n");
    out
}

/// Where a note's bar starts and ends, cut to the window.
fn span(s: &Scale, note: &ReadNote, plot: &Plot) -> (f64, f64) {
    (s.x(note.start.max(plot.from)), s.x(note.end.min(plot.to)))
}

/// A key's name, sharps for black keys: `A4`, `C#5`.
pub fn key_name(key: u8) -> String {
    duet_core::Pitch::new(key).map_or_else(|_| format!("key {key}"), |p| p.to_string())
}

/// Seconds as a label: whole seconds without a point.
fn seconds(t: f64) -> String {
    if (t - t.round()).abs() < 1e-9 {
        format!("{:.0} s", t.round())
    } else {
        format!("{t:.1} s")
    }
}

/// Text made safe inside an SVG element.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
