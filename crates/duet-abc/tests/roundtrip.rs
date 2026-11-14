//! `abc-round-trips-through-notefmt-without-loss-of-score`: every corpus tune comes back
//! from ABC → Song → ABC → Song unchanged, every construct outside the subset is refused
//! by name with its line and column, and a song ABC cannot write as it stands is refused,
//! never quantized.

mod support;

use duet_abc::{Book, Dynamics, Error, REFUSED, ReadOptions, WriteOptions, write};
use duet_core::Song;
use support::{manifest, our_song};

fn read(abc: &str) -> Result<Song, Error> {
    let book = Book::parse(abc)?;
    Ok(book.tunes()[0].to_song(&ReadOptions::default())?.song)
}

#[test]
fn abc_round_trips_through_song() {
    let m = manifest();
    let mut checked = 0;
    for entry in m.tune.iter().filter(|e| e.writer_refuses.is_none()) {
        let song = our_song(entry);
        let written = write(&song, &WriteOptions::default())
            .unwrap_or_else(|e| panic!("{}: writing: {e}", entry.name));
        let again =
            read(&written).unwrap_or_else(|e| panic!("{}: re-reading: {e}\n{written}", entry.name));
        // Every note's position, pitch and length, the key and its mode, the meter, the
        // tempo and their changes, track names, channels and programs.
        assert_eq!(
            again, song,
            "{}: the song changed through ABC:\n{written}",
            entry.name
        );
        // And the writer's form is canonical: what it reads back, it writes the same.
        assert_eq!(
            write(&again, &WriteOptions::default()).expect("rewrite"),
            written,
            "{}: the written ABC is not canonical",
            entry.name
        );
        checked += 1;
    }
    assert!(
        checked >= 25,
        "only {checked} corpus tunes were round-tripped"
    );
}

/// A tune with `header` lines between `X:1` and `K:` (after `T:t`, `M:4/4` and `L:1/8`
/// unless the header gives its own), then `body`.
fn tune(header: &str, key: &str, body: &str) -> String {
    let mut s = String::from("X:1\nT:t\n");
    if !header.contains("M:") {
        s.push_str("M:4/4\n");
    }
    s.push_str("L:1/8\n");
    s.push_str(header);
    s.push_str(key);
    s.push('\n');
    s.push_str(body);
    s
}

const PLAIN: &str = "K:C";

#[test]
fn every_refused_construct_names_itself_and_its_line() {
    // (construct, abc, line, col). The template's K: is line 5, and its body starts on
    // line 6; a header line of the case's own comes before K:, and moves both down one
    // (unless it is the case's M:, which takes the template's M: place).
    let cases: Vec<(&str, String, usize, usize)> = vec![
        ("grace notes", tune("", PLAIN, "C2 {g}D2 E4 |]\n"), 6, 4),
        ("decoration", tune("", PLAIN, "C2 !trill!D2 E4 |]\n"), 6, 4),
        (
            "crescendo / diminuendo",
            tune("", PLAIN, "!crescendo(!C2 D2 E4 |]\n"),
            6,
            1,
        ),
        ("chord symbol", tune("", PLAIN, "\"Am\"C2 D2 E4 |]\n"), 6, 1),
        (
            "lyrics (w:)",
            tune("", PLAIN, "C2 D2 E4 |]\nw:la la la\n"),
            7,
            1,
        ),
        (
            "lyrics (W:)",
            tune("", PLAIN, "C2 D2 E4 |]\nW:la la la\n"),
            7,
            1,
        ),
        ("R:hornpipe", tune("R:hornpipe\n", PLAIN, "C8 |]\n"), 5, 1),
        ("parts (P:)", tune("P:AB\n", PLAIN, "C8 |]\n"), 5, 1),
        (
            "nested repeat",
            tune("", PLAIN, "|: C8 |: D8 :| E8 :|\n"),
            6,
            7,
        ),
        (
            "repeat without a start in a multi-voice tune",
            tune("", PLAIN, "V:1\nC8 :|\nV:2\nC,8 :|\n"),
            7,
            4,
        ),
        (
            "free meter (M:none, or no M:)",
            tune("M:none\n", PLAIN, "C8 |]\n"),
            4,
            1,
        ),
        (
            "Q: unit-length tempo",
            tune("Q:120\n", PLAIN, "C8 |]\n"),
            5,
            1,
        ),
        (
            "%%propagate-accidentals <not|octave>",
            tune("%%propagate-accidentals octave\n", PLAIN, "C8 |]\n"),
            5,
            1,
        ),
        (
            "pickup of a length no meter can hold",
            tune("", PLAIN, "(3::1 C | D8 |]\n"),
            6,
            9,
        ),
        ("K:none", tune("", "K:none", "C8 |]\n"), 5, 1),
        ("K:HP", tune("", "K:HP", "C8 |]\n"), 5, 1),
        (
            "K:<key> <explicit accidentals>",
            tune("", "K:D Phr ^f", "C8 |]\n"),
            5,
            1,
        ),
        (
            "clef, transpose or octave property",
            tune("", "K:C clef=bass", "C8 |]\n"),
            5,
            1,
        ),
        (
            "[K:] inside a multi-voice tune",
            tune("", PLAIN, "V:1\n[K:G] C8 |]\nV:2\nC,8 |]\n"),
            7,
            1,
        ),
        ("macro (m:)", tune("m: ~G2 = GAG\n", PLAIN, "C8 |]\n"), 5, 1),
        (
            "user symbol (U:)",
            tune("U: T = !trill!\n", PLAIN, "C8 |]\n"),
            5,
            1,
        ),
        (
            "%%MIDI <other>",
            tune("", PLAIN, "%%MIDI gchord fzc\nC8 |]\n"),
            6,
            1,
        ),
        (
            "chord of mixed lengths",
            tune("", PLAIN, "[C2E] D2 E4 |]\n"),
            6,
            1,
        ),
        ("tuplet p>9", tune("", PLAIN, "(10CDEFGABcde |]\n"), 6, 1),
        (
            "length that is not a whole tick at 480 ticks per quarter",
            tune("", PLAIN, "D7 C/7 |]\n"),
            6,
            4,
        ),
        (
            "enharmonic spelling <E#/Cb/B#/Fb/^^/__>",
            tune("", PLAIN, "C2 ^E2 E4 |]\n"),
            6,
            4,
        ),
        (
            "Q: text only",
            tune("Q:\"Allegro\"\n", PLAIN, "C8 |]\n"),
            5,
            1,
        ),
        (
            "old chord syntax +CEG+",
            tune("", PLAIN, "+CEG+2 D2 E4 |]\n"),
            6,
            1,
        ),
        (
            "[M:] or [Q:] inside a multi-voice tune",
            tune("", PLAIN, "V:1\nC8 | [M:3/4] C6 |]\nV:2\nC,8 | C,6 |]\n"),
            7,
            6,
        ),
        ("voice overlay (&)", tune("", PLAIN, "C8 & E8 |]\n"), 6, 4),
        (
            "symbol line (s:)",
            tune("", PLAIN, "C2 D2 E4 |]\ns: !f! * *\n"),
            7,
            1,
        ),
        (
            "complex meter (M:(2+3+2)/8)",
            tune("M:(2+3+2)/8\n", PLAIN, "C7 |]\n"),
            4,
            1,
        ),
        (
            "repeat played more than twice (|:: or ::|)",
            tune("", PLAIN, "|:: C8 ::|\n"),
            6,
            1,
        ),
        (
            "%%MIDI program or channel after a voice's first note",
            tune("", PLAIN, "C8 |\n%%MIDI program 5\nD8 |]\n"),
            7,
            1,
        ),
    ];
    // One case per row of the table, so a new row needs a case.
    let named: Vec<&str> = cases.iter().map(|c| c.0).collect();
    for row in REFUSED {
        assert!(
            named.contains(&row.construct),
            "no case for '{}'",
            row.construct
        );
    }
    assert_eq!(cases.len(), REFUSED.len(), "one case per row");

    for (construct, abc, line, col) in cases {
        match read(&abc) {
            Err(Error::Unsupported {
                construct: c,
                line: l,
                col: k,
                ..
            }) => assert_eq!(
                (c, l, k),
                (construct, line, col),
                "the refusal names the construct, its line and its column:\n{abc}"
            ),
            other => panic!("'{construct}' was not refused by name: {other:?}\n{abc}"),
        }
    }

    // A tune with no M: at all is free meter, refused at the K: that ends its header.
    let no_meter = "X:1\nT:t\nL:1/8\nK:C\nC8 |]\n";
    assert!(matches!(
        read(no_meter),
        Err(Error::Unsupported {
            construct: "free meter (M:none, or no M:)",
            line: 4,
            ..
        })
    ));
}

#[test]
fn an_unquantized_song_is_refused_not_quantized() {
    let played = "tempo 96\ntime 4/4\nkey G major\n\n[track \"Voice\" ch1]\n\
                  1.1.10   G4  0.98  95\n1.2.0    A4  1     95\n";
    let song = Song::from_text(played).expect("a played take");
    let before = song.clone();
    match write(&song, &WriteOptions::default()) {
        Err(Error::Unwritable {
            at, track, hint, ..
        }) => {
            assert_eq!(at.to_string(), "1.1.10", "the refusal names where");
            assert_eq!(track.as_deref(), Some("Voice"), "and which track");
            assert!(hint.contains("duet song quantize"), "{hint}");
        }
        other => panic!("an unquantized song was not refused: {other:?}"),
    }
    assert_eq!(song, before, "the song is never changed");

    // Once quantized, explicitly, it writes.
    let quantized = song
        .quantized("1/16".parse().expect("grid"))
        .expect("quantize");
    assert!(write(&quantized, &WriteOptions::default()).is_ok());

    // A velocity that is no dynamic's level is refused by default, naming the note, and
    // snapped only when asked, which the ABC then says.
    let loud = Song::from_text("[track \"P\" ch1]\n1.1.0 C4 4 105\n").expect("song");
    match write(&loud, &WriteOptions::default()) {
        Err(Error::Unwritable { why, .. }) => assert!(why.contains("velocity 105"), "{why}"),
        other => panic!("velocity 105 was not refused: {other:?}"),
    }
    let snapped = write(
        &loud,
        &WriteOptions::default().with_dynamics(Dynamics::Nearest),
    )
    .expect("nearest");
    assert!(
        snapped.contains("% duet: velocities were snapped"),
        "{snapped}"
    );
    assert!(
        snapped.contains("!ff!C8"),
        "105 snaps to ff, 110: {snapped}"
    );

    // Drums are the drum grid's job.
    let drums = Song::from_text("[track \"Kit\" ch10]\n1.1.0 C2 1 95\n").expect("song");
    assert!(matches!(
        write(&drums, &WriteOptions::default()),
        Err(Error::Unwritable { .. })
    ));
}

#[test]
fn notation_only_constructs_are_listed_not_dropped_silently() {
    let abc = "X:1\nT:t\nC:someone\nM:4/4\nL:1/8\nK:C\n(CD EF) \"^up\"G2 y A2 |]\n";
    let notices = duet_abc::check(abc).expect("read");
    let kinds: Vec<(usize, &str)> = notices.iter().map(|n| (n.line, n.construct)).collect();
    for expected in [
        (3, "information field"),
        (7, "slur"),
        (7, "annotation"),
        (7, "spacer (y)"),
    ] {
        assert!(kinds.contains(&expected), "{expected:?} not in {kinds:?}");
    }
}

#[test]
fn a_bar_of_the_wrong_length_is_an_error_naming_the_bar() {
    // abc2midi plays a short bar as written, without a warning; this check is ours.
    let abc = "X:1\nT:t\nM:4/4\nL:1/8\nK:C\nC8 | D6 | E8 |]\n";
    match read(abc) {
        Err(Error::Parse { line, msg, .. }) => {
            assert_eq!(line, 6);
            assert!(
                msg.contains("bar 2") && msg.contains("3/4") && msg.contains("1"),
                "{msg}"
            );
        }
        other => panic!("a short bar was read: {other:?}"),
    }
}
