//! A tiny logger: every line is kept in memory and, if a file is set, appended to it.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Logger {
    lines: Mutex<Vec<String>>,
    file: Option<PathBuf>,
}

impl Logger {
    pub fn memory() -> Logger {
        Logger { lines: Mutex::new(Vec::new()), file: None }
    }

    pub fn to_file(path: PathBuf) -> Logger {
        Logger { lines: Mutex::new(Vec::new()), file: Some(path) }
    }

    pub fn info(&self, msg: &str) {
        self.write("INFO", msg);
    }

    pub fn warn(&self, msg: &str) {
        self.write("WARN", msg);
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    fn write(&self, level: &str, msg: &str) {
        let line = format!("[{level}] {msg}");
        if let Some(path) = &self.file {
            // Logging must never fail the operation being logged: a line that cannot be
            // written to the file is dropped, and the in-memory copy below still has it.
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{line}");
            }
        }
        self.lines.lock().unwrap().push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_lines_in_order() {
        let log = Logger::memory();
        log.info("one");
        log.warn("two");
        assert_eq!(log.lines(), vec!["[INFO] one", "[WARN] two"]);
    }
}
