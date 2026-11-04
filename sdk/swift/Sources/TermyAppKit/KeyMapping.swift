#if os(macOS)
import AppKit
import Termy

/// Translates AppKit key events into libtermy keystrokes.
public enum KeyMapping {
    /// libtermy's name for a non-text key, by macOS virtual key code.
    public static func namedKey(forKeyCode keyCode: UInt16) -> String? {
        switch keyCode {
        case 36, 76: "enter"
        case 48: "tab"
        case 49: "space"
        case 51: "backspace"
        case 53: "escape"
        case 117: "delete"
        case 114: "insert"
        case 115: "home"
        case 119: "end"
        case 116: "pageup"
        case 121: "pagedown"
        case 123: "left"
        case 124: "right"
        case 125: "down"
        case 126: "up"
        case 122: "f1"
        case 120: "f2"
        case 99: "f3"
        case 118: "f4"
        case 96: "f5"
        case 97: "f6"
        case 98: "f7"
        case 100: "f8"
        case 101: "f9"
        case 109: "f10"
        case 103: "f11"
        case 111: "f12"
        case 105: "f13"
        case 107: "f14"
        case 113: "f15"
        case 106: "f16"
        case 64: "f17"
        case 79: "f18"
        case 80: "f19"
        case 90: "f20"
        default: nil
        }
    }

    public static func modifiers(_ flags: NSEvent.ModifierFlags) -> KeyModifiers {
        var modifiers: KeyModifiers = []
        if flags.contains(.control) { modifiers.insert(.control) }
        if flags.contains(.option) { modifiers.insert(.alt) }
        if flags.contains(.shift) { modifiers.insert(.shift) }
        if flags.contains(.command) { modifiers.insert(.platform) }
        return modifiers
    }

    /// Builds a keystroke from key event fields. Returns nil for events that
    /// carry nothing to send, such as a lone modifier.
    public static func keystroke(
        keyCode: UInt16,
        characters: String?,
        charactersIgnoringModifiers: String?,
        modifierFlags: NSEvent.ModifierFlags,
        isRepeat: Bool = false
    ) -> Keystroke? {
        let modifiers = modifiers(modifierFlags)
        let kind: KeyEventKind = isRepeat ? .repeat : .press
        if let name = namedKey(forKeyCode: keyCode) {
            return Keystroke(key: name, modifiers: modifiers, kind: kind)
        }
        guard let base = charactersIgnoringModifiers, !base.isEmpty else { return nil }
        // Drop AppKit's private-use function-key characters.
        if let scalar = base.unicodeScalars.first, (0xF700...0xF8FF).contains(scalar.value) { return nil }
        let text = characters.flatMap { text -> String? in
            guard !text.isEmpty, !text.unicodeScalars.contains(where: { $0.value < 0x20 || $0.value == 0x7F })
            else { return nil }
            return text
        }
        return Keystroke(key: base.lowercased(), text: text, modifiers: modifiers, kind: kind)
    }

    public static func keystroke(for event: NSEvent) -> Keystroke? {
        keystroke(
            keyCode: event.keyCode,
            characters: event.characters,
            charactersIgnoringModifiers: event.charactersIgnoringModifiers,
            modifierFlags: event.modifierFlags,
            isRepeat: event.isARepeat
        )
    }

    /// Whether a key event should go through the input method (IME, dead keys)
    /// rather than straight to the encoder.
    static func routesThroughInputMethod(_ event: NSEvent, optionAsAlt: Bool) -> Bool {
        let flags = event.modifierFlags
        if flags.contains(.control) || flags.contains(.command) { return false }
        if flags.contains(.option), optionAsAlt { return false }
        return namedKey(forKeyCode: event.keyCode) == nil
    }
}
#endif
