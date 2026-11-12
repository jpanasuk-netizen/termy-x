//! Grok takes. The prompt goes in a file, not on the Windows command line.
//! The desk log records the length and the exit status, never the tweet.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::takes::{
    GROK_TAKE_TIMEOUT, apply_hidden_command, background_grok_spawn, is_provider_failure, kill_pid,
    register_job, takes_halted, unregister_job,
};

/// Headless take flags for grok 1.0.77. No `--always-approve`, so a draft
/// cannot wander into web search. `--prompt-file` is the single-turn trigger.
pub fn grok_take_args(prompt_file: &Path, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
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
    ];
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        args.push("--model".into());
        args.push(model.to_string());
    }
    args
}

pub use super::carousel::cli_take_prompt;

pub fn grok_take_log_line(prompt_len: usize, exit: &str, stdout_bytes: usize) -> String {
    format!("INFO grok: prompt_len={prompt_len} exit={exit} stdout_bytes={stdout_bytes}")
}

pub(crate) fn run(post: &super::SourcePost, tone: &str) -> Result<String, String> {
    let _ = take_cleared_grok_model();
    let model = selected_model();
    let prompt = cli_take_prompt(&post.author, &post.text, tone);
    let prompt_len = prompt.len();
    let path = match write_prompt_file(&prompt) {
        Ok(path) => path,
        Err(error) => {
            log_grok(prompt_len, "spawn", 0);
            return Err(error);
        }
    };
    let outcome = match run_file(&path, &post.id, prompt_len, model.as_deref()) {
        Err(error) if model.is_some() && unknown_model(&error) => {
            let id = model.unwrap_or_default();
            note_cleared_model(&id);
            match run_file(&path, &post.id, prompt_len, None) {
                Ok(text) => Ok(text),
                Err(again) => Err(format!(
                    "Cleared unknown Grok model '{id}'. Used the CLI default. {again}"
                )),
            }
        }
        other => other,
    };
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

fn run_file(
    path: &Path,
    post_id: &str,
    prompt_len: usize,
    model: Option<&str>,
) -> Result<String, String> {
    let plan = background_grok_spawn();
    let mut command = std::process::Command::new(plan.program);
    command.args(grok_take_args(path, model));
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
    let partial = PARTIAL.with(|slot| slot.borrow().clone());
    let stdout_rx = drain_pipe(stdout, partial);
    let stderr_rx = drain_pipe(stderr, None);
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
            Ok(None) if started.elapsed() > GROK_TAKE_TIMEOUT => {
                kill_pid(pid);
                let _ = child.kill();
                let _ = child.wait();
                let bytes = stdout_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap_or_default();
                unregister_job(post_id);
                log_grok(prompt_len, "timeout", bytes.len());
                return Err("grok timed out after 5 minutes".into());
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

fn drain_pipe(
    pipe: Option<impl Read + Send + 'static>,
    partial: Option<std::sync::mpsc::Sender<String>>,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 4096];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        all.extend_from_slice(&chunk[..n]);
                        if let Some(partial) = &partial {
                            let text = String::from_utf8_lossy(&all).trim().to_string();
                            if !text.is_empty() {
                                let _ = partial.send(text);
                            }
                        }
                    }
                }
            }
        }
        let _ = tx.send(all);
    });
    rx
}

thread_local! {
    static PARTIAL: std::cell::RefCell<Option<std::sync::mpsc::Sender<String>>> =
        const { std::cell::RefCell::new(None) };
}

pub struct PartialBinding;

impl Drop for PartialBinding {
    fn drop(&mut self) {
        PARTIAL.with(|slot| *slot.borrow_mut() = None);
    }
}

pub fn bind_grok_stream(tx: Option<std::sync::mpsc::Sender<String>>) -> PartialBinding {
    PARTIAL.with(|slot| *slot.borrow_mut() = tx);
    PartialBinding
}

pub fn unknown_model(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("unknown model")
        || lower.contains("couldn't set model")
        || lower.contains("could not set model")
}

pub fn parse_grok_models(text: &str) -> Vec<String> {
    let mut models = Vec::new();
    let push = |models: &mut Vec<String>, id: &str| {
        let id = id
            .trim()
            .trim_matches(|ch: char| ch == '*' || ch == '|' || ch == '"' || ch == ',');
        if id.len() < 3 {
            return;
        }
        if !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
        {
            return;
        }
        let lower = id.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "id" | "model" | "models" | "name" | "default" | "available" | "current"
        ) {
            return;
        }
        if models.iter().any(|have: &String| have == id) {
            return;
        }
        models.push(id.to_string());
    };
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        let rows = value
            .as_array()
            .cloned()
            .or_else(|| {
                value
                    .get("models")
                    .and_then(|item| item.as_array())
                    .cloned()
            })
            .unwrap_or_default();
        for row in rows {
            if let Some(id) = row.as_str() {
                push(&mut models, id);
            } else if let Some(id) = row.get("id").and_then(|item| item.as_str()) {
                push(&mut models, id);
            }
        }
        if !models.is_empty() {
            return models;
        }
    }
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['*', '-', '#']).trim();
        if line.is_empty() || line.ends_with(':') {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or("");
        push(&mut models, token);
    }
    models
}

pub fn load_cached_grok_models(root: &Path) -> Vec<String> {
    let path = root.join("grok-models.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let fresh = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age <= std::time::Duration::from_secs(60 * 60));
    if !fresh {
        return Vec::new();
    }
    parse_grok_models(&text)
}

pub fn refresh_grok_models(root: &Path) -> Vec<String> {
    if let Some(cached) = fresh_cache(root) {
        return cached;
    }
    let listed = run_grok_models();
    if listed.is_empty() {
        return std::fs::read_to_string(root.join("grok-models.txt"))
            .map(|text| parse_grok_models(&text))
            .unwrap_or_default();
    }
    let body = listed.join("\n");
    let _ = std::fs::create_dir_all(root);
    let _ = std::fs::write(root.join("grok-models.txt"), body);
    listed
}

fn fresh_cache(root: &Path) -> Option<Vec<String>> {
    let models = load_cached_grok_models(root);
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
}

fn run_grok_models() -> Vec<String> {
    let plan = background_grok_spawn();
    let mut command = std::process::Command::new(plan.program);
    command.arg("models");
    apply_hidden_command(&mut command);
    let _ = command.stdin(std::process::Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return Vec::new();
    };
    let started = Instant::now();
    let limit = std::time::Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut stdout = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut stdout);
                }
                return parse_grok_models(&stdout);
            }
            Ok(Some(_)) => return Vec::new(),
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(40)),
            Err(_) => return Vec::new(),
        }
    }
}

thread_local! {
    static MODEL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

static CLEARED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub struct ModelBinding;

impl Drop for ModelBinding {
    fn drop(&mut self) {
        MODEL.with(|slot| *slot.borrow_mut() = None);
    }
}

pub fn bind_grok_model(model: Option<String>) -> ModelBinding {
    MODEL.with(|slot| *slot.borrow_mut() = model);
    ModelBinding
}

fn selected_model() -> Option<String> {
    MODEL.with(|slot| slot.borrow().clone())
}

fn note_cleared_model(model: &str) {
    if let Ok(mut slot) = CLEARED.lock() {
        *slot = Some(model.to_string());
    }
}

pub fn take_cleared_grok_model() -> Option<String> {
    CLEARED.lock().ok().and_then(|mut slot| slot.take())
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
        let flat = cli_take_prompt("ada", "Courts are \"testing\" models", "Funny");
        assert!(!flat.contains('\n'));
        assert!(flat.contains("Reply text only"));
        assert!(flat.contains("No tools"));
        assert!(flat.contains("Courts"));
        let path = write_prompt_file(&flat).unwrap();
        let stored = std::fs::read_to_string(&path).unwrap();
        assert_eq!(stored, flat);
        assert!(!stored.contains('\n'));
        let args = grok_take_args(&path, None);
        assert_eq!(args[0], "--prompt-file");
        assert!(!args.iter().any(|arg| arg == "--model"));
        let picked = grok_take_args(&path, Some("grok-3"));
        assert!(
            picked
                .windows(2)
                .any(|pair| pair[0] == "--model" && pair[1] == "grok-3")
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--max-turns" && pair[1] == "1")
        );
        assert!(args.iter().any(|arg| arg == "--disable-web-search"));
        assert!(args.iter().any(|arg| arg == "--no-plan"));
        assert!(args.iter().any(|arg| arg == "--verbatim"));
        assert!(unknown_model(
            "Couldn't set model 'old': Invalid params: \"unknown model id\""
        ));
        assert_eq!(
            parse_grok_models("Available models:\n* grok-3 (default)\ngrok-4\n"),
            vec!["grok-3".to_string(), "grok-4".to_string()]
        );
        let short = cli_take_prompt("ada", &"courts ".repeat(300), "Funny");
        assert!(short.chars().count() <= 400);
        assert!(short.contains("Reply text only"));
        assert_eq!(
            super::super::takes::GROK_TAKE_TIMEOUT,
            std::time::Duration::from_secs(300)
        );
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
