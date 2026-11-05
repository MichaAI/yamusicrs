// Блоб: плоские пастельные метаболы. Поле r²/d² даёт «перемычки» при сближении,
// цвет — острова шаров с мягкими стыками, внутри — медленные изолинии, снаружи — ореол.
// Та же формула — в render::blob_px (CPU).

struct Params {
    c: array<vec4<f32>, 4>,   // палитра (0..1), уже пастельная
    p: vec4<f32>,             // phase, energy, aspect (w/h), сила дизеринга в ступенях 1/255 (0 — нет)
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

// Шар k: центр и радиус в момент t. 0..4 — основные, 5..6 — быстрые капли-спутники.
fn ball(k: i32, t: f32, e: f32) -> vec3<f32> {
    let fk = f32(k);
    if (k < 5) {
        let amp = 0.26 + 0.10 * e;
        let c = vec2<f32>(amp * sin(t * (0.23 + 0.06 * fk) + fk * 2.1),
                          amp * 0.9 * sin(t * (0.17 + 0.05 * fk) + fk * 1.3 + 1.2));
        return vec3<f32>(c, 0.24 + 0.05 * sin(t * 0.31 + fk * 1.7));
    }
    let amp = 0.55 + 0.08 * sin(t * 0.4 + fk);
    let a = t * (0.45 + 0.1 * fk) + fk * 3.0;
    return vec3<f32>(amp * cos(a), amp * 0.8 * sin(a * 1.3), 0.09);
}


@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let t = P.p.x;
    let e = P.p.y;
    let aspect = P.p.z;
    var q = in.uv * 2.0 - 1.0;
    if (aspect > 1.0) { q.x *= aspect; } else { q.y /= aspect; }

    var f = 0.0;
    var wsum = 0.0;
    var acc = vec3<f32>(0.0);
    // Центры шаров; основные мягко расталкиваются, чтобы не сходились почти вплотную.
    var bs: array<vec3<f32>, 7>;
    for (var k = 0; k < 7; k++) { bs[k] = ball(k, t, e); }
    for (var it = 0; it < 2; it++) {
        for (var i = 0; i < 5; i++) {
            var push = vec2<f32>(0.0);
            for (var j = 0; j < 5; j++) {
                if (j == i) { continue; }
                let d = bs[i].xy - bs[j].xy;
                let l = length(d);
                let m = 0.75 * (bs[i].z + bs[j].z);
                if (l < m) {
                    // В совпавшей точке направление берём по номерам — расходятся всегда в разные стороны.
                    let dir = select(normalize(vec2<f32>(f32(i - j), 0.5)), d / l, l > 1e-3);
                    let x = (m - l) / m;
                    push += dir * (m - l) * 0.5 * x * (2.0 - x);
                }
            }
            bs[i] = vec3<f32>(bs[i].xy + push, bs[i].z);
        }
    }
    for (var k = 0; k < 7; k++) {
        let b = bs[k];
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
