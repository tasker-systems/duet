//! The schemas committed under `crates/duet-schemas/schemas/`, generated from the Rust types.
//!
//! - `envelope/<major>.<minor>.json` is the `duet` object ([`envelope_schema`]).
//! - `<kind>/<major>.<minor>.json` is a whole file of that kind: `$schema`, `duet` (the envelope
//!   schema by reference, with its `kind` and `version` pinned), and the `body`
//!   ([`document_schema`]) or, for a stream's first line, the `head`, with `$defs/Record` for
//!   every later line ([`stream_schema`]).
//!
//! The gate (`tests/schemas.rs`) fails when a committed file differs from what these functions
//! generate at the same version.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Map, Value, json};

use crate::{Envelope, Kind, StreamKind, Version, schema_url};

/// The envelope schema's own name in `schemas/`.
pub const ENVELOPE: &str = "envelope";

/// The version of the envelope's schema.
///
/// The envelope is not versioned in any file: every kind's schema names this one. So a change to
/// the envelope's shape is a new major version of every kind, and moves this major with them.
pub const ENVELOPE_VERSION: Version = Version::new(1, 0);

/// The JSON Schema dialect every committed schema is written in.
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Takes `null` out of every optional property: a field that is absent is left out, never written
/// `null`, so the schema says so. A required field, or an item of an array, keeps its `null`:
/// there it is a value, such as an unvoiced frame's pitch.
fn absent_is_omitted(schema: &mut Value) {
    match schema {
        Value::Object(fields) => {
            let required: Vec<String> = fields
                .get("required")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|n| n.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(Value::Object(properties)) = fields.get_mut("properties") {
                for (name, property) in properties.iter_mut() {
                    if !required.contains(name) {
                        drop_null(property);
                    }
                }
            }
            fields.values_mut().for_each(absent_is_omitted);
        }
        Value::Array(items) => items.iter_mut().for_each(absent_is_omitted),
        _ => {}
    }
}

/// `{"type": [T, "null"]}` becomes `{"type": T}`, and `{"anyOf": [S, {"type": "null"}]}`
/// becomes `S`, keeping the property's own keywords such as its description.
fn drop_null(property: &mut Value) {
    let Value::Object(fields) = property else {
        return;
    };
    if let Some(Value::Array(types)) = fields.get_mut("type") {
        types.retain(|t| t != "null");
        if let [only] = types.as_slice() {
            let only = only.clone();
            fields.insert("type".into(), only);
        }
    }
    if let Some(Value::Array(any_of)) = fields.get("anyOf") {
        let null = json!({ "type": "null" });
        let rest: Vec<&Value> = any_of.iter().filter(|s| **s != null).collect();
        if let [Value::Object(only)] = rest.as_slice() {
            let only = only.clone();
            fields.remove("anyOf");
            for (key, value) in only {
                fields.entry(key).or_insert(value);
            }
        }
    }
}

fn generator() -> SchemaGenerator {
    SchemaSettings::draft2020_12().into_generator()
}

/// `T`'s schema as a reference into `generator`'s definitions.
fn reference<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    generator.subschema_for::<T>().to_value()
}

/// The schema of the envelope, the `duet` object: `schemas/envelope/<ENVELOPE_VERSION>.json`.
#[must_use]
pub fn envelope_schema() -> Value {
    let mut schema = Map::new();
    schema.insert("$schema".into(), json!(DIALECT));
    schema.insert("$id".into(), json!(schema_url(ENVELOPE, ENVELOPE_VERSION)));
    schema.insert(
        "title".into(),
        json!(format!("duet envelope {ENVELOPE_VERSION}")),
    );
    if let Value::Object(fields) = generator().into_root_schema_for::<Envelope>().to_value() {
        schema.extend(
            fields
                .into_iter()
                .filter(|(key, _)| key != "$schema" && key != "title"),
        );
    }
    let mut schema = Value::Object(schema);
    absent_is_omitted(&mut schema);
    schema
}

/// The frame shared by both framings: `$schema` pinned to `$id`, `duet` the envelope with this
/// kind and version, and `part` (`body` or `head`) as given.
fn frame(
    name: &str,
    version: Version,
    what: &str,
    part: &str,
    value: Value,
    mut defs: Map<String, Value>,
) -> Value {
    let id = schema_url(name, version);
    defs.sort_keys();
    let mut schema = json!({
        "$schema": DIALECT,
        "$id": id,
        "title": format!("duet {name} {version}"),
        "description": format!("{what} of kind {name}, at schema version {version}."),
        "type": "object",
        "properties": {
            "$schema": { "const": id },
            "duet": {
                "$ref": schema_url(ENVELOPE, ENVELOPE_VERSION),
                "properties": {
                    "kind": { "const": name },
                    "version": { "const": version.to_string() },
                },
            },
            part: value,
        },
        "required": ["duet", part],
        "additionalProperties": false,
        "$defs": defs,
    });
    absent_is_omitted(&mut schema);
    schema
}

/// The schema of a `.duet` document of kind `K`: `schemas/<K::NAME>/<K::VERSION>.json`.
#[must_use]
pub fn document_schema<K: Kind>() -> Value {
    let mut generator = generator();
    let body = reference::<K>(&mut generator);
    let defs = generator.take_definitions(true);
    frame(K::NAME, K::VERSION, "A .duet document", "body", body, defs)
}

/// The schema of a `.duet.jsonl` stream of kind `K`: the first line, with the head, and
/// `$defs/Record`, which each later line validates against on its own.
#[must_use]
pub fn stream_schema<K: StreamKind>() -> Value {
    let mut generator = generator();
    let head = reference::<K>(&mut generator);
    let record = reference::<K::Record>(&mut generator);
    let mut defs = generator.take_definitions(true);
    if record != json!({ "$ref": "#/$defs/Record" }) {
        defs.insert("Record".into(), record);
    }
    frame(
        K::NAME,
        K::VERSION,
        "The first line of a .duet.jsonl stream",
        "head",
        head,
        defs,
    )
}
