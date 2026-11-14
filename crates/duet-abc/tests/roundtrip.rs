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
            "repeat played more than twice (|::, ::|, or a third ending)",
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

/// Reads `abc`, writes it, reads that back: the same song, and the written form canonical.
fn round_trips(abc: &str) -> String {
    let song = read(abc).unwrap_or_else(|e| panic!("reading: {e}\n{abc}"));
    let written =
        write(&song, &WriteOptions::default()).unwrap_or_else(|e| panic!("writing: {e}\n{abc}"));
    let again = read(&written).unwrap_or_else(|e| panic!("re-reading: {e}\n{written}"));
    assert_eq!(
        again, song,
        "the song changed through ABC:\n{abc}\n--- written\n{written}"
    );
    assert_eq!(
        write(&again, &WriteOptions::default()).expect("rewrite"),
        written
    );
    written
}

#[test]
fn the_writer_keeps_spelling_and_changes_that_reviews_found_it_losing() {
    let t = |key: &str, body: &str| format!("X:1\nT:t\nM:4/4\nL:1/8\nK:{key}\n{body}\n");
    // A natural on E, then MIDI F, in F# major: F, not the refused ^E.
    let w = round_trips(&t("F#", "=E2 =F2 F4 |]"));
    assert!(!w.contains("^E"), "{w}");
    // The same for B and Cb in Gb major.
    let w = round_trips(&t("Gb", "=C2 =B2 B4 |]"));
    assert!(!w.contains("_c"), "{w}");
    // A# in F major stays A#, not the key's Bb.
    let w = round_trips(&t("F", "^A2 B2 c4 |]"));
    assert!(w.contains("^A2"), "{w}");
    // A key or tempo change inside a triplet is kept.
    round_trips(&t("C", "(3C[K:G]FG B6 |]"));
    round_trips(&t("C", "(3C[Q:1/4=90]FG B6 |]"));
    // Triplets of unequal notes are written as (3:2:n.
    let w = round_trips(&t("C", "(3:2:2C2D B6 |]"));
    assert!(w.contains("(3:2:2C2D"), "{w}");
    round_trips(&t("C", "(3:2:2c4d2 z4 |]"));
    // Changes on the song's end, after its last bar line.
    round_trips(&t("C", "C8 |] [M:3/4][K:G][Q:1/4=90]"));
}

#[test]
fn the_writer_keeps_what_notefmt_holds_that_reviews_found_it_losing() {
    let through = |text: &str| {
        let mut song = Song::from_text(text).expect("song");
        song.end = Some(song.stated_end().expect("end"));
        let written = write(&song, &WriteOptions::default()).expect("write");
        let again = read(&written).unwrap_or_else(|e| panic!("{e}\n{written}"));
        (song, again, written)
    };
    // A first bar of 1/4 before 6/8 is no pickup of 6/8 (that would read back as 2/8).
    let (song, again, w) = through(
        "time 1/4\ntime 6/8 at 2.1.0\nkey C major\n\n[track \"t\" ch1]\n1.1.0 C4 0.5 95\n2.1.0 D4 3 95\n",
    );
    assert_eq!(again, song, "{w}");
    // A meter, key and tempo change on the end of a one-bar song.
    let (song, again, w) = through(
        "time 3/4\ntime 4/4 at 2.1.0\nkey C major\nkey G major at 2.1.0\ntempo 90 at 2.1.0\n\n\
         [track \"t\" ch1]\n1.1.0 C4 3 95\n",
    );
    assert_eq!(again, song, "{w}");
    // A % in a track name is a literal percent, not a comment.
    let (song, again, w) = through(
        "key C major\n\n[track \"Lead\" ch2]\n1.1.0 C4 4 95\n[track \"50% mix\" ch1]\n1.1.0 E4 4 95\n",
    );
    assert_eq!(again, song, "{w}");
}

#[test]
fn repeats_unfold_as_abc2midi_plays_them_or_are_refused_by_name() {
    let notes = |body: &str| -> Vec<String> {
        let song = read(&format!("X:1\nT:t\nM:4/4\nL:1/4\nK:C\n{body}\n"))
            .unwrap_or_else(|e| panic!("{e}"));
        song.tracks[0]
            .notes
            .iter()
            .map(|n| n.pitch.to_string())
            .collect()
    };
    // A double bar is a bar line like any other inside a repeat, with a start or without.
    let twice = ["C4", "D4", "E4", "F4", "G4", "A4", "B4", "C5"].repeat(2);
    assert_eq!(notes("|: C D E F || G A B c :|"), twice);
    assert_eq!(notes("C D E F || G A B c :|"), twice);
    // A third ending, or a :| that ends the second, asks for a third pass: refused.
    for body in [
        "|: C D E F |1 G A B c :|2 A B c d :|3 B c d e |]",
        "|: C D E F |1,3 G A B c :|2 A B c d :|]",
    ] {
        let abc = format!("X:1\nT:t\nM:4/4\nL:1/4\nK:C\n{body}\n");
        assert!(
            matches!(read(&abc), Err(Error::Unsupported { construct, .. }) if construct.starts_with("repeat played more than twice")),
            "{body}: {:?}",
            read(&abc)
        );
    }
    // |1,3 in a repeat played twice is the first ending.
    assert_eq!(
        notes("|: C D E F |1,3 G A B c :|2 A B c d |]"),
        [
            "C4", "D4", "E4", "F4", "G4", "A4", "B4", "C5", "C4", "D4", "E4", "F4", "A4", "B4",
            "C5", "D5"
        ]
    );
}

#[test]
fn a_repeat_with_no_start_goes_back_past_a_pickup_where_the_standard_does_not() {
    // Where we part from the standard, by the design's rule (§3.9): the standard (§4.8) and
    // abc2midi go back to the start of the tune, which replays the pickup; abc2midi warns
    // of the short bar that makes, so the oracle cannot record it. A song cannot hold a
    // short bar in its middle, so we go back to the first full bar.
    let song = read("X:1\nT:t\nM:4/4\nL:1/4\nK:C\nG | C D E F :| G4 |]\n").expect("read");
    let pitches: Vec<String> = song.tracks[0]
        .notes
        .iter()
        .map(|n| n.pitch.to_string())
        .collect();
    assert_eq!(
        pitches,
        ["G4", "C4", "D4", "E4", "F4", "C4", "D4", "E4", "F4", "G4"]
    );
}

#[test]
fn reserved_characters_are_listed_not_refused() {
    let notices = duet_abc::check("X:1\nT:t\nM:4/4\nL:1/8\nK:C\nC2 # D2 * E4 |]\n").expect("read");
    assert_eq!(
        notices
            .iter()
            .filter(|n| n.construct == "reserved character")
            .count(),
        2
    );
}

#[test]
fn a_tiny_file_cannot_make_the_reader_or_writer_build_without_limit() {
    // Each is refused before anything is built: an ending range, a bar rest, a long song.
    for body in [
        "[1-4294967295 C8 |]",
        "|1-99999999999999999999 C8 |]",
        "Z4294967295 |]",
    ] {
        let abc = format!("X:1\nT:t\nM:4/4\nL:1/8\nK:C\n{body}\n");
        assert!(
            matches!(read(&abc), Err(Error::Parse { line: 6, .. })),
            "{body}: {:?}",
            read(&abc)
        );
    }
    let far =
        Song::from_text("end 200001.1.0\n\n[track \"P\" ch1]\n1.1.0 C4 1 95\n").expect("song");
    assert!(matches!(
        write(&far, &WriteOptions::default()),
        Err(Error::Unwritable { .. })
    ));
}

#[test]
fn the_file_header_is_read_once_and_its_notices_listed_once() {
    let abc = "C:someone\nN:a note\nM:3/4\n\nX:1\nT:a\nK:C\nC6 |]\n\nX:2\nT:b\nK:G\nG6 |]\n";
    let book = Book::parse(abc).expect("parse");
    // Its fields apply to every tune: both are in 3/4.
    for tune in book.tunes() {
        let reading = tune.to_song(&ReadOptions::default()).expect("read");
        assert_eq!(reading.song.time_sig.to_string(), "3/4");
        // A tune read on its own lists the header's notices, which apply to it.
        assert!(
            reading.notices.iter().any(|n| n.line == 1),
            "{:?}",
            reading.notices
        );
    }
    // Reading the whole file lists them once.
    let header_notices = duet_abc::check(abc)
        .expect("check")
        .into_iter()
        .filter(|n| n.line <= 2)
        .count();
    assert_eq!(header_notices, 2);

    // A refusal in the file header is every tune's, with the header's line.
    let refused = Book::parse("U: T = !trill!\n\nX:1\nT:a\nM:3/4\nK:C\nC6 |]\n").expect("parse");
    assert!(matches!(
        refused.tunes()[0].to_song(&ReadOptions::default()),
        Err(Error::Unsupported { line: 1, .. })
    ));
}

#[test]
fn a_file_header_holds_only_the_fields_the_standard_allows_there() {
    // A tune's own fields are refused in a file header, naming the line, so no tune copies
    // a header title or voice into its song.
    for field in [
        "T:a title",
        "V:1 name=\"x\"",
        "K:G",
        "Q:1/4=90",
        "P:AB",
        "w:la",
    ] {
        let abc = format!("{field}\nM:4/4\n\nX:1\nT:t\nK:C\nC8 |]\n");
        let book = Book::parse(&abc).expect("parse");
        assert!(
            matches!(
                book.tunes()[0].to_song(&ReadOptions::default()),
                Err(Error::Parse { line: 1, .. })
            ),
            "{field}"
        );
    }
    // The fields it allows still set every tune's defaults.
    let abc = "C:someone\nM:6/8\nL:1/8\n\nX:1\nT:t\nK:C\nC6 |]\n";
    let song = read(abc).expect("read");
    assert_eq!(song.time_sig.to_string(), "6/8");
}
