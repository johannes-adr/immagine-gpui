// Liquid-glass pass for the Metal renderer.
// Generated from vendor/gpui-pre-wgpu/src/glass.wgsl with `naga glass.wgsl glass.metal`, then the
// placeholder bindings were set by hand: Params = buffer(0), backdrop = texture(0), samp = sampler(0).
// Keep it in sync with the WGSL source when the look changes.
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Params {
    metal::float4 rect;
    metal::float4 misc;
    metal::float4 shape;
    metal::float4 tint;
};
struct VOut {
    metal::float4 pos;
    metal::float2 px;
    char _pad2[8];
};
constant float MARGIN = 2.0;
constant int TAPS = 12;

float sd_round_rect(
    metal::float2 p,
    metal::float2 half_size,
    float r
) {
    metal::float2 q = (metal::abs(p) - half_size) + metal::float2(r, r);
    return (metal::length(metal::max(q, metal::float2(0.0, 0.0))) + metal::min(metal::max(q.x, q.y), 0.0)) - r;
}

float shape_sd(
    metal::float2 p_1,
    constant Params& P
) {
    metal::float4 _e3 = P.rect;
    float _e10 = P.misc.z;
    float _e11 = sd_round_rect(p_1, _e3.zw * 0.5, _e10);
    return _e11;
}

metal::float3 tap(
    metal::float2 px,
    constant Params& P,
    metal::texture2d<float, metal::access::sample> backdrop,
    metal::sampler samp
) {
    metal::float4 _e3 = P.misc;
    metal::float2 uv = metal::clamp(px / _e3.xy, metal::float2(0.0005, 0.0005), metal::float2(0.9995, 0.9995));
    metal::float4 _e16 = backdrop.sample(samp, uv, metal::level(0.0));
    return _e16.xyz;
}

float noise(
    metal::float2 p_2
) {
    return metal::fract(52.982918 * metal::fract(metal::dot(p_2, metal::float2(0.06711056, 0.00583715))));
}

metal::float3 blurred(
    metal::float2 center,
    float radius,
    constant Params& P,
    metal::texture2d<float, metal::access::sample> backdrop,
    metal::sampler samp
) {
    metal::float3 acc = metal::float3(0.0, 0.0, 0.0);
    float wsum = 0.0;
    int i_1 = 0;
    float sigma2_ = metal::max((radius * radius) * 0.35, 0.0001);
    float _e14 = noise(center);
    float rot = _e14 * 6.2831855;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e50 = i_1;
            i_1 = as_type<int>(as_type<uint>(_e50) + as_type<uint>(1));
        }
        loop_init = false;
        int _e19 = i_1;
        if (_e19 < TAPS) {
        } else {
            break;
        }
        {
            int _e22 = i_1;
            float fi = static_cast<float>(_e22);
            float a = (fi * 2.3999631) + rot;
            float r_1 = metal::sqrt((fi + 0.5) / 12.0) * radius;
            float w = metal::exp(-((r_1 * r_1)) / (2.0 * sigma2_));
            metal::float3 _e39 = acc;
            metal::float3 _e45 = tap(center + (metal::float2(metal::cos(a), metal::sin(a)) * r_1), P, backdrop, samp);
            acc = _e39 + (_e45 * w);
            float _e48 = wsum;
            wsum = _e48 + w;
        }
    }
    metal::float3 _e53 = acc;
    float _e54 = wsum;
    return _e53 / metal::float3(_e54);
}

struct vsInput {
};
struct vsOutput {
    metal::float4 pos [[position]];
    metal::float2 px [[user(loc0), center_perspective]];
};
vertex vsOutput vs(
  uint i [[vertex_id]]
, constant Params& P [[buffer(0)]]
) {
    VOut o = {};
    metal::float2 uv_1 = metal::float2(static_cast<float>(i & 1u), static_cast<float>(i >> 1u));
    metal::float4 _e10 = P.rect;
    metal::float4 _e17 = P.rect;
    metal::float2 p_3 = (_e10.xy - metal::float2(2.0)) + (uv_1 * (_e17.zw + metal::float2(4.0)));
    float _e30 = P.misc.x;
    float _e40 = P.misc.y;
    o.pos = metal::float4(((p_3.x / _e30) * 2.0) - 1.0, 1.0 - ((p_3.y / _e40) * 2.0), 0.0, 1.0);
    o.px = p_3;
    VOut _e50 = o;
    const auto _tmp = _e50;
    return vsOutput { _tmp.pos, _tmp.px };
}


struct fsInput {
    metal::float2 px_1 [[user(loc0), center_perspective]];
};
struct fsOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fsOutput fs(
  fsInput varyings_1 [[stage_in]]
, metal::float4 pos [[position]]
, constant Params& P [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> backdrop [[texture(0)]]
, metal::sampler samp [[sampler(0)]]
) {
    const VOut in = { pos, varyings_1.px_1 };
    float coverage = {};
    metal::float3 col = {};
    metal::float4 _e3 = P.rect;
    metal::float4 _e7 = P.rect;
    metal::float2 c = _e3.xy + (_e7.zw * 0.5);
    metal::float2 p_4 = in.px - c;
    float _e14 = shape_sd(p_4, P);
    coverage = 1.0 - metal::smoothstep(-0.75, 0.75, _e14);
    float _e24 = P.shape.z;
    if (_e24 > 0.0) {
        float _e30 = P.rect.y;
        float _e34 = P.rect.w;
        float from_bottom = (_e30 + _e34) - in.px.y;
        float _e42 = P.shape.z;
        float k = metal::clamp(from_bottom / _e42, 0.0, 1.0);
        float _e47 = coverage;
        coverage = ((_e47 * k) * k) * (3.0 - (2.0 * k));
    }
    float _e55 = coverage;
    if (_e55 <= 0.0) {
        metal::discard_fragment();
    }
    float _e61 = P.shape.x;
    float bezel = metal::max(_e61, 1.0);
    float depth = metal::max(-(_e14), 0.0);
    float _e71 = shape_sd(p_4 + metal::float2(1.0, 0.0), P);
    float _e75 = shape_sd(p_4 - metal::float2(1.0, 0.0), P);
    float _e80 = shape_sd(p_4 + metal::float2(0.0, 1.0), P);
    float _e84 = shape_sd(p_4 - metal::float2(0.0, 1.0), P);
    metal::float2 g = metal::float2(_e71 - _e75, _e80 - _e84);
    metal::float2 n = g / metal::float2(metal::max(metal::length(g), 0.0001));
    float t = metal::clamp(1.0 - (depth / bezel), 0.0, 1.0);
    float bend = 1.0 - metal::sqrt(metal::max(1.0 - (t * t), 0.0));
    float _e110 = P.shape.y;
    metal::float2 shift = (n * bend) * _e110;
    float _e117 = P.misc.w;
    metal::float3 _e118 = blurred(in.px + shift, _e117, P, backdrop, samp);
    col = _e118;
    metal::float3 _e120 = col;
    float luma = metal::dot(_e120, metal::float3(0.299, 0.587, 0.114));
    metal::float3 _e127 = col;
    col = metal::mix(metal::float3(luma, luma, luma), _e127, 1.25);
    metal::float3 _e130 = col;
    col = _e130 + metal::float3(0.03, 0.03, 0.03);
    metal::float3 _e136 = col;
    metal::float4 _e139 = P.tint;
    float _e144 = P.tint.w;
    col = metal::mix(_e136, _e139.xyz, _e144);
    metal::float2 light_dir = metal::float2(-0.5523804, -0.8335922);
    float facing = metal::dot(n, light_dir);
    float rim_band = 1.0 - metal::smoothstep(0.0, 1.8, depth);
    float rim_soft = 1.0 - metal::smoothstep(0.0, 5.0, depth);
    float spec = metal::pow(metal::max(facing, 0.0), 1.6) + (0.35 * metal::pow(metal::max(-(facing), 0.0), 1.6));
    metal::float3 _e172 = col;
    float _e187 = P.shape.w;
    col = _e172 + (metal::float3(1.0, 1.0, 1.0) * (((rim_band * (0.12 + (0.88 * spec))) * 0.95) * _e187));
    metal::float3 _e191 = col;
    float _e208 = P.shape.w;
    col = _e191 + (metal::float3(1.0, 1.0, 1.0) * (((rim_soft * (0.05 + (0.25 * metal::max(facing, 0.0)))) * 0.55) * _e208));
    float _e216 = P.rect.w;
    float top = metal::clamp(0.5 - (p_4.y / metal::max(_e216, 1.0)), 0.0, 1.0);
    metal::float3 _e225 = col;
    col = _e225 + ((metal::float3(0.07, 0.07, 0.07) * top) * top);
    metal::float3 _e233 = col;
    col = metal::clamp(_e233, metal::float3(0.0, 0.0, 0.0), metal::float3(1.0, 1.0, 1.0));
    metal::float3 _e243 = col;
    float _e244 = coverage;
    float _e246 = coverage;
    return fsOutput { metal::float4(_e243 * _e244, _e246) };
}

// Final blit of the offscreen scene to the drawable (from glass_blit.wgsl).
struct GlassBlitOut {
    metal::float4 pos [[position]];
    metal::float2 uv;
};

vertex GlassBlitOut glass_blit_vs(uint i [[vertex_id]]) {
    metal::float2 uv = metal::float2(float((i << 1u) & 2u), float(i & 2u));
    GlassBlitOut o;
    o.pos = metal::float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}

fragment metal::float4 glass_blit_fs(
    GlassBlitOut in [[stage_in]],
    metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]],
    metal::sampler scene_samp [[sampler(0)]]
) {
    return scene_tex.sample(scene_samp, in.uv, metal::level(0.0));
}
