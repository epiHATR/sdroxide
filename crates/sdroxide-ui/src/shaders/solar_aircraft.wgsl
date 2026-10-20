// Low-poly ADS-B aircraft on the Earth globe.
//
// One static mesh, placed by `DrawData::model`. Lighting is geometric (face
// normals from screen-space derivatives) so the vertex buffer stays the shared
// pos+uv layout every other solar mesh uses.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_pos: vec4<f32>,
    sun_to_earth: vec4<f32>,
    solar_north: vec4<f32>,
    viewport: vec4<f32>,
    misc: vec4<f32>,
};

struct DrawData {
    model: mat4x4<f32>,
    basis: mat4x4<f32>,
    tint: vec4<f32>,
    tint2: vec4<f32>,
    // x unused, y unused, z opacity, w unused.
    params: vec4<f32>,
    style: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> d: DrawData;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs(@location(0) p: vec3<f32>, @location(1) _uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    let world = d.model * vec4(p, 1.0);
    o.clip = g.view_proj * world;
    o.world = world.xyz;
    return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // Flat-shaded faces without per-vertex normals.
    let n = normalize(cross(dpdx(in.world), dpdy(in.world)));
    let to_sun = normalize(g.sun_pos.xyz - in.world);
    let to_eye = normalize(g.camera_pos.xyz - in.world);
    // Two-sided: thin wings read from both above and below.
    let ndl = abs(dot(n, to_sun)) * 0.55 + 0.40;
    let rim = pow(1.0 - abs(dot(n, to_eye)), 2.2) * 0.30;
    let rgb = d.tint.rgb * (ndl + rim);
    let a = clamp(d.params.z, 0.0, 1.0);
    return vec4(rgb * a, a);
}
