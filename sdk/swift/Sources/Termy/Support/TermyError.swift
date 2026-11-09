internal import CTermy

/// A libtermy call returned a non-OK status.
public struct TermyError: Error, Equatable, Sendable, CustomStringConvertible {
    public enum Code: Equatable, Sendable {
        case nullPointer
        case invalidUTF8
        case spawnFailed
        case configLoadFailed
        case unknownKey
        case writeFailed
        case serializeFailed
        /// libtermy caught a Rust panic before it crossed the C ABI.
        case panicked
        case invalidArgument
        case unknown(UInt32)
    }

    public let code: Code
    /// The C function that failed, for diagnostics.
    public let operation: String

    public init(code: Code, operation: String) {
        self.code = code
        self.operation = operation
    }

    public var description: String { "\(operation) failed: \(code)" }

    init(status: TermyFfiStatus, operation: String) {
        let code: Code = switch status {
        case TERMY_FFI_NULL: .nullPointer
        case TERMY_FFI_INVALID_UTF8: .invalidUTF8
        case TERMY_FFI_SPAWN_FAILED: .spawnFailed
        case TERMY_FFI_CONFIG_LOAD_FAILED: .configLoadFailed
        case TERMY_FFI_UNKNOWN_KEY: .unknownKey
        case TERMY_FFI_WRITE_FAILED: .writeFailed
        case TERMY_FFI_SERIALIZE_FAILED: .serializeFailed
        case TERMY_FFI_PANICKED: .panicked
        case TERMY_FFI_INVALID_ARGUMENT: .invalidArgument
        default: .unknown(status.rawValue)
        }
        self.init(code: code, operation: operation)
    }
}

/// Throws unless `status` is `TERMY_FFI_OK`.
@inline(__always)
func check(_ status: TermyFfiStatus, _ operation: @autoclosure () -> String) throws(TermyError) {
    guard status == TERMY_FFI_OK else {
        throw TermyError(status: status, operation: operation())
    }
}
