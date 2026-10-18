use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use std::io::Write;

/// `CREATE_NO_WINDOW`. Child processes must not flash a console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn exists(&self, program: &str) -> bool;
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, String>;
    /// Bounded variant for probes (doctor / connect) that must never hang
    /// waiting on a browser bridge. Test doubles just run normally.
    fn run_timeout(
        &self,
        program: &str,
        args: &[String],
        _timeout: Duration,
    ) -> Result<CommandOutput, String> {
        self.run(program, args)
    }
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn exists(&self, program: &str) -> bool {
        resolve_program(program).is_some()
    }

    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, String> {
        self.run_timeout(program, args, COMMAND_TIMEOUT)
    }

    fn run_timeout(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<CommandOutput, String> {
        let resolved =
            resolve_program(program).ok_or_else(|| format!("command not found: {program}"))?;
        let mut command = command_for(&resolved, args);
        hide_console(&mut command);
        // OpenCLI drives Chrome through a separate, visible "automation"
        // window that cycles x.com pages and parks on about:blank. Keep
        // that window minimized for the whole call so it runs silently.
        let silence = OpencliSilence::begin(
            is_opencli_program(program)
                || resolved
                    .path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(is_opencli_program),
        );
        let result = output_with_timeout(command, timeout);
        silence.finish();
        result
    }
}

/// True for the OpenCLI binary/shim under any of its installed names
/// (`opencli`, `opencli.cmd`, a full path to either, ...).
fn is_opencli_program(program: &str) -> bool {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let stem = name.split('.').next().unwrap_or(name);
    stem.eq_ignore_ascii_case("opencli")
}

/// Titles the OpenCLI automation window shows while it works: it parks on
/// about:blank when idle, then cycles x.com pages during a call. A user's
/// own X window has these titles too, so callers only auto-track windows
/// that were *not* open when the call started (plus the parked
/// about:blank placeholder, which is the automation window's idle state).
#[allow(dead_code)] // used by the Windows sweep + tests
fn is_automation_title(title: &str) -> bool {
    title == "about:blank - Google Chrome"
        || title == "about:blank"
        || title == "X - Google Chrome"
        || title == "Home / X - Google Chrome"
        || title.ends_with(" / X - Google Chrome")
        || (title.starts_with("x.com") && title.ends_with("- Google Chrome"))
}

/// Keeps OpenCLI's Chrome automation window minimized while a command
/// runs. OpenCLI's Browser Bridge creates that window unfocused but fully
/// visible (`--window background` stops focus theft, not visibility), and
/// leaves it parked on about:blank between calls. Verified live on
/// Windows: once minimized, the window stays minimized while OpenCLI
/// navigates it, and returns to about:blank still minimized.
#[cfg(windows)]
struct OpencliSilence {
    stop: Arc<std::sync::atomic::AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
    tracked: Arc<Mutex<Vec<isize>>>,
}

#[cfg(windows)]
impl OpencliSilence {
    fn begin(active: bool) -> Self {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if !active {
            return Self {
                stop,
                watcher: None,
                tracked: Arc::new(Mutex::new(Vec::new())),
            };
        }
        let baseline: Vec<isize> = chrome_windows().into_iter().map(|(hwnd, _)| hwnd).collect();
        let tracked = Arc::new(Mutex::new(Vec::new()));
        silence_sweep(&baseline, &tracked);
        let watcher = {
            let stop = stop.clone();
            let tracked = tracked.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    silence_sweep(&baseline, &tracked);
                    std::thread::sleep(Duration::from_millis(400));
                }
            })
        };
        Self {
            stop,
            watcher: Some(watcher),
            tracked,
        }
    }

    fn finish(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        // Final sweep: park whatever the call left behind, minimized.
        let tracked = self.tracked.lock().expect("tracked").clone();
        for (hwnd, title) in chrome_windows() {
            if tracked.contains(&hwnd) || title == "about:blank - Google Chrome" {
                minimize_window(hwnd);
            }
        }
    }
}

#[cfg(not(windows))]
struct OpencliSilence;

#[cfg(not(windows))]
impl OpencliSilence {
    fn begin(_active: bool) -> Self {
        Self
    }
    fn finish(self) {}
}

/// One sweep: minimize the parked placeholder and any *new* Chrome
/// window showing an automation title (a window the call itself created).
#[cfg(windows)]
fn silence_sweep(baseline: &[isize], tracked: &Arc<Mutex<Vec<isize>>>) {
    for (hwnd, title) in chrome_windows() {
        let mut tracked = tracked.lock().expect("tracked");
        let known = tracked.contains(&hwnd);
        if known
            || title == "about:blank - Google Chrome"
            || (!baseline.contains(&hwnd) && is_automation_title(&title))
        {
            if !known {
                tracked.push(hwnd);
            }
            drop(tracked);
            minimize_window(hwnd);
        }
    }
}

#[cfg(windows)]
fn minimize_window(hwnd: isize) {
    // SW_MINIMIZE (6). Never touches focus or the window's contents.
    unsafe {
        win32::show_window(hwnd as win32::Hwnd, 6);
    }
}

/// (hwnd, title) for every visible top-level Chrome window.
#[cfg(windows)]
fn chrome_windows() -> Vec<(isize, String)> {
    let mut found: Vec<(isize, String)> = Vec::new();
    unsafe {
        win32::enum_windows(&mut found);
    }
    found
}

#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;

    pub type Hwnd = *mut c_void;
    type Bool = i32;
    type Lparam = isize;

    extern "system" {
        fn EnumWindows(cb: Option<EnumProc>, lparam: Lparam) -> Bool;
        fn IsWindowVisible(hwnd: Hwnd) -> Bool;
        fn IsIconic(hwnd: Hwnd) -> Bool;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> Bool;
        fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn OpenProcess(access: u32, inherit: Bool, pid: u32) -> Hwnd;
        fn CloseHandle(handle: Hwnd) -> Bool;
        fn QueryFullProcessImageNameW(
            handle: Hwnd,
            flags: u32,
            text: *mut u16,
            size: *mut u32,
        ) -> Bool;
    }

    type EnumProc = unsafe extern "system" fn(Hwnd, Lparam) -> Bool;

    pub unsafe fn show_window(hwnd: Hwnd, cmd: i32) {
        unsafe {
            if !hwnd.is_null() && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
                ShowWindow(hwnd, cmd);
            }
        }
    }

    pub unsafe fn enum_windows(found: &mut Vec<(isize, String)>) {
        unsafe extern "system" fn callback(hwnd: Hwnd, lparam: Lparam) -> Bool {
            unsafe {
                let found = &mut *(lparam as *mut Vec<(isize, String)>);
                if IsWindowVisible(hwnd) == 0 {
                    return 1;
                }
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                if pid == 0 || !process_is_chrome(pid) {
                    return 1;
                }
                let mut buffer = [0u16; 512];
                let len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
                if len > 0 {
                    let title = String::from_utf16_lossy(&buffer[..len as usize]);
                    if !title.is_empty() {
                        found.push((hwnd as isize, title));
                    }
                }
                1
            }
        }
        unsafe {
            EnumWindows(Some(callback), found as *mut Vec<(isize, String)> as Lparam);
        }
    }

    unsafe fn process_is_chrome(pid: u32) -> bool {
        // PROCESS_QUERY_LIMITED_INFORMATION
        let handle = unsafe { OpenProcess(0x1000, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut buffer = [0u16; 512];
        let mut size = buffer.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            return false;
        }
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("chrome.exe"))
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
            if !exts
                .iter()
                .any(|have: &String| have.eq_ignore_ascii_case(&ext))
            {
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
    let mut command = Command::new(if cfg!(windows) {
        "powershell.exe"
    } else {
        "pwsh"
    });
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
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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

type ScriptedCallLog = Arc<Mutex<Vec<(String, Vec<String>)>>>;

#[derive(Clone, Default)]
pub struct ScriptedRunner {
    bins: Arc<Mutex<Vec<String>>>,
    scripts: Arc<Mutex<Vec<Script>>>,
    pub calls: ScriptedCallLog,
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

    pub fn script(
        &self,
        program: impl Into<String>,
        args_prefix: &[&str],
        stdout: impl Into<String>,
    ) {
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
        self.bins
            .lock()
            .expect("bins")
            .iter()
            .any(|bin| bin == program)
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
                && args
                    .iter()
                    .zip(&script.args_prefix)
                    .all(|(got, want)| got == want)
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
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
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
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        hide_console(&mut command);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| error.to_string())?;
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
    #[cfg(windows)]
    use super::*;
    use super::{is_automation_title, is_opencli_program};

    #[test]
    fn opencli_silence_matchers_recognise_only_the_automation_window() {
        assert!(is_opencli_program("opencli"));
        assert!(is_opencli_program("opencli.cmd"));
        assert!(is_opencli_program(r"C:\tools\opencli.ps1"));
        assert!(!is_opencli_program("twitter"));
        assert!(!is_opencli_program("opencli-extra"));
        assert!(is_automation_title("about:blank - Google Chrome"));
        assert!(is_automation_title("Home / X - Google Chrome"));
        assert!(is_automation_title("MAGABRO (@ada) / X - Google Chrome"));
        assert!(is_automation_title("x.com/ada - Google Chrome"));
        assert!(!is_automation_title("New Tab - Google Chrome"));
        assert!(!is_automation_title(
            "Kalshi - Prediction Market - Google Chrome"
        ));
        assert!(!is_automation_title("Billing - Google Chrome"));
    }

    #[test]
    #[cfg(windows)]
    fn resolver_finds_cmd_and_ps1_shims_on_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = dir.path().join("shim.cmd");
        let ps1 = dir.path().join("tool.ps1");
        std::fs::write(&cmd, "@echo off\r\n").unwrap();
        std::fs::write(&ps1, "Write-Output ok\r\n").unwrap();
        let path = std::ffi::OsString::from(dir.path());
        let resolved_cmd = resolve_with_path("shim", Some(path.as_os_str())).expect("cmd shim");
        assert_eq!(resolved_cmd.kind, ProgramKind::Cmd);
        assert!(
            resolved_cmd
                .path
                .to_string_lossy()
                .eq_ignore_ascii_case(&cmd.to_string_lossy())
        );
        let resolved_ps1 = resolve_with_path("tool", Some(path.as_os_str())).expect("ps1 shim");
        assert_eq!(resolved_ps1.kind, ProgramKind::PowerShell);
        assert!(
            resolved_ps1
                .path
                .to_string_lossy()
                .eq_ignore_ascii_case(&ps1.to_string_lossy())
        );
        assert!(resolve_with_path("missing-tool", Some(path.as_os_str())).is_none());
    }

    #[test]
    #[cfg(windows)]
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
        assert!(
            resolved
                .path
                .to_string_lossy()
                .eq_ignore_ascii_case(&cmd.to_string_lossy())
        );
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
