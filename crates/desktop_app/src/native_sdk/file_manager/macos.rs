use super::OPEN_TAB_HERE_LABEL;
use std::path::{Path, PathBuf};

// Use the same relocatable service for DMG packaging and startup registration.
const SERVICE_INFO: &str = include_str!("../../../../../scripts/file-manager/macos/Info.plist");
const SERVICE_WORKFLOW: &str =
    include_str!("../../../../../scripts/file-manager/macos/document.wflow");

pub(super) fn register(
    _executable: &Path,
    _on_open_directory: impl Fn(PathBuf) + Send + Sync + 'static,
) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    let service_dir = PathBuf::from(home)
        .join("Library/Services")
        .join(format!("{OPEN_TAB_HERE_LABEL}.workflow"))
        .join("Contents");
    // Overwrite the old absolute-path workflow on the first launch after updating.
    if !install_user_service(&service_dir)? {
        return Ok(());
    }
    // This runs during startup on the main thread; refreshing the Services
    // registry can take a while, so do it in the background.
    std::thread::Builder::new()
        .name("termy-finder-services".to_string())
        .spawn(|| {
            match std::process::Command::new("/System/Library/CoreServices/pbs")
                .arg("-update")
                .status()
            {
                Ok(status) if status.success() => {}
                Ok(status) => log::warn!("failed to refresh Finder services: {status}"),
                Err(error) => log::warn!("failed to refresh Finder services: {error}"),
            }
        })
        .map_err(|error| format!("failed to refresh Finder services: {error}"))?;
    Ok(())
}

/// Writes the service files that differ from the shipped ones. Returns whether
/// anything changed, so unchanged launches skip the Services refresh.
fn install_user_service(service_dir: &Path) -> Result<bool, String> {
    let files = [
        ("Info.plist", SERVICE_INFO),
        ("document.wflow", SERVICE_WORKFLOW),
    ];
    let mut changed = false;
    for (name, contents) in files {
        let path = service_dir.join(name);
        if std::fs::read(&path).is_ok_and(|existing| existing == contents.as_bytes()) {
            continue;
        }
        std::fs::create_dir_all(service_dir)
            .map_err(|error| format!("failed to create Finder service: {error}"))?;
        std::fs::write(&path, contents)
            .map_err(|error| format!("failed to write Finder service {name}: {error}"))?;
        changed = true;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_sdk::file_manager::posix_single_quote;
    use std::process::Command;

    #[test]
    fn finder_service_upgrade_replaces_stale_executable_and_preserves_folder_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let service_dir = temp.path().join("service/Contents");
        std::fs::create_dir_all(&service_dir).unwrap();
        let workflow = service_dir.join("document.wflow");
        std::fs::write(&workflow, "/old/build/Termy.app/Contents/MacOS/Termy").unwrap();
        assert!(install_user_service(&service_dir).unwrap());
        assert!(
            !install_user_service(&service_dir).unwrap(),
            "an up-to-date service must not be rewritten"
        );

        let extracted = Command::new("/usr/bin/plutil")
            .args([
                "-extract",
                "actions.0.action.ActionParameters.COMMAND_STRING",
                "raw",
                "-o",
                "-",
            ])
            .arg(&workflow)
            .output()
            .unwrap();
        assert!(extracted.status.success());
        let script = String::from_utf8(extracted.stdout).unwrap();

        // Exercise the shipped shell script, replacing only the OS launch command
        // so this test cannot launch apps or change the user's Services registry.
        let capture = temp.path().join("arguments");
        let stub = temp.path().join("capture-open.sh");
        std::fs::write(&stub, "printf '%s\\0' \"$@\" >> \"$TERMY_TEST_ARGUMENTS\"\nexit \"$TERMY_TEST_OPEN_STATUS\"\n").unwrap();
        let script = script.replace(
            "/usr/bin/open",
            &format!("/bin/bash {}", posix_single_quote(&stub.to_string_lossy())),
        );
        let folder = temp.path().join("it's a folder & $(echo nope)");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("a file.txt");
        std::fs::write(&file, "").unwrap();
        let status = Command::new("/bin/bash")
            .args(["-c", &script, "finder-service"])
            .arg(&folder)
            .arg(&file)
            .env("TERMY_TEST_ARGUMENTS", &capture)
            .env("TERMY_TEST_OPEN_STATUS", "0")
            .status()
            .unwrap();
        assert!(status.success());
        let args = std::fs::read(&capture).unwrap();
        let expected = format!("-b\0com.lassevestergaard.termy\0{}\0", folder.display()).repeat(2);
        assert_eq!(args, expected.as_bytes());

        let status = Command::new("/bin/bash")
            .args(["-c", &script, "finder-service"])
            .arg(&folder)
            .env("TERMY_TEST_ARGUMENTS", &capture)
            .env("TERMY_TEST_OPEN_STATUS", "7")
            .status()
            .unwrap();
        assert_eq!(
            status.code(),
            Some(7),
            "launch failures must reach Automator"
        );
    }
}
