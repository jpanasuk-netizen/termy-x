//! Built-in Supabase project for optional account sync.
//!
//! The project URL is public. The anon key is public by design: row level
//! security forces every row to belong to `auth.uid()`. A release build may
//! override it with `TERMY_X_SUPABASE_DEFAULT_ANON_KEY`. Without either key,
//! sync stays off. A service-role key must never be placed here.

/// Hosted project. Advanced installs can override this with `supabase_url`.
pub const DEFAULT_URL: &str = "https://imvenlnmmkxojczzazpq.supabase.co";

/// Marker used when a build-time override is intentionally empty.
pub const ANON_KEY_PLACEHOLDER: &str = "REPLACE_WITH_TERMY_X_SUPABASE_DEFAULT_ANON_KEY";

/// Public anon key for the hosted project. Not a service-role key.
const COMMITTED_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImltdmVubG5tbWt4b2pjenphenBxIiwicm9sZSI6ImFub24iLCJpYXQiOjE3OTQyMjU3OTcsImV4cCI6MjEwOTgwMTc5N30.7Im0NLJY36oUkW9HiiTJC4wO4D23DOzUYLtUHzVDtvk";

/// Anon key compiled in from `TERMY_X_SUPABASE_DEFAULT_ANON_KEY`, or the committed public anon key.
pub fn builtin_anon_key() -> Option<&'static str> {
    if let Some(key) = option_env!("TERMY_X_SUPABASE_DEFAULT_ANON_KEY") {
        if let Some(key) = usable(key) {
            return Some(key);
        }
    }
    usable(COMMITTED_ANON_KEY)
}

fn usable(key: &str) -> Option<&str> {
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
    fn the_hosted_anon_key_is_usable_and_not_a_service_role() {
        assert!(DEFAULT_URL.starts_with("https://"));
        assert!(!DEFAULT_URL.contains("service_role"));
        let key = builtin_anon_key().expect("hosted anon key");
        assert_ne!(key, ANON_KEY_PLACEHOLDER);
        assert!(!key.contains("service_role"));
        assert!(key.starts_with("eyJ"));
    }
}
