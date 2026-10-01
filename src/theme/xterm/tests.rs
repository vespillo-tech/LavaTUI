use super::*;

fn reference_pair(c: Rgb) -> Pair {
    let cands = candidates();
    let x = Lab::of(c);
    let hues = Hues::of(c);
    let xl = linear(c);
    let mut ranked: Vec<(f32, usize)> = cands
        .iter()
        .enumerate()
        .filter(|(_, cand)| hues.allow(cand.lab))
        .map(|(i, cand)| (distance(x, cand.lab), i))
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (single, best) = ranked[0];
    let alone = Pair {
        near: 16 + best as u8,
        far: 16 + best as u8,
        level: 0,
    };
    // Only dark tints whose single match loses the hue: anywhere else the
    // single match is close enough, and a flat colour beats a pattern.
    let chroma = x.chroma();
    if x.l > DARK || chroma < TINT || cands[best].lab.chroma() > HUE_KEPT * chroma {
        return alone;
    }
    let mut found = (single * GAIN, alone);
    for &(_, a) in ranked.iter().take(NEAR_ENDS) {
        let ca = &cands[a];
        for &(_, b) in &ranked {
            let cb = &cands[b];
            let spread = distance(ca.lab, cb.lab);
            if spread < SAME {
                continue;
            }
            let d: [f32; 3] = std::array::from_fn(|k| cb.linear[k] - ca.linear[k]);
            let len2: f32 = d.iter().map(|v| v * v).sum();
            let proj: f32 = (0..3).map(|k| (xl[k] - ca.linear[k]) * d[k]).sum::<f32>() / len2;
            let level = (proj * LEVELS as f32).round();
            if !(1.0..LEVELS as f32).contains(&level) {
                continue;
            }
            let f = level / LEVELS as f32;
            let mix = Lab::of_linear(std::array::from_fn(|k| ca.linear[k] + d[k] * f));
            let score = distance(x, mix) + PATTERN_COST * spread;
            if score < found.0 {
                found = (
                    score,
                    Pair {
                        near: 16 + a as u8,
                        far: 16 + b as u8,
                        level: level as u8,
                    },
                );
            }
        }
    }
    found.1
}

#[test]
fn pair_search_preserves_stable_ranking() {
    for i in 0..8192u32 {
        let key = (i * 7919) % (1 << (3 * BITS));
        let (_, rep) = bucket(Rgb(
            ((key >> 12) << 2) as u8,
            ((key >> 6) << 2) as u8,
            (key << 2) as u8,
        ));
        assert_eq!(search_pair(rep), reference_pair(rep), "{rep:?}");
    }
}

#[test]
#[ignore = "cold matcher benchmark"]
fn bench_xterm_cold() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::time::Instant;
    let mut fingerprint = DefaultHasher::new();
    let mut times = Vec::new();
    for i in 0..(1 << (3 * BITS)) {
        let key = (i * 7919) % (1 << (3 * BITS));
        let (_, rep) = bucket(Rgb(
            ((key >> 12) << 2) as u8,
            ((key >> 6) << 2) as u8,
            (key << 2) as u8,
        ));
        let start = Instant::now();
        let pair = search_pair(rep);
        times.push(start.elapsed().as_nanos() as u64);
        pair.pack().hash(&mut fingerprint);
    }
    times.sort_unstable();
    println!(
        "cold pairs mean/p99/max us {:.2}/{:.2}/{:.2} hash {:016x}",
        times.iter().sum::<u64>() as f64 / times.len() as f64 / 1000.,
        times[times.len() * 99 / 100] as f64 / 1000.,
        times[times.len() - 1] as f64 / 1000.,
        fingerprint.finish()
    );
}
