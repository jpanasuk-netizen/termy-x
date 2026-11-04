internal import CTermy

/// Cell metrics in points, used to size special-glyph geometry.
public struct GlyphMetrics: Hashable, Sendable {
    public var cellWidth: Float
    public var cellHeight: Float
    public var fontSize: Float

    public init(cellWidth: Float, cellHeight: Float, fontSize: Float) {
        self.cellWidth = cellWidth
        self.cellHeight = cellHeight
        self.fontSize = fontSize
    }
}

public enum GlyphRenderKind: Hashable, Sendable {
    case blockElement, boxDrawing, sextant, braille, roundedCorner, diagonal, unknown(UInt32)

    init(rawValue: UInt32) {
        self = switch rawValue {
        case 1: .blockElement
        case 2: .boxDrawing
        case 3: .sextant
        case 4: .braille
        case 5: .roundedCorner
        case 6: .diagonal
        default: .unknown(rawValue)
        }
    }
}

/// A filled rectangle in cell-relative coordinates (0...1 on each axis).
public struct GlyphRect: Hashable, Sendable {
    public enum Snap: Hashable, Sendable { case nearest, outward }

    public var left: Float
    public var top: Float
    public var right: Float
    public var bottom: Float
    public var alpha: Float
    /// How to round the edges to device pixels.
    public var snap: Snap
}

public struct GlyphPoint: Hashable, Sendable {
    public var x: Float
    public var y: Float
}

/// A stroked path in cell-relative coordinates.
public struct GlyphStroke: Hashable, Sendable {
    public enum Kind: Hashable, Sendable { case line, roundedCorner }

    public var kind: Kind
    /// Fraction of the cell width.
    public var width: Float
    /// Two points for lines; six for rounded corners.
    public var points: [GlyphPoint]
}

/// Geometry for one cell that should be drawn instead of shaping its glyph.
public struct GlyphPlanEntry: Hashable, Sendable {
    /// Row-major index into the full frame.
    public var cellIndex: Int
    public var kind: GlyphRenderKind
    public var rects: [GlyphRect]
    public var strokes: [GlyphStroke]
}

public enum GlyphRenderPlan {
    /// Builds geometry for block elements, box drawing, sextants, Braille,
    /// rounded corners, and diagonals in a full row-major frame.
    ///
    /// Pass nil `spans` to plan the whole frame, or dirty spans to plan only
    /// those cells. Cells without an entry are shaped as ordinary text.
    static func build(
        cells: [TermyFfiCell], cols: Int, rows: Int, spans: [DirtySpan]?, metrics: GlyphMetrics
    ) throws(TermyError) -> [GlyphPlanEntry] {
        guard !cells.isEmpty else { return [] }
        if let spans, spans.isEmpty { return [] }
        let ffiSpans = spans?.map(\.ffi) ?? []
        var plan = TermyFfiGlyphRenderPlan()
        let ffiMetrics = TermyFfiGlyphMetrics(
            cell_width: metrics.cellWidth, cell_height: metrics.cellHeight, font_size: metrics.fontSize)
        let status = cells.withUnsafeBufferPointer { cellBuffer in
            ffiSpans.withUnsafeBufferPointer { spanBuffer in
                termy_cells_build_glyph_render_plan(
                    cellBuffer.baseAddress, cellBuffer.count, UInt16(cols), UInt16(rows),
                    spanBuffer.baseAddress, spanBuffer.count, ffiMetrics, &plan)
            }
        }
        defer { termy_glyph_render_plan_free(&plan) }
        try check(status, "termy_cells_build_glyph_render_plan")

        let rects = UnsafeBufferPointer(start: plan.rects_ptr, count: plan.rects_len)
        let strokes = UnsafeBufferPointer(start: plan.strokes_ptr, count: plan.strokes_len)
        return UnsafeBufferPointer(start: plan.entries_ptr, count: plan.entries_len).map { entry in
            GlyphPlanEntry(
                cellIndex: entry.cell_index,
                kind: GlyphRenderKind(rawValue: entry.render_kind),
                rects: rects[entry.rect_start ..< entry.rect_start + entry.rect_len].map(GlyphRect.init),
                strokes: strokes[entry.stroke_start ..< entry.stroke_start + entry.stroke_len]
                    .map(GlyphStroke.init)
            )
        }
    }
}

extension FrameStore {
    /// Special-glyph geometry for the retained frame. Pass nil to plan every
    /// cell, or the dirty spans of the last update to plan only those.
    public func glyphPlan(metrics: GlyphMetrics, spans: [DirtySpan]? = nil) throws(TermyError) -> [GlyphPlanEntry] {
        try GlyphRenderPlan.build(cells: rawCells, cols: cols, rows: rows, spans: spans, metrics: metrics)
    }
}

extension GlyphRect {
    init(_ rect: TermyFfiGlyphRect) {
        left = rect.left
        top = rect.top
        right = rect.right
        bottom = rect.bottom
        alpha = rect.alpha
        snap = rect.snap_kind == TERMY_FFI_GLYPH_RECT_SNAP_OUTWARD.rawValue ? .outward : .nearest
    }
}

extension GlyphStroke {
    init(_ stroke: TermyFfiGlyphStroke) {
        kind = stroke.kind == TERMY_FFI_GLYPH_STROKE_ROUNDED_CORNER.rawValue ? .roundedCorner : .line
        width = stroke.width
        let all = [stroke.point_0, stroke.point_1, stroke.point_2,
                   stroke.point_3, stroke.point_4, stroke.point_5]
        points = all.prefix(Int(min(stroke.point_count, 6))).map { GlyphPoint(x: $0.x, y: $0.y) }
    }
}
