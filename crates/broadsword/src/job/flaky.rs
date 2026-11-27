//! Lexical validation of the configured flaky-test allow-list path; no file access.

/// Accept the off sentinel or a root-relative path with portable separators.
pub(crate) fn check_flaky_path(value: &str) -> Result<(), String> {
    let has_drive_prefix =
        matches!(value.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    if value.starts_with('/')
        || value.contains('\\')
        || has_drive_prefix
        || value.split('/').any(|part| part == "..")
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(format!(
            "flaky_tests_file = {value:?}: expected a root-relative path using `/`, without `..`, a drive prefix, control characters, or line breaks"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_flaky_path;
    use crate::config::Config;
    use std::path::Path;

    const SHIPPED: &str = include_str!("../../../../broadsword.toml");

    fn config_with(value: &str) -> Result<Config, String> {
        let text = SHIPPED.replace(
            "flaky_tests_file = \"\"",
            &format!("flaky_tests_file = \"{value}\""),
        );
        Config::from_text(&text, Path::new("p"))
    }

    #[test]
    fn flaky_tests_file_empty_is_off() {
        assert_eq!(check_flaky_path(""), Ok(()));
        let cfg = Config::from_text(SHIPPED, Path::new("p")).unwrap();
        assert_eq!(cfg.flaky_tests_file, "");

        // A quoted `#` is part of the path, not the start of a config comment.
        let relative = "state/flaky # tests.txt";
        assert_eq!(config_with(relative).unwrap().flaky_tests_file, relative);
    }

    #[test]
    fn flaky_tests_file_refuses_unsafe_paths() {
        let long_path = format!("{}..", "dir/".repeat(16_384));
        for value in [
            "..",
            "state/../list",
            "/abs",
            "C:x",
            "c:/abs",
            r"a\b",
            "x\n",
            "x\r",
            "x\t",
            "x\0",
            "x\u{2028}",
            "x\u{2029}",
            long_path.as_str(),
        ] {
            let Err(error) = check_flaky_path(value) else {
                panic!("accepted unsafe path: {value:?}");
            };
            assert!(
                error.starts_with(&format!("flaky_tests_file = {value:?}: ")),
                "{error}"
            );
        }

        let value = "state/../list";
        let error = config_with(value).unwrap_err();
        assert!(
            error.starts_with(&format!("flaky_tests_file = {value:?}: ")),
            "{error}"
        );

        // A duplicate must not let an unsafe value replace the shipped empty one.
        let duplicate = format!("{SHIPPED}\nflaky_tests_file = \"{value}\"");
        assert!(
            Config::from_text(&duplicate, Path::new("p")).is_err(),
            "accepted unsafe duplicate value: {value:?}"
        );
    }
}
