use std::path::Path;
use std::process::{Command, Output};

fn prettify(xdg_config_home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_termy-cli"))
        .arg("-prettify-config")
        .env("XDG_CONFIG_HOME", xdg_config_home)
        .env("APPDATA", xdg_config_home)
        .output()
        .unwrap()
}

fn config_file(xdg_config_home: &Path) -> std::path::PathBuf {
    // Both XDG_CONFIG_HOME and APPDATA resolve to <dir>/termy/config.txt.
    let dir = xdg_config_home.join("termy");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.txt")
}

#[test]
fn prettify_refuses_to_drop_malformed_lines() {
    let temp = tempfile::tempdir().unwrap();
    let config = config_file(temp.path());
    let original = "theme = nord\nthis line has no equals sign\nfont_size = 14\n";
    std::fs::write(&config, original).unwrap();

    let output = prettify(temp.path());

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("this line has no equals sign"));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
}

#[test]
fn prettify_writes_backup_with_original_contents() {
    let temp = tempfile::tempdir().unwrap();
    let config = config_file(temp.path());
    let original = "# my note\nfont_size = 14\ntheme = nord\n";
    std::fs::write(&config, original).unwrap();

    let output = prettify(temp.path());

    assert!(output.status.success(), "{output:?}");
    let backup = config.with_file_name("config.txt.bak");
    assert_eq!(std::fs::read_to_string(backup).unwrap(), original);
    let rewritten = std::fs::read_to_string(&config).unwrap();
    assert!(!rewritten.contains("# my note"));
    assert!(rewritten.contains("theme = nord"));
    assert!(rewritten.contains("font_size = 14"));
}

#[cfg(unix)]
#[test]
fn prettify_keeps_config_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let config = config_file(temp.path());
    let dotfiles = temp.path().join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let target = dotfiles.join("termy.txt");
    std::fs::write(&target, "font_size = 14\ntheme = nord\n").unwrap();
    std::os::unix::fs::symlink(&target, &config).unwrap();

    let output = prettify(temp.path());

    assert!(output.status.success(), "{output:?}");
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let rewritten = std::fs::read_to_string(&target).unwrap();
    assert!(rewritten.starts_with("theme = nord\n"), "{rewritten}");
}

#[test]
fn prettify_exits_non_zero_when_config_cannot_be_read() {
    let temp = tempfile::tempdir().unwrap();
    let config = config_file(temp.path());
    // A directory where the config file should be makes reading fail.
    std::fs::create_dir_all(&config).unwrap();

    let output = prettify(temp.path());

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Failed to read config file"));
}
