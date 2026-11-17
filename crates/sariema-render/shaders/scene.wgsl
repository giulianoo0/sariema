// SPDX-License-Identifier: Apache-2.0
// Room geometry with clustered forward lighting. Appended to common.wgsl.
// Light units and falloff follow three.js r182 (physically based lights).

const PI: f32 = 3.14159265;
const SPOT: u32 = 1u;
const COUNT_BITS: u32 = 8u;
// A highlight's own colour is its base over its brightest channel, never
// over less than this; its rim falls off as (1 − n·v) to this power, wider
// than a material's rim so it reads at a distance.
const GLOW_FLOOR: f32 = 0.08;
const GLOW_RIM_POWER: f32 = 3.0;

struct Light {
    position: vec3<f32>,
    // 0 means no cutoff.
    range: f32,
    // Linear colour × candela.
    color: vec3<f32>,
    cone_cos: f32,
    // Unit vector from the spot's target towards the light.
    direction: vec3<f32>,
    penumbra_cos: f32,
    // Shadow atlas slot, -1 for none.
    shadow: i32,
    flags: u32,
}

struct Material {
    base_color: vec4<f32>,
    emissive: vec3<f32>,
    roughness: f32,
    uv_offset: vec2<f32>,
    uv_scale: vec2<f32>,
    metallic: f32,
    rim: f32,
    // Written to the mask in alpha: 1 for the "drawn" look.
    drawn: f32,
    // The actor face whose atlas cell sets the offset, -1 for none.
    face: f32,
}

// The per-frame arrays, read through light_at, cluster_at, light_index_at
// and shadow_at, are in tables_buffers.wgsl or tables_textures.wgsl.

struct Shadow {
    // World to the atlas: texture coordinates in xy and depth in z, after dividing by w.
    to_atlas: mat4x4<f32>,
    // The slot's texture coordinates, min in xy and max in zw.
    bounds: vec4<f32>,
    // World size of a texel: x + y × distance to the light.
    texel: vec2<f32>,
}

@group(0) @binding(4) var material_sampler: sampler;
@group(0) @binding(5) var shadow_atlas: texture_depth_2d;
@group(0) @binding(6) var shadow_sampler: sampler_comparison;
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var base_texture: texture_2d<f32>;
@group(2) @binding(2) var emissive_texture: texture_2d<f32>;

override LIT: bool = true;
// Premultiplied output with opacity in alpha, for blended and additive pipelines.
override BLEND: bool = false;
// Shadow lookups compare a PCF_SIDE × PCF_SIDE grid of texels.
override PCF_SIDE: u32 = 1u;
// An opaque draw with presence below 1 drops that share of its pixels, in a
// pattern that changes every pixel; a negative presence uses the pattern's
// complement, so two rooms at p and −(1 − p) never draw the same pixel. Only
// its own pipelines discard, so the others keep early depth testing on
// tile-based GPUs.
override DITHER: bool = false;
// Actors take the character light, and the colour of the room's light only
// as far as its room tint says.
override ACTOR: bool = false;
// Additive: fog fades what it adds instead of turning it the fog's colour.
override GLOW: bool = false;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) view_depth: f32,
    @location(5) @interpolate(flat) presence: f32,
    // How much it glows in its own colour: over all of it, and at its rim.
    @location(6) @interpolate(flat) glow: vec2<f32>,
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let world = draw.model * vec4(v.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * world;
    out.world = world.xyz;
    out.normal = draw_normal() * v.normal;
    out.presence = draw.normal_x.w;
    out.glow = vec2(draw.normal_y.w, draw.normal_z.w);
    out.uv = v.uv * material.uv_scale + material.uv_offset;
    out.color = v.color;
    out.view_depth = -(frame.view * world).z;
    return out;
}

@vertex
fn vs_depth(v: VertexIn) -> @builtin(position) vec4<f32> {
    return frame.view_proj * draw.model * vec4(v.position, 1.0);
}

struct DepthOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex
fn vs_depth_cut(v: VertexIn) -> DepthOut {
    // The product in the order vs_depth takes it, for the same depth.
    let clip = frame.view_proj * draw.model * vec4(v.position, 1.0);
    return DepthOut(clip, (draw.model * vec4(v.position, 1.0)).xyz);
}

// The depth of a draw in a cutaway, without what is cut.
@fragment
fn fs_depth_cut(in: DepthOut, @builtin(front_facing) front: bool) {
    if cut_drops(in.world, front, dither(in.clip.xy), frame.camera_pos.xyz) {
        discard;
    }
}

// Jimenez's interleaved gradient noise: a threshold in [0, 1) per pixel
// with no visible pattern at a glance.
fn dither(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(floor(pixel), vec2(0.06711056, 0.00583715))));
}

// Whether a dissolving draw leaves this pixel out.
fn dithered_out(in: VertexOut) -> bool {
    let d = dither(in.clip.xy);
    return abs(in.presence) <= select(d, 1.0 - d, in.presence < 0.0);
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if DITHER && dithered_out(in) {
        discard;
    }
    let base = material.base_color * textureSample(base_texture, material_sampler, in.uv) * in.color;
    let emissive = material.emissive * textureSample(emissive_texture, material_sampler, in.uv).rgb;
    return shade(in, front, base, emissive);
}

// A draw in a cutaway: without what is cut, its inside in the cap's colour.
@fragment
fn fs_cut(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let eye = frame.camera_pos.xyz;
    if (DITHER && dithered_out(in)) || cut_drops(in.world, front, dither(in.clip.xy), eye) {
        discard;
    }
    let base = material.base_color * textureSample(base_texture, material_sampler, in.uv) * in.color;
    let emissive = material.emissive * textureSample(emissive_texture, material_sampler, in.uv).rgb;
    // Selected, not branched on: shading takes derivatives, which a browser's
    // WGSL allows only in uniform control flow.
    // The cap is lit as a lid facing the camera, whichever inside face shows it.
    let capped = cut_capped(in.world, front, eye);
    let shown = select(base, vec4(cut.cap.rgb, base.a), capped);
    var lid = in;
    lid.normal = select(in.normal, in.world - eye, capped);
    return shade(lid, front, shown, select(emissive, vec3(0.0), capped));
}

fn shade(in: VertexOut, front: bool, base: vec4<f32>, emissive: vec3<f32>) -> vec4<f32> {
    let presence = abs(in.presence);
    // A back face, drawn only for double-sided materials, is shaded from its own side.
    let n = normalize(in.normal) * select(-1.0, 1.0, front);
    let v = normalize(frame.camera_pos.xyz - in.world);
    var color = base.rgb;
    if LIT {
        color = lit(in, n, v, base.rgb);
        if ACTOR && character_on() {
            color = character_light(color, n, v, base.rgb * (1.0 - material.metallic));
        }
    }
    color += emissive;
    if material.rim > 0.0 {
        color += material.rim * pow(1.0 - saturate(dot(n, v)), 5.2);
    }
    if any(in.glow > vec2(0.0)) {
        color += glow(base.rgb, in.glow, n, v);
    }
    if frame.fog.w > 0.0 {
        let f = fog_share(distance(frame.camera_pos.xyz, in.world), frame.fog);
        color = select(mix(color, frame.fog_color.rgb, f), color * (1.0 - f), GLOW);
    }
    if BLEND {
        return vec4(color * base.a, base.a) * presence;
    }
    return vec4(color, material.drawn);
}

// A highlight in the surface's own colour, never white: its albedo and
// vertex colour scaled so the brightest channel is 1, a dark piece keeping
// its hue and a near-black one a faint grey.
fn glow(base: vec3<f32>, levels: vec2<f32>, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let own = base / max(max(base.r, max(base.g, base.b)), GLOW_FLOOR);
    let rim = pow(1.0 - saturate(dot(n, v)), GLOW_RIM_POWER);
    return own * (levels.x + levels.y * rim);
}

fn lit(in: VertexOut, n: vec3<f32>, v: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    let diffuse = base * (1.0 - material.metallic);
    let f0 = mix(vec3(0.04), base, material.metallic);
    // Widen highlights where the normal changes fast across a pixel, as three.js does.
    let dn = max(abs(dpdx(n)), abs(dpdy(n)));
    let roughness = min(max(material.roughness, 0.0525) + max(dn.x, max(dn.y, dn.z)), 1.0);
    let alpha = roughness * roughness;

    var color = room_light(ambient(n)) * diffuse / PI;
    let sun = shadow(i32(frame.sun_dir.w), in.world, n, 0.0);
    color += room_light(frame.sun_color.rgb) * sun * brdf(n, v, frame.sun_dir.xyz, diffuse, f0, alpha);
    let cell = cluster_at(cluster_index(in.clip.xy, in.view_depth));
    let first = cell >> COUNT_BITS;
    let count = cell & ((1u << COUNT_BITS) - 1u);
    for (var i = 0u; i < count; i++) {
        let light = light_at(light_index_at(first + i));
        let to_light = light.position - in.world;
        let d2 = dot(to_light, to_light);
        let l = to_light * inverseSqrt(max(d2, 1e-12));
        let reaching = falloff(light, l, d2) * shadow(light.shadow, in.world, n, sqrt(d2));
        color += room_light(light.color) * reaching * brdf(n, v, l, diffuse, f0, alpha);
    }
    return color;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// The colour of the room's light `c` as it reaches what is drawn: whole, or
// on an actor mixed towards its luminance as far as the room tint is below 1.
// The test is the same for every fragment of a frame, so a room that keeps
// the default tint skips the mix without diverging.
fn room_light(c: vec3<f32>) -> vec3<f32> {
    if ACTOR && frame.character.w < 1.0 {
        return mix(vec3(luminance(c)), c, frame.character.w);
    }
    return c;
}

// Whether the room gives actors any fill, rim or least brightness.
fn character_on() -> bool {
    return frame.character_color.w > 0.0 || frame.character.x > 0.0 || frame.character.z > 0.0;
}

// An actor's lit `color` with the character light: a fill that casts no
// shadow and gives no highlight, wrapped past the side it faces as Godot's
// DIFFUSE_LAMBERT_WRAP; a rim tinted by the albedo, as Godot's rim with
// rim_tint 1, so a thin facet seen edge-on brightens in its own colour and
// never white; then whatever the least brightness still asks for, made up
// in the albedo's own hue.
fn character_light(color: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>) -> vec3<f32> {
    let light = frame.character_color.rgb;
    let wrap = frame.character_dir.w;
    let facing = saturate((dot(n, frame.character_dir.xyz) + wrap) / (1.0 + wrap));
    var c = color + light * frame.character_color.w * facing * diffuse / PI;
    // The floor keeps pow away from 0⁰, which WGSL leaves undefined.
    let edge = pow(max(1.0 - saturate(dot(n, v)), 1e-4), frame.character.y);
    c += light * frame.character.x * edge * diffuse;
    let albedo = max(luminance(diffuse), 1e-4);
    let least = frame.character.z * albedo;
    return c + diffuse * max(least - luminance(c), 0.0) / albedo;
}

// Fraction of a light reaching `world` past the casters in shadow slot
// `slot`, 1 for no slot. Bilinear comparisons make each tap a 2×2 filter.
fn shadow(slot: i32, world: vec3<f32>, n: vec3<f32>, light_distance: f32) -> f32 {
    if slot < 0 {
        return 1.0;
    }
    let s = shadow_at(slot);
    let texel = s.texel.x + s.texel.y * light_distance;
    // Looking up from beyond the filter's reach off the surface keeps lit faces from shadowing themselves.
    let p = s.to_atlas * vec4(world + n * texel * (0.5 * f32(PCF_SIDE) + 1.0), 1.0);
    let at = p.xyz / p.w;
    if p.w <= 0.0 || any(at.xy < s.bounds.xy) || any(at.xy > s.bounds.zw) {
        return 1.0;
    }
    // A receiver beyond the far plane, behind every caster, compares there.
    let depth = min(at.z, 1.0);
    let step = 1.0 / vec2<f32>(textureDimensions(shadow_atlas));
    let centre = 0.5 * f32(PCF_SIDE - 1u);
    var lit = 0.0;
    for (var i = 0u; i < PCF_SIDE * PCF_SIDE; i++) {
        let offset = (vec2(f32(i % PCF_SIDE), f32(i / PCF_SIDE)) - centre) * step;
        let uv = clamp(at.xy + offset, s.bounds.xy, s.bounds.zw);
        lit += textureSampleCompareLevel(shadow_atlas, shadow_sampler, uv, depth);
    }
    return lit / f32(PCF_SIDE * PCF_SIDE);
}

fn cluster_index(pixel: vec2<f32>, depth: f32) -> u32 {
    let g = frame.grid;
    let tile = min(vec2<u32>(pixel / f32(g.w)), g.xy - 1u);
    let slice = clamp(floor(log(depth) * frame.slicing.x + frame.slicing.y), 0.0, f32(g.z - 1u));
    return (u32(slice) * g.y + tile.y) * g.x + tile.x;
}

// three.js getDistanceAttenuation with decay 2, times getSpotAttenuation.
fn falloff(light: Light, l: vec3<f32>, d2: f32) -> f32 {
    var f = 1.0 / max(d2, 0.01);
    if light.range > 0.0 {
        let x = d2 / (light.range * light.range);
        let window = saturate(1.0 - x * x);
        f *= window * window;
    }
    if (light.flags & SPOT) != 0u {
        // smoothstep(cone_cos, penumbra_cos, ·), still defined when the two meet.
        let t = saturate((dot(l, light.direction) - light.cone_cos) / max(light.penumbra_cos - light.cone_cos, 1e-4));
        f *= t * t * (3.0 - 2.0 * t);
    }
    return f;
}

// Radiance reflected towards v per unit of light irradiance from l: Lambert
// diffuse plus GGX with height-correlated Smith visibility and Schlick
// Fresnel (f90 = 1), times n·l.
fn brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, alpha: f32) -> vec3<f32> {
    let hv = l + v;
    let h = hv * inverseSqrt(max(dot(hv, hv), 1e-12));
    let nl = saturate(dot(n, l));
    let nv = saturate(dot(n, v));
    let nh = saturate(dot(n, h));
    let vh = saturate(dot(v, h));
    let a2 = alpha * alpha;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * dd * dd);
    let visibility = 0.5 / max(nl * sqrt(a2 + (1.0 - a2) * nv * nv) + nv * sqrt(a2 + (1.0 - a2) * nl * nl), 1e-6);
    return (diffuse / PI + fresnel * distribution * visibility) * nl;
}

// Same basis and order as sariema_format::eval_sh9.
fn ambient(n: vec3<f32>) -> vec3<f32> {
    let s = frame.sh;
    return s[0].rgb * 0.2820948
        + s[1].rgb * (0.4886025 * n.y)
        + s[2].rgb * (0.4886025 * n.z)
        + s[3].rgb * (0.4886025 * n.x)
        + s[4].rgb * (1.0925484 * n.x * n.y)
        + s[5].rgb * (1.0925484 * n.y * n.z)
        + s[6].rgb * (0.31539157 * (3.0 * n.z * n.z - 1.0))
        + s[7].rgb * (1.0925484 * n.x * n.z)
        + s[8].rgb * (0.5462742 * (n.x * n.x - n.y * n.y));
}

struct BackgroundOut {
    @builtin(position) clip: vec4<f32>,
    // 0 at the bottom of the screen, 1 at the top.
    @location(0) up: f32,
}

@vertex
fn vs_background(@builtin(vertex_index) i: u32) -> BackgroundOut {
    let corner = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return BackgroundOut(vec4(corner * 2.0 - 1.0, 0.0, 1.0), corner.y);
}

@fragment
fn fs_background(in: BackgroundOut) -> @location(0) vec4<f32> {
    return vec4(mix(frame.background_bottom.rgb, frame.background_top.rgb, in.up), 0.0);
}
