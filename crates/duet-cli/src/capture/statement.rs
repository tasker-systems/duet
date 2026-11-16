//! What `duet capture` states about a take: each estimate, the lead-in, and the snap. Each
//! statement is one line, written on standard error and as a comment in the files.
//!
//! A given value is used exactly and is not stated: only an estimate is reported as one.

use duet_capture::{
    KeyEstimate, LeadIn, Merge, Origin, ReaderName, Reading, TempoAlternatives, Timing, Tuning,
    WEAK_CONFIDENCE, felt_quarters, is_compound,
};
use duet_core::{Grid, TimeSig};

/// The line the score states, in place of its lead-in, when it has had the lead-in removed.
pub const LEAD_IN_REMOVED: &str = "lead-in: removed from the score, where no note starts \
                                   before the downbeat once quantized; the .duet keeps it";

/// The tempo, meter and downbeat, each stated only when it was estimated.
pub fn timing(timing: &Timing, rate: u32) -> Vec<String> {
    let mut out = Vec::new();
    if let Origin::Estimated { value, confidence } = timing.tempo_bpm {
        let alternatives = timing
            .tempo_alternatives
            .unwrap_or_else(|| TempoAlternatives::of(value));
        let (half, double) = (alternatives.half, alternatives.double);
        let felt = felt_beat(*timing.meter.value(), value)
            .map(|f| format!(" ({f})"))
            .unwrap_or_default();
        out.push(format!(
            "tempo: estimated {} bpm{felt} (half {}, double {}), {}",
            number(value),
            number(half),
            number(double),
            sureness(confidence)
        ));
    }
    if let Origin::Estimated { value, confidence } = timing.meter {
        out.push(format!(
            "meter: estimated {value}, {}",
            sureness(confidence)
        ));
    }
    if let Origin::Estimated { value, confidence } = timing.downbeat_frame {
        out.push(format!(
            "downbeat: estimated at {:.3} s, {}",
            value as f64 / f64::from(rate),
            sureness(confidence)
        ));
    }
    out
}

/// The tuning, stated only when it was estimated: A4 in Hz and in cents from 440 Hz, how
/// widely the take spread about it, and how sure the estimate is.
pub fn tuning(tuning: &Tuning) -> Option<String> {
    let Origin::Estimated { confidence, .. } = tuning.a4 else {
        return None;
    };
    let spread = tuning
        .spread_cents
        .map(|s| format!(", spread {}", cents(format!("{s:.0}"))))
        .unwrap_or_default();
    Some(format!(
        "tuning: estimated A4 = {:.1} Hz ({}){spread}, {}",
        tuning.a4_hz(),
        cents(format!("{:+.0}", tuning.offset_cents())),
        sureness(confidence)
    ))
}

/// What to do about a weak tuning estimate: give the tuning. Advice, not a statement, so it is
/// written on standard error only, never into the files. None for a given or a firm tuning.
pub fn tuning_advice(tuning: &Tuning) -> Option<&'static str> {
    match tuning.a4 {
        Origin::Estimated { confidence, .. } if confidence < WEAK_CONFIDENCE => {
            Some(WEAK_TUNING_ADVICE)
        }
        _ => None,
    }
}

/// The line `duet capture` writes after a weak tuning estimate.
pub const WEAK_TUNING_ADVICE: &str = "tuning is weak; give it with --tuning 440|+44c";

/// A whole number of cents, as written: `+1 cent`, `-12 cents`.
fn cents(n: String) -> String {
    if n.trim_start_matches(['+', '-']) == "1" {
        format!("{n} cent")
    } else {
        format!("{n} cents")
    }
}

/// The reader, stated only when it is not the note HMM (the default): the segmentation rules.
pub fn reader(reading: &Reading) -> Option<String> {
    match reading.reader?.name {
        ReaderName::Rules => Some("reader: the segmentation rules".to_owned()),
        ReaderName::BasicPitch => {
            let mut said =
                "reader: Spotify's Basic Pitch (basic-pitch-rs), each note keyed by the \
                            performance on the take's tuning"
                    .to_owned();
            if let Some(t) = reading.reader?.thresholds {
                said.push_str(&format!(
                    "; decoded at onset threshold {}, frame threshold {}, notes over {} ms",
                    t.onset_threshold, t.frame_threshold, t.minimum_note_length_ms
                ));
            }
            Some(said)
        }
        _ => None,
    }
}

/// What became of Basic Pitch's notes: how many it found, and how many were cut back, dropped
/// or keyed differently on the way to a reading of one note at a time.
#[cfg(feature = "basic-pitch")]
pub fn basic_pitch(read: &duet_capture::BasicPitchRead) -> String {
    format!(
        "basic-pitch: {} notes found; {} cut back and {} dropped where a louder note overlapped \
         them; {} dropped with no voiced frame; {} on another key than on A440",
        read.found, read.trimmed, read.overlapped, read.unvoiced, read.rekeyed
    )
}

/// How many notes were read, and how many of them are between keys on the take's tuning.
pub fn notes_read(reading: &Reading) -> String {
    let n = reading.notes.len();
    format!("notes: {n} read, {} between keys", reading.between_keys())
}

/// How many same-key splits the reading's pass rejoined, and what they did to the loss, in nats:
/// stated only when it rejoined any, so that a rejoin that raised the loss is never hidden.
pub fn rejoined(reading: &Reading) -> Option<String> {
    let n = reading.rejoins.len();
    if n == 0 {
        return None;
    }
    let change: f64 = reading.rejoins.iter().map(|r| r.loss_change).sum();
    Some(format!(
        "rejoined: {n} same-key split{} around a dip within a whole tone, the loss {change:+.1} nats",
        if n == 1 { "" } else { "s" }
    ))
}

/// How many notes the reading's breathy pass carried on over the weakly voiced frames after them,
/// for how long in all, and what it did to the loss, in nats: stated only when it carried any
/// on, so that reading frames #48 would leave out is never hidden.
pub fn breathy(reading: &Reading, rate: u32) -> Option<String> {
    let n = reading.breathy.len();
    if n == 0 {
        return None;
    }
    let samples: u64 = reading.breathy.iter().map(|b| b.end - b.decoded.end).sum();
    let ms = samples as f64 * 1000.0 / f64::from(rate);
    let change: f64 = reading.breathy.iter().map(|b| b.loss_change).sum();
    Some(format!(
        "breathy: {n} note{} carried on over {ms:.0} ms of weakly voiced frames whose pitch goes on, the loss {change:+.1} nats",
        if n == 1 { "" } else { "s" }
    ))
}

/// The key, stated only when it was estimated, with its relative major or minor.
pub fn key(key: Option<&KeyEstimate>) -> Option<String> {
    let estimate = key?;
    match estimate.key {
        Origin::Given(_) => None,
        Origin::Estimated { value, confidence } => Some(format!(
            "key: estimated {value} (relative {}), {}",
            estimate.alternative,
            sureness(confidence)
        )),
    }
}

/// What was done with the notes before the downbeat: nothing needed doing, a pickup, or bars
/// of rests. `downbeat_tick` is where the downbeat bar starts.
pub fn lead_in(lead_in: LeadIn, downbeat_tick: u64, meter: TimeSig, tpq: u16) -> String {
    match lead_in {
        LeadIn::None => "lead-in: none".to_owned(),
        LeadIn::Pickup(sig) => format!("lead-in: a pickup of {sig} before the downbeat"),
        LeadIn::RestBar => {
            let bar = 4 * u64::from(tpq) / u64::from(meter.den) * u64::from(meter.num);
            let bars = downbeat_tick / bar;
            format!(
                "lead-in: {bars} bar{} of rests before the downbeat",
                if bars == 1 { "" } else { "s" }
            )
        }
    }
}

/// The score's grid, stated only when it was refined from `asked`, the default or the one
/// `--grid` gave, to `used`, so that no two notes (for a rhythm, no two hits on one lane)
/// share a step.
pub fn refined(asked: Grid, given: bool, used: Grid, rhythm: bool) -> String {
    let what = if rhythm {
        "no two hits on a lane"
    } else {
        "no two notes"
    };
    format!(
        "grid: {}{asked} refined to {used}, so {what} share a step; the .duet keeps them as played",
        if given { "--grid " } else { "" }
    )
}

/// Notes on one step of the finest grid, merged into the longest of them.
pub fn merged_notes(merge: &Merge) -> String {
    format!(
        "merged: {} notes at {} into one, the longest ({}); the .duet keeps them all",
        merge.count, merge.at, merge.kept.pitch
    )
}

/// Hits on one lane and one step of the finest grid, merged into the strongest of them.
pub fn merged_hits(merge: &Merge, lane: &str) -> String {
    format!(
        "merged: {} {lane} hits at {} into one, the strongest; the .duet keeps them all",
        merge.count, merge.at
    )
}

/// The snap, stated only when a velocity moved: how many, and to what.
pub fn snapped(moved: usize, to: &str) -> String {
    format!("velocities: {moved} snapped to {to}; the .duet keeps them exact")
}

/// The felt beat of a compound meter at `quarter_bpm`, such as `dotted quarter = 90` in 6/8
/// at 135.
fn felt_beat(meter: TimeSig, quarter_bpm: f64) -> Option<String> {
    if !is_compound(meter) {
        return None;
    }
    let name = match meter.den {
        8 => "dotted quarter",
        16 => "dotted eighth",
        32 => "dotted sixteenth",
        _ => "dotted thirty-second",
    };
    Some(format!(
        "{name} = {}",
        number(quarter_bpm / felt_quarters(meter))
    ))
}

/// A tempo as a whole number when it is one, and otherwise to one decimal: 46, 46.5.
fn number(bpm: f64) -> String {
    if (bpm - bpm.round()).abs() < 1e-9 {
        format!("{bpm:.0}")
    } else {
        format!("{bpm:.1}")
    }
}

/// An estimate's confidence, marked weak below [`WEAK_CONFIDENCE`].
fn sureness(confidence: f32) -> String {
    if confidence < WEAK_CONFIDENCE {
        format!("confidence {confidence:.2}, weak")
    } else {
        format!("confidence {confidence:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_estimated_tuning_is_stated_and_a_given_one_is_not() {
        let estimated: Tuning = serde_json::from_value(serde_json::json!({
            "a4": { "estimated": { "value": 6944.0, "confidence": 0.62 } },
            "spread_cents": 26.4
        }))
        .expect("a tuning");
        assert_eq!(
            tuning(&estimated).as_deref(),
            Some("tuning: estimated A4 = 451.3 Hz (+44 cents), spread 26 cents, confidence 0.62")
        );
        assert_eq!(tuning(&Tuning::given(6944.0)), None);
    }

    #[test]
    fn a_weak_tuning_estimate_is_advised_and_a_firm_or_given_one_is_not() {
        let estimated = |confidence: f64| -> Tuning {
            serde_json::from_value(serde_json::json!({
                "a4": { "estimated": { "value": 6944.0, "confidence": confidence } },
                "spread_cents": 26.4
            }))
            .expect("a tuning")
        };
        assert_eq!(tuning_advice(&estimated(0.3)), Some(WEAK_TUNING_ADVICE));
        assert!(tuning(&estimated(0.3)).expect("stated").ends_with(", weak"));
        assert_eq!(tuning_advice(&estimated(0.8)), None);
        assert_eq!(tuning_advice(&Tuning::given(6944.0)), None);
    }

    fn sig(s: &str) -> TimeSig {
        s.parse().expect("a time signature")
    }

    fn estimated_timing(bpm: f64, meter: &str) -> Timing {
        serde_json::from_value(serde_json::json!({
            "tempo_bpm": { "estimated": { "value": bpm, "confidence": 0.9 } },
            "tempo_alternatives": { "half": bpm / 2.0, "double": bpm * 2.0 },
            "beat_period_frames": 22_050.0 * 60.0 / bpm,
            "meter": { "estimated": { "value": meter, "confidence": 0.3 } },
            "downbeat_frame": { "estimated": { "value": 11_025, "confidence": 0.75 } },
        }))
        .expect("a timing")
    }

    #[test]
    fn an_estimated_tempo_gives_its_half_and_double() {
        let lines = timing(&estimated_timing(92.0, "4/4"), 22_050);
        assert_eq!(
            lines,
            [
                "tempo: estimated 92 bpm (half 46, double 184), confidence 0.90",
                "meter: estimated 4/4, confidence 0.30, weak",
                "downbeat: estimated at 0.500 s, confidence 0.75",
            ]
        );
    }

    #[test]
    fn a_half_that_is_not_whole_has_one_decimal() {
        let lines = timing(&estimated_timing(93.0, "4/4"), 22_050);
        assert!(
            lines[0].starts_with("tempo: estimated 93 bpm (half 46.5, double 186)"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_compound_meter_gives_its_felt_beat() {
        let lines = timing(&estimated_timing(135.0, "6/8"), 22_050);
        assert!(
            lines[0].starts_with("tempo: estimated 135 bpm (dotted quarter = 90) (half 67.5"),
            "{lines:?}"
        );
        assert_eq!(
            felt_beat(sig("12/16"), 120.0).as_deref(),
            Some("dotted eighth = 160")
        );
        assert_eq!(felt_beat(sig("6/4"), 120.0), None);
        assert_eq!(felt_beat(sig("3/8"), 120.0), None);
    }

    #[test]
    fn given_values_are_not_stated() {
        let timing: Timing = serde_json::from_value(serde_json::json!({
            "tempo_bpm": { "given": 92.5 },
            "beat_period_frames": 14_302.7,
            "meter": { "given": "6/8" },
            "downbeat_frame": { "given": 0 },
        }))
        .expect("a timing");
        assert!(super::timing(&timing, 22_050).is_empty());
    }

    #[test]
    fn rest_bars_are_counted() {
        let four = sig("4/4");
        assert_eq!(lead_in(LeadIn::None, 0, four, 480), "lead-in: none");
        assert_eq!(
            lead_in(LeadIn::Pickup(sig("1/4")), 480, four, 480),
            "lead-in: a pickup of 1/4 before the downbeat"
        );
        assert_eq!(
            lead_in(LeadIn::RestBar, 1920, four, 480),
            "lead-in: 1 bar of rests before the downbeat"
        );
        assert_eq!(
            lead_in(LeadIn::RestBar, 2 * 1440, sig("3/4"), 480),
            "lead-in: 2 bars of rests before the downbeat"
        );
    }
}
