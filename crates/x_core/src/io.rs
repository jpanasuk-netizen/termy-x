use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// `CREATE_NO_WINDOW`. Child processes must not flash a console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn exists(&self, program: &str) -> bool;
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, String>;
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn exists(&self, program: &str) -> bool {
        resolve_program(program).is_some()
    }

    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, String> {
        let resolved = resolve_program(program).ok_or_else(|| format!("command not found: {program}"))?;
        let mut command = command_for(&resolved, args);
        hide_console(&mut command);
        output_with_timeout(command, COMMAND_TIMEOUT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramKind {
    Native,
    Cmd,
    PowerShell,
}

struct ResolvedProgram {
    kind: ProgramKind,
    path: PathBuf,
}

fn resolve_program(program: &str) -> Option<ResolvedProgram> {
    resolve_with_path(program, std::env::var_os("PATH").as_deref())
}

fn resolve_with_path(program: &str, path_env: Option<&std::ffi::OsStr>) -> Option<ResolvedProgram> {
    let direct = Path::new(program);
    if is_explicit_path(program) || direct.is_file() {
        return classify_existing(direct);
    }
    let path_env = path_env?;
    let exts = search_extensions();
    for dir in std::env::split_paths(path_env) {
        if let Some(found) = find_in_dir(&dir, program, &exts) {
            return Some(found);
        }
    }
    None
}

/// Windows runs `opencli.cmd`, not the extensionless `#!/bin/sh` shim npm
/// installs beside it. `.ps1` is still searched when no PATHEXT match exists.
fn find_in_dir(dir: &Path, program: &str, exts: &[String]) -> Option<ResolvedProgram> {
    #[cfg(windows)]
    {
        for ext in exts {
            if let Some(found) = classify_existing(&dir.join(format!("{program}{ext}"))) {
                return Some(found);
            }
        }
        let bare = dir.join(program);
        if let Some(found) = classify_existing(&bare) {
            if found.kind != ProgramKind::Native || pe_executable(&bare) {
                return Some(found);
            }
        }
        return None;
    }
    #[cfg(not(windows))]
    {
        if let Some(found) = classify_existing(&dir.join(program)) {
            return Some(found);
        }
        for ext in exts {
            if let Some(found) = classify_existing(&dir.join(format!("{program}{ext}"))) {
                return Some(found);
            }
        }
        None
    }
}

#[cfg(windows)]
fn pe_executable(path: &Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0u8; 2];
    std::io::Read::read_exact(&mut file, &mut magic).is_ok() && &magic == b"MZ"
}

fn is_explicit_path(program: &str) -> bool {
    program.contains('\\') || program.contains('/') || Path::new(program).is_absolute()
}

fn search_extensions() -> Vec<String> {
    let mut exts = Vec::new();
    if let Ok(pathext) = std::env::var("PATHEXT") {
        for ext in pathext.split(';') {
            let ext = ext.trim();
            if ext.is_empty() {
                continue;
            }
            let ext = if ext.starts_with('.') {
                ext.to_string()
            } else {
                format!(".{ext}")
            };
            if !exts.iter().any(|have: &String| have.eq_ignore_ascii_case(&ext)) {
                exts.push(ext);
            }
        }
    }
    for required in [".CMD", ".BAT", ".PS1", ".EXE"] {
        if !exts.iter().any(|have| have.eq_ignore_ascii_case(required)) {
            exts.push(required.to_string());
        }
    }
    exts
}

fn classify_existing(path: &Path) -> Option<ResolvedProgram> {
    if !path.is_file() {
        return None;
    }
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let kind = if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") {
        ProgramKind::Cmd
    } else if ext.eq_ignore_ascii_case("ps1") {
        ProgramKind::PowerShell
    } else {
        ProgramKind::Native
    };
    Some(ResolvedProgram {
        kind,
        path: path.to_path_buf(),
    })
}

fn command_for(resolved: &ResolvedProgram, args: &[String]) -> Command {
    match resolved.kind {
        ProgramKind::Native => {
            let mut command = Command::new(&resolved.path);
            command.args(args);
            command
        }
        ProgramKind::Cmd => cmd_command(&resolved.path, args),
        ProgramKind::PowerShell => powershell_command(&resolved.path, args),
    }
}

fn cmd_command(path: &Path, args: &[String]) -> Command {
    let mut command = Command::new(if cfg!(windows) { "cmd.exe" } else { "cmd" });
    command.arg("/D");
    #[cfg(windows)]
    {
        // `/S /C` plus an outer quoted command line. A trailing space keeps
        // `/S` from eating the closing quote of a quoted argument.
        command.arg("/S").arg("/C");
        command.raw_arg(cmd_c_argument(path, args));
    }
    #[cfg(not(windows))]
    {
        command.arg("/C").arg(path);
        command.args(args);
    }
    command
}

fn powershell_command(path: &Path, args: &[String]) -> Command {
    let mut command = Command::new(if cfg!(windows) { "powershell.exe" } else { "pwsh" });
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]);
    command.arg(path);
    command.args(args);
    command
}

#[cfg(windows)]
fn cmd_c_argument(path: &Path, args: &[String]) -> String {
    let mut inner = quote_cmd_token(&path.to_string_lossy());
    for arg in args {
        inner.push(' ');
        inner.push_str(&quote_cmd_token(arg));
    }
    format!("\"{inner} \"")
}

#[cfg(windows)]
fn quote_cmd_token(token: &str) -> String {
    if token.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = token.chars().any(|ch| {
        matches!(
            ch,
            ' ' | '\t' | '"' | '&' | '|' | '<' | '>' | '^' | '%' | '!' | '(' | ')'
        )
    });
    if !needs_quotes {
        return token.to_string();
    }
    let mut quoted = String::from('"');
    for ch in token.chars() {
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        let _ = command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn output_with_timeout(mut command: Command, timeout: Duration) -> Result<CommandOutput, String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_handle = std::thread::spawn(move || read_pipe(stdout));
    let err_handle = std::thread::spawn(move || read_pipe(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("command timed out".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(error.to_string()),
        }
    };
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Ok(CommandOutput {
        status: status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn read_pipe(pipe: Option<impl Read>) -> Vec<u8> {
    let mut buffer = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buffer);
    }
    buffer
}

#[derive(Clone, Default)]
pub struct ScriptedRunner {
    bins: Arc<Mutex<Vec<String>>>,
    scripts: Arc<Mutex<Vec<Script>>>,
    pub calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
}

struct Script {
    program: String,
    args_prefix: Vec<String>,
    result: Result<String, String>,
}

impl ScriptedRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&self, program: impl Into<String>) {
        self.bins.lock().expect("bins").push(program.into());
    }

    pub fn script(&self, program: impl Into<String>, args_prefix: &[&str], stdout: impl Into<String>) {
        let program = program.into();
        self.install(program.clone());
        self.scripts.lock().expect("scripts").push(Script {
            program,
            args_prefix: args_prefix.iter().map(|part| (*part).to_string()).collect(),
            result: Ok(stdout.into()),
        });
    }

    pub fn fail(&self, program: impl Into<String>, args_prefix: &[&str], error: impl Into<String>) {
        let program = program.into();
        self.install(program.clone());
        self.scripts.lock().expect("scripts").push(Script {
            program,
            args_prefix: args_prefix.iter().map(|part| (*part).to_string()).collect(),
            result: Err(error.into()),
        });
    }
}

impl CommandRunner for ScriptedRunner {
    fn exists(&self, program: &str) -> bool {
        self.bins.lock().expect("bins").iter().any(|bin| bin == program)
    }

    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, String> {
        self.calls
            .lock()
            .expect("calls")
            .push((program.to_string(), args.to_vec()));
        let scripts = self.scripts.lock().expect("scripts");
        let matched = scripts.iter().rev().find(|script| {
            script.program == program
                && args.len() >= script.args_prefix.len()
                && args.iter().zip(&script.args_prefix).all(|(got, want)| got == want)
        });
        match matched {
            Some(script) => match &script.result {
                Ok(stdout) => Ok(CommandOutput {
                    status: 0,
                    stdout: stdout.clone(),
                    stderr: String::new(),
                }),
                Err(error) => Err(error.clone()),
            },
            None => Err(format!("no script for {program} {args:?}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub body: Option<String>,
    pub bearer: Option<String>,
    pub content_type: Option<String>,
}

pub trait Http: Send + Sync {
    fn request(&self, request: HttpRequest) -> Result<String, String>;
}

pub struct UreqHttp {
    agent: ureq::Agent,
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(8))
                .timeout_connect(Duration::from_secs(2))
                .build(),
        }
    }
}

impl Http for UreqHttp {
    fn request(&self, request: HttpRequest) -> Result<String, String> {
        let mut call = match request.method {
            "POST" => self.agent.post(&request.url),
            _ => self.agent.get(&request.url),
        };
        if let Some(token) = &request.bearer {
            call = call.set("Authorization", &format!("Bearer {token}"));
        }
        if let Some(content_type) = &request.content_type {
            call = call.set("Content-Type", content_type);
        }
        let response = if let Some(body) = &request.body {
            call.send_string(body)
        } else {
            call.call()
        };
        match response {
            Ok(response) => response.into_string().map_err(|error| error.to_string()),
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(format!("http {code}: {body}"))
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

#[derive(Default)]
pub struct MapHttp {
    pub routes: Mutex<HashMap<String, Result<String, String>>>,
    pub calls: Mutex<Vec<String>>,
}

impl Http for MapHttp {
    fn request(&self, request: HttpRequest) -> Result<String, String> {
        self.calls.lock().expect("calls").push(request.url.clone());
        let routes = self.routes.lock().expect("routes");
        routes
            .get(&request.url)
            .cloned()
            .or_else(|| {
                routes
                    .iter()
                    .find(|(key, _)| request.url.starts_with(key.as_str()))
                    .map(|(_, value)| value.clone())
            })
            .unwrap_or_else(|| Err(format!("no http script for {}", request.url)))
    }
}

pub trait Opener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}

pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "xdg-open"
        };
        let mut command = Command::new(program);
        if cfg!(target_os = "windows") {
            command.args(["/C", "start", "", url]);
        } else {
            command.arg(url);
        }
        hide_console(&mut command);
        command.spawn().map(|_| ()).map_err(|error| error.to_string())
    }
}

#[derive(Default)]
pub struct RecordingOpener {
    pub urls: Mutex<Vec<String>>,
}

impl Opener for RecordingOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        self.urls.lock().expect("urls").push(url.to_string());
        Ok(())
    }
}

pub trait Clipboard: Send + Sync {
    fn copy(&self, text: &str) -> Result<(), String>;
}

pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy(&self, text: &str) -> Result<(), String> {
        copy_text(text)
    }
}

#[derive(Default)]
pub struct RecordingClipboard {
    pub texts: Mutex<Vec<String>>,
}

impl Clipboard for RecordingClipboard {
    fn copy(&self, text: &str) -> Result<(), String> {
        self.texts.lock().expect("texts").push(text.to_string());
        Ok(())
    }
}

fn copy_text(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ]);
        command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        hide_console(&mut command);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
        }
        let output = child.wait_with_output().map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("clipboard is only wired on Windows in this build".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_finds_cmd_and_ps1_shims_on_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = dir.path().join("shim.cmd");
        let ps1 = dir.path().join("tool.ps1");
        std::fs::write(&cmd, "@echo off\r\n").unwrap();
        std::fs::write(&ps1, "Write-Output ok\r\n").unwrap();
        let path = std::ffi::OsString::from(dir.path());
        let resolved_cmd = resolve_with_path("shim", Some(path.as_os_str())).expect("cmd shim");
        assert_eq!(resolved_cmd.kind, ProgramKind::Cmd);
        assert!(resolved_cmd
            .path
            .to_string_lossy()
            .eq_ignore_ascii_case(&cmd.to_string_lossy()));
        let resolved_ps1 = resolve_with_path("tool", Some(path.as_os_str())).expect("ps1 shim");
        assert_eq!(resolved_ps1.kind, ProgramKind::PowerShell);
        assert!(resolved_ps1
            .path
            .to_string_lossy()
            .eq_ignore_ascii_case(&ps1.to_string_lossy()));
        assert!(resolve_with_path("missing-tool", Some(path.as_os_str())).is_none());
    }

    #[test]
    fn windows_shims_run_without_a_console() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = dir.path().join("echo-shim.cmd");
        std::fs::write(&cmd, "@echo off\r\necho shim-ok\r\n").unwrap();
        let output = SystemRunner
            .run(cmd.to_str().unwrap(), &[])
            .expect("cmd shim runs");
        assert!(output.stdout.contains("shim-ok"), "{}", output.stdout);

        let ps1 = dir.path().join("echo-shim.ps1");
        std::fs::write(&ps1, "Write-Output ps1-ok\r\nWrite-Output $args[0]\r\n").unwrap();
        let output = SystemRunner
            .run(ps1.to_str().unwrap(), &["ps1-arg".into()])
            .expect("ps1 shim runs");
        assert!(output.stdout.contains("ps1-ok"), "{output:?}");
        assert!(output.stdout.contains("ps1-arg"), "{output:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefers_cmd_shim_over_extensionless_shell_script() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("opencli");
        let cmd = dir.path().join("opencli.cmd");
        std::fs::write(&bare, "#!/bin/sh\necho shell\n").unwrap();
        std::fs::write(&cmd, "@echo off\r\n").unwrap();
        let path = std::ffi::OsString::from(dir.path());
        let resolved = resolve_with_path("opencli", Some(path.as_os_str())).expect("cmd shim");
        assert_eq!(resolved.kind, ProgramKind::Cmd);
        assert!(resolved
            .path
            .to_string_lossy()
            .eq_ignore_ascii_case(&cmd.to_string_lossy()));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_receives_arguments_including_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("dir with space");
        std::fs::create_dir(&nested).unwrap();
        let cmd = nested.join("echo-shim.cmd");
        std::fs::write(&cmd, "@echo off\r\necho [%~1]\r\n").unwrap();
        let output = SystemRunner
            .run(cmd.to_str().unwrap(), &["hello world".into()])
            .expect("cmd shim forwards args");
        assert!(
            output.stdout.contains("[hello world]"),
            "status {} stdout {:?} stderr {:?}",
            output.status,
            output.stdout,
            output.stderr
        );
    }
}
