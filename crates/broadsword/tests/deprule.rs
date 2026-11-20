//! Tests for the mode dependency rule check. Broadsword is a binary crate (no lib.rs), so this test includes the two
//! source files it needs by path; `deprule.rs` reaches the JSON parser as `crate::json`, which resolves to the copy
//! included here.

#[path = "../src/json.rs"]
mod json;

#[path = "../src/deprule.rs"]
mod deprule;

use deprule::{check, DepRuleError, Rules, Violation};

/// `cargo metadata` shaped text: (name, source is registry, dependency names).
fn metadata(pkgs: &[(&str, bool, &[&str])]) -> String {
    let items: Vec<String> = pkgs
        .iter()
        .map(|(name, registry, deps)| {
            let source = if *registry {
                "\"registry+https://github.com/rust-lang/crates.io-index\""
            } else {
                "null"
            };
            let ds: Vec<String> = deps
                .iter()
                .map(|d| format!("{{\"name\":\"{d}\",\"kind\":null,\"req\":\"*\"}}"))
                .collect();
            format!(
                "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"source\":{source},\"dependencies\":[{}]}}",
                ds.join(",")
            )
        })
        .collect();
    format!(
        "{{\"packages\":[{}],\"workspace_members\":[]}}",
        items.join(",")
    )
}

#[test]
fn render_depending_on_physics_is_rejected_naming_both_crates_and_groups() {
    let md = metadata(&[
        ("rend", false, &["phys"]),
        ("phys", false, &["base"]),
        ("base", false, &[]),
    ]);
    let r = Rules::new(
        &[("rend", "render"), ("phys", "physics"), ("base", "core")],
        &SHIPPED,
    );
    let err = check(&md, &r).unwrap_err();
    assert_eq!(
        err,
        DepRuleError::Forbidden(vec![Violation {
            from: "rend".into(),
            from_group: "render".into(),
            to: "phys".into(),
            to_group: "physics".into(),
        }])
    );
    let text = err.to_string();
    for needle in ["rend", "phys", "render", "physics"] {
        assert!(text.contains(needle), "{needle} missing from: {text}");
    }
}

/// The shipped allow table (modes depend on core only), as `Rules::shipped` has it.
const SHIPPED: [(&str, &[&str]); 5] = [
    ("core", &[]),
    ("render", &["core"]),
    ("physics", &["core"]),
    ("game", &["core"]),
    ("app", &["core", "render", "physics", "game"]),
];

#[test]
fn shipped_table_matches_the_table_these_tests_use() {
    let shipped = Rules::shipped();
    for (g, to) in SHIPPED {
        let want: std::collections::BTreeSet<String> = to.iter().map(|s| s.to_string()).collect();
        assert_eq!(shipped.allow.get(g), Some(&want), "group {g}");
    }
    assert_eq!(shipped.allow.len(), SHIPPED.len());
}

#[test]
fn crate_missing_from_the_map_is_an_error_even_without_edges() {
    let md = metadata(&[("base", false, &[]), ("stray", false, &[])]);
    let r = Rules::new(&[("base", "core")], &SHIPPED);
    let err = check(&md, &r).unwrap_err();
    assert_eq!(err, DepRuleError::Unmapped(vec!["stray".into()]));
    assert!(err.to_string().contains("stray"));
}

#[test]
fn group_without_an_allow_row_is_an_error() {
    let md = metadata(&[("base", false, &[])]);
    let r = Rules::new(&[("base", "mystery")], &SHIPPED);
    assert_eq!(
        check(&md, &r).unwrap_err(),
        DepRuleError::UnknownGroup(vec![("base".into(), "mystery".into())])
    );
}

#[test]
fn allowed_edges_and_same_group_edges_pass_and_registry_crates_are_ignored() {
    let md = metadata(&[
        ("app", false, &["rend", "rend2", "base", "serde"]),
        ("rend", false, &["rend2", "base"]),
        ("rend2", false, &["base"]),
        ("base", false, &["serde"]),
        ("serde", true, &["serde_derive"]),
    ]);
    let r = Rules::new(
        &[
            ("app", "app"),
            ("rend", "render"),
            ("rend2", "render"),
            ("base", "core"),
        ],
        &SHIPPED,
    );
    check(&md, &r).unwrap();
}

#[test]
fn every_forbidden_edge_is_listed_sorted_and_the_reverse_direction_is_checked_too() {
    let md = metadata(&[
        ("phys", false, &["rend", "game"]),
        ("rend", false, &["phys"]),
        ("game", false, &[]),
        ("base", false, &["rend"]),
    ]);
    let r = Rules::new(
        &[
            ("phys", "physics"),
            ("rend", "render"),
            ("game", "game"),
            ("base", "core"),
        ],
        &SHIPPED,
    );
    let DepRuleError::Forbidden(v) = check(&md, &r).unwrap_err() else {
        panic!("expected Forbidden")
    };
    let pairs: Vec<(&str, &str)> = v.iter().map(|x| (x.from.as_str(), x.to.as_str())).collect();
    assert_eq!(
        pairs,
        [
            ("base", "rend"),
            ("phys", "game"),
            ("phys", "rend"),
            ("rend", "phys")
        ]
    );
}

#[test]
fn bad_metadata_is_an_error_not_a_pass() {
    let r = Rules::shipped();
    for text in [
        "",
        "{}",
        "{\"packages\":3}",
        "{\"packages\":[{\"source\":null,\"dependencies\":[]}]}",
        "{\"packages\":[{\"name\":\"a\",\"dependencies\":[]}]}",
        "{\"packages\":[{\"name\":\"a\",\"source\":null}]}",
        "{\"packages\":[{\"name\":\"a\",\"source\":null,\"dependencies\":[{}]}]}",
    ] {
        assert!(
            matches!(check(text, &r), Err(DepRuleError::Metadata(_))),
            "accepted: {text}"
        );
    }
}

#[test]
fn a_workspace_with_no_crates_is_an_error_not_a_vacuous_pass() {
    assert!(matches!(
        check(&metadata(&[]), &Rules::shipped()),
        Err(DepRuleError::Metadata(_))
    ));
}
