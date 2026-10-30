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
