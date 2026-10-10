//! Phase 1 tests for `00-overview.md` §7: canonical JSON and content hashes.

use ezsdr_kernel::hash::{ContentHash, HashError, canonical_json};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::spec::Value;
use ezsdr_kernel::time::{ClockRelation, Duration, TimeError, TimePoint, Validity};
use serde_json::json;

fn canon(v: serde_json::Value) -> String {
    canonical_json(&v).expect("canonicalises")
}

#[test]
fn ov_15_integer_profile_is_exact_at_every_magnitude() {
    // Through a strict RFC 8785 double these lose their last bits, which is why the
    // integer profile is a named deviation (OV-15a).
    for v in [
        9_007_199_254_740_992i64,     // 2^53
        9_007_199_254_740_993,        // 2^53 + 1
        1_700_000_000_000_000_000,    // a realistic epoch-to-UTC offset in nanoseconds
        i64::MIN,
    ] {
        assert_eq!(canon(json!({ "ticks": v })), format!("{{\"ticks\":{v}}}"));
    }
    // Neighbours must not collide.
    let h = |v: i64| ContentHash::of(&json!({ "ticks": v })).expect("hashes");
    assert_ne!(h(9_007_199_254_740_992), h(9_007_199_254_740_993));
    assert_ne!(h(1_700_000_000_000_000_000), h(1_700_000_000_000_000_001));
}

#[test]
#[allow(clippy::excessive_precision)] // the RFC 8785 vector is written as published
fn ov_15_float_layout_follows_rfc_8785() {
    for (v, want) in [
        (1e21f64, "1e+21"),
        (1e-7, "1e-7"),
        (0.000001, "0.000001"),
        (333333333.33333329, "333333333.3333333"),
        (5e-324, "5e-324"),
        (1.7976931348623157e308, "1.7976931348623157e+308"),
        (2400000000.0, "2400000000"),
    ] {
        assert_eq!(canon(json!({ "drift": v })), format!("{{\"drift\":{want}}}"), "for {v}");
    }
}

#[test]
fn ov_15_non_finite_is_rejected_not_null() {
    // `serde_json` maps a non-finite float to `null` while building the `Value`, so
    // by the time the canonicaliser runs the information is gone. Left unguarded, a
    // NaN drift hashes identically to a drift that was never measured — in the one
    // Manifest field OV-15a exists to protect. `ClockRelation::new` refuses one, and
    // `Finite::new` refuses one for a scalar.
    let relation = |drift: f64| {
        ClockRelation::new(
            ClockDomainId::local(2),
            ClockDomainId::UTC,
            TimePoint::new(ClockDomainId::local(2), 0),
            TimePoint::new(ClockDomainId::UTC, 0),
            drift,
            0.0,
            Duration::new(ClockDomainId::UTC, 1),
            "test.poll".to_owned(),
            Validity { from: TimePoint::new(ClockDomainId::local(2), 0), to: None },
        )
    };
    // A non-finite drift is no relation at all (TM-14), so it never reaches a hash.
    for drift in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(relation(drift), Err(TimeError::Malformed { field: "drift", .. })));
    }
    let hash = |drift| ContentHash::of(&relation(drift).expect("finite"));
    assert!(hash(1e-6).is_ok());
    // Two finite drifts one ulp apart must not collide either.
    assert_ne!(hash(1e-6), hash(1.0000000000000002e-6));

    // The Kernel's other float, `Scalar::Num` (SB-4, SC-2), is finite by
    // construction: a non-finite one cannot be built, so it never reaches a hash.
    assert!(Value::num(f64::NAN).is_none());
    assert!(ContentHash::of(&Value::num(1.5).unwrap()).is_ok());
    assert_ne!(
        ContentHash::of(&Value::num(1.5).unwrap()).expect("hashes"),
        ContentHash::of(&Value::num(2.5).unwrap()).expect("hashes")
    );

    // `serde_json::Number` still cannot hold one, so `canonical_json` never sees it.
    assert!(serde_json::Number::from_f64(f64::NAN).is_none());
    // An integral float takes the ECMAScript form, not `1.0`.
    assert_eq!(canon(json!(1.0f64)), "1");
}

#[test]
fn ov_15_keys_are_sorted_and_whitespace_is_gone() {
    let a = json!({ "b": 1, "a": 2, "c": { "z": 1, "y": 2 } });
    let b: serde_json::Value =
        serde_json::from_str("  { \"c\" : { \"y\" : 2 , \"z\" : 1 } , \"a\" : 2 , \"b\" : 1 }  ")
            .expect("parses");
    assert_eq!(canon(a.clone()), "{\"a\":2,\"b\":1,\"c\":{\"y\":2,\"z\":1}}");
    assert_eq!(canon(a.clone()), canon(b.clone()));
    assert_eq!(ContentHash::of(&a).expect("hashes"), ContentHash::of(&b).expect("hashes"));
}

#[test]
fn ov_15_non_ascii_key_is_refused() {
    let v = json!({ "kéy": 1 });
    assert!(matches!(canonical_json(&v), Err(HashError::NonAsciiKey { .. })));
}

#[test]
fn ov_15_strings_use_rfc_8785_escaping() {
    assert_eq!(canon(json!("a\"b\\c\nd\te\u{1}f")), "\"a\\\"b\\\\c\\nd\\te\\u0001f\"");
    assert_eq!(canon(json!("日本語")), "\"日本語\"", "non-ASCII values stay literal");
}

#[test]
fn ov_14_sha256_known_answers() {
    assert_eq!(
        ContentHash::of_bytes(b"").as_str(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        ContentHash::of_bytes(b"abc").as_str(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn ov_14_content_hash_round_trips() {
    let h = ContentHash::of_bytes(b"abc");
    assert_eq!(ContentHash::parse(h.as_str()), Ok(h.clone()));
    assert_eq!(ContentHash::parse("sha256:zz"), Err(HashError::MalformedHash));
    assert_eq!(ContentHash::parse("md5:00"), Err(HashError::MalformedHash));
    assert_eq!(
        ContentHash::parse("sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"),
        Err(HashError::MalformedHash),
        "the hex is lowercase"
    );
}

#[test]
fn ov_16_equal_inputs_hash_equally_across_orders() {
    let items = json!([{ "k": 1, "j": 2 }, { "j": 2, "k": 1 }]);
    let serde_json::Value::Array(pair) = items else { unreachable!() };
    assert_eq!(
        ContentHash::of(&pair[0]).expect("hashes"),
        ContentHash::of(&pair[1]).expect("hashes")
    );
}
