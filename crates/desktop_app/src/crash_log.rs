use std::{
    backtrace::Backtrace,
    env, fs,
    io::{self, Write},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const CRASH_LOG_ENV: &str = "TERMY_CRASH_LOG_PATH";

pub(crate) fn install_panic_hook() {
    let previous_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        if let Err(error) = append_panic_report(info) {
            eprintln!("Failed to write Termy crash log: {error}");
        }
        previous_hook(info);
    }));
}

pub(crate) fn crash_log_path() -> PathBuf {
    crash_log_path_with(
        |name| env::var(name).ok(),
        dirs::data_local_dir(),
        env::current_dir().ok(),
    )
}

fn crash_log_path_with(
    get_var: impl Fn(&str) -> Option<String>,
    data_local_dir: Option<PathBuf>,
    current_dir: Option<PathBuf>,
) -> PathBuf {
    if let Some(path) = get_var(CRASH_LOG_ENV).filter(|path| !path.trim().is_empty()) {
        return PathBuf::from(path);
    }

    if let Some(data_local_dir) = data_local_dir {
        return data_local_dir.join("termy").join("crash.log");
    }

    current_dir
        .unwrap_or_else(env::temp_dir)
        .join("termy-crash.log")
}

fn append_panic_report(info: &PanicHookInfo<'_>) -> io::Result<()> {
    let path = crash_log_path();
    let message = panic_message(info);
    let location = info.location().map_or_else(
        || "unknown".to_string(),
        |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        },
    );
    let backtrace = Backtrace::force_capture().to_string();

    append_crash_report(&path, &message, &location, &backtrace)
}

fn append_crash_report(
    path: &Path,
    message: &str,
    location: &str,
    backtrace: &str,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "===== Termy panic =====")?;
    writeln!(file, "time_unix_secs={}", unix_timestamp_secs())?;
    writeln!(file, "version={}", crate::APP_VERSION)?;
    writeln!(file, "thread={:?}", std::thread::current().name())?;
    writeln!(file, "location={location}")?;
    writeln!(file, "message={message}")?;
    writeln!(file, "backtrace:\n{backtrace}")?;
    writeln!(file)?;
    Ok(())
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = info.payload().downcast_ref::<String>() {
        return message.clone();
    }
    "<non-string panic payload>".to_string()
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_log_path_prefers_env_override() {
        let path = crash_log_path_with(
            |name| (name == CRASH_LOG_ENV).then(|| "/tmp/custom-crash.log".to_string()),
            Some(PathBuf::from("/tmp/data")),
            Some(PathBuf::from("/tmp/cwd")),
        );

        assert_eq!(path, PathBuf::from("/tmp/custom-crash.log"));
    }

    #[test]
    fn crash_log_path_uses_local_data_directory() {
        let path = crash_log_path_with(
            |_| None,
            Some(PathBuf::from("/tmp/data")),
            Some(PathBuf::from("/tmp/cwd")),
        );

        assert_eq!(path, PathBuf::from("/tmp/data/termy/crash.log"));
    }

    #[test]
    fn crash_log_path_falls_back_to_current_directory() {
        let path = crash_log_path_with(|_| None, None, Some(PathBuf::from("/tmp/cwd")));

        assert_eq!(path, PathBuf::from("/tmp/cwd/termy-crash.log"));
    }

    #[test]
    fn append_crash_report_creates_parent_and_appends() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("nested").join("crash.log");

        append_crash_report(&path, "first panic", "first.rs:1:2", "first trace")
            .expect("write first crash report");
        append_crash_report(&path, "second panic", "second.rs:3:4", "second trace")
            .expect("write second crash report");

        let contents = fs::read_to_string(path).expect("read crash log");
        assert!(contents.contains("version="));
        assert!(contents.contains("first panic"));
        assert!(contents.contains("first.rs:1:2"));
        assert!(contents.contains("first trace"));
        assert!(contents.contains("second panic"));
        assert!(contents.contains("second.rs:3:4"));
        assert!(contents.contains("second trace"));
        assert_eq!(contents.matches("===== Termy panic =====").count(), 2);
    }
}
