//! Where two readings of one take disagree: the stretches between their seams, and the notes in
//! each stretch that the readings do not read alike, found by kind.
//!
//! This is the design `docs/superpowers/specs/2026-11-20-reader-disagreement-design.md`, as
//! ruled (2026-11-20):
//!
//! - **A seam** is a frame where no note of either reading is sounding across it (the variants
//!   design's §3, ruled). Between two seams lies a [`Stretch`]: every note of each reading inside
//!   it starts and ends there, so a stretch holds a whole disagreement and nothing else.
//! - **A stretch is a place when its keys differ**: when its two readings do not hold the same
//!   keys in the same order. A stretch that holds the same keys is no place, whatever its
//!   boundaries ([`Agreed`]); its largest boundary difference is kept, to be stated as a count.
//! - **Two notes sound together** when at least half of the shorter overlaps the longer
//!   ([`sound_together`]). Half is a share, not a time, so it scales with the notes, and there is
//!   no tolerance to set.
//! - **Each note of a place is of one [`Kind`]**, and each place is stated once: a split is named
//!   by the note that is split, and its pieces are not listed again; a key pair is named once, by
//!   the first reading's note.
//!
//! The comparison is symmetric: swapping the readings finds the same places, with each
//! [`Found`]'s side swapped and a key pair named by the other note.

use crate::ReadNote;

/// Which of the two readings a note is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The first reading.
    A,
    /// The second.
    B,
}

impl Side {
    /// The other reading.
    #[must_use]
    pub const fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }
}

/// The frames between two seams of two readings, and the notes of each inside them, by index
/// into its reading's notes, in the order they start.
///
/// Its `start` and `end` are seams, and so is every frame outside the stretches; no frame
/// strictly between them is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stretch {
    /// The first note's start: a seam.
    pub start: u64,
    /// The last note's end: a seam.
    pub end: u64,
    /// The first reading's notes in the stretch.
    pub a: Vec<usize>,
    /// The second reading's.
    pub b: Vec<usize>,
}

impl Stretch {
    /// The notes of `side` in the stretch.
    #[must_use]
    pub fn notes(&self, side: Side) -> &[usize] {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }
}

/// What a note in a place is, against the other reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// It sounds together with two or more of the other reading's notes, and each of those with
    /// no other note of its own reading: the other reader read it as several notes, a candidate
    /// ornament or a slide read as notes.
    Split,
    /// It sounds together with exactly one of the other's notes, on another key, and that note
    /// with only it: the readers disagree on which key was sung.
    Key,
    /// It sounds together with none of the other's notes: one reader reads a note where the
    /// other reads none.
    Only,
    /// Anything else, such as two overlapping chains: listed, so nothing is dropped.
    Other,
}

impl Kind {
    /// The kind as a word: the first word of a place's label.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Kind::Split => "split",
            Kind::Key => "key",
            Kind::Only => "only",
            Kind::Other => "other",
        }
    }
}

/// A note of a place, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub kind: Kind,
    /// The reading the note is of.
    pub side: Side,
    /// The note, by index into its reading's notes.
    pub note: usize,
    /// The other reading's notes it sounds together with, in the order they start: a split's
    /// pieces, a key pair's other note, none for `only`.
    pub with: Vec<usize>,
}

/// A stretch whose readings hold different keys, and what each of its notes is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub stretch: Stretch,
    /// Each note of the place that is stated, in the order they start: never a split's piece,
    /// a key pair's second note, or a note both readings read alike.
    pub found: Vec<Found>,
}

impl Place {
    /// The kind of the place's first stated note: what the place is named by.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.found.first().map_or(Kind::Other, |f| f.kind)
    }
}

/// A stretch whose readings hold the same keys in the same order: no place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agreed {
    pub stretch: Stretch,
    /// The largest difference, in frames, between a boundary of one reading's note and the same
    /// boundary of the other's note in the same place in order.
    pub boundary: u64,
}

/// Every stretch of two readings, as places and agreements, in the order they start.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Disagreements {
    pub places: Vec<Place>,
    pub agreed: Vec<Agreed>,
}

/// Whether two notes sound together: at least half of the shorter overlaps the longer.
#[must_use]
pub fn sound_together(x: &ReadNote, y: &ReadNote) -> bool {
    let overlap = x.end.min(y.end).saturating_sub(x.start.max(y.start));
    let shorter = (x.end - x.start).min(y.end - y.start);
    overlap > 0 && 2 * overlap >= shorter
}

/// The stretches between the seams of readings `a` and `b`, in the order they start. Two notes
/// that only meet, one ending on the frame the next starts on, have a seam between them.
#[must_use]
pub fn seams(a: &[ReadNote], b: &[ReadNote]) -> Vec<Stretch> {
    let mut all: Vec<(u64, u64, Side, usize)> = a
        .iter()
        .enumerate()
        .map(|(i, n)| (n.start, n.end, Side::A, i))
        .chain(
            b.iter()
                .enumerate()
                .map(|(i, n)| (n.start, n.end, Side::B, i)),
        )
        .collect();
    all.sort_by_key(|&(start, end, side, i)| (start, end, side == Side::B, i));
    let mut out: Vec<Stretch> = Vec::new();
    for (start, end, side, i) in all {
        match out.last_mut() {
            Some(open) if start < open.end => {
                open.end = open.end.max(end);
                push(open, side, i);
            }
            _ => {
                let mut stretch = Stretch {
                    start,
                    end,
                    a: Vec::new(),
                    b: Vec::new(),
                };
                push(&mut stretch, side, i);
                out.push(stretch);
            }
        }
    }
    out
}

fn push(stretch: &mut Stretch, side: Side, i: usize) {
    match side {
        Side::A => stretch.a.push(i),
        Side::B => stretch.b.push(i),
    }
}

/// Where readings `a` and `b` of one take, on one tuning, disagree: every stretch between their
/// seams, as a [`Place`] when its keys differ and as [`Agreed`] when they do not.
#[must_use]
pub fn disagreements(a: &[ReadNote], b: &[ReadNote]) -> Disagreements {
    let mut out = Disagreements::default();
    for stretch in seams(a, b) {
        let keys = |side: Side, notes: &[ReadNote]| -> Vec<u8> {
            stretch.notes(side).iter().map(|&i| notes[i].key).collect()
        };
        if keys(Side::A, a) == keys(Side::B, b) {
            let boundary = stretch
                .a
                .iter()
                .zip(&stretch.b)
                .map(|(&i, &j)| {
                    a[i].start
                        .abs_diff(b[j].start)
                        .max(a[i].end.abs_diff(b[j].end))
                })
                .max()
                .unwrap_or(0);
            out.agreed.push(Agreed { stretch, boundary });
        } else {
            let found = classify(&stretch, a, b);
            out.places.push(Place { stretch, found });
        }
    }
    out
}

/// What each note of a place's stretch is, stated once each.
fn classify(stretch: &Stretch, a: &[ReadNote], b: &[ReadNote]) -> Vec<Found> {
    let notes = |side: Side| match side {
        Side::A => a,
        Side::B => b,
    };
    // Each note's partners: the other reading's notes in the stretch it sounds together with.
    let partners = |side: Side, i: usize| -> Vec<usize> {
        let note = &notes(side)[i];
        stretch
            .notes(side.other())
            .iter()
            .copied()
            .filter(|&j| sound_together(note, &notes(side.other())[j]))
            .collect()
    };
    let only_with = |side: Side, i: usize, j: usize| partners(side.other(), j) == [i];
    let is_split = |side: Side, i: usize, with: &[usize]| {
        with.len() >= 2 && with.iter().all(|&j| only_with(side, i, j))
    };

    let mut found: Vec<Found> = Vec::new();
    for side in [Side::A, Side::B] {
        for &i in stretch.notes(side) {
            let with = partners(side, i);
            let kind = match with.as_slice() {
                [] => Some(Kind::Only),
                _ if is_split(side, i, &with) => Some(Kind::Split),
                &[j] if only_with(side, i, j) => {
                    let differ = notes(side)[i].key != notes(side.other())[j].key;
                    // A key pair is named once, by the first reading's note.
                    (differ && side == Side::A).then_some(Kind::Key)
                }
                // A piece of the other reading's split is stated with it, not again.
                &[j] if is_split(side.other(), j, &partners(side.other(), j)) => None,
                _ => Some(Kind::Other),
            };
            if let Some(kind) = kind {
                found.push(Found {
                    kind,
                    side,
                    note: i,
                    with,
                });
            }
        }
    }
    // Keys that differ always leave a note unpaired, but should none be stated, every note of
    // the stretch is, so nothing is dropped.
    if found.is_empty() {
        for side in [Side::A, Side::B] {
            for &i in stretch.notes(side) {
                found.push(Found {
                    kind: Kind::Other,
                    side,
                    note: i,
                    with: partners(side, i),
                });
            }
        }
    }
    found.sort_by_key(|f| (notes(f.side)[f.note].start, f.side == Side::B, f.note));
    found
}

#[cfg(test)]
mod tests;
