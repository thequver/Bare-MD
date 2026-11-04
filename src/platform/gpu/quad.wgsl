// One pipeline draws everything: rounded rectangles, borders and glyphs.
// Every primitive is an instanced quad with a texture lookup into the glyph
// atlas (which also holds a single white texel used for solid fills), a clip
// rectangle and an optional signed-distance rounded-outline.

struct Globals {
    // xy = viewport size in physical pixels
    screen: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_smp: sampler;
// A second texture for coloured content (images). Coverage comes from the atlas
// for everything else; `params.z` picks which texture a quad reads.
@group(0) @binding(3) var pix_tex: texture_2d<f32>;

struct Inst {
    // x0, y0, x1, y1 in physical pixels
    @location(0) rect: vec4<f32>,
    // u0, v0, u1, v1
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    // clip: x0, y0, x1, y1
    @location(3) clip: vec4<f32>,
    // params: radius, border_width, texture (0 = atlas coverage, 1 = colour),
    //         solid (1 = a flat fill: the colour is known, so no atlas fetch)
    @location(4) params: vec4<f32>,
    @location(5) border_color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) px: vec2<f32>,
    @location(3) rect: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) border_color: vec4<f32>,
};

const CORNERS = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    var out: VsOut;
    let c = CORNERS[vi];
    let r = inst.rect;
    let cl = inst.clip;

    let x0 = max(r.x, cl.x);
    let y0 = max(r.y, cl.y);
    let x1 = min(r.z, cl.z);
    let y1 = min(r.w, cl.w);

    if (x1 <= x0 || y1 <= y0) {
        // Degenerate after clipping: park the triangle outside the viewport.
        out.pos = vec4<f32>(4.0, 4.0, 4.0, 1.0);
        out.uv = vec2<f32>(0.0);
        out.color = vec4<f32>(0.0);
        out.px = vec2<f32>(0.0);
        out.rect = r;
        out.params = inst.params;
        out.border_color = inst.border_color;
        return out;
    }

    let px = mix(x0, x1, c.x);
    let py = mix(y0, y1, c.y);

    let fw = max(r.z - r.x, 0.00001);
    let fh = max(r.w - r.y, 0.00001);
    let u = mix(inst.uv.x, inst.uv.z, (px - r.x) / fw);
    let v = mix(inst.uv.y, inst.uv.w, (py - r.y) / fh);

    out.pos = vec4<f32>(
        px / globals.screen.x * 2.0 - 1.0,
        1.0 - py / globals.screen.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = vec2<f32>(u, v);
    out.color = inst.color;
    out.px = vec2<f32>(px, py);
    out.rect = r;
    out.params = inst.params;
    out.border_color = inst.border_color;
    return out;
}

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let rr = min(r, min(b.x, b.y));
    let q = abs(p) - b + vec2<f32>(rr, rr);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - rr;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let radius = in.params.x;
    let border = in.params.y;
    var col = in.color;
    var coverage = 1.0;
    if (in.params.w > 0.5) {
        // A flat fill - a background, a panel, the dialog scrim. The colour is in the
        // instance, so the atlas texel would be pure white and fetching it is wasted
        // work: on a software rasteriser that fetch was most of the cost of every
        // full-screen rectangle in the interface.
        coverage = 1.0;
    } else if (in.params.z > 0.5) {
        // Colour texture: the texel *is* the colour, tinted by the instance
        // colour (white keeps a photo untouched, a theme ink tints an icon).
        let texel = textureSample(pix_tex, atlas_smp, in.uv);
        col = vec4<f32>(texel.rgb * in.color.rgb, texel.a * in.color.a);
        coverage = 1.0;
    } else {
        coverage = textureSample(atlas_tex, atlas_smp, in.uv).r;
        // A gentle S-curve on the anti-aliased coverage. Every covered pixel is
        // either more or less covered than it was, so the stems of a letter get
        // their full ink back and only the (already half-covered) fringe stays
        // soft: the same bitmap reads noticeably sharper on screen, and unlike a
        // gamma change it cannot make thin strokes disappear.
        coverage = clamp(coverage * coverage * (3.0 - 2.0 * coverage), 0.0, 1.0);
    }

    if (radius > 0.0 || border > 0.0) {
        let center = vec2<f32>((in.rect.x + in.rect.z) * 0.5, (in.rect.y + in.rect.w) * 0.5);
        let half = vec2<f32>((in.rect.z - in.rect.x) * 0.5, (in.rect.w - in.rect.y) * 0.5);
        let d = sd_round_box(in.px - center, half, radius);
        let aa = 1.0 - smoothstep(-0.7, 0.7, d);
        if (border > 0.0) {
            let inner = 1.0 - smoothstep(-0.7, 0.7, d + border);
            col = mix(in.border_color, in.color, inner);
        }
        col.a = col.a * aa;
    }

    let a = col.a * coverage;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(col.rgb, a);
}
