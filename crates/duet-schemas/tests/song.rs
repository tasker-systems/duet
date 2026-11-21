//! song 1.0: a song document round-trips exactly through its reader and validates against the
//! committed schema, and what the format refuses is refused by name: a meter change off a bar
//! line, a map out of order or not from the start, a place written as the bars do not write it,
//! an unknown part in a placement, two parts with one name or one id. The status moves only as
//! the state machine draws.

use std::path::Path;

use duet_core::Position;
use duet_dsp::TakeDigest;
use duet_schemas::journal::Id;
use duet_schemas::project::{Defaults, Name};
use duet_schemas::song::{
    Composition, KeyAt, MeterAt, Placed, Placement, Section, Status, TakeLink, TempoAt,
    distinct_ids, part_names,
};
use duet_schemas::{AnyDocument, Document, Error, Song, Tool};
use serde_json::{Value, json};

fn read_json(relative: &str) -> Value {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The committed envelope and song schemas, registered by their `$id`, as a validator.
fn validator() -> jsonschema::Validator {
    let envelope = read_json("schemas/envelope/1.0.json");
    let song = read_json("schemas/song/1.0.json");
    let registry = jsonschema::Registry::new()
        .add(envelope["$id"].as_str().unwrap(), &envelope)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&song)
        .unwrap()
}

fn pos(s: &str) -> Position {
    s.parse().unwrap()
}

fn name(s: &str) -> Name {
    s.parse().unwrap()
}

fn id(n: u8) -> Id {
    Id::from_parts(1_795_446_245_000 + u64::from(n), [n; 10])
}

/// A take's digest, one per `n`.
fn digest(n: u8) -> TakeDigest {
    TakeDigest([n; 32])
}

/// A song unlike the committed example in its values: odd floats, 6/8 to 7/8, a modal key, a
/// tick offset, a part placed inside a beat, and a title and intent that need escaping.
fn song() -> Document<Song> {
    let mut composition = Composition::new(0.1 + 0.2, "6/8".parse().unwrap(), None);
    composition.ticks_per_quarter = 960;
    composition
        .tempo
        .push(TempoAt::new(pos("3.4.240"), 133.333));
    composition
        .meter
        .push(MeterAt::new(pos("5.1.0"), "7/8".parse().unwrap()));
    composition
        .key
        .push(KeyAt::new(pos("1.1.0"), "D dorian".parse().unwrap()));
    composition.sections = vec![Section::new("A \"head\"", pos("1.1.0"))];
    let mut lick = Placement::new(name("lick-2"), pos("5.7.120"));
    lick.repeat = Some(16);
    lick.from = Some(pos("1.2.0"));
    composition.placements = vec![lick];
    let mut song = Song::new(id(1), "Blood Sport — \"v2\"", composition);
    song.intent = Some("a line\nand another".into());
    let mut estimated = TakeLink::new(name("wayfaring"), digest(1));
    estimated.r#for = vec![name("melody")];
    estimated.placed = Some(Placed::estimated(pos("1.1.0"), 1.057, 88.6));
    let mut clicked = TakeLink::new(name("wayfaring-2"), digest(2));
    clicked.placed = Some(Placed::click(pos("9.1.0"), 31.4, Some(-41.0)));
    song.takes = vec![estimated, clicked];
    Document::new(Tool::new("duet", "0.3.0"), song)
}

/// A song's document as JSON, broken by `f`, read back.
fn read_broken(f: impl FnOnce(&mut Value)) -> Error {
    let mut doc: Value = serde_json::from_str(&song().write().unwrap()).unwrap();
    f(&mut doc["body"]);
    Document::<Song>::read(&doc.to_string()).unwrap_err()
}

#[test]
fn a_song_round_trips_exactly_and_validates_against_its_schema() {
    let doc = song();
    let text = doc.write().unwrap();
    let back = Document::<Song>::read(&text).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.write().unwrap(), text, "it writes back identical");
    let AnyDocument::Song(any) = AnyDocument::read(&text).unwrap() else {
        panic!("not read as a song");
    };
    assert_eq!(any, doc, "read by its envelope");
    let value: Value = serde_json::from_str(&text).unwrap();
    let validator = validator();
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}\n{text}");
    // The status is written as the machine names it, and the id as it was minted.
    assert_eq!(value["body"]["status"], json!("idea"));
    assert_eq!(value["body"]["id"], json!(id(1).as_str()));
}

#[test]
fn a_meter_change_off_a_bar_line_is_refused_by_name() {
    let err = read_broken(|body| body["composition"]["meter"][1]["at"] = json!("5.3.0"));
    let message = err.to_string();
    assert!(
        message.contains("composition.meter[1]")
            && message.contains("5.3.0")
            && message.contains("off a bar line"),
        "{message}"
    );
}

#[test]
fn a_map_that_does_not_start_at_the_start_or_runs_backwards_is_refused_by_name() {
    for (field, broken, says) in [
        (
            "tempo",
            json!([{ "at": "2.1.0", "bpm": 90.0 }]),
            "starts at 2.1.0",
        ),
        (
            "meter",
            json!([{ "at": "1.2.0", "sig": "4/4" }]),
            "starts at 1.2.0",
        ),
        ("meter", json!([]), "is empty"),
        ("tempo", json!([]), "is empty"),
        (
            "tempo",
            json!([{ "at": "1.1.0", "bpm": 90.0 }, { "at": "1.1.0", "bpm": 91.0 }]),
            "not after",
        ),
        (
            "key",
            json!([{ "at": "1.1.0", "key": "G minor" }, { "at": "1.1.0", "key": "A minor" }]),
            "not after",
        ),
        (
            "sections",
            json!([{ "name": "b", "at": "3.1.0" }, { "name": "a", "at": "2.1.0" }]),
            "not after",
        ),
        ("tempo", json!([{ "at": "1.1.0", "bpm": 0.0 }]), "positive"),
    ] {
        let message = read_broken(|body| body["composition"][field] = broken.clone()).to_string();
        assert!(
            message.contains(&format!("composition.{field}")) && message.contains(says),
            "{field} {broken}: {message}"
        );
    }
}

#[test]
fn a_place_the_bars_write_another_way_is_refused_naming_how_they_write_it() {
    // Bar 3 is in 6/8, six beats: its seventh is bar 4's first.
    let message =
        read_broken(|body| body["composition"]["sections"][0]["at"] = json!("3.7.0")).to_string();
    assert!(
        message.contains("composition.sections[0].at is 3.7.0") && message.contains("4.1.0"),
        "{message}"
    );
    // A tick past the beat's end, at 960 per quarter and an eighth's beat.
    let message = read_broken(|body| body["composition"]["placements"][0]["at"] = json!("1.1.480"))
        .to_string();
    assert!(
        message.contains("composition.placements[0].at") && message.contains("1.2.0"),
        "{message}"
    );
}

#[test]
fn a_placement_that_plays_nothing_is_refused_by_name() {
    let message =
        read_broken(|body| body["composition"]["placements"][0]["repeat"] = json!(0)).to_string();
    assert!(message.contains("repeat is 0"), "{message}");
    let message =
        read_broken(|body| body["composition"]["placements"][0]["to"] = json!("1.1.0")).to_string();
    assert!(message.contains("runs backwards"), "{message}");
}

#[test]
fn an_unknown_field_and_an_unknown_status_are_refused_by_name() {
    let message = read_broken(|body| body["parts"] = json!([])).to_string();
    assert!(message.contains("`parts`"), "{message}");
    let message = read_broken(|body| body["composition"]["swing"] = json!(0.6)).to_string();
    assert!(message.contains("`swing`"), "{message}");
    let message = read_broken(|body| body["status"] = json!("finished")).to_string();
    assert!(message.contains("finished"), "{message}");
    let message = read_broken(|body| body["title"] = json!("  ")).to_string();
    assert!(message.contains("title is empty"), "{message}");
}

#[test]
fn an_unknown_part_in_a_placement_is_refused_by_name() {
    let song = song().body;
    song.check_parts([&name("lick-2"), &name("bass")]).unwrap();
    let err = song
        .check_parts([&name("bass"), &name("melody")])
        .unwrap_err();
    assert!(
        err.contains("composition.placements[0]")
            && err.contains("\"lick-2\"")
            && err.contains("its parts are bass, melody"),
        "{err}"
    );
    let err = song.check_parts([]).unwrap_err();
    assert!(err.contains("it has none"), "{err}");
}

#[test]
fn two_parts_with_one_name_or_one_id_are_refused_by_name() {
    let parts = part_names(["melody.duet", "bass.duet", "notes.txt", "kit.duet"]).unwrap();
    let names: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["bass", "kit", "melody"],
        "only .duet files, by name"
    );

    let err = part_names(["melody.duet", "bass.duet", "melody.DUET"]).unwrap_err();
    assert!(
        err.contains("two parts are named \"melody\"")
            && err.contains("parts/melody.duet")
            && err.contains("parts/melody.DUET"),
        "{err}"
    );
    let err = part_names(["Melody.duet"]).unwrap_err();
    assert!(err.contains("parts/Melody.duet"), "{err}");

    let (melody, bass) = (name("melody"), name("bass"));
    distinct_ids([(&melody, &id(1)), (&bass, &id(2))]).unwrap();
    let err = distinct_ids([(&melody, &id(1)), (&bass, &id(1))]).unwrap_err();
    assert!(err.contains("\"melody\" and \"bass\" have one id"), "{err}");
}

#[test]
fn the_status_makes_the_four_moves_and_refuses_every_other_by_name() {
    let allowed = [
        (Status::Idea, Status::Sketch),
        (Status::Sketch, Status::Developing),
        (Status::Developing, Status::Arranged),
        (Status::Arranged, Status::Developing),
    ];
    for from in Status::ALL {
        for to in Status::ALL {
            let mut song = song().body;
            song.status = from;
            let before = song.clone();
            match song.set_status(to) {
                Ok(()) => {
                    assert!(allowed.contains(&(from, to)), "{from} → {to} was made");
                    assert_eq!(song.status, to);
                }
                Err(why) => {
                    assert!(!allowed.contains(&(from, to)), "{from} → {to} was refused");
                    assert_eq!(song, before, "a refused move changes nothing");
                    let moves: Vec<&str> = from.moves().iter().map(|s| s.as_str()).collect();
                    assert!(
                        why.contains(from.as_str())
                            && why.contains(&format!("moves only to {}", moves.join(" or "))),
                        "{from} → {to}: {why}"
                    );
                }
            }
        }
    }
    assert_eq!("developing".parse::<Status>(), Ok(Status::Developing));
    assert!("Developing".parse::<Status>().is_err());
}

#[test]
fn a_new_song_starts_at_idea_from_its_flags_then_the_projects_defaults() {
    let mut defaults = Defaults::default();
    defaults.tempo_bpm = Some(92.0);
    defaults.meter = Some("3/4".parse().unwrap());
    defaults.key = Some("Ab".parse().unwrap());
    let from_project = Composition::starting(None, None, None, Some(&defaults));
    assert_eq!(from_project.tempo, [TempoAt::new(pos("1.1.0"), 92.0)]);
    assert_eq!(
        from_project.meter,
        [MeterAt::new(pos("1.1.0"), "3/4".parse().unwrap())]
    );
    assert_eq!(from_project.key[0].key.to_string(), "Ab major");
    let flags = Composition::starting(
        Some(70.0),
        Some("6/8".parse().unwrap()),
        None,
        Some(&defaults),
    );
    assert_eq!(flags.tempo[0].bpm, 70.0);
    assert_eq!(flags.meter[0].sig.to_string(), "6/8");
    assert_eq!(flags.key[0].key.to_string(), "Ab major", "a flag not given");
    let bare = Composition::starting(None, None, None, None);
    assert_eq!(bare.tempo[0].bpm, 120.0);
    assert_eq!(bare.meter[0].sig.to_string(), "4/4");
    assert!(bare.key.is_empty());
    let song = Song::new(id(2), "Porch", bare);
    assert_eq!(song.status, Status::Idea);
    song.check().unwrap();
}

#[test]
fn a_song_with_no_takes_key_reads_as_linking_none_and_writes_it_empty() {
    let mut doc: Value = serde_json::from_str(&song().write().unwrap()).unwrap();
    doc["body"]
        .as_object_mut()
        .unwrap()
        .remove("takes")
        .unwrap();
    let back = Document::<Song>::read(&doc.to_string()).unwrap();
    assert!(back.body.takes.is_empty());
    let text = back.write().unwrap();
    assert!(text.contains("\"takes\": []"), "{text}");
}

#[test]
fn a_placement_that_breaks_its_kind_is_refused_by_name() {
    for (broken, says) in [
        // A click has no tempo of its own.
        (("takes", 1, "bpm", json!(90.0)), "takes[1].placed.bpm"),
        (("takes", 0, "bpm", Value::Null), "takes[0].placed.bpm"),
        (
            ("takes", 0, "clock_ppm", json!(12.0)),
            "takes[0].placed.clock_ppm",
        ),
        (
            ("takes", 1, "at_secs", json!(-1.0)),
            "takes[1].placed.at_secs",
        ),
        (("takes", 0, "bpm", json!(0.0)), "takes[0].placed.bpm"),
    ] {
        let (_, n, field, value) = broken.clone();
        let message = read_broken(|body| {
            let placed = &mut body["takes"][n]["placed"];
            if value.is_null() {
                placed.as_object_mut().unwrap().remove(field);
            } else {
                placed[field] = value.clone();
            }
        })
        .to_string();
        assert!(message.contains(says), "{broken:?}: {message}");
    }
    // A `set` placement needs its tempo as an estimate does.
    let message = read_broken(|body| {
        let placed = body["takes"][0]["placed"].as_object_mut().unwrap();
        placed.insert("by".into(), json!("set"));
        placed.remove("bpm");
    })
    .to_string();
    assert!(
        message.contains("takes[0].placed.bpm") && message.contains("set"),
        "{message}"
    );
    // NaN is not a number in JSON, so it is built, not read: the check names it all the same.
    let mut bad = song().body;
    bad.takes[0].placed = Some(Placed::estimated(pos("1.1.0"), f64::NAN, 90.0));
    let message = bad.check().unwrap_err();
    assert!(message.contains("takes[0].placed.at_secs"), "{message}");
}

#[test]
fn a_placement_off_the_bars_is_refused_naming_how_they_write_it() {
    let mut composition = Composition::new(100.0, "3/4".parse().unwrap(), None);
    let mut link = TakeLink::new(name("a"), digest(9));
    link.placed = Some(Placed::set(pos("3.3.0"), 0.0, 100.0));
    let mut song = Song::new(id(3), "Waltz", composition.clone());
    song.takes = vec![link];
    let text = Document::new(Tool::new("duet", "0.3.0"), song)
        .write()
        .unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["body"]["takes"][0]["placed"]["bar"] = json!("3.4.0");
    let message = Document::<Song>::read(&doc.to_string())
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("takes[0].placed.bar")
            && message.contains("not how the song's bars write that place")
            && message.contains("4.1.0"),
        "{message}"
    );
    composition.end = None;
}

#[test]
fn two_links_to_one_take_by_name_or_digest_are_refused_by_name() {
    let message = read_broken(|body| body["takes"][1]["take"] = json!("wayfaring")).to_string();
    assert!(
        message.contains("takes[1]")
            && message.contains("wayfaring")
            && message.contains("takes[0]"),
        "{message}"
    );
    let first = song().body.takes[0].digest.to_string();
    let message = read_broken(|body| body["takes"][1]["digest"] = json!(first)).to_string();
    assert!(
        message.contains("takes[1]") && message.contains("digest") && message.contains("takes[0]"),
        "{message}"
    );
}

#[test]
fn a_for_that_names_a_part_twice_is_refused_and_one_naming_no_part_is_only_stated() {
    let message =
        read_broken(|body| body["takes"][0]["for"] = json!(["melody", "melody"])).to_string();
    assert!(
        message.contains("takes[0].for") && message.contains("melody"),
        "{message}"
    );
    let song = song().body;
    let lines = song.check_for([&name("bass")]);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("melody"), "{lines:?}");
    assert!(song.check_for([&name("melody")]).is_empty());
}

#[test]
fn the_song_finds_a_link_by_name_or_digest() {
    let mut song = song().body;
    assert_eq!(song.link(&name("wayfaring")).unwrap().digest, digest(1));
    assert!(song.link(&name("nothing")).is_none());
    assert_eq!(song.linking(&digest(2)).unwrap().take, name("wayfaring-2"));
    assert!(song.linking(&digest(3)).is_none());
    song.link_mut(&name("wayfaring")).unwrap().placed = None;
    assert!(song.link(&name("wayfaring")).unwrap().placed.is_none());
}

#[test]
fn the_composition_says_what_holds_at_a_place() {
    let mut c = Composition::new(
        92.0,
        "4/4".parse().unwrap(),
        Some("G minor".parse().unwrap()),
    );
    c.tempo.push(TempoAt::new(pos("9.1.0"), 120.0));
    assert_eq!(c.tempo_at(pos("1.1.0")).unwrap(), 92.0);
    assert_eq!(c.tempo_at(pos("5.1.0")).unwrap(), 92.0);
    assert_eq!(c.tempo_at(pos("9.1.0")).unwrap(), 120.0);
    assert_eq!(c.meter_at(pos("5.1.0")).unwrap(), "4/4".parse().unwrap());
    assert_eq!(c.key_at(pos("5.1.0")), Some("G minor".parse().unwrap()));
    let bare = Composition::new(92.0, "4/4".parse().unwrap(), None);
    assert_eq!(bare.key_at(pos("5.1.0")), None);

    let near = |got: f64, want: f64| assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    near(
        c.secs_between(pos("1.1.0"), pos("9.1.0")).unwrap(),
        8.0 * 4.0 * 60.0 / 92.0,
    );
    near(
        c.secs_between(pos("1.1.0"), pos("10.1.0")).unwrap(),
        8.0 * 4.0 * 60.0 / 92.0 + 4.0 * 0.5,
    );
    near(
        c.secs_between(pos("9.1.0"), pos("1.1.0")).unwrap(),
        -(8.0 * 4.0 * 60.0 / 92.0),
    );
}
