//! Value noise, for the rust's texture and the ragged edge where chrome meets rust.
//! Deterministic: the same picture every time, on every machine.

/// A number in 0–1 for the integer pair `(a, b)`.
pub fn hash(a: i32, b: i32) -> f32 {
    let mut h = (a as u32).wrapping_mul(0x8da6_b343) ^ (b as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 16;
    (h & 0xff_ffff) as f32 / 0xff_ffff as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Smooth noise in 0–1.
pub fn value(x: f32, y: f32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - xi), smooth(y - yi));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash(xi, yi);
    let b = hash(xi + 1, yi);
    let c = hash(xi, yi + 1);
    let d = hash(xi + 1, yi + 1);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

/// `octaves` of noise, each finer and fainter, in 0–1.
pub fn fbm(x: f32, y: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
    for _ in 0..octaves {
        sum += value(x * freq, y * freq) * amp;
        total += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / total
}
