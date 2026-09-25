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

fn is_token_at(lower: &str, start: usize, end: usize) -> bool {
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    boundary(lower[..start].chars().next_back()) && boundary(lower[end..].chars().next())
}

/// The Kernel's rule prefixes: Phase 1's six (`00-overview.md` §2) and Phase 2's
/// three (`plan/phase2/06-kernel-coordinator.md`). A public item's doc comment must
/// cite at least one (OV-23, KA-20).
const RULE_PREFIXES: [&str; 9] = [
    "OV-", "TM-", "SC-", "SB-", "RS-", "MA-", "KA-", "KC-", "UC-",
];

/// Every module name of this crate, taken from `src/` itself so the set cannot go
/// stale, so that a `pub use` can be told from a re-export of a dependency's type
/// (OV-23).
fn crate_modules() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (path, text) in sources() {
        // Inline modules too: `mod manual` inside `time/authority.rs` is a module of
        // this crate even though it is not a file.
        for line in text.lines().map(str::trim_start) {
            let decl = line.strip_prefix("pub ").unwrap_or(line);
            let decl = decl.strip_prefix("pub(crate) ").unwrap_or(decl);
            if let Some(rest) = decl.strip_prefix("mod ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.insert(name);
                }
            }
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if stem != "mod" && stem != "lib" {
                out.insert(stem.to_owned());
            }
        }
        if let Some(dir) = path
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|d| d.to_str())
        {
            if dir != "src" {
                out.insert(dir.to_owned());
            }
        }
    }
    out
}

fn cites_a_rule(doc: &str) -> bool {
    RULE_PREFIXES.iter().any(|p| {
        doc.match_indices(p).any(|(i, _)| {
            let rest = &doc[i + p.len()..];
            let number: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if number.is_empty() {
                return false;
            }
            let suffix: String = rest[number.len()..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase())
                .collect();
            // A withdrawn rule keeps its number (OV-1) but no longer states an
            // obligation, so citing one satisfies nothing.
            !WITHDRAWN.contains(&format!("{p}{number}{suffix}").as_str())
        })
    })
}

#[test]
fn ov_01_citing_a_withdrawn_rule_satisfies_nothing() {
    // A withdrawn rule keeps its number and states no obligation (OV-1), so a doc
    // comment citing only one cites no rule, and one citing a live rule beside it does.
    assert!(!cites_a_rule("Rule: SB-25a."));
    assert!(!cites_a_rule("Rule: MA-43."));
    assert!(cites_a_rule("Rule: SB-25a, SB-25."));
}

/// The rules withdrawn so far. OV-1 keeps their numbers, and nothing may cite one as
/// the rule it implements.
const WITHDRAWN: [&str; 7] = [
    "SB-25a", "SB-28", "SB-32", "RS-32a", "RS-37", "MA-4", "MA-43",
];

/// The allow-list key for an item: `<module>::<name>`.
///
/// Keyed by module and not by bare name, because two items of the same name in
/// different files would otherwise collapse and the later one would inherit the
/// earlier one's allow-list entry and doc comment — which is how a leak walks past
/// this check (OV-23).
fn allow_key(path: &Path, inline_mods: &[String], name: &str) -> String {
    let module = path
        .strip_prefix(crate_dir().join("src"))
        .unwrap_or(path)
        .with_extension("")
        .to_string_lossy()
        .replace(['/', '\\'], "::");
    let module = module.strip_suffix("::mod").unwrap_or(&module).to_owned();
    // Inline `mod` blocks nest under the file's module **by name**. Keying them by
    // depth alone collapsed two inline modules of one file that declare a same-named
    // item onto one key, so the second inherited the first's allow-list entry and the
    // gate reported one missing item for two.
    let inline: String = inline_mods.iter().map(|m| format!("{m}::")).collect();
    if module == "lib" {
        format!("{inline}{name}")
    } else {
        format!("{module}::{inline}{name}")
    }
}

/// Module-level `pub` items, which is what the allow-list governs: a method on a
/// public type is not a Kernel concept of its own (OV-23b).
///
/// The source is **parsed**, not lexed. Five review passes demonstrated eleven ways
/// past a line-based scan — an unbalanced brace in a comment, a multi-line string, a
/// raw string, a raw byte string, `union`, a name on the next line, a foreign
/// `pub use`, an attribute before the item, `pub async`, `pub extern` and a bare
/// `pub` — and every one was a lexing failure, a spelling the keyword and modifier
/// lists did not cover. Two of them were hiding public items in the shipped crate.
/// A parser closes that class by construction, and a file it cannot parse fails this
/// test, so the gate's failure mode is a red test on the MSRV toolchain and never a
/// silent pass (OV-23, X11).
///
/// What no source parser sees is code produced at **expansion**, and that class is
/// closed by refusing its mechanisms — a `macro_rules!` body carrying `pub`, an
/// `extern` block, and a `mod` carrying `#[path]` or `#[cfg_attr]`. Proc-macros are
/// confined to `serde_derive` and `schemars_derive` by X6 and generate no
/// module-level items of this crate.
///
/// A `mod` carrying `#[cfg(feature = "testing")]` is skipped whole, which is the one
/// allow-list line OV-20 and OV-23 give the feature.
fn module_level_public_items() -> BTreeMap<String, (PathBuf, String)> {
    let mut out = BTreeMap::new();
    let modules = crate_modules();
    for (path, text) in sources() {
        let file = syn::parse_file(&text).unwrap_or_else(|e| {
            panic!(
                "OV-23: {} does not parse, so its surface is unknown: {e}",
                path.display()
            )
        });
        walk_items(&path, &file.items, &[], &modules, &mut out);
    }
    out
}

/// The `#[doc]` text of an item, joined as the old scan's contiguous `///` block was.
fn doc_of(attrs: &[syn::Attribute]) -> String {
    let mut out = String::new();
    for a in attrs {
        if !a.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &a.meta {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                out.push_str(&s.value());
                out.push('\n');
            }
        }
    }
    out
}

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Whether an item is gated **on** the `testing` feature, which OV-20 and OV-23 give
/// one allow-list line instead of an entry per item.
///
/// The predicate is an exact match on `feature = "testing"`, not a substring search
/// for `testing`: `#[cfg(not(feature = "testing"))]` contains the word and is the
/// **default** build, so a substring match skipped items that ship. Anything else
/// mentioning the feature is scanned normally, which is the safe direction — it is
/// then required on the allow-list like any other public item.
fn is_testing_gated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && matches!(&a.meta, syn::Meta::List(l)
                if l.tokens.to_string().replace(' ', "") == "feature=\"testing\"")
    })
}

fn walk_items(
    path: &Path,
    items: &[syn::Item],
    inline_mods: &[String],
    modules: &BTreeSet<String>,
    out: &mut BTreeMap<String, (PathBuf, String)>,
) {
    for item in items {
        // Expansion-time constructs this parser cannot see into.
        if let syn::Item::Macro(m) = item {
            // `include!("x.in")` splices a file this scan never opens: `rust_sources`
            // collects `.rs`, and the macro body it can see is one string literal.
            assert!(
                !m.mac.path.is_ident("include"),
                "OV-23: {} uses `include!`, which splices items from a file this \
                 scan does not read",
                path.display()
            );
            let body = m.mac.tokens.to_string();
            assert!(
                !body
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == "pub"),
                "OV-23: {} has a macro whose body carries `pub`; a parser does not expand \
                 macros, so anything public in one is invisible to the allow-list — declare \
                 it directly",
                path.display()
            );
            continue;
        }
        if matches!(item, syn::Item::ForeignMod(_)) {
            panic!(
                "OV-23: {} declares an extern block, whose surface this scan does not model",
                path.display()
            );
        }
        // `pub extern crate serde_json as x;` puts a whole dependency on the surface
        // under a name this scan read as no item at all, because `ExternCrate` fell
        // through to the catch-all arm below.
        if let syn::Item::ExternCrate(e) = item {
            assert!(
                !matches!(e.vis, syn::Visibility::Public(_)),
                "OV-23: {} re-exports a whole crate with `pub extern crate`",
                path.display()
            );
            continue;
        }

        let (attrs, vis, name): (&[syn::Attribute], Option<&syn::Visibility>, Option<String>) =
            match item {
                syn::Item::Struct(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Enum(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Union(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Trait(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Type(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Const(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Static(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Fn(i) => (&i.attrs, Some(&i.vis), Some(i.sig.ident.to_string())),
                syn::Item::Mod(i) => (&i.attrs, Some(&i.vis), Some(i.ident.to_string())),
                syn::Item::Use(i) => (&i.attrs, Some(&i.vis), None),
                _ => (&[], None, None),
            };

        if let syn::Item::Use(u) = item {
            // Only a **public** re-export reaches the surface; a private `use` is an
            // import and says nothing about what the crate exposes.
            if !matches!(u.vis, syn::Visibility::Public(_)) {
                continue;
            }
            // A dependency's type must not reach the surface unlisted.
            // A glob names no item the allow-list can hold, and the root check
            // short-circuits at the first segment — so `pub use crate::internal::*;`
            // named a module of this crate and put everything behind it on the
            // surface with no allow-list line at all.
            assert!(
                !has_glob(&u.tree),
                "OV-23: {} has a glob `pub use`, which puts items on the surface that \
                 this scan cannot enumerate — re-export them by name",
                path.display()
            );
            let mut roots = Vec::new();
            use_roots(&u.tree, &mut roots);
            // A brace group has one root per branch and a glob has none it can name.
            // Returning the empty string for both put them in the accepted set, so
            // `pub use {serde_json::Value as X};` re-exported a dependency's type past
            // the check that exists to refuse exactly that.
            assert!(
                !roots.is_empty(),
                "OV-23: {} has a `pub use` with no root",
                path.display()
            );
            for first in roots {
                assert!(
                    matches!(first.as_str(), "crate" | "self" | "super")
                        || modules.contains(&first),
                    "OV-23: {} re-exports `{first}`, which is not a module of this crate",
                    path.display()
                );
            }
            continue;
        }

        if let syn::Item::Mod(m) = item {
            if is_testing_gated(&m.attrs) {
                continue;
            }
            assert!(
                !has_attr(&m.attrs, "path") && !has_attr(&m.attrs, "cfg_attr"),
                "OV-23: {} has a `mod` carrying `#[path]` or `#[cfg_attr]`, which can move or \
                 rewrite the module this scan believes it read",
                path.display()
            );
            if let Some((_, inner)) = &m.content {
                let mut nested = inline_mods.to_vec();
                nested.push(m.ident.to_string());
                walk_items(path, inner, &nested, modules, out);
            }
        }

        if matches!(vis, Some(syn::Visibility::Public(_))) {
            if let Some(name) = name {
                out.insert(
                    allow_key(path, inline_mods, &name),
                    (path.to_path_buf(), doc_of(attrs)),
                );
            }
        }
    }
}

/// Whether a `use` tree re-exports with `*` anywhere inside it.
fn has_glob(tree: &syn::UseTree) -> bool {
    match tree {
        syn::UseTree::Glob(_) => true,
        syn::UseTree::Path(p) => has_glob(&p.tree),
        syn::UseTree::Group(g) => g.items.iter().any(has_glob),
        syn::UseTree::Name(_) | syn::UseTree::Rename(_) => false,
    }
}

/// Every first segment a `use` tree names: one per branch of a brace group, and the
/// unusable `*` for a glob, so neither can pass the module check by naming nothing.
fn use_roots(tree: &syn::UseTree, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(p) => out.push(p.ident.to_string()),
        syn::UseTree::Name(n) => out.push(n.ident.to_string()),
        syn::UseTree::Rename(r) => out.push(r.ident.to_string()),
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(g) => {
            for t in &g.items {
                use_roots(t, out);
            }
        }
    }
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
    let missing: Vec<&String> = items
        .keys()
        .filter(|name| !allowed.contains_key(*name))
        .collect();
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
    assert!(
        stale.is_empty(),
        "OV-23: stale allow-list entries: {stale:#?}"
    );
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
    let new: BTreeSet<&String> = allowed
        .iter()
        .filter(|(_, why)| why.starts_with("NEW:"))
        .map(|(n, _)| n)
        .collect();
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
    eprintln!(
        "OV-23b: Kernel growth = {} NEW: items of {total} public items",
        new.len()
    );
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
    // `uncertainty` still does not match `taint`, because that collision is
    // letter-adjacent; `check_replay_target` does match `replay`, which is why its
    // declaration line cites OV-23a.
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
    assert!(
        !items.contains_key("Fragment"),
        "a bare name is never a key"
    );
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
    assert!(
        time_mod.contains("#[cfg(feature = \"testing\")]\npub use authority::ManualTimeAuthority;")
    );
}

#[test]
fn sc_07_unsafe_code_remains_forbidden() {
    let path = crate_dir().join("src/lib.rs");
    let source = std::fs::read_to_string(&path).expect("lib.rs is readable");
    let file = syn::parse_file(&source).expect("lib.rs parses");
    let forbidden = file.attrs.iter().any(|attr| {
        attr.path().is_ident("forbid")
            && matches!(&attr.meta, syn::Meta::List(list)
                if list.tokens.to_string().replace(' ', "") == "unsafe_code")
    });
    assert!(
        forbidden,
        "SC-7: src/lib.rs must keep #![forbid(unsafe_code)]"
    );
}

const ROLE_TRAITS: [&str; 5] = ["Provider", "Executor", "Sink", "Link", "Authority"];

/// Every path segment a piece of syntax names: the type names a signature or a
/// field mentions, role traits among them (MA-16).
#[derive(Default)]
struct Named(BTreeSet<String>);
impl<'ast> syn::visit::Visit<'ast> for Named {
    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        self.0.insert(segment.ident.to_string());
        syn::visit::visit_path_segment(self, segment);
    }
}

/// For every struct, enum, type alias and trait in `src/`, by name, what it names: a
/// struct's or enum's generics and fields, an alias's generics and target, a trait's
/// supertraits, method signatures and associated types, and — for any `impl` block,
/// inherent or of a trait — the implemented trait, its associated types and its method
/// signatures, credited to the self type. Items are found wherever syn sees them:
/// inside modules, function bodies and `const _` blocks alike. A name defined twice
/// merges both, so the walk below over-approximates rather than missing one. A type
/// generated by a macro is not seen (MA-16, D72).
fn field_names(items: &[syn::Item], out: &mut BTreeMap<String, BTreeSet<String>>) {
    struct Collect<'o>(&'o mut BTreeMap<String, BTreeSet<String>>);
    impl Collect<'_> {
        fn credit(&mut self, ident: &syn::Ident, named: Named) {
            self.0.entry(ident.to_string()).or_default().extend(named.0);
        }
    }
    impl<'ast> syn::visit::Visit<'ast> for Collect<'_> {
        fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
            let mut named = Named::default();
            syn::visit::Visit::visit_generics(&mut named, &s.generics);
            syn::visit::Visit::visit_fields(&mut named, &s.fields);
            self.credit(&s.ident, named);
        }
        fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
            let mut named = Named::default();
            syn::visit::Visit::visit_generics(&mut named, &e.generics);
            for variant in &e.variants {
                syn::visit::Visit::visit_fields(&mut named, &variant.fields);
            }
            self.credit(&e.ident, named);
        }
        fn visit_item_type(&mut self, alias: &'ast syn::ItemType) {
            let mut named = Named::default();
            syn::visit::Visit::visit_generics(&mut named, &alias.generics);
            syn::visit::Visit::visit_type(&mut named, &alias.ty);
            self.credit(&alias.ident, named);
        }
        fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
            // A role trait's own body is the seed, not a reachable type: it names
            // itself, and the other roles' bodies are checked on their own turn.
            if !ROLE_TRAITS.contains(&t.ident.to_string().as_str()) {
                let mut named = Named::default();
                syn::visit::Visit::visit_item_trait(&mut named, t);
                self.credit(&t.ident, named);
            }
            syn::visit::visit_item_trait(self, t);
        }
        // A method is reachable through any value of its type, inherent or a
        // trait's: `PrepareContext::peer()` and an `impl Iterator for PrepareContext`
        // yielding `&dyn Sink` both hand a peer to whoever holds the context.
        // The self type need not be a plain path: `impl IntoIterator for &Ctx`,
        // `impl dyn Events` and `impl Ext for [Ctx]` reach a `Ctx` or an `Events`
        // holder just the same, so every type name in the self type is credited.
        fn visit_item_impl(&mut self, imp: &'ast syn::ItemImpl) {
            let mut named = Named::default();
            if let Some((path, _)) = &imp.trait_ {
                syn::visit::Visit::visit_path(&mut named, path);
            }
            for member in &imp.items {
                match member {
                    syn::ImplItem::Fn(method) => {
                        syn::visit::Visit::visit_signature(&mut named, &method.sig)
                    }
                    syn::ImplItem::Type(assoc) => {
                        syn::visit::Visit::visit_type(&mut named, &assoc.ty)
                    }
                    syn::ImplItem::Const(c) => syn::visit::Visit::visit_type(&mut named, &c.ty),
                    _ => {}
                }
            }
            let mut owners = Named::default();
            syn::visit::Visit::visit_type(&mut owners, &imp.self_ty);
            for owner in owners.0 {
                self.0
                    .entry(owner)
                    .or_default()
                    .extend(named.0.iter().cloned());
            }
            syn::visit::visit_item_impl(self, imp);
        }
    }
    let mut collect = Collect(out);
    for item in items {
        syn::visit::Visit::visit_item(&mut collect, item);
    }
}

fn data_types() -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for (path, text) in sources() {
        let file = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        field_names(&file.items, &mut out);
    }
    out
}

/// The role traits reachable from `seed`: the names themselves, then the fields of
/// every struct or enum they name, transitively. A peer reached through a field of
/// `PrepareContext` is as much a reference to a peer as one in a signature (MA-16,
/// D72).
fn roles_reachable(
    seed: BTreeSet<String>,
    types: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut work: Vec<String> = seed.into_iter().collect();
    while let Some(name) = work.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        work.extend(
            types
                .get(&name)
                .into_iter()
                .flatten()
                .filter(|n| !seen.contains(*n))
                .cloned(),
        );
    }
    seen.into_iter()
        .filter(|n| ROLE_TRAITS.contains(&n.as_str()))
        .collect()
}

/// What a role trait's header names: its supertraits and generics, which are part of
/// every method's contract — a role declared `Provider: Link` hands a Provider a
/// Link (MA-16, D72).
fn role_header_names(role_trait: &syn::ItemTrait) -> BTreeSet<String> {
    let mut header = Named::default();
    for bound in &role_trait.supertraits {
        syn::visit::Visit::visit_type_param_bound(&mut header, bound);
    }
    syn::visit::Visit::visit_generics(&mut header, &role_trait.generics);
    header.0
}

#[test]
fn ma_16_role_trait_signatures_do_not_name_peer_roles() {
    let types = data_types();
    let path = crate_dir().join("src/module_api.rs");
    let source = std::fs::read_to_string(&path).expect("module_api.rs is readable");
    let file = syn::parse_file(&source).expect("module_api.rs parses");
    let mut checked = 0;
    for item in &file.items {
        let syn::Item::Trait(role_trait) = item else {
            continue;
        };
        let role = role_trait.ident.to_string();
        if !ROLE_TRAITS.contains(&role.as_str()) {
            continue;
        }
        checked += 1;
        let peers: Vec<String> = roles_reachable(role_header_names(role_trait), &types)
            .into_iter()
            .filter(|r| r != &role)
            .collect();
        assert!(
            peers.is_empty(),
            "MA-16: {role}'s supertraits or generics reach another role trait: {peers:?}"
        );
        for member in &role_trait.items {
            let mut named = Named::default();
            let what = match member {
                syn::TraitItem::Fn(method) => {
                    syn::visit::Visit::visit_signature(&mut named, &method.sig);
                    method.sig.ident.to_string()
                }
                // An associated type's bounds can name a peer as well (MA-16).
                syn::TraitItem::Type(assoc) => {
                    syn::visit::Visit::visit_trait_item_type(&mut named, assoc);
                    assoc.ident.to_string()
                }
                _ => continue,
            };
            let peers: Vec<String> = roles_reachable(named.0, &types)
                .into_iter()
                .filter(|r| r != &role)
                .collect();
            assert!(
                peers.is_empty(),
                "MA-16: {role}::{what} reaches another role trait through its signature or the \
                 fields of a type it names: {peers:?}"
            );
        }
    }
    assert_eq!(checked, ROLE_TRAITS.len(), "every role trait is found");
}

#[test]
fn ma_16_the_gate_sees_a_peer_behind_a_context_field() {
    // D72: the bypass a signature-only check missed — a peer smuggled in as a field of
    // a context type the signature names, one struct removed.
    let file: syn::File = syn::parse_str(
        "struct Ctx<'a> { inner: Inner<'a> } struct Inner<'a> { peer: Option<&'a mut dyn Sink> }",
    )
    .expect("synthetic items parse");
    let mut types = BTreeMap::new();
    field_names(&file.items, &mut types);
    let signature: syn::Signature =
        syn::parse_str("fn prepare(&mut self, ctx: &mut Ctx<'_>)").expect("parses");
    let mut named = Named::default();
    syn::visit::Visit::visit_signature(&mut named, &signature);
    assert_eq!(
        roles_reachable(named.0, &types)
            .into_iter()
            .collect::<Vec<_>>(),
        ["Sink"]
    );
    // Through a type alias, and through a method of a non-role trait the context
    // holds (the two routes the first extension missed).
    let file: syn::File = syn::parse_str(
        "type PeerHandle<'a> = &'a dyn Sink; struct Ctx<'a> { h: PeerHandle<'a>, e: &'a dyn Events } \
         trait Events { fn peer(&self) -> Option<&dyn Link>; }",
    )
    .expect("synthetic items parse");
    let mut types = BTreeMap::new();
    field_names(&file.items, &mut types);
    let mut named = Named::default();
    syn::visit::Visit::visit_signature(
        &mut named,
        &syn::parse_str("fn prepare(&mut self, ctx: &mut Ctx<'_>)").expect("parses"),
    );
    assert_eq!(
        roles_reachable(named.0, &types)
            .into_iter()
            .collect::<Vec<_>>(),
        ["Link", "Sink"]
    );
    // One peer role per route, each on its own, so deleting any one route from the
    // walk changes the answer (D72).
    let routes = [
        (
            "struct generic",
            "struct Ctx<'a> { h: Hidden<'a> } struct Hidden<'a, S: ?Sized + Link = dyn Link> { s: &'a S }",
        ),
        (
            "enum generic",
            "struct Ctx { h: Choice } enum Choice<S: ?Sized + Link = dyn Link> { A(Box<S>) }",
        ),
        (
            "alias generic",
            "struct Ctx<'a> { h: H<'a> } type H<'a, S = dyn Link> = &'a S;",
        ),
        (
            "inherent method",
            "struct Ctx; impl Ctx { fn peer(&self) -> Option<&'static dyn Link> { None } }",
        ),
        (
            "trait impl",
            "struct Ctx; impl Iterator for Ctx { type Item = &'static dyn Link; fn next(&mut self) -> Option<Self::Item> { None } }",
        ),
        (
            "impl in a fn body",
            "struct Ctx; fn hide() { impl Ctx { fn peer(&self) -> &'static dyn Link { todo!() } } }",
        ),
        (
            "impl in a const block",
            "struct Ctx; const _: () = { impl Ctx { fn peer(&self) -> &'static dyn Link { todo!() } } };",
        ),
        (
            "impl for a reference",
            "struct Ctx; impl<'a> IntoIterator for &'a Ctx { type Item = &'static dyn Link; type IntoIter = std::iter::Empty<Self::Item>; fn into_iter(self) -> Self::IntoIter { todo!() } }",
        ),
        (
            "impl on a trait object",
            "struct Ctx<'a> { e: &'a dyn Events } trait Events {} impl dyn Events { fn peer(&self) -> Option<&dyn Link> { None } }",
        ),
        (
            "impl assoc const",
            "struct Ctx; impl Ctx { const P: Option<&'static dyn Link> = None; }",
        ),
        (
            "impl through its trait path",
            "struct Ctx; trait Tag<T: ?Sized> {} impl Tag<dyn Link> for Ctx {}",
        ),
    ];
    for (route, source) in routes {
        let file: syn::File = syn::parse_str(source).expect("synthetic items parse");
        let mut types = BTreeMap::new();
        field_names(&file.items, &mut types);
        let mut named = Named::default();
        syn::visit::Visit::visit_signature(
            &mut named,
            &syn::parse_str("fn prepare(&mut self, ctx: &mut Ctx)").expect("parses"),
        );
        assert_eq!(
            roles_reachable(named.0, &types)
                .into_iter()
                .collect::<Vec<_>>(),
            ["Link"],
            "{route}"
        );
    }
    // A role trait's supertraits are checked like its signatures.
    let role: syn::ItemTrait =
        syn::parse_str("trait Provider<T: Sink>: Send + Link {}").expect("parses");
    assert_eq!(
        roles_reachable(role_header_names(&role), &BTreeMap::new())
            .into_iter()
            .collect::<Vec<_>>(),
        ["Link", "Sink"]
    );
    // And directly in a signature, as before.
    let signature: syn::Signature =
        syn::parse_str("fn peer(&self, value: &dyn Sink)").expect("parses");
    let mut named = Named::default();
    syn::visit::Visit::visit_signature(&mut named, &signature);
    assert_eq!(
        roles_reachable(named.0, &BTreeMap::new())
            .into_iter()
            .collect::<Vec<_>>(),
        ["Sink"]
    );
}

/// The gate's own predicates, against the spellings that walked past them.
///
/// Five evasions were demonstrated against this file and fixed here; without a
/// fixture the fixes are only as good as the reading that produced them, and one of
/// the five was open in the **shipped** crate rather than a demonstration. These pin
/// the three predicates whose bug was in the predicate itself; `extern crate` and
/// `include!` are single match arms whose absence the walker's own asserts state.
///
/// Rule: OV-23, X11.
#[test]
fn ov_23_the_gate_predicates_answer_the_demonstrated_evasions() {
    let attrs_of = |src: &str| -> Vec<syn::Attribute> {
        let f: syn::File = syn::parse_str(src).expect("the fixture parses");
        match f.items.into_iter().next().expect("one item") {
            syn::Item::Mod(m) => m.attrs,
            _ => panic!("the fixture declares a mod"),
        }
    };
    // The feature gate OV-20 gives one allow-list line.
    assert!(is_testing_gated(&attrs_of(
        r#"#[cfg(feature = "testing")] mod x {}"#
    )));
    // And its negation, which is the **default** build: matching any `cfg` whose
    // tokens contain "testing" skipped items that ship.
    assert!(!is_testing_gated(&attrs_of(
        r#"#[cfg(not(feature = "testing"))] mod x {}"#
    )));
    assert!(!is_testing_gated(&attrs_of(
        r#"#[cfg(all(unix, feature = "testing"))] mod x {}"#
    )));
    assert!(!is_testing_gated(&attrs_of("mod x {}")));

    let use_tree = |src: &str| -> syn::UseTree {
        let f: syn::File = syn::parse_str(src).expect("the fixture parses");
        let syn::Item::Use(u) = f.items.into_iter().next().expect("one item") else {
            panic!("the fixture declares a use")
        };
        u.tree
    };
    let roots = |src: &str| -> Vec<String> {
        let mut out = Vec::new();
        use_roots(&use_tree(src), &mut out);
        out
    };
    assert_eq!(roots("pub use crate::spec::Value;"), ["crate"]);
    // A brace group has one root per branch. Returning the empty string for the group
    // put it in the accepted set, so a dependency's type re-exported inside braces
    // walked past the check written to refuse exactly that.
    assert_eq!(
        roots("pub use {serde_json::Value as X, crate::spec::Key};"),
        ["serde_json", "crate"]
    );
    assert_eq!(roots("pub use crate::{spec::Key, id::Ident};"), ["crate"]);
    // A glob is refused wherever it sits: the root check short-circuits at the first
    // segment, so `pub use crate::internal::*;` named a module of this crate and put
    // every item behind it on the surface with no allow-list line at all.
    assert!(has_glob(&use_tree("pub use crate::internal::*;")));
    assert!(has_glob(&use_tree("pub use crate::{spec::Key, id::*};")));
    assert!(!has_glob(&use_tree(
        "pub use crate::{spec::Key, id::Ident};"
    )));

    // Two inline modules of one file that declare a same-named item are two items.
    // Keying on depth collapsed them onto one key, so the second inherited the
    // first's allow-list entry and the gate reported one missing item for two.
    let f = std::path::Path::new("src/x.rs");
    assert_ne!(
        allow_key(f, &["m1".to_owned()], "Dup"),
        allow_key(f, &["m2".to_owned()], "Dup")
    );
    assert_ne!(
        allow_key(f, &[], "Dup"),
        allow_key(f, &["m1".to_owned()], "Dup")
    );
}

/// The `macro_rules!` definitions `src/` may hold, by name. A macro can emit an item
/// the MA-16 walk never sees, so each one is reviewed and listed here (D100).
const LISTED_MACROS: [&str; 2] = ["display_newtype", "document_inserts"];

/// Every rename (`use … as`, `extern crate … as`) and every `macro_rules!` outside
/// [`LISTED_MACROS`] in one parsed file, wherever it sits — a module, a function body,
/// a `const _` block. The MA-16 walk reads names, so a rename or a macro-emitted type
/// is invisible to it; refusing the two mechanisms closes that class, as OV-23 refuses
/// `include!`, where listing what they can spell did not (MA-16, D100).
fn renames_and_macros(file: &syn::File) -> Vec<String> {
    struct Find(Vec<String>);
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_use_rename(&mut self, r: &'ast syn::UseRename) {
            self.0.push(format!("use {} as {}", r.ident, r.rename));
            syn::visit::visit_use_rename(self, r);
        }
        fn visit_item_extern_crate(&mut self, e: &'ast syn::ItemExternCrate) {
            if let Some((_, rename)) = &e.rename {
                self.0.push(format!("extern crate {} as {rename}", e.ident));
            }
            syn::visit::visit_item_extern_crate(self, e);
        }
        fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
            if m.mac.path.is_ident("macro_rules") {
                let name = m.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                if !LISTED_MACROS.contains(&name.as_str()) {
                    self.0.push(format!("macro_rules! {name}"));
                }
            }
            syn::visit::visit_item_macro(self, m);
        }
    }
    let mut find = Find(Vec::new());
    syn::visit::Visit::visit_file(&mut find, file);
    find.0
}

#[test]
fn ma_16_the_names_the_walk_reads_are_the_names_declared() {
    for (path, text) in sources() {
        let file = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let found = renames_and_macros(&file);
        assert!(
            found.is_empty(),
            "MA-16: {} holds {found:?}; the peer walk reads names, so a rename or an unlisted \
             macro is refused rather than followed",
            path.display()
        );
    }
    // The predicate's own red test: each spelling is caught once, wherever it sits.
    for (what, source) in [
        (
            "a renamed import",
            "use crate::module_api::Sink as Fragment;",
        ),
        (
            "a rename in a brace group",
            "use crate::module_api::{Provider, Sink as Peer};",
        ),
        (
            "a rename inside a fn body",
            "fn hide() { use crate::module_api::Sink as Peer; }",
        ),
        ("a renamed extern crate", "extern crate serde as s;"),
        ("an unlisted macro", "macro_rules! hidden { () => {} }"),
        (
            "an unlisted macro inside a fn body",
            "fn hide() { macro_rules! hidden { () => {} } }",
        ),
        (
            "an unlisted macro inside a const block",
            "const _: () = { macro_rules! hidden { () => {} } };",
        ),
    ] {
        let file: syn::File = syn::parse_str(source).expect("the fixture parses");
        assert_eq!(renames_and_macros(&file).len(), 1, "{what}");
    }
    let listed: syn::File =
        syn::parse_str("macro_rules! display_newtype { () => {} } use std::fmt;").expect("parses");
    assert!(
        renames_and_macros(&listed).is_empty(),
        "a listed macro and a plain import pass"
    );
}

fn assert_document<T: serde::Serialize + serde::de::DeserializeOwned + schemars::JsonSchema>() {}

/// MA-6's document types that role signatures name, each asserted to be a document at
/// compile time; the names they are matched by come from the same list.
macro_rules! ma6_documents {
    ($($($segment:ident)::+),* $(,)?) => {
        fn ma6_documents() -> Vec<&'static str> {
            $( assert_document::<ezsdr_kernel::$($segment)::+>(); )*
            vec![$( stringify!($($segment)::+).rsplit(':').next().expect("a name").trim() ),*]
        }
    };
}
ma6_documents! {
    module_api::ProviderInstance, module_api::Requested, module_api::CoerceReport,
    module_api::ModuleError, module_api::StopMode, module_api::StepOutcome,
    module_api::ExecutorDescriptor, module_api::IslandDecl, module_api::SinkDescriptor,
    module_api::LinkDescriptor, module_api::AuthorityDescriptor, plan::Fragment,
    plan::PrepareReport, time::TimePoint, manifest::ArtifactRef, stream::DataLinkDecl,
}

/// MA-6's Kernel handles, and the wrappers it allows over its categories.
const MA6_HANDLES: [&str; 7] = [
    "PrepareContext",
    "EventSink",
    "ActionReceiver",
    "ActionSubmitter",
    "Endpoint",
    "DataLink",
    "TimeAuthority",
];
const MA6_WRAPPERS: [&str; 5] = ["Option", "Result", "Vec", "Box", "Arc"];

/// What in one signature MA-6 does not allow: a name outside its categories, a slice, a
/// bare `fn`, an `impl Trait` or a generic parameter. A closure is `dyn Fn…`, whose name
/// is outside the list (MA-6, D100).
fn ma6_violations(sig: &syn::Signature, allowed: &BTreeSet<&str>) -> Vec<String> {
    struct Check<'a> {
        allowed: &'a BTreeSet<&'a str>,
        out: Vec<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for Check<'_> {
        fn visit_type(&mut self, t: &'ast syn::Type) {
            match t {
                syn::Type::Slice(_) => self.out.push("a slice".to_owned()),
                syn::Type::FnPtr(_) => self.out.push("a bare fn".to_owned()),
                syn::Type::ImplTrait(_) => self.out.push("impl Trait".to_owned()),
                _ => {}
            }
            syn::visit::visit_type(self, t);
        }
        fn visit_path_segment(&mut self, s: &'ast syn::PathSegment) {
            let name = s.ident.to_string();
            if !self.allowed.contains(name.as_str()) {
                self.out.push(name);
            }
            syn::visit::visit_path_segment(self, s);
        }
    }
    let mut check = Check {
        allowed,
        out: Vec::new(),
    };
    for input in &sig.inputs {
        if let syn::FnArg::Typed(t) = input {
            syn::visit::Visit::visit_type(&mut check, &t.ty);
        }
    }
    if let syn::ReturnType::Type(_, t) = &sig.output {
        syn::visit::Visit::visit_type(&mut check, t);
    }
    if sig
        .generics
        .params
        .iter()
        .any(|p| !matches!(p, syn::GenericParam::Lifetime(_)))
    {
        check.out.push("a generic parameter".to_owned());
    }
    check.out
}

#[test]
fn ma_06_role_signatures_name_only_documents_and_handles() {
    let allowed: BTreeSet<&str> = ma6_documents()
        .into_iter()
        .chain(MA6_HANDLES)
        .chain(MA6_WRAPPERS)
        .collect();
    let source = std::fs::read_to_string(crate_dir().join("src/module_api.rs")).expect("readable");
    let file = syn::parse_file(&source).expect("module_api.rs parses");
    let mut checked = 0;
    for item in &file.items {
        let syn::Item::Trait(role) = item else {
            continue;
        };
        if !ROLE_TRAITS.contains(&role.ident.to_string().as_str()) {
            continue;
        }
        checked += 1;
        // MA-2: no shared lifecycle supertrait; the only bounds are the auto traits
        // `Send` (MA-5) and `Sync`.
        let bounds: Vec<String> = role
            .supertraits
            .iter()
            .map(|b| match b {
                syn::TypeParamBound::Trait(t) => t
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
                _ => "a non-trait bound".to_owned(),
            })
            .collect();
        assert!(
            bounds.iter().all(|b| b == "Send" || b == "Sync"),
            "MA-2: {} has a supertrait other than Send and Sync: {bounds:?}",
            role.ident
        );
        for member in &role.items {
            if let syn::TraitItem::Fn(method) = member {
                let bad = ma6_violations(&method.sig, &allowed);
                assert!(
                    bad.is_empty(),
                    "MA-6: {}::{} names {bad:?}",
                    role.ident,
                    method.sig.ident
                );
            }
        }
    }
    assert_eq!(checked, ROLE_TRAITS.len(), "every role trait is found");
    // MA-46: each document a role signature carries has a frozen schema of its own, so
    // the list cannot again fall behind the signatures (D104).
    let frozen = ezsdr_kernel::schema::document_schemas();
    for doc in ma6_documents() {
        let snake = doc
            .chars()
            .enumerate()
            .fold(String::new(), |mut s, (i, c)| {
                if c.is_ascii_uppercase() && i > 0 {
                    s.push('_');
                }
                s.push(c.to_ascii_lowercase());
                s
            });
        assert!(
            frozen.contains_key(snake.as_str()),
            "MA-46: {doc} has no schema under schemas/"
        );
    }
    // The check's own red test, one per thing MA-6 names.
    for (what, sig) in [
        ("a raw slice", "fn f(&self, bytes: &[u8])"),
        ("a closure", "fn f(&self, f: Box<dyn Fn(u32)>)"),
        (
            "an iterator",
            "fn f(&self) -> impl Iterator<Item = TimePoint>",
        ),
        ("a bare fn", "fn f(&self, f: fn(u32))"),
        ("a generic parameter", "fn f<T>(&self, x: T)"),
        ("a type outside the list", "fn f(&self, x: &String)"),
    ] {
        let sig: syn::Signature = syn::parse_str(sig).expect("parses");
        assert!(!ma6_violations(&sig, &allowed).is_empty(), "{what}");
    }
    let fine: syn::Signature = syn::parse_str(
        "fn stop(&mut self, mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError>",
    )
    .expect("parses");
    assert!(ma6_violations(&fine, &allowed).is_empty());
}
