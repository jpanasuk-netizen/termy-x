pub fn splash_ansi() -> String {
    let blue = "\u{1b}[38;2;47;123;255m";
    let cyan = "\u{1b}[38;2;122;246;255m";
    let pink = "\u{1b}[38;2;255;61;154m";
    let dim = "\u{1b}[38;2;90;122;156m";
    let reset = "\u{1b}[0m";
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
    fn svg_is_an_original_ultrawide_bird_with_a_heart() {
        assert!(BIRD_SVG.contains("viewBox=\"0 0 3440 1440\""));
        assert!(BIRD_SVG.contains("+1"));
        assert!(BIRD_SVG.contains("#ff3d9a"));
        assert!(BIRD_SVG.to_ascii_lowercase().contains("swift"));
        let lower = BIRD_SVG.to_ascii_lowercase();
        assert!(!lower.contains("twitter"));
        assert!(!lower.contains("logo"));
    }

    #[test]
    fn ansi_splash_uses_color_and_a_heart() {
        let art = splash_ansi();
        assert!(art.contains('♥'));
        assert!(art.contains("+1"));
        assert!(art.contains("Termy X"));
    }
}
