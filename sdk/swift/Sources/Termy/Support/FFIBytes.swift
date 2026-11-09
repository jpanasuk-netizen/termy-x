import Foundation
internal import CTermy

extension TermyFfiBytes {
    /// Copies the owned buffer. The caller still frees it.
    func copyData() -> Data {
        guard let ptr, len > 0 else { return Data() }
        return Data(bytes: ptr, count: len)
    }

    /// Decodes the owned buffer as UTF-8, repairing invalid sequences.
    func copyString() -> String {
        guard let ptr, len > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: ptr, count: len), as: UTF8.self)
    }
}

extension TermyFfiByteSlice {
    func copyData() -> Data {
        guard let ptr, len > 0 else { return Data() }
        return Data(bytes: ptr, count: len)
    }

    func copyString() -> String {
        guard let ptr, len > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: ptr, count: len), as: UTF8.self)
    }
}

/// Calls a libtermy function that returns an owned `TermyFfiBytes`, copies the
/// result, and frees the buffer.
func takeBytes(
    _ operation: String,
    _ call: (UnsafeMutablePointer<TermyFfiBytes>) -> TermyFfiStatus
) throws(TermyError) -> Data {
    var bytes = termy_null_buffer()
    let status = call(&bytes)
    defer { termy_buffer_free(bytes) }
    try check(status, operation)
    return bytes.copyData()
}

func takeString(
    _ operation: String,
    _ call: (UnsafeMutablePointer<TermyFfiBytes>) -> TermyFfiStatus
) throws(TermyError) -> String {
    String(decoding: try takeBytes(operation, call), as: UTF8.self)
}

/// Passes a string's UTF-8 bytes as a (pointer, length) pair valid for `body`.
func withUTF8<R, E: Error>(
    _ string: String,
    _ body: (UnsafePointer<UInt8>?, Int) throws(E) -> R
) throws(E) -> R {
    // String.withUTF8 only rethrows untyped errors before Swift 6.4's stdlib.
    try withBytes(Array(string.utf8), body)
}

/// Like `withUTF8`, but passes `(nil, 0)` for an absent string.
func withOptionalUTF8<R, E: Error>(
    _ string: String?,
    _ body: (UnsafePointer<UInt8>?, Int) throws(E) -> R
) throws(E) -> R {
    guard let string else { return try body(nil, 0) }
    return try withUTF8(string, body)
}

func withBytes<R, E: Error>(
    _ data: some DataProtocol,
    _ body: (UnsafePointer<UInt8>?, Int) throws(E) -> R
) throws(E) -> R {
    let bytes = Array(data)
    return try bytes.withUnsafeBufferPointer { (buffer) throws(E) -> R in
        try body(buffer.baseAddress, buffer.count)
    }
}
