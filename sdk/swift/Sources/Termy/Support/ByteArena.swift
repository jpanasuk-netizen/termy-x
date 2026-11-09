import Foundation
internal import CTermy

/// Owns copies of byte buffers so C structs can point at them for the duration
/// of a call. Everything is released together when the arena is deallocated.
final class ByteArena {
    private var allocations: [UnsafeMutableRawPointer] = []

    deinit {
        allocations.forEach { $0.deallocate() }
    }

    func slice(_ data: some DataProtocol) -> TermyFfiByteSlice {
        let count = data.count
        guard count > 0 else { return TermyFfiByteSlice(ptr: nil, len: 0) }
        let raw = UnsafeMutableRawPointer.allocate(byteCount: count, alignment: 1)
        allocations.append(raw)
        let typed = raw.bindMemory(to: UInt8.self, capacity: count)
        _ = UnsafeMutableBufferPointer(start: typed, count: count).initialize(from: data)
        return TermyFfiByteSlice(ptr: UnsafePointer(typed), len: count)
    }

    func slice(_ string: String) -> TermyFfiByteSlice {
        slice(Array(string.utf8))
    }

    /// Copies `values` into a stable C array.
    func array<T>(_ values: [T]) -> UnsafePointer<T>? {
        guard !values.isEmpty else { return nil }
        let raw = UnsafeMutableRawPointer.allocate(
            byteCount: MemoryLayout<T>.stride * values.count,
            alignment: MemoryLayout<T>.alignment
        )
        allocations.append(raw)
        let typed = raw.bindMemory(to: T.self, capacity: values.count)
        _ = UnsafeMutableBufferPointer(start: typed, count: values.count).initialize(from: values)
        return UnsafePointer(typed)
    }
}
