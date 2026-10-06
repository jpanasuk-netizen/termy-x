pub fn splash_ansi() -> String {
    render_splash(color_enabled())
}

fn color_enabled() -> bool {
    match std::env::var_os("NO_COLOR") {
        Some(value) if !value.is_empty() => false,
        _ => true,
    }
}

pub fn render_splash(color: bool) -> String {
    let (blue, cyan, pink, dim, reset) = if color {
        (
            "\u{1b}[38;2;47;123;255m",
            "\u{1b}[38;2;122;246;255m",
            "\u{1b}[38;2;255;61;154m",
            "\u{1b}[38;2;90;122;156m",
            "\u{1b}[0m",
        )
    } else {
        ("", "", "", "", "")
    };
    format!(
        "{dim}·  ·    ·      ·        ·{reset}\n\
         {cyan}      ▟▙{reset}\n\
         {blue}   ▟████▙{cyan}     {pink}♥ +1{reset}\n\
         {blue} ▟████████▙{reset}\n\
         {cyan}  ▜██████▛{reset}\n\
         {blue}    ▜██▛{reset}   {dim}Termy X{reset}\n"
    )
}

pub const BIRD_SVG: &str = include_str!("../../../assets/x/termy-x-bird.svg");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_is_an_original_desktop_bird_with_a_heart() {
        assert!(BIRD_SVG.contains("viewBox=\"0 0 1600 1000\""));
        assert!(BIRD_SVG.contains("+1"));
        assert!(BIRD_SVG.contains("#ff3d9a"));
        assert!(BIRD_SVG.to_ascii_lowercase().contains("swift"));
        let lower = BIRD_SVG.to_ascii_lowercase();
        assert!(!lower.contains("twitter"));
        assert!(!lower.contains("logo"));
    }

    #[test]
    fn ansi_splash_uses_color_and_a_heart() {
        let art = render_splash(true);
        assert!(art.contains('♥'));
        assert!(art.contains("+1"));
        assert!(art.contains("Termy X"));
        assert!(art.contains('\u{1b}'));
    }

    #[test]
    fn no_color_strips_ansi() {
        let art = render_splash(false);
        assert!(art.contains('♥'));
        assert!(art.contains("Termy X"));
        assert!(!art.contains('\u{1b}'));
    }
}
