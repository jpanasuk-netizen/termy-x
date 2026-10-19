//! Platform-specific locations, kept behind `#[cfg]` so shared logic in
//! `config`/`io`/`doctor` stays platform-free.
//!
//! Windows behaviour is the original one: config lives under
//! `%APPDATA%\termy` (via `dirs`), and Chrome profiles under
//! `%LOCALAPPDATA%\Google\Chrome\User Data`. Linux uses the XDG base
//! directory spec instead (`$XDG_CONFIG_HOME` or `~/.config`), and
//! Chrome/Chromium/Brave profiles under the usual `~/.config`, snap,
//! and flatpak locations. Nothing here ever creates a directory or
//! panics; a missing home/config dir simply yields no fallback and
//! callers keep their previous last-resort behaviour.

use std::path::PathBuf;

fn nonempty_path(value: Option<String>) -> Option<PathBuf> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Pure Windows config base: `%APPDATA%`, else `~/AppData/Roaming`.
#[allow(dead_code)] // exercised by tests / the Windows fallback below
pub(crate) fn windows_config_base(
    appdata: Option<String>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    nonempty_path(appdata).or_else(|| home.map(|h| h.join("AppData").join("Roaming")))
}

/// Pure Linux/XDG config base: absolute `$XDG_CONFIG_HOME`, else `~/.config`.
#[allow(dead_code)] // exercised by tests / the Linux fallback below
pub(crate) fn linux_config_base(
    xdg_config_home: Option<String>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(xdg) = nonempty_path(xdg_config_home).filter(|p| p.is_absolute()) {
        return Some(xdg);
    }
    home.map(|h| h.join(".config"))
}

/// Config base used only when `dirs::config_dir()` returns `None`
/// (no home directory discoverable). The primary path is unchanged on
/// every platform; this just avoids falling back to the CWD.
pub fn config_base_fallback() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_config_base(std::env::var("APPDATA").ok(), dirs::home_dir())
    }
    #[cfg(target_os = "linux")]
    {
        linux_config_base(std::env::var("XDG_CONFIG_HOME").ok(), dirs::home_dir())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        linux_config_base(std::env::var("XDG_CONFIG_HOME").ok(), dirs::home_dir())
    }
}

/// Candidate browser profile directories ("User Data" roots), in the
/// order a user most likely runs them. Unfiltered: a path may not
/// exist. This is discovery only — Termy X never reads cookies or
/// profile contents; OpenCLI/Browser Bridge owns the live session.
pub fn browser_profile_dirs() -> Vec<PathBuf> {
    let mut dirs_out: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = nonempty_path(std::env::var("LOCALAPPDATA").ok())
            .or_else(|| dirs::home_dir().map(|h| h.join("AppData").join("Local")))
        {
            dirs_out.push(local.join("Google").join("Chrome").join("User Data"));
            dirs_out.push(local.join("Chromium").join("User Data"));
            dirs_out.push(
                local
                    .join("BraveSoftware")
                    .join("Brave-Browser")
                    .join("User Data"),
            );
            dirs_out.push(local.join("Microsoft").join("Edge").join("User Data"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(config) =
            linux_config_base(std::env::var("XDG_CONFIG_HOME").ok(), dirs::home_dir())
        {
            dirs_out.push(config.join("google-chrome"));
            dirs_out.push(config.join("google-chrome-beta"));
            dirs_out.push(config.join("chromium"));
            dirs_out.push(config.join("BraveSoftware").join("Brave-Browser"));
            dirs_out.push(config.join("microsoft-edge"));
        }
        if let Some(home) = dirs::home_dir() {
            // Snap and flatpak installs keep profiles outside ~/.config.
            dirs_out.push(
                home.join("snap")
                    .join("chromium")
                    .join("common")
                    .join("chromium"),
            );
            dirs_out.push(
                home.join("snap")
                    .join("google-chrome")
                    .join("common")
                    .join("google-chrome"),
            );
            dirs_out.push(
                home.join(".var")
                    .join("app")
                    .join("org.chromium.Chromium")
                    .join("config")
                    .join("chromium"),
            );
            dirs_out.push(
                home.join(".var")
                    .join("app")
                    .join("com.google.Chrome")
                    .join("config")
                    .join("google-chrome"),
            );
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        if let Some(home) = dirs::home_dir() {
            let support = home.join("Library").join("Application Support");
            dirs_out.push(support.join("Google").join("Chrome"));
            dirs_out.push(support.join("Chromium"));
            dirs_out.push(support.join("BraveSoftware").join("Brave-Browser"));
        }
    }
    dirs_out
}

/// The subset of [`browser_profile_dirs`] that actually exists here.
pub fn existing_browser_profile_dirs() -> Vec<PathBuf> {
    browser_profile_dirs()
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_config_base_prefers_absolute_xdg_then_home_dot_config() {
        let home = PathBuf::from("/home/ada");
        assert_eq!(
            linux_config_base(Some("/xdg/cfg".into()), Some(home.clone())),
            Some(PathBuf::from("/xdg/cfg"))
        );
        // A relative XDG_CONFIG_HOME is invalid per spec: ignore it.
        assert_eq!(
            linux_config_base(Some("relative/cfg".into()), Some(home.clone())),
            Some(PathBuf::from("/home/ada/.config"))
        );
        assert_eq!(
            linux_config_base(None, Some(home)),
            Some(PathBuf::from("/home/ada/.config"))
        );
        assert_eq!(linux_config_base(None, None), None);
    }

    #[test]
    fn windows_config_base_prefers_appdata_then_roaming() {
        let home = PathBuf::from(r"C:\Users\ada");
        assert_eq!(
            windows_config_base(Some(r"D:\Roaming".into()), Some(home.clone())),
            Some(PathBuf::from(r"D:\Roaming"))
        );
        assert_eq!(
            windows_config_base(None, Some(home)),
            Some(
                PathBuf::from(r"C:\Users\ada")
                    .join("AppData")
                    .join("Roaming")
            )
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_browser_profiles_cover_config_snap_and_flatpak() {
        let dirs = browser_profile_dirs();
        assert!(
            dirs.iter().any(|p| p.ends_with("google-chrome")),
            "{dirs:?}"
        );
        assert!(dirs.iter().any(|p| p.ends_with("chromium")), "{dirs:?}");
        assert!(
            dirs.iter().any(|p| p.to_string_lossy().contains("snap")),
            "{dirs:?}"
        );
        assert!(
            dirs.iter()
                .any(|p| p.to_string_lossy().contains(".var/app")),
            "{dirs:?}"
        );
    }
}
