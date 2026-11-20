//! Tests for affected-package selection (src/affected.rs): `cargo metadata`-shaped JSON in, D / R(D) / reasons out. No
//! cargo runs; every fixture is built here in cargo's `--format-version 1` shape (with `resolve`).

// The binary crate has no lib target, so the module and the JSON parser it uses are included by path.
#[path = "../src/affected.rs"]
mod affected;
#[path = "../src/json.rs"]
mod json;

use affected::{select, Picked, Selection};
use json::J;

fn s(v: &str) -> J {
    J::Str(v.to_string())
}
fn obj(m: Vec<(&str, J)>) -> J {
    J::Obj(m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A package of the fixture: name, folder relative to the root ("" = the root itself; None = not a member, outside it).
struct P<'a> {
    name: &'a str,
    dir: Option<&'a str>,
}

/// How a node lists its dependencies, so both forms cargo emits are covered.
#[derive(Clone, Copy)]
enum Form {
    Both,
    DependenciesOnly,
    DepsOnly,
}

fn id(name: &str) -> String {
    format!("path+file:///ws/{name}#{name}@0.1.0")
}

/// Metadata for `pkgs` under `root` (with `sep` as the path separator), with `edges` (from, to, kind: "" normal, "dev",
/// "build"): `from` depends on `to`.
fn meta_with(
    root: &str,
    sep: &str,
    pkgs: &[P],
    edges: &[(&str, &str, &str)],
    form: Form,
) -> String {
    let manifest = |p: &P| match p.dir {
        Some("") => format!("{root}{sep}Cargo.toml"),
        Some(d) => format!("{root}{sep}{}{sep}Cargo.toml", d.replace('/', sep)),
        None => format!("{sep}elsewhere{sep}{}{sep}Cargo.toml", p.name),
    };
    let packages = pkgs
        .iter()
        .map(|p| {
            obj(vec![
                ("name", s(p.name)),
                ("version", s("0.1.0")),
                ("id", s(&id(p.name))),
                ("manifest_path", s(&manifest(p))),
            ])
        })
        .collect();
    let members = pkgs
        .iter()
        .filter(|p| p.dir.is_some())
        .map(|p| s(&id(p.name)))
        .collect();
    let nodes = pkgs
        .iter()
        .map(|p| {
            let mine: Vec<_> = edges.iter().filter(|e| e.0 == p.name).collect();
            let mut node = vec![("id", s(&id(p.name)))];
            if !matches!(form, Form::DepsOnly) {
                node.push((
                    "dependencies",
                    J::Arr(mine.iter().map(|e| s(&id(e.1))).collect()),
                ));
            }
            if !matches!(form, Form::DependenciesOnly) {
                let deps = mine
                    .iter()
                    .map(|e| {
                        let kind = if e.2.is_empty() { J::Null } else { s(e.2) };
                        obj(vec![
                            ("name", s(e.1)),
                            ("pkg", s(&id(e.1))),
                            (
                                "dep_kinds",
                                J::Arr(vec![obj(vec![("kind", kind), ("target", J::Null)])]),
                            ),
                        ])
                    })
                    .collect();
                node.push(("deps", J::Arr(deps)));
            }
            node.push(("features", J::Arr(vec![])));
            obj(node)
        })
        .collect();
    obj(vec![
        ("packages", J::Arr(packages)),
        ("workspace_members", J::Arr(members)),
        (
            "resolve",
            obj(vec![("nodes", J::Arr(nodes)), ("root", J::Null)]),
        ),
        ("target_directory", s(&format!("{root}{sep}target"))),
        ("version", J::Num("1".into())),
        ("workspace_root", s(root)),
        ("metadata", J::Null),
    ])
    .to_json()
}

/// leaf <- mid <- top (three levels), an unrelated `other`, and a registry crate `serde` that leaf uses.
fn three_level(form: Form) -> String {
    let pkgs = [
        P {
            name: "leaf",
            dir: Some("crates/leaf"),
        },
        P {
            name: "mid",
            dir: Some("crates/mid"),
        },
        P {
            name: "top",
            dir: Some("crates/top"),
        },
        P {
            name: "other",
            dir: Some("crates/other"),
        },
        P {
            name: "serde",
            dir: None,
        },
    ];
    meta_with(
        r"C:\ws",
        r"\",
        &pkgs,
        &[
            ("mid", "leaf", ""),
            ("top", "mid", ""),
            ("leaf", "serde", ""),
        ],
        form,
    )
}

fn names(v: &[Picked]) -> Vec<&str> {
    v.iter().map(|p| p.name.as_str()).collect()
}

fn packages(sel: Selection) -> (Vec<Picked>, Vec<Picked>) {
    match sel {
        Selection::Packages { direct, affected } => (direct, affected),
        Selection::Full(why) => panic!("expected a package selection, got Full({why:?})"),
    }
}

fn full(sel: Selection) -> Vec<String> {
    match sel {
        Selection::Full(why) => {
            assert!(!why.is_empty(), "Full must say why");
            why
        }
        other => panic!("expected Full, got {other:?}"),
    }
}

fn reason<'a>(v: &'a [Picked], name: &str) -> &'a str {
    &v.iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{name} not selected: {v:?}"))
        .reason
}

#[test]
fn three_level_reverse_dependency_selects_the_leafs_transitive_dependent() {
    let (direct, affected) =
        packages(select(&three_level(Form::Both), &["crates/leaf/src/lib.rs"]).unwrap());
    println!("direct = {direct:?}\naffected = {affected:?}");
    assert_eq!(names(&direct), ["leaf"]);
    assert_eq!(
        names(&affected),
        ["leaf", "mid", "top"],
        "top depends on leaf through mid and must be selected; other must not"
    );
    assert_eq!(reason(&direct, "leaf"), "changed: crates/leaf/src/lib.rs");
    assert_eq!(reason(&affected, "mid"), "depends on leaf (changed)");
    assert_eq!(reason(&affected, "top"), "depends on mid -> leaf (changed)");
}

#[test]
fn a_change_in_the_middle_selects_only_it_and_what_is_above() {
    let (direct, affected) =
        packages(select(&three_level(Form::Both), &["crates/mid/src/lib.rs"]).unwrap());
    assert_eq!(names(&direct), ["mid"]);
    assert_eq!(names(&affected), ["mid", "top"]);
}

#[test]
fn both_forms_of_a_node_give_the_same_graph() {
    for form in [Form::Both, Form::DependenciesOnly, Form::DepsOnly] {
        let (_, affected) =
            packages(select(&three_level(form), &["crates/leaf/build.rs"]).unwrap());
        assert_eq!(names(&affected), ["leaf", "mid", "top"]);
    }
}

#[test]
fn dev_and_build_dependencies_count() {
    let pkgs = [
        P {
            name: "a",
            dir: Some("a"),
        },
        P {
            name: "b",
            dir: Some("b"),
        },
        P {
            name: "c",
            dir: Some("c"),
        },
    ];
    let m = meta_with(
        "/ws",
        "/",
        &pkgs,
        &[("b", "a", "dev"), ("c", "a", "build")],
        Form::DepsOnly,
    );
    let (_, affected) = packages(select(&m, &["a/src/lib.rs"]).unwrap());
    assert_eq!(names(&affected), ["a", "b", "c"]);
}

#[test]
fn a_dev_dependency_cycle_terminates() {
    let pkgs = [
        P {
            name: "a",
            dir: Some("a"),
        },
        P {
            name: "b",
            dir: Some("b"),
        },
    ];
    let m = meta_with(
        "/ws",
        "/",
        &pkgs,
        &[("b", "a", ""), ("a", "b", "dev")],
        Form::Both,
    );
    let (direct, affected) = packages(select(&m, &["a/src/lib.rs"]).unwrap());
    assert_eq!(names(&direct), ["a"]);
    assert_eq!(names(&affected), ["a", "b"]);
    assert_eq!(
        reason(&affected, "a"),
        "changed: a/src/lib.rs",
        "a changed package keeps its own reason"
    );
}

#[test]
fn a_dependency_path_through_a_non_member_is_followed() {
    let pkgs = [
        P {
            name: "leaf",
            dir: Some("leaf"),
        },
        P {
            name: "shim",
            dir: None,
        },
        P {
            name: "top",
            dir: Some("top"),
        },
    ];
    let m = meta_with(
        "/ws",
        "/",
        &pkgs,
        &[("shim", "leaf", ""), ("top", "shim", "")],
        Form::Both,
    );
    let (_, affected) = packages(select(&m, &["leaf/src/lib.rs"]).unwrap());
    assert_eq!(
        names(&affected),
        ["leaf", "top"],
        "a non-member is never selected, but what depends on it is"
    );
    assert_eq!(
        reason(&affected, "top"),
        "depends on shim -> leaf (changed)"
    );
}

#[test]
fn several_changed_packages_and_files() {
    let changed = [
        "crates/other/src/a.rs",
        "crates/leaf/src/a.rs",
        "crates/leaf/src/b.rs",
        "crates/leaf/src/c.rs",
        "crates/leaf/src/d.rs",
        "crates/leaf/src/a.rs",
    ];
    let (direct, affected) = packages(select(&three_level(Form::Both), &changed).unwrap());
    assert_eq!(names(&direct), ["leaf", "other"]);
    assert_eq!(names(&affected), ["leaf", "mid", "other", "top"]);
    assert_eq!(
        reason(&direct, "leaf"),
        "changed: crates/leaf/src/a.rs, crates/leaf/src/b.rs, crates/leaf/src/c.rs and 1 more"
    );
}

#[test]
fn backslash_paths_map_and_member_manifests_are_not_global() {
    let (direct, affected) =
        packages(select(&three_level(Form::Both), &[r"crates\leaf\Cargo.toml"]).unwrap());
    assert_eq!(names(&direct), ["leaf"]);
    assert_eq!(reason(&direct, "leaf"), "changed: crates/leaf/Cargo.toml");
    assert_eq!(names(&affected), ["leaf", "mid", "top"]);
}

#[test]
fn global_files_select_full() {
    let m = three_level(Form::Both);
    for g in [
        "Cargo.lock",
        "Cargo.toml",
        "cargo.TOML",
        "CARGO.LOCK",
        "crates/leaf/Cargo.lock",
        "broadsword.toml",
        "rust-toolchain",
        "rust-toolchain.toml",
        ".cargo/config.toml",
        "crates/leaf/.cargo/config",
        r".Cargo\config.toml",
    ] {
        let why = full(select(&m, &["crates/leaf/src/lib.rs", g]).unwrap());
        assert_eq!(why, [format!("global file changed: {g}")], "{g}");
    }
}

#[test]
fn unmapped_files_select_full_and_every_cause_is_named() {
    let m = three_level(Form::Both);
    let why = full(
        select(
            &m,
            &[
                "README.md",
                "crates/leaf/src/lib.rs",
                "Cargo.lock",
                "crates/leafy/src/lib.rs",
            ],
        )
        .unwrap(),
    );
    assert_eq!(
        why,
        [
            "changed file maps to no workspace package: README.md",
            "global file changed: Cargo.lock",
            "changed file maps to no workspace package: crates/leafy/src/lib.rs"
        ]
    );
    // A case mismatch, a package folder itself and a deleted package all map to nothing.
    for p in [
        "Crates/leaf/src/lib.rs",
        "crates/leaf",
        "crates/gone/src/lib.rs",
    ] {
        assert_eq!(
            full(select(&m, &[p]).unwrap()),
            [format!("changed file maps to no workspace package: {p}")]
        );
    }
}

#[test]
fn no_changed_files_selects_full() {
    assert_eq!(
        full(select(&three_level(Form::Both), &[]).unwrap()),
        ["no changed files were given"]
    );
}

#[test]
fn a_root_package_takes_what_no_deeper_member_holds() {
    let pkgs = [
        P {
            name: "app",
            dir: Some(""),
        },
        P {
            name: "lib",
            dir: Some("crates/lib"),
        },
    ];
    let m = meta_with("/ws", "/", &pkgs, &[("app", "lib", "")], Form::Both);
    let (direct, affected) = packages(select(&m, &["README.md", "src/main.rs"]).unwrap());
    assert_eq!(names(&direct), ["app"]);
    assert_eq!(names(&affected), ["app"]);
    let (direct, affected) = packages(select(&m, &["crates/lib/src/lib.rs"]).unwrap());
    assert_eq!(names(&direct), ["lib"], "the longest prefix wins");
    assert_eq!(names(&affected), ["app", "lib"]);
    assert_eq!(
        full(select(&m, &["Cargo.toml"]).unwrap()),
        ["global file changed: Cargo.toml"],
        "the root manifest is still the workspace manifest"
    );
}

#[test]
fn the_workspace_root_prefix_ignores_drive_letter_case() {
    let pkgs = [P {
        name: "a",
        dir: Some("a"),
    }];
    let good = meta_with(r"C:\ws", r"\", &pkgs, &[], Form::Both);
    let root = format!("\"workspace_root\":{}", json::string(r"C:\ws"));
    let m = good.replace(
        &root,
        &format!("\"workspace_root\":{}", json::string(r"c:\ws")),
    );
    assert_ne!(m, good, "the fixture edit did not apply");
    let (direct, _) = packages(select(&m, &["a/x.rs"]).unwrap());
    assert_eq!(names(&direct), ["a"]);
}

#[test]
fn bad_changed_paths_are_errors() {
    let m = three_level(Form::Both);
    for (p, want) in [
        ("", "is empty"),
        ("/crates/leaf/src/lib.rs", "is absolute"),
        (r"\crates\leaf\src\lib.rs", "is absolute"),
        (r"C:\ws\crates\leaf\src\lib.rs", "holds `:`"),
        ("crates/leaf/src/lib.rs:stream", "holds `:`"),
        ("crates/../crates/leaf/src/lib.rs", "`..`"),
        ("./crates/leaf/src/lib.rs", "`.`"),
        ("crates//leaf/src/lib.rs", "empty component"),
        ("crates/leaf/src/lib.rs/", "empty component"),
        ("crates/leaf/src/lib.rs\r", "control character"),
        ("crates/leaf/a\nCargo.lock", "control character"),
        ("\"crates/leaf/src/\\303\\251.rs\"", "is quoted"),
    ] {
        let e = select(&m, &["crates/leaf/src/lib.rs", p]).unwrap_err();
        assert!(e.contains(want), "{p:?}: {e}");
    }
    // A bad path is reported even when another path alone would already give Full.
    assert!(select(&m, &["Cargo.lock", "../x"]).is_err());
}

fn meta_error(text: &str) -> String {
    select(text, &["crates/leaf/src/lib.rs"]).unwrap_err()
}

#[test]
fn unreadable_or_untrustworthy_metadata_is_an_error() {
    let good = three_level(Form::Both);
    let leaf = id("leaf");
    let cases: Vec<(String, &str)> = vec![
        (String::new(), "cannot read cargo metadata"),
        ("not json".into(), "cannot read cargo metadata"),
        (
            good[..good.len() - 1].to_string(),
            "cannot read cargo metadata",
        ),
        ("[]".into(), "workspace_root"),
        (
            good.replace(
                "\"resolve\":{\"nodes\"",
                "\"resolve\":null,\"x\":{\"nodes\"",
            ),
            "--no-deps",
        ),
        (
            good.replace("\"resolve\":", "\"resolved\":"),
            "no `resolve`",
        ),
        (
            good.replace("\"workspace_members\":", "\"members\":"),
            "workspace_members",
        ),
        (
            good.replace(
                "\"workspace_members\":[",
                "\"workspace_members\":[\"ghost\",",
            ),
            "`ghost` is not in `packages`",
        ),
        (
            good.replace("\"workspace_members\":[", "\"workspace_members\":[7,"),
            "not a string",
        ),
        (good.replace("\"packages\":", "\"pkgs\":"), "`packages`"),
        (
            good.replace("\"workspace_root\":\"C:\\\\ws\"", "\"workspace_root\":\"\""),
            "`workspace_root` is empty",
        ),
        (
            good.replace(
                "\"workspace_root\":\"C:\\\\ws\"",
                "\"workspace_root\":\"C:\\\\other\"",
            ),
            "outside the workspace root",
        ),
        (
            good.replace(
                "\"pkg\":\"path+file:///ws/leaf#leaf@0.1.0\"",
                "\"pkg\":\"nowhere\"",
            ),
            "`nowhere`, which is not in `packages`",
        ),
        (
            good.replace(
                "\"dependencies\":[\"path+file:///ws/leaf#leaf@0.1.0\"]",
                "\"dependencies\":[\"nowhere\"]",
            ),
            "`nowhere`, which is not in `packages`",
        ),
        (
            good.replace(
                &format!("{{\"id\":{}", json::string(&leaf)),
                "{\"id\":\"stray\"",
            ),
            "`stray` is not in `packages`",
        ),
        (
            good.replace(
                "crates\\\\leaf\\\\Cargo.toml",
                "crates\\\\leaf\\\\Other.toml",
            ),
            "is not a Cargo.toml",
        ),
        (
            good.replace(
                "crates\\\\mid\\\\Cargo.toml",
                "crates\\\\leaf\\\\Cargo.toml",
            ),
            "share a folder",
        ),
        (
            good.replace(
                "\"name\":\"leaf\",\"version\"",
                "\"name\":\"-p x\",\"version\"",
            ),
            "not a plain cargo name",
        ),
        (
            good.replace("\"name\":\"leaf\",\"version\"", "\"version\""),
            "no string `name`",
        ),
    ];
    for (text, want) in cases {
        assert_ne!(text, good, "the fixture edit for {want:?} did not apply");
        let e = meta_error(&text);
        assert!(e.contains(want), "want {want:?}, got: {e}");
    }
}

#[test]
fn metadata_graph_gaps_are_errors() {
    // A member with no node, a node listed twice, a node with no dependency list and a package id listed twice.
    let pkgs = [
        P {
            name: "a",
            dir: Some("a"),
        },
        P {
            name: "b",
            dir: Some("b"),
        },
    ];
    let good = meta_with("/ws", "/", &pkgs, &[("b", "a", "")], Form::DependenciesOnly);
    let a_node = "{\"id\":\"path+file:///ws/a#a@0.1.0\",\"dependencies\":[],\"features\":[]}";
    assert!(good.contains(a_node), "{good}");
    let cases = [
        (good.replace(&format!("{a_node},"), ""), "has no node in `resolve`"),
        (good.replace(a_node, &format!("{a_node},{a_node}")), "lists the node"),
        (good.replace(a_node, "{\"id\":\"path+file:///ws/a#a@0.1.0\",\"features\":[]}"), "neither `dependencies` nor `deps`"),
        (good.replacen("\"id\":\"path+file:///ws/b#b@0.1.0\"", "\"id\":\"path+file:///ws/a#a@0.1.0\"", 1), "twice"),
        (good.replace("\"workspace_members\":[\"path+file:///ws/a#a@0.1.0\",\"path+file:///ws/b#b@0.1.0\"]", "\"workspace_members\":[]"), "no workspace members"),
    ];
    for (text, want) in cases {
        assert_ne!(text, good, "the fixture edit for {want:?} did not apply");
        let e = select(&text, &["a/x.rs"]).unwrap_err();
        assert!(e.contains(want), "want {want:?}, got: {e}");
    }
}
