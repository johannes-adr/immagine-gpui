// Liquid glass: blurs and refracts a copy of the frame painted so far through a rounded rect.
// Uses only a uniform buffer, a texture and a sampler so it runs on WebGPU and WebGL2 alike.

struct Params {
    rect: vec4<f32>,   // x, y, w, h in device px
    misc: vec4<f32>,   // viewport w, viewport h, corner radius, blur radius
    shape: vec4<f32>,  // bezel width, max refraction, dispersion, light strength
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var backdrop: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,
};

const MARGIN: f32 = 2.0;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let p = P.rect.xy - MARGIN + uv * (P.rect.zw + 2.0 * MARGIN);
    var o: VOut;
    o.pos = vec4<f32>(p.x / P.misc.x * 2.0 - 1.0, 1.0 - p.y / P.misc.y * 2.0, 0.0, 1.0);
    o.px = p;
    return o;
}

fn sd_round_rect(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn shape_sd(p: vec2<f32>) -> f32 {
    return sd_round_rect(p, P.rect.zw * 0.5, P.misc.z);
}

fn tap(px: vec2<f32>) -> vec3<f32> {
    let uv = clamp(px / P.misc.xy, vec2<f32>(0.0005, 0.0005), vec2<f32>(0.9995, 0.9995));
    return textureSampleLevel(backdrop, samp, uv, 0.0).rgb;
}

// Golden-angle spiral blur, one tap set per color channel so dispersion can split them.
const TAPS: i32 = 20;

fn blurred(center: vec2<f32>, radius: f32) -> vec3<f32> {
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    var wsum = 0.0;
    let sigma2 = max(radius * radius * 0.35, 0.0001);
    for (var i = 0; i < TAPS; i = i + 1) {
        let fi = f32(i);
        let a = fi * 2.39996323;
        let r = sqrt((fi + 0.5) / f32(TAPS)) * radius;
        let w = exp(-(r * r) / (2.0 * sigma2));
        acc = acc + tap(center + vec2<f32>(cos(a), sin(a)) * r) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let c = P.rect.xy + P.rect.zw * 0.5;
    let p = in.px - c;
    let d = shape_sd(p);
    let coverage = 1.0 - smoothstep(-0.75, 0.75, d);
    if (coverage <= 0.0) {
        discard;
    }

    let bezel = max(P.shape.x, 1.0);
    let depth = max(-d, 0.0);

    // Outward normal of the glass edge from the SDF gradient.
    let e = 1.0;
    let g = vec2<f32>(
        shape_sd(p + vec2<f32>(e, 0.0)) - shape_sd(p - vec2<f32>(e, 0.0)),
        shape_sd(p + vec2<f32>(0.0, e)) - shape_sd(p - vec2<f32>(0.0, e)),
    );
    let n = g / max(length(g), 0.0001);

    // Convex-lens profile: displacement grows steeply toward the rim and is zero in the flat middle.
    let t = clamp(1.0 - depth / bezel, 0.0, 1.0);
    let bend = 1.0 - sqrt(max(1.0 - t * t, 0.0));
    // The rim pulls in what lies just outside the glass, squeezed into the bezel.
    let shift = n * bend * P.shape.y;

    let blur = P.misc.w;
    let disp = P.shape.z * bend * 6.0;
    var col: vec3<f32>;
    col.r = blurred(in.px + shift * (1.0 + P.shape.z), blur).r;
    col.g = blurred(in.px + shift, blur).g;
    col.b = blurred(in.px + shift * (1.0 - P.shape.z), blur).b;

    // Vibrancy: saturate and lift, as the system material does.
    let luma = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = mix(vec3<f32>(luma, luma, luma), col, 1.25);
    col = col + vec3<f32>(0.03, 0.03, 0.03);

    // Tint (alpha controls how much of it).
    col = mix(col, P.tint.rgb, P.tint.a);

    // Light from the top-left: the rim facing it glows brightly, the opposite rim catches a faint bounce.
    let light_dir = normalize(vec2<f32>(-0.55, -0.83));
    let facing = dot(n, light_dir);
    let rim_band = 1.0 - smoothstep(0.0, 1.8, depth);
    let rim_soft = 1.0 - smoothstep(0.0, 5.0, depth);
    let spec = pow(max(facing, 0.0), 1.6) + 0.35 * pow(max(-facing, 0.0), 1.6);
    col = col + vec3<f32>(1.0, 1.0, 1.0) * (rim_band * (0.12 + 0.88 * spec) * 0.95 * P.shape.w);
    col = col + vec3<f32>(1.0, 1.0, 1.0) * (rim_soft * (0.05 + 0.25 * max(facing, 0.0)) * 0.55 * P.shape.w);

    // Broad soft sheen over the upper half.
    let top = clamp(0.5 - p.y / max(P.rect.w, 1.0), 0.0, 1.0);
    col = col + vec3<f32>(0.07, 0.07, 0.07) * top * top;

    col = clamp(col, vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(1.0, 1.0, 1.0));
    return vec4<f32>(col * coverage, coverage);
}
