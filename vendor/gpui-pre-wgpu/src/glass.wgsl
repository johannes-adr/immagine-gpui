// Liquid glass: blurs and refracts a copy of the frame painted so far through a rounded rect.
// Uses only a uniform buffer, a texture and a sampler so it runs on WebGPU and WebGL2 alike.

struct Params {
    rect: vec4<f32>,   // x, y, w, h in device px
    misc: vec4<f32>,   // viewport w, viewport h, corner radius, blur radius
    shape: vec4<f32>,  // bezel width, max refraction, bottom fade height, light strength
    tint: vec4<f32>,
    extra: vec4<f32>,  // x: look variant (0 = original, 1 = Apple-like)
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var backdrop: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,
};

const MARGIN: f32 = 2.0;
// The Apple-like look draws a soft drop shadow outside the shape, so its quad is larger.
const MARGIN_V2: f32 = 20.0;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = select(MARGIN, MARGIN_V2, P.extra.x > 0.5);
    let p = P.rect.xy - m + uv * (P.rect.zw + 2.0 * m);
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

// Golden-angle spiral blur. The spiral is rotated per pixel (interleaved gradient noise) so
// the sparse taps turn into fine grain instead of visible ghost copies.
const TAPS: i32 = 12;

fn noise(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

fn blurred(center: vec2<f32>, radius: f32) -> vec3<f32> {
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    var wsum = 0.0;
    let sigma2 = max(radius * radius * 0.35, 0.0001);
    let rot = noise(center) * 6.2831853;
    for (var i = 0; i < TAPS; i = i + 1) {
        let fi = f32(i);
        let a = fi * 2.39996323 + rot;
        let r = sqrt((fi + 0.5) / f32(TAPS)) * radius;
        let w = exp(-(r * r) / (2.0 * sigma2));
        acc = acc + tap(center + vec2<f32>(cos(a), sin(a)) * r) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}

// Variant 1: closer to Apple's Liquid Glass. Refraction lives in a thin band at the edge (steep profile,
// a little dispersion), the middle is only lightly blurred so content stays readable, the tint adapts to
// the brightness behind it, the rim is a thin bright line lit from the top left and (fainter) the bottom
// right, and a soft shadow grounds the shape.
fn glass_v2(in: VOut) -> vec4<f32> {
    let c = P.rect.xy + P.rect.zw * 0.5;
    let p = in.px - c;
    let d = shape_sd(p);
    var fade = 1.0;
    if (P.shape.z > 0.0) {
        let from_bottom = P.rect.y + P.rect.w - in.px.y;
        let k = clamp(from_bottom / P.shape.z, 0.0, 1.0);
        fade = k * k * (3.0 - 2.0 * k);
    }
    let coverage = (1.0 - smoothstep(-0.75, 0.75, d)) * fade;

    // Drop shadow, only for shapes with a rim light (not for the full-width blur bars).
    var shadow = 0.0;
    if (P.shape.w > 0.0 && P.shape.z <= 0.0) {
        let sd = shape_sd(p - vec2<f32>(0.0, 4.0));
        shadow = 0.26 * P.shape.w * (1.0 - smoothstep(-3.0, 16.0, sd));
    }
    let behind = shadow * (1.0 - coverage);
    if (coverage <= 0.0) {
        if (behind <= 0.002) {
            discard;
        }
        return vec4<f32>(0.0, 0.0, 0.0, behind);
    }

    let bezel = max(P.shape.x, 1.0);
    let depth = max(-d, 0.0);
    let e = 1.0;
    let g = vec2<f32>(
        shape_sd(p + vec2<f32>(e, 0.0)) - shape_sd(p - vec2<f32>(e, 0.0)),
        shape_sd(p + vec2<f32>(0.0, e)) - shape_sd(p - vec2<f32>(0.0, e)),
    );
    let n = g / max(length(g), 0.0001);

    // Steep lens profile: nothing in the middle, strong bending in the outer part of the bezel.
    let t = clamp(1.0 - depth / bezel, 0.0, 1.0);
    let bend = t * t * t;
    let shift = n * bend * P.shape.y * 1.7;

    // Light blur in the middle, more smear toward the edge.
    let radius = mix(P.misc.w * 0.42, P.misc.w * 0.9, t * t);
    var col = blurred(in.px + shift, radius);

    // Dispersion in the edge band only (three single taps, so the middle pays nothing).
    if (bend > 0.04) {
        let spread = 0.10 * bend;
        let split = vec3<f32>(
            tap(in.px + shift * (1.0 + spread)).r,
            tap(in.px + shift).g,
            tap(in.px + shift * (1.0 - spread)).b,
        );
        col = mix(col, split, clamp(bend * 0.6, 0.0, 0.6));
    }

    // Slight frosting and vibrancy.
    let luma = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = mix(vec3<f32>(luma, luma, luma), col, 1.15);
    col = mix(col, vec3<f32>(0.5, 0.5, 0.5), 0.06) + vec3<f32>(0.025, 0.025, 0.025);

    // Adaptive tint: more of it over bright content, so the glass stays legible and never washes out.
    let tint_a = clamp(P.tint.a * (0.6 + 0.9 * luma), 0.0, 1.0);
    col = mix(col, P.tint.rgb, tint_a);

    // Thin bright rim with two specular lobes (top left strong, bottom right nearly as strong),
    // plus a soft inner glow just inside it.
    let light_dir = normalize(vec2<f32>(-0.55, -0.83));
    let facing = dot(n, light_dir);
    let spec = pow(max(facing, 0.0), 2.0) + 0.85 * pow(max(-facing, 0.0), 2.0);
    let rim_line = 1.0 - smoothstep(0.35, 1.5, depth);
    let rim_glow = 1.0 - smoothstep(0.0, max(bezel * 0.55, 3.0), depth);
    col = col + vec3<f32>(1.0, 1.0, 1.0) * (rim_line * (0.20 + 0.80 * spec) * 0.9 * P.shape.w);
    col = col + vec3<f32>(1.0, 1.0, 1.0) * (rim_glow * (0.03 + 0.12 * spec) * P.shape.w);

    col = clamp(col, vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(1.0, 1.0, 1.0));
    return vec4<f32>(col * coverage, coverage + behind);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if (P.extra.x > 0.5) {
        return glass_v2(in);
    }
    let c = P.rect.xy + P.rect.zw * 0.5;
    let p = in.px - c;
    let d = shape_sd(p);
    var coverage = 1.0 - smoothstep(-0.75, 0.75, d);
    // Soft bottom edge: the effect fades out over the last `fade` px.
    if (P.shape.z > 0.0) {
        let from_bottom = P.rect.y + P.rect.w - in.px.y;
        let k = clamp(from_bottom / P.shape.z, 0.0, 1.0);
        coverage = coverage * k * k * (3.0 - 2.0 * k);
    }
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

    // One blur of the refracted backdrop (the old per-channel dispersion cost 3x the taps).
    var col = blurred(in.px + shift, P.misc.w);

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
