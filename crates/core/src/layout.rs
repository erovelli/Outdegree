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
    let k = (w * w / n as f32).sqrt();
    // Hard cap on |position| so a runaway force can never push a node to a
    // non-finite or absurd coordinate (which would blank the whole view).
    let bound = 4.0 * w;

    // Deterministic initial placement: warm-start from finite, in-bounds seeds;
    // otherwise a golden-angle spiral so nodes start well-separated and bounded.
    let mut pos: Vec<Pos> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            if let Some(&(x, y)) = seed.get(key) {
                if x.is_finite() && y.is_finite() && x.abs() <= bound && y.abs() <= bound {
                    return Pos { x, y };
                }
            }
            let angle = i as f32 * 2.399_963_2; // golden angle (radians)
            let radius = 0.5 * w * ((i as f32 + 1.0) / n as f32).sqrt();
            Pos {
                x: radius * angle.cos(),
                y: radius * angle.sin(),
            }
        })
        .collect();

    if n == 1 {
        return vec![Pos { x: 0.0, y: 0.0 }];
    }

    // Gravity toward the origin keeps disconnected components from drifting
    // off-screen (the common cause of a "blank" graph). With FR's long-range
    // (1/dist) repulsion, a disconnected cloud settles at radius ≈ w/√gravity, so
    // this value keeps it compact (≈1400) and well inside `bound`.
    let gravity = 0.5f32;
    let mut temp = w / 10.0;
    let cool = temp / (iters.max(1) as f32 + 1.0);

    for _ in 0..iters {
        // Repulsion via Barnes–Hut (O(n log n)) so the layout scales past a few
        // hundred hosts; θ = 0.9 trades a little accuracy for speed.
        let mut disp = barnes_hut_repulsion(&pos, k, 0.9);

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

        // Centering gravity.
        for (i, d) in disp.iter_mut().enumerate() {
            d.0 -= pos[i].x * gravity;
            d.1 -= pos[i].y * gravity;
        }

        // Apply displacement (capped by temperature), then sanitize + clamp so a
        // position can never become non-finite or escape the bound.
        for i in 0..n {
            let mut dx = disp[i].0;
            let mut dy = disp[i].1;
            if !dx.is_finite() {
                dx = 0.0;
            }
            if !dy.is_finite() {
                dy = 0.0;
            }
            let d = (dx * dx + dy * dy).sqrt();
            if d > 1e-6 {
                let lim = d.min(temp);
                pos[i].x += dx / d * lim;
                pos[i].y += dy / d * lim;
            }
            if !pos[i].x.is_finite() {
                pos[i].x = 0.0;
            }
            if !pos[i].y.is_finite() {
                pos[i].y = 0.0;
            }
            pos[i].x = pos[i].x.clamp(-bound, bound);
            pos[i].y = pos[i].y.clamp(-bound, bound);
        }
        temp = (temp - cool).max(0.0);
    }

    pos
}

// ───────────────────────────── Barnes–Hut repulsion ─────────────────────────────

const BH_MAX_DEPTH: u32 = 32;

/// A node of the arena-backed quadtree. `com_*` holds the running sum of member
/// positions during insertion and the average after [`QuadTree::finalize`].
struct QNode {
    cx: f32,
    cy: f32,
    half: f32,
    mass: f32,
    com_x: f32,
    com_y: f32,
    children: [i32; 4],
    body: i32,
    body_x: f32,
    body_y: f32,
}

struct QuadTree {
    nodes: Vec<QNode>,
}

impl QuadTree {
    fn new(cx: f32, cy: f32, half: f32) -> Self {
        QuadTree {
            nodes: vec![QNode {
                cx,
                cy,
                half,
                mass: 0.0,
                com_x: 0.0,
                com_y: 0.0,
                children: [-1; 4],
                body: -1,
                body_x: 0.0,
                body_y: 0.0,
            }],
        }
    }

    fn quadrant(&self, ni: usize, x: f32, y: f32) -> usize {
        let n = &self.nodes[ni];
        (x >= n.cx) as usize + ((y >= n.cy) as usize) * 2
    }

    fn child(&mut self, ni: usize, q: usize) -> usize {
        if self.nodes[ni].children[q] >= 0 {
            return self.nodes[ni].children[q] as usize;
        }
        let (cx, cy, half) = {
            let n = &self.nodes[ni];
            let h = n.half * 0.5;
            let dx = if q & 1 == 1 { h } else { -h };
            let dy = if q & 2 == 2 { h } else { -h };
            (n.cx + dx, n.cy + dy, h)
        };
        let idx = self.nodes.len();
        self.nodes.push(QNode {
            cx,
            cy,
            half,
            mass: 0.0,
            com_x: 0.0,
            com_y: 0.0,
            children: [-1; 4],
            body: -1,
            body_x: 0.0,
            body_y: 0.0,
        });
        self.nodes[ni].children[q] = idx as i32;
        idx
    }

    fn insert(&mut self, body: usize, x: f32, y: f32) {
        self.insert_at(0, body, x, y, 0);
    }

    fn insert_at(&mut self, ni: usize, body: usize, x: f32, y: f32, depth: u32) {
        // accumulate this body into the subtree rooted at ni
        let was_empty = self.nodes[ni].mass == 0.0;
        {
            let n = &mut self.nodes[ni];
            n.mass += 1.0;
            n.com_x += x;
            n.com_y += y;
        }
        if was_empty && self.nodes[ni].body < 0 {
            let n = &mut self.nodes[ni];
            n.body = body as i32;
            n.body_x = x;
            n.body_y = y;
            return;
        }
        if depth >= BH_MAX_DEPTH {
            return; // near-coincident points share this bucket
        }
        // split: relocate an existing single body into a child first
        let existing = self.nodes[ni].body;
        if existing >= 0 {
            let (ox, oy) = (self.nodes[ni].body_x, self.nodes[ni].body_y);
            self.nodes[ni].body = -1;
            let q = self.quadrant(ni, ox, oy);
            let c = self.child(ni, q);
            self.insert_at(c, existing as usize, ox, oy, depth + 1);
        }
        let q = self.quadrant(ni, x, y);
        let c = self.child(ni, q);
        self.insert_at(c, body, x, y, depth + 1);
    }

    fn finalize(&mut self) {
        for n in &mut self.nodes {
            if n.mass > 0.0 {
                n.com_x /= n.mass;
                n.com_y /= n.mass;
            }
        }
    }

    fn force(&self, ni: usize, x: f32, y: f32, i: usize, theta: f32, k2: f32) -> (f32, f32) {
        let n = &self.nodes[ni];
        if n.mass == 0.0 {
            return (0.0, 0.0);
        }
        if n.body >= 0 {
            if n.body as usize == i {
                return (0.0, 0.0); // self
            }
            return repulse(x, y, n.com_x, n.com_y, k2, n.mass);
        }
        let dx = x - n.com_x;
        let dy = y - n.com_y;
        let dist = (dx * dx + dy * dy).sqrt().max(1e-4);
        let has_children = n.children.iter().any(|&c| c >= 0);
        if !has_children || n.half * 2.0 / dist < theta {
            return repulse(x, y, n.com_x, n.com_y, k2, n.mass);
        }
        let mut f = (0.0, 0.0);
        for &c in &n.children {
            if c >= 0 {
                let cf = self.force(c as usize, x, y, i, theta, k2);
                f.0 += cf.0;
                f.1 += cf.1;
            }
        }
        f
    }
}

fn repulse(x: f32, y: f32, ox: f32, oy: f32, k2: f32, mass: f32) -> (f32, f32) {
    let dx = x - ox;
    let dy = y - oy;
    let dist = (dx * dx + dy * dy).sqrt().max(1e-4);
    let force = k2 / dist * mass;
    (dx / dist * force, dy / dist * force)
}

/// Approximate all-pairs repulsion with a Barnes–Hut quadtree (θ controls the
/// accuracy/speed trade-off). Deterministic given the input positions.
fn barnes_hut_repulsion(pos: &[Pos], k: f32, theta: f32) -> Vec<(f32, f32)> {
    let n = pos.len();
    let mut disp = vec![(0.0f32, 0.0f32); n];
    if n < 2 {
        return disp;
    }
    let (mut minx, mut miny) = (f32::INFINITY, f32::INFINITY);
    let (mut maxx, mut maxy) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in pos {
        minx = minx.min(p.x);
        miny = miny.min(p.y);
        maxx = maxx.max(p.x);
        maxy = maxy.max(p.y);
    }
    let cx = (minx + maxx) * 0.5;
    let cy = (miny + maxy) * 0.5;
    let half = ((maxx - minx).max(maxy - miny) * 0.5).max(1.0) + 1.0;

    let mut tree = QuadTree::new(cx, cy, half);
    for (i, p) in pos.iter().enumerate() {
        tree.insert(i, p.x, p.y);
    }
    tree.finalize();

    let k2 = k * k;
    for (i, p) in pos.iter().enumerate() {
        let (fx, fy) = tree.force(0, p.x, p.y, i, theta, k2);
        disp[i].0 = fx;
        disp[i].1 = fy;
    }
    disp
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

    #[test]
    fn barnes_hut_scales_and_stays_finite() {
        // a 400-node ring exercises the quadtree path; must be finite + deterministic
        let n = 400;
        let keys: Vec<String> = (0..n).map(|i| format!("n{i}")).collect();
        let edges: Vec<(usize, usize)> = (0..n).map(|i| (i, (i + 1) % n)).collect();
        let seed = HashMap::new();
        let a = fruchterman_reingold(&keys, &edges, 30, &seed);
        let b = fruchterman_reingold(&keys, &edges, 30, &seed);
        assert_eq!(a.len(), n);
        assert_eq!(a, b, "Barnes–Hut layout must be deterministic");
        assert!(a.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }

    #[test]
    fn coincident_points_do_not_hang_or_nan() {
        // all-same seed positions stress the depth guard
        let keys: Vec<String> = (0..16).map(|i| format!("n{i}")).collect();
        let mut seed = HashMap::new();
        for k in &keys {
            seed.insert(k.clone(), (5.0, 5.0));
        }
        let p = fruchterman_reingold(&keys, &[(0, 1), (1, 2)], 20, &seed);
        assert!(p.iter().all(|q| q.x.is_finite() && q.y.is_finite()));
    }

    #[test]
    fn nan_huge_seeds_stay_finite_and_bounded() {
        // Polluted/runaway seeds (NaN, ±inf, absurd magnitudes) must never yield a
        // non-finite or off-the-map position — this is what blanked the graph.
        let keys: Vec<String> = (0..6).map(|i| format!("n{i}")).collect();
        let mut seed = HashMap::new();
        seed.insert("n0".into(), (f32::NAN, 0.0));
        seed.insert("n1".into(), (f32::INFINITY, f32::NEG_INFINITY));
        seed.insert("n2".into(), (1e30, -1e30));
        let edges = vec![(0, 1), (2, 3)];
        let p = fruchterman_reingold(&keys, &edges, 40, &seed);
        let bound = 4.0 * 1000.0 + 1.0;
        assert!(
            p.iter().all(|q| q.x.is_finite() && q.y.is_finite()),
            "non-finite position: {p:?}"
        );
        assert!(p.iter().all(|q| q.x.abs() <= bound && q.y.abs() <= bound));
    }

    #[test]
    fn disconnected_nodes_stay_centered_by_gravity() {
        // No edges at all: gravity must keep every node within a sane radius of
        // the origin (not drifting off-screen).
        let keys: Vec<String> = (0..30).map(|i| format!("n{i}")).collect();
        let seed = HashMap::new();
        let p = fruchterman_reingold(&keys, &[], 200, &seed);
        assert!(p.iter().all(|q| q.x.is_finite() && q.y.is_finite()));
        // gravity vs. repulsion equilibrium keeps the cloud compact
        let max_r = p
            .iter()
            .map(|q| (q.x * q.x + q.y * q.y).sqrt())
            .fold(0.0f32, f32::max);
        assert!(max_r < 3000.0, "nodes drifted too far: max_r = {max_r}");
    }
}
