//! W4 of the reader-disagreement design: the classification on readings built by hand. Each
//! kind with its edge, a boundary-only stretch as no place, each place stated once, and the
//! comparison's symmetry.

use super::*;

const G3: u8 = 55;
const G_SHARP_3: u8 = 56;
const A3: u8 = 57;
const C4: u8 = 60;

/// A note on `key` from frame `start` to `end`, held from its start, on its key.
fn n(key: u8, start: u64, end: u64) -> ReadNote {
    ReadNote {
        start,
        end,
        held_from: start,
        key,
        centre: 0.0,
        between_keys: false,
        confidence: 1.0,
        strength: 1.0,
        residual: None,
    }
}

/// A stated note: its kind, side, index and the other reading's notes it sounds with.
type Stated = (PlaceKind, Side, usize, Vec<usize>);

/// The places' stated notes, without the stretches.
fn stated(d: &Disagreements) -> Vec<Vec<Stated>> {
    d.places
        .iter()
        .map(|p| {
            p.found
                .iter()
                .map(|f| (f.kind, f.side, f.note, f.with.clone()))
                .collect()
        })
        .collect()
}

#[test]
fn notes_that_only_meet_have_a_seam_between_them_and_overlapping_ones_do_not() {
    let a = [n(G3, 0, 100), n(A3, 100, 200), n(C4, 300, 400)];
    let b = [n(G3, 0, 120), n(A3, 120, 200)];
    let stretches = seams(&a, &b);
    let spans: Vec<(u64, u64, Vec<usize>, Vec<usize>)> = stretches
        .iter()
        .map(|s| (s.start, s.end, s.a.clone(), s.b.clone()))
        .collect();
    // a's G3 and b's G3 overlap b's A3 through 100..120, so the first two notes of each are one
    // stretch; the C4 after silence is its own.
    assert_eq!(
        spans,
        [
            (0, 200, vec![0, 1], vec![0, 1]),
            (300, 400, vec![2], vec![]),
        ]
    );
    let met = seams(&[n(G3, 0, 100)], &[n(A3, 100, 200)]);
    assert_eq!(met.len(), 2, "{met:?}");
}

#[test]
fn two_notes_sound_together_from_half_the_shorter_and_not_just_under() {
    // The shorter is 40 frames: an overlap of 20 is half, of 19 just under.
    let long = n(G3, 0, 100);
    assert!(sound_together(&long, &n(A3, 80, 120)));
    assert!(sound_together(&n(A3, 80, 120), &long));
    assert!(!sound_together(&long, &n(A3, 81, 121)));
    assert!(
        !sound_together(&long, &n(A3, 100, 140)),
        "meeting is no overlap"
    );
}

#[test]
fn a_note_read_as_several_is_a_split_named_once_by_the_split_note() {
    // A held G3 with a turn: the first reader holds it, the second reads G3, A3, G3.
    let a = [n(G3, 0, 1000)];
    let b = [n(G3, 0, 450), n(A3, 450, 550), n(G3, 550, 1000)];
    let d = disagreements(&a, &b);
    assert_eq!(
        stated(&d),
        [vec![(PlaceKind::Split, Side::A, 0, vec![0, 1, 2])]],
        "the pieces are not listed again"
    );
    assert_eq!(d.places[0].kind(), PlaceKind::Split);
    assert!(d.agreed.is_empty());
}

#[test]
fn a_split_needs_each_piece_to_sound_with_the_split_note_alone() {
    // b's A3 sounds with a's G3 and with a's A3: two chains overlap, which is no split.
    let a = [n(G3, 0, 100), n(A3, 100, 200)];
    let b = [n(G3, 0, 50), n(A3, 50, 150), n(G3, 150, 200)];
    let d = disagreements(&a, &b);
    let kinds: Vec<PlaceKind> = d.places[0].found.iter().map(|f| f.kind).collect();
    assert!(
        kinds.iter().all(|&k| k == PlaceKind::Other),
        "two chains are other, every note listed: {:?}",
        d.places[0].found
    );
    assert_eq!(d.places[0].found.len(), 5, "nothing is dropped");
}

#[test]
fn one_note_on_another_key_is_a_key_pair_named_once_by_the_first_reading() {
    let a = [n(G3, 0, 100)];
    let b = [n(G_SHARP_3, 5, 100)];
    let d = disagreements(&a, &b);
    assert_eq!(stated(&d), [vec![(PlaceKind::Key, Side::A, 0, vec![0])]]);
}

#[test]
fn a_note_with_no_note_of_the_other_reading_is_only() {
    let a = [n(G3, 0, 100), n(C4, 200, 300)];
    let b = [n(G3, 0, 100)];
    let d = disagreements(&a, &b);
    assert_eq!(stated(&d), [vec![(PlaceKind::Only, Side::A, 1, vec![])]]);
    assert_eq!(d.agreed.len(), 1);
}

#[test]
fn a_note_overlapping_just_under_half_is_only_on_both_sides() {
    // Within one stretch: the overlap is 19 of the shorter's 40 frames.
    let a = [n(G3, 0, 100)];
    let b = [n(A3, 81, 121)];
    let d = disagreements(&a, &b);
    assert_eq!(
        stated(&d),
        [vec![
            (PlaceKind::Only, Side::A, 0, vec![]),
            (PlaceKind::Only, Side::B, 0, vec![])
        ]]
    );
    // And from exactly half, they sound together: a key pair.
    let d = disagreements(&a, &[n(A3, 80, 120)]);
    assert_eq!(stated(&d), [vec![(PlaceKind::Key, Side::A, 0, vec![0])]]);
}

#[test]
fn a_stretch_of_the_same_keys_is_no_place_and_keeps_its_largest_boundary_difference() {
    // The second reading starts each note earlier, by up to 15 frames.
    let a = [n(G3, 10, 100), n(A3, 100, 200)];
    let b = [n(G3, 0, 85), n(A3, 85, 203)];
    let d = disagreements(&a, &b);
    assert!(d.places.is_empty(), "{:?}", d.places);
    assert_eq!(d.agreed.len(), 1);
    assert_eq!(d.agreed[0].boundary, 15);
    assert_eq!(
        (d.agreed[0].stretch.start, d.agreed[0].stretch.end),
        (0, 203)
    );
}

#[test]
fn swapping_the_readings_swaps_the_names_and_changes_nothing_else() {
    let a = [
        n(G3, 0, 1000),
        n(G3, 1200, 1300),
        n(A3, 1500, 1600),
        n(C4, 1800, 1900),
    ];
    let b = [
        n(G3, 0, 450),
        n(A3, 450, 550),
        n(G3, 550, 1000),
        n(G_SHARP_3, 1200, 1290),
        n(A3, 1490, 1600),
    ];
    let ab = disagreements(&a, &b);
    let ba = disagreements(&b, &a);
    assert_eq!(ab.places.len(), 3, "{:?}", ab.places);
    assert_eq!(ab.places.len(), ba.places.len());
    for (x, y) in ab.places.iter().zip(&ba.places) {
        assert_eq!(
            (x.stretch.start, x.stretch.end),
            (y.stretch.start, y.stretch.end)
        );
        assert_eq!((&x.stretch.a, &x.stretch.b), (&y.stretch.b, &y.stretch.a));
        let swapped: Vec<Stated> = y
            .found
            .iter()
            .map(|f| (f.kind, f.side.other(), f.note, f.with.clone()))
            .collect();
        let own: Vec<Stated> = x
            .found
            .iter()
            .map(|f| (f.kind, f.side, f.note, f.with.clone()))
            .collect();
        if x.kind() == PlaceKind::Key {
            // A key pair is named by the first reading's note: the other note, swapped.
            let [(_, side, note, with)] = own.as_slice() else {
                panic!("{own:?}")
            };
            assert_eq!(
                swapped,
                [(PlaceKind::Key, side.other(), with[0], vec![*note])],
                "{x:?}"
            );
        } else {
            assert_eq!(swapped, own);
        }
    }
    assert_eq!(
        ab.places.iter().map(Place::kind).collect::<Vec<_>>(),
        [PlaceKind::Split, PlaceKind::Key, PlaceKind::Only]
    );
    assert_eq!(ab.agreed.len(), 1, "the A3 that starts 10 frames apart");
    assert_eq!(ba.agreed, {
        let mut agreed = ab.agreed.clone();
        for x in &mut agreed {
            std::mem::swap(&mut x.stretch.a, &mut x.stretch.b);
        }
        agreed
    });
}

#[test]
fn one_long_note_against_many_short_ones_is_one_split_found_in_bounded_work() {
    // 20 000 pieces under one note: quadratic partner scans would take minutes.
    let pieces: Vec<ReadNote> = (0..20_000)
        .map(|k| n(if k % 2 == 0 { G3 } else { A3 }, k * 10, k * 10 + 10))
        .collect();
    let a = [n(G3, 0, 200_000)];
    let started = std::time::Instant::now();
    let d = disagreements(&a, &pieces);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    assert_eq!(d.places.len(), 1);
    assert_eq!(d.places[0].found.len(), 1);
    assert_eq!(d.places[0].kind(), PlaceKind::Split);
    assert_eq!(d.places[0].found[0].with.len(), 20_000);
}

#[test]
fn notes_out_of_order_or_overlapping_are_not_in_order() {
    assert!(in_order(&[n(G3, 0, 100), n(A3, 100, 200)]));
    assert!(!in_order(&[n(G3, 0, 100), n(A3, 99, 200)]));
    assert!(!in_order(&[n(A3, 100, 200), n(G3, 0, 100)]));
    assert!(!in_order(&[n(G3, 100, 100)]));
}
