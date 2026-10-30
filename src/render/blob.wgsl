// Блоб: плоское пятно из метаболов, внутри — «жидкий мрамор» из искажённого шума
// и цветных разводов в четырёх цветах. Та же формула — в render::blob_px (CPU).

struct Params {
    c: array<vec4<f32>, 4>,   // палитра (0..1)
    p: vec4<f32>,             // phase, energy, aspect (w/h), _
};

@group(0) @binding(0) var<uniform> P: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return o;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    return 0.55 * noise(p) + 0.30 * noise(p * 2.03 + 7.1) + 0.15 * noise(p * 4.01 + 3.7);
}

// Циклическая палитра из четырёх цветов с плавными переходами.
fn palette(x: f32) -> vec3<f32> {
    let s = fract(x) * 4.0;
    let i = i32(floor(s));
    let f = smoothstep(0.0, 1.0, fract(s));
    return mix(P.c[i % 4].rgb, P.c[(i + 1) % 4].rgb, f);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let t = P.p.x;
    let e = P.p.y;
    let aspect = P.p.z;
    var q = in.uv * 2.0 - 1.0;
    if (aspect > 1.0) { q.x *= aspect; } else { q.y /= aspect; }

    // Форма: метаболы, плавно меняющие положение и размер.
    var f = 0.0;
    for (var k = 0; k < 5; k++) {
        let fk = f32(k);
        let amp = 0.28 + 0.10 * e;
        let c = vec2<f32>(amp * sin(t * (0.21 + 0.07 * fk) + fk * 1.9), amp * cos(t * (0.17 + 0.05 * fk) + fk * 2.7));
        let r = 0.36 + 0.10 * sin(t * 0.33 + fk * 1.3);
        f += exp(-dot(q - c, q - c) / (r * r));
    }
    // Край чуть «дышит» шумом — живой, а не циркульный.
    f += (fbm(q * 2.2 + t * 0.15) - 0.5) * 0.35;
    let a = smoothstep(0.42, 0.62, f);

    // Внутри: поле шума, искажённое самим собой, — разводы и полосы.
    let w1 = vec2<f32>(fbm(q * 1.6 + vec2<f32>(t * 0.11, -t * 0.07)), fbm(q * 1.6 + vec2<f32>(5.2 - t * 0.09, 1.3 + t * 0.05)));
    let w2 = vec2<f32>(fbm(q * 1.8 + w1 * 2.4 + 1.7), fbm(q * 1.8 + w1 * 2.4 + 9.2));
    let v = fbm(q * 1.3 + w2 * 2.2 + t * 0.04);
    let bands = 0.5 + 0.5 * sin(v * 9.0 + t * 0.6 + w1.x * 4.0);
    let col = mix(palette(v * 1.6 + t * 0.03), palette(v * 1.6 + 0.5 + t * 0.03), bands * 0.45);

    let n = fract(sin(dot(in.pos.xy, vec2<f32>(12.9898, 78.233))) * 43758.547) - 0.5;
    return vec4<f32>(clamp(col + n / 255.0, vec3<f32>(0.0), vec3<f32>(1.0)), a * 0.95);
}
