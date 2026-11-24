/// Whether a line ends the done-check section, rather than merely using bold prose.
pub(super) fn is_section_end(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with('#') {
        return true;
    }
    let Some(rest) = t.strip_prefix("**") else {
        return false;
    };
    let Some(k) = rest.find("**") else {
        return false;
    };
    let after = &rest[k + 2..];
    rest[..k].trim_end().ends_with(':') || after.starts_with(':') || after.is_empty()
}

#[cfg(test)]
mod tests {
    use super::is_section_end;
    use std::path::Path;

    #[test]
    fn bold_prose_line_inside_done_check_keeps_later_commands() {
        let brief = "## Done check\n\n1. `broadsword test broadsword --task t-1` -> ok\n\
                     **Both** lines run in the worktree.\n2. `python -I -B x.py` -> ok\n";
        let plan = crate::donecheck::plan(brief, Path::new("projects"), "broadsword", "t-1");
        assert!(plan.found);
        assert_eq!(plan.items.len(), 2);
    }

    #[test]
    fn bold_label_line_ends_done_check() {
        let brief = "## Done check\n\n1. `broadsword test broadsword --task t-1` -> ok\n\
                     **Measure:** later.\n2. `python -I -B x.py` -> ok\n";
        let plan = crate::donecheck::plan(brief, Path::new("projects"), "broadsword", "t-1");
        assert!(plan.found);
        assert_eq!(plan.items.len(), 1);
    }

    #[test]
    fn hash_heading_and_whole_bold_line_end_done_check() {
        for heading in ["## Report", "**Report (under 200 words)**"] {
            let brief = format!(
                "## Done check\n\n1. `broadsword test broadsword --task t-1` -> ok\n\
                 {heading}\n2. `python -I -B x.py` -> ok\n"
            );
            let plan = crate::donecheck::plan(&brief, Path::new("projects"), "broadsword", "t-1");
            assert!(plan.found, "{heading}");
            assert_eq!(plan.items.len(), 1, "{heading}");
        }
    }

    #[test]
    fn is_section_end_cases() {
        for (line, expected) in [
            ("## Report", true),
            ("**Measure:** x", true),
            ("**Measure**: x", true),
            ("**Report (under 200 words)**", true),
            ("**Both** commands run.", false),
            ("**not** run", false),
            ("**unclosed", false),
            ("1. **bold** step", false),
            ("", false),
            (" \t**Measure: \t** x\r\n", true),
            ("**Measure** : x", false),
            ("**é:** x", true),
            ("**é** prose **later:**", false),
            ("****", true),
            (" \r\n", false),
        ] {
            assert_eq!(is_section_end(line), expected, "{line:?}");
        }
    }
}
