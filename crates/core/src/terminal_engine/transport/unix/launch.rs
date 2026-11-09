// Program lookup, argument and environment encoding for the Unix PTY child.
// Included into `unix.rs`, so it shares that module's imports and items.

fn c_string(value: &str, label: &str) -> io::Result<CString> {
    CString::new(value).map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{label} cannot contain NUL bytes"),
        )
    })
}

fn c_string_os(value: &OsStr, label: &str) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{label} cannot contain NUL bytes"),
        )
    })
}

fn child_environment(
    overrides: &[(String, String)],
    working_directory: Option<&Path>,
    inherit: bool,
) -> io::Result<Vec<CString>> {
    let mut environment = if inherit {
        std::env::vars_os()
            .filter(|(name, _)| !super::belongs_to_parent_terminal(name))
            .collect::<BTreeMap<OsString, OsString>>()
    } else {
        BTreeMap::new()
    };
    for (name, value) in overrides {
        if name.is_empty() || name.as_bytes().contains(&b'=') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "terminal environment names cannot be empty or contain '='",
            ));
        }
        environment.insert(OsString::from(name), OsString::from(value));
    }
    if let Some(working_directory) = working_directory {
        environment.insert(
            OsString::from("PWD"),
            working_directory.as_os_str().to_os_string(),
        );
    }

    environment
        .into_iter()
        .map(|(name, value)| {
            let mut entry = Vec::with_capacity(name.as_bytes().len() + value.as_bytes().len() + 1);
            entry.extend_from_slice(name.as_bytes());
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            CString::new(entry).map_err(|_| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    "terminal environment cannot contain NUL bytes",
                )
            })
        })
        .collect()
}

fn resolve_program(config: &SpawnConfig) -> io::Result<PathBuf> {
    resolve_program_from(config, std::env::current_dir)
}

/// [`resolve_program`], with the parent's working directory from
/// `current_dir`, which is asked only when a relative directory needs it:
/// that directory may have been deleted, which skips relative entries.
fn resolve_program_from(
    config: &SpawnConfig,
    current_dir: impl Fn() -> io::Result<PathBuf>,
) -> io::Result<PathBuf> {
    let program = PathBuf::from(&config.program);
    if config.program.as_bytes().contains(&b'/') {
        return Ok(program);
    }

    let child_directory = || match config.working_directory.as_deref() {
        Some(directory) if directory.is_absolute() => Ok(directory.to_path_buf()),
        Some(directory) => Ok(current_dir()?.join(directory)),
        None => current_dir(),
    };
    let mut resolved_child_directory = None;

    let path = config
        .environment
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| OsString::from(value))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let directory = if directory.is_absolute() {
            directory
        } else {
            // Like `execvp`, skip an entry that can't be searched.
            let base = resolved_child_directory.get_or_insert_with(|| child_directory().ok());
            let Some(base) = base else {
                continue;
            };
            base.join(directory)
        };
        let Ok(candidate) = directory.join(&program).canonicalize() else {
            continue;
        };
        if candidate
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            return Ok(candidate);
        }
    }

    Err(io::Error::new(
        ErrorKind::NotFound,
        format!(
            "terminal program '{}' was not found in PATH",
            config.program
        ),
    ))
}
