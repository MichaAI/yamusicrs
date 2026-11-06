// Блоб: плоские пастельные метаболы. Поле r²/d² даёт «перемычки» при сближении,
// цвет — острова шаров с мягкими стыками, внутри — медленные изолинии, снаружи — ореол.
// Та же формула — в render::blob_px (CPU).

struct Params {
    c: array<vec4<f32>, 4>,   // палитра (0..1), уже пастельная
    p: vec4<f32>,             // phase, energy, aspect (w/h), сила дизеринга в ступенях 1/255 (0 — нет)
    b: array<vec4<f32>, 7>,   // шары кадра (x, y, r, —) — считаются на CPU (render::balls)
    bars: vec4<f32>,          // полоса спектра в долях кадра (x0, y0, x1, y1); x1 <= x0 — нет спектра
    lv: array<vec4<f32>, 8>,  // уровни 32 столбиков
    fl: vec4<f32>,            // x = 1: без блоба, только столбики
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

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = P.p.z;
    var q = in.uv * 2.0 - 1.0;
    if (aspect > 1.0) { q.x *= aspect; } else { q.y /= aspect; }

    var f = 0.0;
    var wsum = 0.0;
    var acc = vec3<f32>(0.0);
    for (var k = 0; k < 7; k++) {
        let b = P.b[k].xyz;
        let d2 = max(dot(q - b.xy, q - b.xy), 1e-4);
        let v = b.z * b.z / d2;
        f += v;
        // Острова цвета: вес шара резко растёт с его влиянием → мягкие, но узкие стыки.
        let w = pow(v, 3.0);
        wsum += w;
        acc += w * P.c[k % 4].rgb;
    }
    let col = acc / max(wsum, 1e-6);


    // Тело: край по изоповерхности f=1, сглаженный; снаружи — мягкий ореол.
    // Сами метаболы плавно растворяются к краю (без отдельного свечения):
    // прозрачность растёт по широкой зоне поля, к середине — почти сплошная заливка.
    let body = smoothstep(0.30, 1.35, f);
    // Затухание к краям холста: ореол не упирается в границу картинки.
    let edge = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
    let a = body * 0.94 * smoothstep(0.0, 0.12, edge);
    var out = vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), a);
    if (P.fl.x > 0.5) { out = vec4<f32>(0.0); }
    // Спектр: столбики с круглыми концами в полосе между обложкой и текстом.
    if (P.bars.z > P.bars.x) {
        let n = 32.0;
        let slot = (P.bars.z - P.bars.x) / n;
        let xu = (in.uv.x - P.bars.x) / slot;
        if (xu >= 0.0 && xu < n && in.uv.y >= P.bars.y && in.uv.y <= P.bars.w) {
            let i = u32(floor(xu));
            let bw = slot * aspect * 0.62;
            let cx = P.bars.x + (f32(i) + 0.5) * slot;
            let total = P.bars.w - P.bars.y;
            let hgt = max(P.lv[i / 4u][i % 4u] * total, bw);
            let d0 = vec2<f32>((in.uv.x - cx) * aspect, in.uv.y - (P.bars.w - hgt * 0.5));
            let q = abs(d0) - vec2<f32>(bw * 0.5, hgt * 0.5) + vec2<f32>(bw * 0.5);
            let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - bw * 0.5;
            let cov = clamp(0.5 - d / fwidth(in.uv.y), 0.0, 1.0) * 0.9;
            let t = clamp((P.bars.w - in.uv.y) / total, 0.0, 1.0);
            let m = mix(P.c[1].rgb, P.c[2].rgb, t);
            let bc = m + (vec3<f32>(1.0) - m) * 0.2;
            let ra = cov + out.a * (1.0 - cov);
            out = vec4<f32>((bc * cov + out.rgb * out.a * (1.0 - cov)) / max(ra, 1e-6), ra);
        }
    }
    // Дизеринг против «колец» на градиентах: шум в одну ступень 8 бит (interleaved
    // gradient noise — равномерный, без комков; после растяжения в kitty незаметен).
    let amp = P.p.w / 255.0;
    if (amp > 0.0) {
        let ign = fract(52.9829189 * fract(dot(in.pos.xy, vec2<f32>(0.06711056, 0.00583715))));
        let ign2 = fract(ign + 0.5);
        out = clamp(out + vec4<f32>(vec3<f32>(ign - 0.5), ign2 - 0.5) * amp, vec4<f32>(0.0), vec4<f32>(1.0));
    }
    return out;
}
