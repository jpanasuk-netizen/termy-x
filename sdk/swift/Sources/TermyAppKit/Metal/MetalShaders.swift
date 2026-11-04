#if os(macOS)
import simd

/// One instanced quad, in device pixels with a top-left origin.
///
/// The layout matches `Quad` in `metalShaderSource`; keep them in sync.
struct MetalQuad {
    enum Mode: UInt32 {
        /// Fill with `color`.
        case solid = 0
        /// Tint the texture's coverage (alpha) with `color`.
        case mask = 1
        /// Draw the texture's own colors at pixel scale, faded by `color.a`.
        case colorGlyph = 2
        /// Draw the texture's own colors, filtered, faded by `color.a`.
        case image = 3
    }

    var rect: SIMD4<Float>
    var uv: SIMD4<Float>
    var color: SIMD4<Float>
    var mode: UInt32
    private var padding: SIMD3<UInt32> = .zero

    init(rect: SIMD4<Float>, uv: SIMD4<Float> = .zero, color: SIMD4<Float>, mode: Mode) {
        self.rect = rect
        self.uv = uv
        self.color = color
        self.mode = mode.rawValue
    }
}

/// Compiled at runtime: SwiftPM command-line builds don't compile `.metal` files.
let metalShaderSource = """
#include <metal_stdlib>
using namespace metal;

struct Quad {
    float4 rect;   // x, y, width, height in pixels
    float4 uv;     // u0, v0, u1, v1 in texels
    float4 color;  // sRGB-encoded, straight alpha
    uint mode;
    uint3 padding;
};

struct Varyings {
    float4 position [[position]];
    float2 uv;
    float4 color;
    uint mode [[flat]];
};

vertex Varyings quad_vertex(
    uint vertexID [[vertex_id]],
    uint instanceID [[instance_id]],
    const device Quad *quads [[buffer(0)]],
    constant float2 &viewport [[buffer(1)]]
) {
    Quad quad = quads[instanceID];
    float2 corner = float2(vertexID & 1, vertexID >> 1);
    float2 point = quad.rect.xy + corner * quad.rect.zw;
    Varyings out;
    out.position = float4(point.x / viewport.x * 2.0 - 1.0, 1.0 - point.y / viewport.y * 2.0, 0.0, 1.0);
    out.uv = mix(quad.uv.xy, quad.uv.zw, corner);
    out.color = quad.color;
    out.mode = quad.mode;
    return out;
}

fragment float4 quad_fragment(Varyings in [[stage_in]], texture2d<float> texture [[texture(0)]]) {
    constexpr sampler nearest(coord::pixel, filter::nearest, address::clamp_to_edge);
    constexpr sampler linear(coord::pixel, filter::linear, address::clamp_to_edge);
    switch (in.mode) {
    case 0:
        return float4(in.color.rgb * in.color.a, in.color.a);
    case 1: {
        float alpha = texture.sample(nearest, in.uv).a * in.color.a;
        return float4(in.color.rgb * alpha, alpha);
    }
    case 2:
        return texture.sample(nearest, in.uv) * in.color.a;
    default:
        return texture.sample(linear, in.uv) * in.color.a;
    }
}
"""
#endif
