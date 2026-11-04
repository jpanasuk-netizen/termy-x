/// Modifier keys held during an input event.
public struct KeyModifiers: OptionSet, Hashable, Sendable {
    public let rawValue: UInt8

    public init(rawValue: UInt8) {
        self.rawValue = rawValue
    }

    public static let control = KeyModifiers(rawValue: 1 << 0)
    public static let alt = KeyModifiers(rawValue: 1 << 1)
    public static let shift = KeyModifiers(rawValue: 1 << 2)
    /// Command on macOS, Super/Windows elsewhere.
    public static let platform = KeyModifiers(rawValue: 1 << 3)
    public static let function = KeyModifiers(rawValue: 1 << 4)
}

public enum KeyEventKind: UInt32, Hashable, Sendable {
    case press = 1
    case `repeat` = 2
    case release = 3
}

/// A logical key event for libtermy's mode-aware encoder.
///
/// `key` is a lowercase logical name: a single character such as `"a"`, or a
/// named key such as `"enter"`, `"tab"`, `"escape"`, `"backspace"`, `"space"`,
/// `"up"`, `"down"`, `"left"`, `"right"`, `"home"`, `"end"`, `"pageup"`,
/// `"pagedown"`, `"insert"`, `"delete"`, or `"f1"` through `"f35"`.
/// `text` is the text the key produced, if any (for example `"A"` for
/// shift+a, or `"é"` from a dead key).
public struct Keystroke: Hashable, Sendable {
    public var key: String
    public var text: String?
    public var modifiers: KeyModifiers
    public var kind: KeyEventKind

    public init(
        key: String, text: String? = nil, modifiers: KeyModifiers = [], kind: KeyEventKind = .press
    ) {
        self.key = key
        self.text = text
        self.modifiers = modifiers
        self.kind = kind
    }

    /// A press that types `text`, such as committed IME input.
    public static func text(_ text: String) -> Keystroke {
        Keystroke(key: text.lowercased(), text: text)
    }

    public static let enter = Keystroke(key: "enter")
    public static let tab = Keystroke(key: "tab")
    public static let escape = Keystroke(key: "escape")
    public static let backspace = Keystroke(key: "backspace")
    public static let up = Keystroke(key: "up")
    public static let down = Keystroke(key: "down")
    public static let left = Keystroke(key: "left")
    public static let right = Keystroke(key: "right")

    /// Control plus a letter, such as `.control("c")` for an interrupt.
    public static func control(_ key: Character) -> Keystroke {
        Keystroke(key: String(key).lowercased(), modifiers: .control)
    }
}
