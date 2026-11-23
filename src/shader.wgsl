struct U {
    viewport: vec2<f32>,
    cell: vec2<f32>,
    pad: vec2<f32>,
    opacity: f32,
    tiles_per_row: u32,
    cols: u32,
    cursor: u32,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var atlas: texture_2d<f32>;

fn unpack(c: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(c & 255u), f32((c >> 8u) & 255u), f32((c >> 16u) & 255u), f32((c >> 24u) & 255u)
    ) / 255.0;
}

fn to_clip(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0, 0.0, 1.0);
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) tf: u32,
    @location(2) @interpolate(flat) fg: vec4<f32>,
    @location(3) @interpolate(flat) bg: vec4<f32>,
};

const QUAD = array<vec2<f32>, 6>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

// Special atlas tiles and `tf` flag bits: keep in sync with gpu.rs.
const T_BEAM: u32 = 1u;
const T_BAR: u32 = 2u;
const T_BOX: u32 = 3u;
const T_UNDER: u32 = 4u;
const T_STRIKE: u32 = 5u;

// ---- Cells: one instance per grid cell, position = instance index ----------
// `tf`: tile in the low 16 bits; flags above: 16 underline, 17 strikeout,
// 18-19 cursor shape (0 none, 1 beam, 2 bar, 3 box).
@vertex
fn vs_cell(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    @location(0) tf: u32,
    @location(1) fg: u32,
    @location(2) bg: u32,
) -> VOut {
    var o: VOut;
    // Nothing to draw (blank tile, no flags, transparent background): degenerate triangle.
    if tf == 0u && (bg >> 24u) == 0u {
        o.pos = vec4(-2.0, -2.0, 0.0, 1.0);
        o.local = vec2(0.0);
        o.tf = 0u;
        o.fg = vec4(0.0);
        o.bg = vec4(0.0);
        return o;
    }
    let col = f32(ii % u.cols);
    let row = f32(ii / u.cols);
    let q = QUAD[vi];
    o.pos = to_clip(u.pad + (vec2(col, row) + q) * u.cell);
    o.local = q * u.cell;
    o.tf = tf;
    o.fg = unpack(fg);
    o.bg = unpack(bg);
    return o;
}

fn coverage(tile: u32, local: vec2<f32>) -> f32 {
    let cw = u32(u.cell.x);
    let ch = u32(u.cell.y);
    let tx = (tile % u.tiles_per_row) * cw + min(u32(local.x), cw - 1u);
    let ty = (tile / u.tiles_per_row) * ch + min(u32(local.y), ch - 1u);
    return textureLoad(atlas, vec2<u32>(tx, ty), 0).r;
}

// Overlay (underline, strikeout, cursor): `color` at coverage `c` over the premultiplied `dst`.
fn overlay(dst: vec4<f32>, color: vec3<f32>, c: f32) -> vec4<f32> {
    return vec4(color * c, c) + dst * (1.0 - c);
}

@fragment
fn fs_cell(i: VOut) -> @location(0) vec4<f32> {
    let flags = i.tf >> 16u;
    let cov = coverage(i.tf & 0xffffu, i.local);
    let a = mix(i.bg.a, 1.0, cov);
    var px = vec4(i.fg.rgb * cov + i.bg.rgb * i.bg.a * (1.0 - cov), a);
    if (flags & 1u) != 0u {
        px = overlay(px, i.fg.rgb, coverage(T_UNDER, i.local));
    }
    if (flags & 2u) != 0u {
        px = overlay(px, i.fg.rgb, coverage(T_STRIKE, i.local));
    }
    let shape = (flags >> 2u) & 3u;
    if shape != 0u {
        let tile = select(select(T_BOX, T_BAR, shape == 2u), T_BEAM, shape == 1u);
        px = overlay(px, unpack(u.cursor).rgb, coverage(tile, i.local));
    }
    return px;
}

// ---- Background: full-screen triangle with gradient and neon border -------
@vertex
fn vs_bg(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_bg(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag.xy;
    let uv = p / u.viewport;
    let top = vec3(0.030, 0.030, 0.030);
    let bot = vec3(0.045, 0.045, 0.045);
    var c = mix(top, bot, uv.y);
    let d = min(min(p.x, u.viewport.x - p.x), min(p.y, u.viewport.y - p.y));
    let accent = vec3(1.0);
    let glow = exp(-d / 7.0) * 0.16 + select(0.0, 0.55, d < 1.0);
    c = c + accent * glow;
    let a = clamp(u.opacity + glow * 0.5, 0.0, 1.0);
    return vec4(c * a, a);
}
