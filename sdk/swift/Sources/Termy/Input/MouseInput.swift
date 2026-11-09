public enum MouseButton: UInt32, Hashable, Sendable {
    case left = 1
    case middle = 2
    case right = 3
}

/// A pointer event at a zero-based viewport cell.
public struct MouseInput: Hashable, Sendable {
    public enum Kind: Hashable, Sendable {
        case press(MouseButton)
        case release(MouseButton)
        case drag(MouseButton)
        case move
        case wheelUp, wheelDown, wheelLeft, wheelRight

        var rawKind: UInt32 {
            switch self {
            case .press: 1
            case .release: 2
            case .drag: 3
            case .move: 4
            case .wheelUp: 5
            case .wheelDown: 6
            case .wheelLeft: 7
            case .wheelRight: 8
            }
        }

        var rawButton: UInt32 {
            switch self {
            case .press(let button), .release(let button), .drag(let button): button.rawValue
            default: 0
            }
        }
    }

    public var kind: Kind
    public var position: GridPosition
    /// Only control, alt, and shift are reported to programs.
    public var modifiers: KeyModifiers

    public init(_ kind: Kind, at position: GridPosition, modifiers: KeyModifiers = []) {
        self.kind = kind
        self.position = position
        self.modifiers = modifiers
    }
}
