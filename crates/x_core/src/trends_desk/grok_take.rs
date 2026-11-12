//! Grok takes. The prompt goes in a file, not on the Windows command line.
//! The desk log records the length and the exit status, never the tweet.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::takes::{
    HERMES_TAKE_TIMEOUT, apply_hidden_command, background_grok_spawn, is_provider_failure,
    kill_pid, register_job, takes_halted, unregister_job,
};

/// One line. Newlines and quotes in a tweet stay out of argv.
pub fn flatten_grok_prompt(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{flat} Answer with the reply text only. Do not use tools, search, or browse.")
}

/// Headless take flags for grok 1.0.77. No `--always-approve`, so a draft
/// cannot wander into web search. `--prompt-file` is the single-turn trigger.
pub fn grok_take_args(prompt_file: &Path) -> Vec<String> {
    vec![
        "--prompt-file".into(),
        prompt_file.display().to_string(),
        "--verbatim".into(),
        "--output-format".into(),
        "plain".into(),
        "--no-alt-screen".into(),
        "--disable-web-search".into(),
        "--no-subagents".into(),
        "--no-plan".into(),
        "--max-turns".into(),
        "1".into(),
        "--permission-mode".into(),
        "dontAsk".into(),
    ]
}

pub fn grok_take_log_line(prompt_len: usize, exit: &str, stdout_bytes: usize) -> String {
    format!("INFO grok: prompt_len={prompt_len} exit={exit} stdout_bytes={stdout_bytes}")
}

pub(crate) fn run(post: &super::SourcePost, tone: &str) -> Result<String, String> {
    let prompt = flatten_grok_prompt(&super::agents::reply_prompt(&post.author, &post.text, tone));
    let prompt_len = prompt.len();
    let path = match write_prompt_file(&prompt) {
        Ok(path) => path,
        Err(error) => {
            log_grok(prompt_len, "spawn", 0);
            return Err(error);
        }
    };
    let outcome = run_file(&path, &post.id, prompt_len);
    let _ = std::fs::remove_file(&path);
    outcome
}

fn write_prompt_file(prompt: &str) -> Result<PathBuf, String> {
    let name = format!(
        "termy-xl-grok-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let path = std::env::temp_dir().join(name);
    let mut file = std::fs::File::create(&path).map_err(|error| error.to_string())?;
    file.write_all(prompt.as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(path)
}

fn run_file(path: &Path, post_id: &str, prompt_len: usize) -> Result<String, String> {
    let plan = background_grok_spawn();
    let mut command = std::process::Command::new(plan.program);
    command.args(grok_take_args(path));
    apply_hidden_command(&mut command);
    let _ = command.stdin(std::process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            log_grok(prompt_len, "spawn", 0);
            return Err(error.to_string());
        }
    };
    let pid = child.id();
    register_job(post_id, pid);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_rx = drain_pipe(stdout);
    let stderr_rx = drain_pipe(stderr);
    let started = Instant::now();
    let status = loop {
        if takes_halted() {
            kill_pid(pid);
            let _ = child.kill();
            let _ = child.wait();
            unregister_job(post_id);
            log_grok(prompt_len, "stopped", 0);
            return Err("takes stopped".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > HERMES_TAKE_TIMEOUT => {
                kill_pid(pid);
                let _ = child.kill();
                let _ = child.wait();
                let bytes = stdout_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap_or_default();
                unregister_job(post_id);
                log_grok(prompt_len, "timeout", bytes.len());
                return Err("grok timed out after 90s".into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(40)),
            Err(error) => {
                unregister_job(post_id);
                log_grok(prompt_len, "spawn", 0);
                return Err(error.to_string());
            }
        }
    };
    unregister_job(post_id);
    let code = status.code().unwrap_or(-1);
    let stdout = stdout_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_default();
    let stderr = stderr_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_default();
    log_grok(prompt_len, &code.to_string(), stdout.len());
    let text = String::from_utf8_lossy(&stdout).trim().to_string();
    if text.is_empty() {
        let err = String::from_utf8_lossy(&stderr).trim().to_string();
        if err.is_empty() {
            Err(format!("grok exited {code} with no text"))
        } else {
            Err(err)
        }
    } else if is_provider_failure(&text) {
        Err(text)
    } else {
        Ok(text)
    }
}

fn drain_pipe(pipe: Option<impl Read + Send + 'static>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 4096];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => all.extend_from_slice(&chunk[..n]),
                }
            }
        }
        let _ = tx.send(all);
    });
    rx
}

fn log_grok(prompt_len: usize, exit: &str, stdout_bytes: usize) {
    let line = grok_take_log_line(prompt_len, exit, stdout_bytes);
    super::log::append_desk_log(&super::store::termy_xl_data_root(), &line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_prompt_is_one_line_in_a_file_and_the_log_omits_the_tweet() {
        let tweet = "Courts\nare \"testing\" models\nhttps://x.com/ada/status/9";
        let flat = flatten_grok_prompt(&format!("Post:\n{tweet}"));
        assert!(!flat.contains('\n'));
        assert!(!flat.contains('\r'));
        assert!(flat.contains("Answer with the reply text only"));
        assert!(flat.contains("Do not use tools"));
        assert!(flat.contains("Courts are \"testing\" models"));
        let path = write_prompt_file(&flat).unwrap();
        let stored = std::fs::read_to_string(&path).unwrap();
        assert_eq!(stored, flat);
        assert!(!stored.contains('\n'));
        let args = grok_take_args(&path);
        assert_eq!(args[0], "--prompt-file");
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--max-turns" && pair[1] == "1")
        );
        assert!(args.iter().any(|arg| arg == "--disable-web-search"));
        assert!(args.iter().any(|arg| arg == "--no-plan"));
        assert!(args.iter().any(|arg| arg == "--verbatim"));
        assert!(!args.iter().any(|arg| arg == "--always-approve"));
        assert!(!args.iter().any(|arg| arg.contains("Courts")));
        assert!(!args.iter().any(|arg| arg.contains('\n')));
        let _ = std::fs::remove_file(&path);
        let logged = grok_take_log_line(flat.len(), "0", 12);
        assert!(logged.contains(&format!("prompt_len={}", flat.len())));
        assert!(logged.contains("exit=0"));
        assert!(logged.contains("stdout_bytes=12"));
        assert!(!logged.contains("Courts"));
        assert!(!logged.contains("testing"));
        assert!(!logged.contains("https://"));
    }
}
