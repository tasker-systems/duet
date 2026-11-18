//! Notes in a key of six accidentals, or with a double sharp for a leading tone, are spelled as
//! the key spells them, in the part, its notes view and the ABC duet writes: Cb under E-flat
//! minor's six flats, E# under F-sharp major's six sharps, F## leading to G-sharp minor, and the
//! E-flat minor of "Wayfaring Stranger" (`crates/duet-capture/corpus`, CC BY 4.0) in flats under a
//! six-flat signature.

use std::path::Path;

use duet_core::Part;
use duet_e2e::{duet, ok, root, s, scratch, view_text};
use duet_schemas::Document;

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The pitches of a notes view, in order.
fn pitches(view: &str) -> Vec<String> {
    view.lines()
        .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
        .map(|l| l.split_whitespace().nth(1).expect("a pitch").to_owned())
        .collect()
}

/// The tune's body: the lines after `K:`.
fn body(abc: &str) -> String {
    abc.split_once("\nK:")
        .and_then(|(_, rest)| rest.split_once('\n'))
        .map(|(_, body)| body.to_owned())
        .expect("a K: line")
}

#[test]
fn six_accidental_scales_read_from_midi_are_spelled_by_the_key_in_the_view_and_the_abc() {
    let dir = scratch("six-accidentals");
    for (name, key, abc_key, scale, spelled, abc) in [
        (
            "eb-minor",
            "Eb minor",
            "K:Ebm",
            // Eb natural minor, then the dorian C, a natural under the signature.
            "1.1.0 Eb4 1 80\n1.2.0 F4 1 80\n1.3.0 Gb4 1 80\n1.4.0 Ab4 1 80\n\
             2.1.0 Bb4 1 80\n2.2.0 B4 1 80\n2.3.0 Db5 1 80\n2.4.0 Eb5 1 80\n3.1.0 C5 4 80\n",
            "Eb4 F4 Gb4 Ab4 Bb4 Cb5 Db5 Eb5 C5",
            "V:1 name=\"P\"\n!mf!E2 F2 G2 A2 | B2 c2 d2 e2 | =c8 |]\n",
        ),
        (
            "f-sharp-major",
            "F# major",
            "K:F#",
            "1.1.0 F#4 1 80\n1.2.0 G#4 1 80\n1.3.0 A#4 1 80\n1.4.0 B4 1 80\n\
             2.1.0 C#5 1 80\n2.2.0 D#5 1 80\n2.3.0 F5 1 80\n2.4.0 F#5 1 80\n",
            "F#4 G#4 A#4 B4 C#5 D#5 E#5 F#5",
            "V:1 name=\"P\"\n!mf!F2 G2 A2 B2 | c2 d2 e2 f2 | z8 |]\n",
        ),
        (
            // G# minor, which the key estimate can give: its raised leading tone is F##.
            "g-sharp-minor",
            "G# minor",
            "K:G#m",
            "1.1.0 G#4 1 80\n1.2.0 A#4 1 80\n1.3.0 B4 1 80\n1.4.0 C#5 1 80\n\
             2.1.0 D#5 1 80\n2.2.0 E5 1 80\n2.3.0 G5 1 80\n2.4.0 G#5 1 80\n",
            "G#4 A#4 B4 C#5 D#5 E5 F##5 G#5",
            "V:1 name=\"P\"\n!mf!G2 A2 B2 c2 | d2 e2 ^^f2 g2 | z8 |]\n",
        ),
    ] {
        // The scale as MIDI holds it: note numbers and a key signature, no spelling.
        let notes = dir.join(format!("{name}.notes"));
        std::fs::write(
            &notes,
            format!("key {key}\nend 4.1.0\n\n[track \"P\" ch1]\n{scale}"),
        )
        .expect("write");
        let typed = dir.join(format!("{name}.typed.duet"));
        ok(
            &duet(
                &dir,
                &["apply", s(&notes), "--as", "notes", "-o", s(&typed)],
            ),
            "apply",
        );
        let mid = dir.join(format!("{name}.mid"));
        ok(&duet(&dir, &["convert", s(&typed), s(&mid)]), "to MIDI");
        let doc = dir.join(format!("{name}.duet"));
        ok(&duet(&dir, &["convert", s(&mid), s(&doc)]), "from MIDI");

        let view = view_text(&dir, &doc, "notes", &[]);
        assert_eq!(pitches(&view).join(" "), spelled, "{name}:\n{view}");
        let part: Part = Document::read(&read(&doc)).expect("a part").body;
        assert_eq!(
            part.tracks[0].notes[5].pitch.to_string(),
            spelled.split(' ').nth(5).unwrap(),
            "{name}: the part document spells as the view does"
        );

        let out = dir.join(format!("{name}.abc"));
        ok(&duet(&dir, &["convert", s(&doc), s(&out)]), "to ABC");
        let written = read(&out);
        assert!(written.contains(&format!("\n{abc_key}\n")), "{written}");
        assert_eq!(body(&written), abc, "{name}:\n{written}");

        // The ABC reads back to the same spelling, and writes back the same text.
        let again = dir.join(format!("{name}.again.duet"));
        ok(&duet(&dir, &["convert", s(&out), s(&again)]), "from ABC");
        let view = view_text(&dir, &again, "notes", &[]);
        assert_eq!(
            pitches(&view).join(" "),
            spelled,
            "{name} from ABC:\n{view}"
        );
        let rewritten = dir.join(format!("{name}.again.abc"));
        ok(
            &duet(&dir, &["convert", s(&again), s(&rewritten)]),
            "to ABC",
        );
        assert_eq!(
            body(&read(&rewritten)),
            abc,
            "{name}: the ABC is not stable"
        );
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn wayfaring_stranger_is_captured_in_flats_under_a_six_flat_signature() {
    let dir = scratch("wayfaring-spelling");
    let take = dir.join("wayfaring-stranger.flac");
    std::fs::copy(
        root().join("crates/duet-capture/corpus/wayfaring-stranger.flac"),
        &take,
    )
    .expect("copy the take");
    ok(&duet(&dir, &["capture", "melody", s(&take)]), "capture");

    let part: Part = Document::read(&read(&dir.join("wayfaring-stranger.duet")))
        .expect("a part")
        .body;
    let key = part.key.expect("a key");
    assert_eq!(key.to_string(), "Eb minor");
    for note in &part.tracks[0].notes {
        assert_eq!(
            note.pitch,
            key.spell(note.pitch),
            "{} is not spelled as Eb minor spells it",
            note.pitch
        );
        assert!(!note.pitch.to_string().contains('#'), "{}", note.pitch);
    }

    let abc = read(&dir.join("wayfaring-stranger.abc"));
    assert!(abc.contains("\nK:Ebm\n"), "{abc}");
    let notes = body(&abc);
    assert!(!notes.contains('^'), "a sharp under six flats:\n{abc}");
    // The C the tune sings is C natural, not the signature's Cb: it is marked.
    assert!(notes.contains("=C"), "the C natural is not marked:\n{abc}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
