//! Force-directed layout (§7.6): Fruchterman–Reingold with warm-start from saved
//! positions so reopening preserves spatial memory.
//!
//! Deviation from the §7.6 sketch: takes `keys: &[String]` (index→key) instead of
//! a bare `n`, so warm-start by key works when the node set changes between
//! opens. Uses a small deterministic PRNG (no `rand` dependency) to keep the core
//! pure and reproducible under `cargo test`.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

/// Layout `keys.len()` nodes connected by `edges` (index pairs). Existing nodes
/// warm-start from `seed[key]`; new nodes get deterministic fresh placement.
pub fn fruchterman_reingold(
    keys: &[String],
    edges: &[(usize, usize)],
    iters: u32,
    seed: &HashMap<String, (f32, f32)>,
) -> Vec<Pos> {
    let n = keys.len();
    if n == 0 {
        return Vec::new();
    }

    let w = 1000.0f32;
    let area = w * w;
    let k = (area / n as f32).sqrt();

    let mut pos: Vec<Pos> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| match seed.get(key) {
            Some(&(x, y)) => Pos { x, y },
            None => {
                let mut s = (i as u64)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    ^ 0xD1B5_4A32_D192_ED03;
                let rx = (next_f32(&mut s) - 0.5) * w;
                let ry = (next_f32(&mut s) - 0.5) * w;
                Pos { x: rx, y: ry }
            }
        })
        .collect();

    if n == 1 {
        return pos;
    }

    let mut temp = w / 10.0;
    let cool = temp / (iters.max(1) as f32 + 1.0);

    for _ in 0..iters {
        let mut disp = vec![(0.0f32, 0.0f32); n];

        // Repulsion between every pair.
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].x - pos[j].x;
                let dy = pos[i].y - pos[j].y;
                let dist = (dx * dx + dy * dy).sqrt().max(1e-4);
                let force = k * k / dist;
                let (ux, uy) = (dx / dist, dy / dist);
                disp[i].0 += ux * force;
                disp[i].1 += uy * force;
                disp[j].0 -= ux * force;
                disp[j].1 -= uy * force;
            }
        }

        // Attraction along edges.
        for &(a, b) in edges {
            if a >= n || b >= n || a == b {
                continue;
            }
            let dx = pos[a].x - pos[b].x;
            let dy = pos[a].y - pos[b].y;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-4);
            let force = dist * dist / k;
            let (ux, uy) = (dx / dist, dy / dist);
            disp[a].0 -= ux * force;
            disp[a].1 -= uy * force;
            disp[b].0 += ux * force;
            disp[b].1 += uy * force;
        }

        // Apply displacement, capped by the cooling temperature.
        for i in 0..n {
            let (dx, dy) = disp[i];
            let d = (dx * dx + dy * dy).sqrt().max(1e-4);
            let lim = d.min(temp);
            pos[i].x += dx / d * lim;
            pos[i].y += dy / d * lim;
        }
        temp = (temp - cool).max(0.0);
    }

    pos
}

fn splitmix_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn next_f32(state: &mut u64) -> f32 {
    // 24 random bits mapped to [0, 1).
    (splitmix_next(state) >> 40) as f32 / (1u64 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_start_preserves_seeded_positions_with_zero_iters() {
        let keys: Vec<String> = vec!["a".into(), "b".into(), "new".into()];
        let mut seed = HashMap::new();
        seed.insert("a".to_string(), (12.0, 34.0));
        seed.insert("b".to_string(), (-5.0, 7.0));
        let pos = fruchterman_reingold(&keys, &[(0, 1)], 0, &seed);
        assert_eq!(pos.len(), 3);
        assert_eq!(pos[0], Pos { x: 12.0, y: 34.0 });
        assert_eq!(pos[1], Pos { x: -5.0, y: 7.0 });
        // fresh node placed somewhere finite
        assert!(pos[2].x.is_finite() && pos[2].y.is_finite());
    }

    #[test]
    fn deterministic_and_finite() {
        let keys: Vec<String> = (0..8).map(|i| format!("n{i}")).collect();
        let edges = vec![(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (6, 7)];
        let seed = HashMap::new();
        let a = fruchterman_reingold(&keys, &edges, 50, &seed);
        let b = fruchterman_reingold(&keys, &edges, 50, &seed);
        assert_eq!(a.len(), 8);
        assert_eq!(a, b, "layout must be deterministic");
        assert!(a.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }
}
