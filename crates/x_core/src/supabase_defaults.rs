//! Built-in Supabase project for optional account sync.
//!
//! The project URL is public. The anon key is public by design: row level
//! security forces every row to belong to `auth.uid()`. Release builds set
//! `TERMY_X_SUPABASE_DEFAULT_ANON_KEY`. Without that key, sync stays off.
//! A service-role key must never be placed here.

/// Hosted project. Advanced installs can override this with `supabase_url`.
pub const DEFAULT_URL: &str = "https://imvenlnmmkxojczzazpq.supabase.co";

/// Marker left in the source when the release build did not supply a key.
pub const ANON_KEY_PLACEHOLDER: &str = "REPLACE_WITH_TERMY_X_SUPABASE_DEFAULT_ANON_KEY";

/// Anon key compiled in from `TERMY_X_SUPABASE_DEFAULT_ANON_KEY`.
/// `None` when the variable was unset or still the placeholder.
pub fn builtin_anon_key() -> Option<&'static str> {
    let key = option_env!("TERMY_X_SUPABASE_DEFAULT_ANON_KEY")?;
    let key = key.trim();
    if key.is_empty() || key == ANON_KEY_PLACEHOLDER || key.starts_with("REPLACE_") {
        None
    } else if key.contains("service_role") {
        None
    } else {
        Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_is_not_a_usable_key() {
        assert!(ANON_KEY_PLACEHOLDER.starts_with("REPLACE_"));
        assert!(DEFAULT_URL.starts_with("https://"));
        assert!(!DEFAULT_URL.contains("service_role"));
        if let Some(key) = builtin_anon_key() {
            assert!(!key.contains("service_role"));
            assert_ne!(key, ANON_KEY_PLACEHOLDER);
        }
    }
}
