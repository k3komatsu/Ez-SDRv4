//! `00-overview.md` OV-10, OV-12, OV-22: the committed schema is the contract.
//!
//! Regenerates every document schema with the pinned settings and compares it byte
//! for byte with `schemas/`, saying whether a difference is one of shape (OV-12). `EZSDR_UPDATE_SCHEMAS=1` rewrites the files from
//! inside this test; there is no build script and no `xtask`.

use std::path::PathBuf;

use ezsdr_kernel::contract::{Port, PortRef};
use ezsdr_kernel::schema::{document_schemas, file_name, render};

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

#[test]
fn ov_22_schema_freeze() {
    let dir = schemas_dir();
    let update = std::env::var("EZSDR_UPDATE_SCHEMAS").is_ok();
    if update {
        std::fs::create_dir_all(&dir).expect("schemas/ is writable");
    }
    let mut stale = Vec::new();
    let schemas = document_schemas();
    // And the other way: a committed schema no registered type generates is a frozen
    // contract for nothing, and removing a registration left its file passing.
    if !update {
        for entry in std::fs::read_dir(&dir).expect("schemas/ is readable") {
            let file = entry.expect("an entry").file_name().to_string_lossy().into_owned();
            if file.ends_with(".json") && !schemas.keys().any(|n| file_name(n) == file) {
                stale.push(format!("{file}: no registered document generates it"));
            }
        }
    }
    for (name, schema) in schemas {
        let path = dir.join(file_name(name));
        let rendered = render(&schema);
        if update {
            std::fs::write(&path, &rendered).expect("schemas/ is writable");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(committed) => {
                if let Some(how) = ezsdr_kernel::schema::drift(&committed, &rendered) {
                    stale.push(format!("{name}: differs from the committed schema {how}"));
                }
            }
            Err(_) => stale.push(format!("{name}: no committed schema at {}", path.display())),
        }
    }
    assert!(
        stale.is_empty(),
        "OV-10: the committed schema is the contract. Re-run with EZSDR_UPDATE_SCHEMAS=1 \
         and add a SCHEMA_CHANGELOG.md entry (OV-12).\n{}",
        stale.join("\n")
    );
}

#[test]
fn ov_12_a_description_or_title_is_not_the_shape() {
    use ezsdr_kernel::schema::drift;
    let schema = serde_json::json!({
        "title": "Doc",
        "description": "A document.",
        "type": "object",
        "properties": {
            "description": { "type": "string", "description": "A member named so." },
            "title": { "type": "string", "default": "x" }
        },
        "$defs": { "Inner": { "title": "Inner", "type": "object",
            "default": { "description": "data, not an annotation" } } },
        "oneOf": [{ "description": "A variant.", "required": ["title"] }]
    });
    let render = ezsdr_kernel::schema::render;
    let edited = |edit: fn(&mut serde_json::Value)| {
        let mut changed = schema.clone();
        edit(&mut changed);
        drift(&render(&schema), &render(&changed))
    };
    assert_eq!(drift(&render(&schema), &render(&schema)), None);
    let annotation = Some("in a description or title only");
    assert_eq!(edited(|s| s["description"] = "Corrected.".into()), annotation);
    assert_eq!(edited(|s| s["title"] = "Renamed".into()), annotation);
    assert_eq!(
        edited(|s| s["properties"]["description"]["description"] = "Fixed.".into()),
        annotation
    );
    assert_eq!(edited(|s| s["$defs"]["Inner"]["title"] = "Other".into()), annotation);
    assert_eq!(edited(|s| s["oneOf"][0]["description"] = "Fixed.".into()), annotation);
    let shape = Some("in shape, which after the freeze is a new major (OV-12)");
    assert_eq!(
        edited(|s| {
            s["properties"].as_object_mut().unwrap().remove("description");
        }),
        shape,
        "a property named `description` is shape"
    );
    assert_eq!(edited(|s| s["properties"]["title"]["default"] = "y".into()), shape);
    assert_eq!(
        edited(|s| s["$defs"]["Inner"]["default"]["description"] = "other data".into()),
        shape,
        "a default's member named `description` is data"
    );
    assert_eq!(drift("not json", &render(&schema)), shape);
}

#[test]
fn ov_10_every_document_has_a_schema() {
    // A spot check that the list did not silently shrink: the three types the audit
    // singles out, plus the Manifest envelope.
    let schemas = document_schemas();
    for name in ["time_point", "continuity_map", "resource", "manifest", "action",
    ] {
        assert!(schemas.contains_key(name), "{name} has no committed schema (OV-10)");
    }
    assert!(schemas.len() >= 40, "the document list shrank to {}", schemas.len());
}

#[test]
fn x8_in_process_types_have_no_schema() {
    // SampleBlock, BufferRef and the link handles are in-process only by X8; a
    // SampleBlock schema would invite someone to serialise the real-time path.
    let schemas = document_schemas();
    for name in ["sample_block", "buffer_ref", "event_record", "time_authority",
    ] {
        assert!(!schemas.contains_key(name), "{name} must have no schema (X8)");
    }
}

#[test]
fn sc_01_port_shapes_are_pinned_by_the_schema_freeze() {
    fn properties<T: schemars::JsonSchema>() -> std::collections::BTreeSet<String> {
        let schema =
            serde_json::to_value(ezsdr_kernel::schema::generator().into_root_schema_for::<T>())
                .expect("generated schema is JSON");
        schema["properties"]
            .as_object()
            .expect("the document is an object")
            .keys()
            .cloned()
            .collect()
    }

    assert_eq!(
        properties::<Port>(),
        ["contract", "direction", "name"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(
        properties::<PortRef>(),
        ["component", "port"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
}
