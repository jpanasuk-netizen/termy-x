use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
        match Command::new(program).arg("--version").output() {
            Ok(output) => output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty(),
            Err(_) => false,
        }
    }

    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, String> {
        let program = program.to_string();
        let args = args.to_vec();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = Command::new(program).args(args).output();
            let _ = tx.send(result);
        });
        match rx.recv_timeout(Duration::from_secs(25)) {
            Ok(Ok(output)) => Ok(CommandOutput {
                status: output.status.code().unwrap_or(1),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            }),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("command timed out".to_string()),
        }
    }
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

pub struct UreqHttp;

impl Http for UreqHttp {
    fn request(&self, request: HttpRequest) -> Result<String, String> {
        let mut call = match request.method {
            "POST" => ureq::post(&request.url),
            _ => ureq::get(&request.url),
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
