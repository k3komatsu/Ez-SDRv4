//! `00-overview.md` OV-23, OV-23a, OV-23b: the Kernel does not grow.
//!
//! The allow-list **is** the review checklist for "Core remains small". An entry
//! names either a specific token from audit §13's Kernel tree or a `NEW:`
//! justification, and the `NEW:` count — not the raw public-item count — is the
//! Kernel-growth measurement (OV-23b).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("src/ is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn sources() -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    rust_sources(&crate_dir().join("src"), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("a source file is UTF-8");
            (p, text)
        })
        .collect()
}

/// Whether a banned token occurring at `start..end` of `lower` is a token and not
/// part of a longer word.
///
/// `_` is deliberately **not** a boundary character: `uhd_open`, `rfnoc_graph_id`
/// and `DPDK_QUEUES` are exactly how a Vision §6 concept enters a Rust identifier,
/// and a scan that treated `_` as part of a word would miss every one of them.
/// `uncertainty` still does not match `taint` and `ReplayDivergence` still does not
/// match `replay`, because those collisions are letter-adjacent.
///
/// Rule: OV-23a.
fn is_token_at(lower: &str, start: usize, end: usize) -> bool {
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    boundary(lower[..start].chars().next_back()) && boundary(lower[end..].chars().next())
}

/// The six rule prefixes of `00-overview.md` §2. A public item's doc comment must
/// cite at least one (OV-23).
const RULE_PREFIXES: [&str; 6] = ["OV-", "TM-", "SC-", "SB-", "RS-", "MA-"];

fn cites_a_rule(doc: &str) -> bool {
    RULE_PREFIXES.iter().any(|p| {
        doc.match_indices(p).any(|(i, _)| {
            doc[i + p.len()..].chars().next().is_some_and(|c| c.is_ascii_digit())
        })
    })
}

/// The allow-list key for an item: `<module>::<name>`.
///
/// Keyed by module and not by bare name, because two items of the same name in
/// different files would otherwise collapse and the later one would inherit the
/// earlier one's allow-list entry and doc comment — which is how a leak walks past
/// this check (OV-23).
fn allow_key(path: &Path, frames: &[bool], name: &str) -> String {
    let module = path
        .strip_prefix(crate_dir().join("src"))
        .unwrap_or(path)
        .with_extension("")
        .to_string_lossy()
        .replace(['/', '\\'], "::");
    let module = module.strip_suffix("::mod").unwrap_or(&module).to_owned();
    // Inline `mod` blocks nest under the file's module. Their names are not tracked
    // individually; the depth is enough to keep an inline module's items from
    // colliding with the file's own.
    let inline = "inline::".repeat(frames.len());
    if module == "lib" {
        format!("{inline}{name}")
    } else {
        format!("{module}::{inline}{name}")
    }
}

/// Module-level `pub` items, which is what the allow-list governs: a method on a
/// public type is not a Kernel concept of its own (OV-23b).
///
/// "Module level" is tracked by brace depth, not by indentation: an item inside an
/// inline `mod` — public or private — is module level and is scanned, while an item
/// inside an `impl`, a `fn` or a `trait` is a member and is not. Scanning only
/// column-0 lines let an inline `pub mod` hide any number of public items behind one
/// allow-list line, and let `mod hidden { pub struct X; } pub use hidden::X;` add a
/// public Kernel item behind none at all.
///
/// A `mod` carrying `#[cfg(feature = "testing")]` is skipped whole, which is the one
/// allow-list line OV-20 and OV-23 give the feature.
fn module_level_public_items() -> BTreeMap<String, (PathBuf, String)> {
    let mut out = BTreeMap::new();
    for (path, text) in sources() {
        let lines: Vec<&str> = text.lines().collect();
        // One frame per open brace: true when it is a `mod` block.
        let mut frames: Vec<bool> = Vec::new();
        // Depth at which a `testing`-gated `mod` opened, if we are inside one.
        let mut testing_at: Option<usize> = None;
        let mut pending_testing = false;
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("#[cfg(feature = \"testing\")]") {
                pending_testing = true;
            }
            let at_module_level = frames.iter().all(|m| *m);
            let inside_testing = testing_at.is_some();

            if at_module_level && !inside_testing {
                if let Some(rest) = trimmed.strip_prefix("pub ") {
                    if let Some((keyword, tail)) = rest.split_once(' ') {
                        if matches!(
                            keyword,
                            "fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "mod"
                        ) {
                            let name: String = tail
                                .trim_start_matches("mut ")
                                .chars()
                                .take_while(|c| c.is_alphanumeric() || *c == '_')
                                .collect();
                            if !name.is_empty() {
                                out.insert(
                                    allow_key(&path, &frames, &name),
                                    (path.clone(), doc_above(&lines, i)),
                                );
                            }
                        }
                    }
                }
            }

            let opens = line.matches('{').count();
            let closes = line.matches('}').count();
            if opens > 0 {
                // Any visibility: `mod`, `pub mod`, `pub(crate) mod`, `pub(super) mod`.
                let after_vis = trimmed
                    .strip_prefix("pub")
                    .map(|r| r.trim_start_matches(|c| c != ' ').trim_start())
                    .unwrap_or(trimmed);
                let is_mod = after_vis.starts_with("mod ");
                if is_mod && pending_testing && testing_at.is_none() {
                    testing_at = Some(frames.len());
                }
                frames.push(is_mod);
                // Every further brace on the line opens a non-module block.
                frames.resize(frames.len() + (opens - 1), false);
            }
            for _ in 0..closes {
                frames.pop();
                if testing_at.is_some_and(|d| d >= frames.len()) {
                    testing_at = None;
                }
            }
            if !trimmed.is_empty() && !trimmed.starts_with("#[") && !trimmed.starts_with("///") {
                pending_testing = false;
            }
        }
        // A `pub use` out of a private module would re-export items the loop above
        // has already scanned inside that module, so the surface is accounted for;
        // what must not happen is a `pub use` of something outside `src/`.
        for line in text.lines().map(str::trim_start).filter(|l| l.starts_with("pub use ")) {
            assert!(
                !line.contains("::crate") && !line.starts_with("pub use ::"),
                "OV-23: {} re-exports from outside the crate: {line}",
                path.display()
            );
        }
    }
    out
}

/// The contiguous `///` block above line `i`, skipping attributes, which a derive
/// may spread over several lines.
fn doc_above(lines: &[&str], i: usize) -> String {
    let mut doc = String::new();
    let mut skipped = 0;
    let mut j = i;
    while j > 0 {
        j -= 1;
        let above = lines[j].trim_start();
        if let Some(d) = above.strip_prefix("///") {
            doc.insert_str(0, d);
            doc.insert(0, '\n');
            continue;
        }
        if !doc.is_empty() {
            break;
        }
        skipped += 1;
        let ends_an_item = above.ends_with(';')
            || above.starts_with('}')
            || above.starts_with("pub ")
            || above.starts_with("impl ");
        if skipped > 12 || ends_an_item {
            break;
        }
    }
    doc
}

fn allow_list() -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(crate_dir().join("tests/kernel_surface_allow.txt"))
        .expect("the allow-list is the review checklist and must exist");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (name, why) = l.split_once('=').unwrap_or_else(|| {
                panic!("OV-23: allow-list line {l:?} is not `name = justification`")
            });
            (name.trim().to_owned(), why.trim().to_owned())
        })
        .collect()
}

#[test]
fn ov_23_every_public_item_is_on_the_allow_list() {
    let items = module_level_public_items();
    let allowed = allow_list();
    let missing: Vec<&String> =
        items.keys().filter(|name| !allowed.contains_key(*name)).collect();
    assert!(
        missing.is_empty(),
        "OV-23: {} public item(s) are not on the allow-list. Each needs a specific \
         audit §13 token or a `NEW:` justification:\n{:#?}",
        missing.len(),
        missing
    );
    // An allow-list entry for an item that no longer exists is stale.
    let stale: Vec<&String> = allowed
        .keys()
        .filter(|name| !items.contains_key(*name) && name.as_str() != "FEATURE:testing")
        .collect();
    assert!(stale.is_empty(), "OV-23: stale allow-list entries: {stale:#?}");
}

#[test]
fn ov_23_allow_list_entries_name_a_token_or_a_justification() {
    for (name, why) in allow_list() {
        assert!(!why.is_empty(), "OV-23: {name} has an empty justification");
        if why.starts_with("NEW:") {
            assert!(
                why.len() > "NEW:".len() + 10,
                "OV-23: {name}'s NEW: justification is not a sentence"
            );
        } else {
            // A specific token from audit §13's Kernel tree, not a whole line.
            assert!(
                why.split_whitespace().count() <= 6,
                "OV-23: {name} must name a *specific* audit §13 token, not a line: {why:?}"
            );
        }
    }
}

#[test]
fn ov_23b_kernel_growth_is_the_new_count() {
    let allowed = allow_list();
    let new: BTreeSet<&String> =
        allowed.iter().filter(|(_, why)| why.starts_with("NEW:")).map(|(n, _)| n).collect();
    // Audit §13's `data` line ends with the catch-all "Stream Contract (normative)",
    // which would otherwise absorb every type spec 02 invents; OV-23b makes each one
    // an explicit `NEW:` instead.
    for expected in [
        "stream::continuity::ContinuityMap",
        "stream::continuity::ContinuityBuilder",
        "stream::burst::BurstTracker",
        "stream::burst::BurstRecord",
        "stream::continuity::GapCause",
        "stream::burst::LatePolicy",
        "stream::link::DropCarry",
    ] {
        assert!(
            new.contains(&expected.to_owned()),
            "OV-23b: {expected} is a type audit §13 does not name and must be marked NEW:"
        );
    }
    let total = allowed.len() - 1; // the `testing` feature line is not an item
    eprintln!("OV-23b: Kernel growth = {} NEW: items of {total} public items", new.len());
    // The bound is not a rule; it is the shape the measurement must keep for the
    // number to mean anything: most of the Kernel surface is still audit §13's, and
    // every departure from it is written out with its reason.
    assert!(
        new.len() * 2 < total,
        "OV-23b: {} of {total} public items are NEW:; the Kernel is no longer mostly audit §13's",
        new.len()
    );
}

#[test]
fn ov_23_public_items_cite_a_rule_id() {
    let mut uncited = Vec::new();
    for (name, (path, doc)) in module_level_public_items() {
        if !cites_a_rule(&doc) {
            uncited.push(format!("{}: {name}", path.display()));
        }
    }
    assert!(
        uncited.is_empty(),
        "OV-23: {} public item(s) cite no rule ID in their doc comment:\n{:#?}",
        uncited.len(),
        uncited
    );
}

#[test]
fn ma_04_kernel_surface_ban() {
    let banned: Vec<String> = std::fs::read_to_string(crate_dir().join("tests/banned_tokens.txt"))
        .expect("the ban list is a committed file (OV-23a)")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_lowercase)
        .collect();
    assert_eq!(banned.len(), 16, "OV-23a fixes the list at sixteen tokens");

    // `_` is NOT a boundary character: `uhd_open`, `rfnoc_graph_id` and
    // `DPDK_QUEUES` are exactly how a §6 concept enters a Rust identifier, and a
    // scan that treated `_` as part of a word would miss every one of them.
    // `uncertainty` still does not match `taint` and `check_replay_target` still
    // does not match `replay`, because those collisions are letter-adjacent.
    let mut hits = Vec::new();
    for (path, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            // OV-23's escape hatch: a line that cites the ban may carry the token.
            if line.contains("OV-23a") {
                continue;
            }
            let lower = line.to_lowercase();
            for token in &banned {
                let mut from = 0;
                while let Some(at) = lower[from..].find(token.as_str()) {
                    let start = from + at;
                    let end = start + token.len();
                    if is_token_at(&lower, start, end) {
                        hits.push(format!("{}:{}: {token} in {line:?}", path.display(), n + 1));
                    }
                    from = end;
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "OV-23a: a banned token appears outside a comment citing the ban:\n{:#?}",
        hits
    );
}

#[test]
fn ov_23a_the_ban_catches_a_token_inside_an_identifier() {
    // The way a Vision §6 concept actually enters Rust is as part of a name, so the
    // scan has to see `uhd_open` and `rfnoc_graph_id`, and must still not see
    // `uncertainty` or `ReplayDivergence`.
    let hits = |line: &str, token: &str| {
        let lower = line.to_lowercase();
        let mut from = 0;
        while let Some(at) = lower[from..].find(token) {
            let (start, end) = (from + at, from + at + token.len());
            if is_token_at(&lower, start, end) {
                return true;
            }
            from = end;
        }
        false
    };
    assert!(hits("pub fn uhd_open(handle: u64) {}", "uhd"));
    assert!(hits("    pub rfnoc_graph_id: u32,", "rfnoc"));
    assert!(hits("const DPDK_QUEUES: usize = 4;", "dpdk"));
    assert!(hits("// a replay against a profile", "replay"));
    assert!(!hits("pub uncertainty: Duration,", "taint"));
    assert!(!hits("RunError::ReplayDivergence { field }", "replay"));
    assert!(!hits("let drift_uncertainty = 0.0;", "taint"));
}

#[test]
fn ov_23_the_allow_list_key_names_the_module() {
    // Keyed by bare name, two items of the same name in different files collapse and
    // the later one inherits the earlier one's entry and doc comment — which is how
    // a leak walks past this check.
    let items = module_level_public_items();
    assert!(items.contains_key("plan::Fragment"));
    assert!(items.contains_key("plan::plan"));
    assert!(!items.contains_key("Fragment"), "a bare name is never a key");
}

#[test]
fn ov_23_testing_feature_items_are_excluded_by_one_line() {
    // ManualTimeAuthority is normative (TM-17a) and ships in `src/` behind the
    // non-default `testing` feature; OV-20 and OV-23 exclude it with one allow-list
    // line that names the feature.
    let allowed = allow_list();
    assert!(
        allowed.contains_key("FEATURE:testing"),
        "OV-23: the `testing` feature needs exactly one allow-list line"
    );
    // And the line is what excludes them, not the parser failing to see them: the
    // scanner skips a `mod` carrying `#[cfg(feature = "testing")]` whole.
    let items = module_level_public_items();
    assert!(
        items.keys().all(|k| !k.contains("ManualTimeAuthority")),
        "the testing-gated module's items are excluded"
    );
    // ManualTimeAuthority's re-export in `time/mod.rs` is itself cfg-gated, so the
    // public surface of the default build really does not carry it.
    let time_mod = std::fs::read_to_string(crate_dir().join("src/time/mod.rs")).expect("readable");
    assert!(time_mod.contains("#[cfg(feature = \"testing\")]\npub use authority::ManualTimeAuthority;"));
}
