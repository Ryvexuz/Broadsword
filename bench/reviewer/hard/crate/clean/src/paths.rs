//! Turns a job id into the file that holds its record.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum PathError {
    Empty,
    TooLong,
    Absolute,
    Escapes,
}

pub const MAX_ID_LEN: usize = 128;

/// Maps a job id to its record file inside `root`. An id may contain `/` to group jobs in sub-folders.
pub fn record_path(root: &Path, id: &str) -> Result<PathBuf, PathError> {
    if id.is_empty() {
        return Err(PathError::Empty);
    }
    if id.len() > MAX_ID_LEN {
        return Err(PathError::TooLong);
    }
    let rel = Path::new(id);
    if rel.is_absolute() || rel.has_root() {
        return Err(PathError::Absolute);
    }
    for part in rel.components() {
        match part {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(PathError::Escapes),
        }
    }
    Ok(root.join(format!("{id}.job")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_nested_ids() {
        let p = record_path(Path::new("q"), "daily/report-7").unwrap();
        assert!(p.ends_with("report-7.job"));
    }

    #[test]
    fn rejects_a_leading_parent_dir() {
        assert_eq!(record_path(Path::new("q"), "../x"), Err(PathError::Escapes));
    }

    #[test]
    fn rejects_empty_and_long_ids() {
        assert_eq!(record_path(Path::new("q"), ""), Err(PathError::Empty));
        assert_eq!(record_path(Path::new("q"), &"a".repeat(MAX_ID_LEN + 1)), Err(PathError::TooLong));
    }
}
