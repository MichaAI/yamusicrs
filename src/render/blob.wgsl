// Блоб Моей волны: метаболы с объёмным освещением. Та же формула — в render::blob_pixel (CPU).

struct Params {
    c0: vec4<f32>,      // основной цвет (0..1)
    c1: vec4<f32>,      // второй цвет
    c2: vec4<f32>,      // промежуточный оттенок
    p: vec4<f32>,       // phase, energy, aspect (w/h в пикселях), _
};

@group(0) @binding(0) var<uniform> P: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    // Один треугольник на весь экран.
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return o;
}

const BALLS = array<vec4<f32>, 5>(
    vec4<f32>(0.31, 0.53, 0.0, 0.50),
    vec4<f32>(0.47, 0.29, 2.1, 0.43),
    vec4<f32>(0.23, 0.41, 4.2, 0.38),
    vec4<f32>(0.39, 0.61, 1.3, 0.33),
    vec4<f32>(0.17, 0.37, 5.1, 0.30),
);

fn field(q: vec2<f32>, t: f32, e: f32, w: ptr<function, vec3<f32>>) -> f32 {
    var f = 0.0;
    var acc = vec3<f32>(0.0);
    for (var k = 0; k < 5; k++) {
        let b = BALLS[k];
        let amp = 0.30 + 0.12 * e;
        let c = vec2<f32>(amp * sin(t * b.x + b.z), amp * cos(t * b.y + b.z * 1.7));
        let r = b.w * (0.92 + 0.08 * sin(t * 0.7 + b.z));
        let d2 = dot(q - c, q - c);
        let v = exp(-d2 / (r * r));
        f += v;
        // Каждому шару — свой цвет из трёх.
        let m = k % 3;
        acc += v * select(select(P.c2.rgb, P.c1.rgb, m == 1), P.c0.rgb, m == 0);
    }
    *w = acc / max(f, 1e-4);
    return f;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let t = P.p.x;
    let e = P.p.y;
    let aspect = P.p.z;
    // Координаты в [-1,1] с учётом пропорций: блоб круглый, а не растянутый.
    var q = in.uv * 2.0 - 1.0;
    if (aspect > 1.0) { q.x *= aspect; } else { q.y /= aspect; }
    var col: vec3<f32>;
    let f = field(q, t, e, &col);
    // Объём: свет сверху-слева по градиенту поля.
    var tmp: vec3<f32>;
    let eps = 0.01;
    let gx = field(q + vec2<f32>(eps, 0.0), t, e, &tmp) - f;
    let gy = field(q + vec2<f32>(0.0, eps), t, e, &tmp) - f;
    let n = normalize(vec3<f32>(-gx / eps, -gy / eps, 1.2));
    let light = clamp(dot(n, normalize(vec3<f32>(-0.5, -0.6, 0.6))), 0.0, 1.0);
    let lit = col * (0.78 + 0.32 * light) + vec3<f32>(pow(light, 18.0) * 0.18);
    // Мягкий край и тихое свечение вокруг.
    let core = smoothstep(0.30, 1.05, f);
    let glow = smoothstep(0.05, 0.6, f) * 0.25;
    let a = clamp(max(core, glow) * 0.9, 0.0, 1.0);
    // Дизеринг против полос на плавных переходах.
    let n2 = fract(sin(dot(in.pos.xy, vec2<f32>(12.9898, 78.233))) * 43758.547) - 0.5;
    return vec4<f32>(clamp(lit + n2 / 255.0, vec3<f32>(0.0), vec3<f32>(1.0)), a);
}
