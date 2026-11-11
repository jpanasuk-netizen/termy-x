//! Termy XL log file. Secrets are stripped before anything is written.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const MAX_BYTES: u64 = 256 * 1024;

pub fn redact_log(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        let marker = ["bearer ", "token=", "api_key=", "apikey="]
            .iter()
            .filter_map(|needle| lower.find(needle).map(|at| (at, needle.len())))
            .min_by_key(|(at, _)| *at);
        let Some((at, skip)) = marker else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at + skip]);
        rest = &rest[at + skip..];
        let value_end = rest
            .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'' || ch == ',')
            .unwrap_or(rest.len());
        out.push_str("[redacted]");
        rest = &rest[value_end..];
    }
    out
}

pub fn append_desk_log(root: &Path, line: &str) {
    append_desk_log_limited(root, line, MAX_BYTES);
}

pub fn append_desk_log_limited(root: &Path, line: &str, max_bytes: u64) {
    let dir = root.join("logs");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("termy-xl.log");
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() > max_bytes
    {
        let _ = fs::rename(&path, dir.join("termy-xl.log.1"));
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = writeln!(file, "{}", redact_log(line));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_stripped_and_the_file_rotates() {
        let line = redact_log("probe failed Bearer super-secret-value token=abc123 done");
        assert!(!line.contains("super-secret-value"));
        assert!(!line.contains("abc123"));
        assert!(line.contains("[redacted]"));
        let dir = std::env::temp_dir().join(format!("xl-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        append_desk_log_limited(&dir, "0123456789", 8);
        append_desk_log_limited(&dir, "next", 8);
        assert!(dir.join("logs").join("termy-xl.log").is_file());
        assert!(dir.join("logs").join("termy-xl.log.1").is_file());
        let kept = std::fs::read_to_string(dir.join("logs").join("termy-xl.log")).unwrap();
        assert!(!kept.contains("super"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
