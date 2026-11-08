//! Where each node sits: a force layout that gives the same graph the same picture. A node starts
//! where a hash of its key puts it, or beside a neighbour already placed, never where a random
//! draw puts it. So a graph opened twice looks the same, and a graph that grows keeps what a
//! person has already seen where it was. A node someone drags stays where it was put.
use eframe::egui::{vec2, Pos2, Rect, Vec2};
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

/// The length a link settles at between the edges of its two nodes, in points, so a line shows
/// about this much whatever the size of the names at its ends.
const LINK: f32 = 56.0;
/// Repulsion reaches no further than this, so separate parts of a graph sit near each other.
const REACH: f32 = 4.0 * LINK;
/// The space kept between the boxes of two nodes.
const GAP: f32 = 12.0;
/// A pull toward the middle, which keeps unconnected parts in view.
const GRAVITY: f32 = 0.02;
const COOLING: f32 = 0.955;
const SETTLED: f32 = 0.5;
/// Nothing is placed further out than this, whatever the forces say.
const BOUND: f32 = 50_000.0;
/// A layout that has not settled after this many steps stops where it is.
const MAX_STEPS: usize = 2_000;

/// A number in [0, 1) that depends only on `key` and `salt`.
fn unit(key: &str, salt: u8) -> f32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[salt]);
    hasher.update(key.as_bytes());
    let bytes = hasher.finalize();
    let word = u32::from_le_bytes([
        bytes.as_bytes()[0],
        bytes.as_bytes()[1],
        bytes.as_bytes()[2],
        bytes.as_bytes()[3],
    ]);
    (word >> 8) as f32 / (1u32 << 24) as f32
}

fn direction(key: &str) -> Vec2 {
    let angle = unit(key, 0) * TAU;
    vec2(angle.cos(), angle.sin())
}

/// How far a box reaches from its centre in direction `d`.
fn reach(half: Vec2, d: Vec2) -> f32 {
    let x = if d.x.abs() > f32::EPSILON {
        half.x / d.x.abs()
    } else {
        f32::INFINITY
    };
    let y = if d.y.abs() > f32::EPSILON {
        half.y / d.y.abs()
    } else {
        f32::INFINITY
    };
    x.min(y) * d.length()
}

#[derive(Default)]
pub struct Layout {
    keys: Vec<String>,
    pos: Vec<Pos2>,
    half: Vec<Vec2>,
    pinned: Vec<bool>,
    /// How freely each node moves, 0 or 1. When a graph grows or a node is dragged, what changed and
    /// the nodes beside it move, and the rest only make room, so a change stays where it happened.
    mobility: Vec<f32>,
    links: Vec<(usize, usize)>,
    temperature: f32,
    steps: usize,
    settled: bool,
}

impl Layout {
    /// Follows a new or grown graph. Known nodes keep their place and pin; a new node starts
    /// beside the placed nodes it is linked to, or where its key says when it has none.
    pub fn sync(&mut self, keys: &[String], half: &[Vec2], links: &[(usize, usize)]) {
        let known: HashMap<String, (Pos2, bool)> = self
            .keys
            .drain(..)
            .zip(self.pos.drain(..).zip(self.pinned.drain(..)))
            .collect();
        let fresh = known.is_empty();
        // A layout still arranging itself keeps arranging everything.
        let arranging = !fresh && !self.settled;
        let n = keys.len();
        let mut pos: Vec<Option<Pos2>> = vec![None; n];
        let mut pinned = vec![false; n];
        for (i, key) in keys.iter().enumerate() {
            if let Some(&(at, pin)) = known.get(key) {
                pos[i] = Some(at);
                pinned[i] = pin;
            }
        }
        let new: Vec<bool> = pos.iter().map(Option::is_none).collect();
        let mut adjacent = vec![Vec::new(); n];
        for &(a, b) in links {
            adjacent[a].push(b);
            adjacent[b].push(a);
        }
        let spread = LINK * (n as f32).sqrt().max(1.0) * 0.5;
        loop {
            let mut placed_any = false;
            for i in 0..n {
                if pos[i].is_some() {
                    continue;
                }
                let near: Vec<Pos2> = adjacent[i].iter().filter_map(|&j| pos[j]).collect();
                if near.is_empty() {
                    continue;
                }
                let centre =
                    near.iter().fold(Vec2::ZERO, |sum, p| sum + p.to_vec2()) / near.len() as f32;
                let away = LINK + half[i].x + half[i].y;
                pos[i] = Some((centre + direction(&keys[i]) * away).to_pos2());
                placed_any = true;
            }
            if placed_any {
                continue;
            }
            // The first node of a part with nothing placed yet starts where its key says.
            match (0..n).find(|&i| pos[i].is_none()) {
                Some(i) => {
                    let radius = spread * unit(&keys[i], 1).sqrt();
                    pos[i] = Some((direction(&keys[i]) * radius).to_pos2());
                }
                None => break,
            }
        }
        self.mobility = if fresh || arranging {
            vec![1.0; n]
        } else {
            settling(&new, &adjacent)
        };
        self.keys = keys.to_vec();
        self.pos = pos.into_iter().flatten().collect();
        self.half = half.to_vec();
        self.pinned = pinned;
        self.links = links.to_vec();
        // A grown graph starts warm enough for its new nodes to find a place; mobility keeps the
        // rest where they were.
        self.temperature = if fresh {
            LINK
        } else {
            self.temperature.max(LINK * 0.5)
        };
        self.steps = 0;
        self.settled = n == 0;
    }

    /// Starts again from where each key says, forgetting every drag.
    pub fn rearrange(&mut self) {
        let (keys, half, links) = (self.keys.clone(), self.half.clone(), self.links.clone());
        self.keys.clear();
        self.pos.clear();
        self.pinned.clear();
        self.sync(&keys, &half, &links);
    }

    /// Node sizes come from their text, which the view measures. A change moves things gently.
    pub fn resize(&mut self, half: &[Vec2]) {
        if half.len() == self.half.len() && half != self.half.as_slice() {
            self.half = half.to_vec();
            self.temperature = self.temperature.max(LINK * 0.1);
            self.steps = 0;
            self.settled = false;
        }
    }

    pub fn settled(&self) -> bool {
        self.settled
    }

    pub fn position(&self, node: usize) -> Pos2 {
        self.pos[node]
    }

    #[cfg(test)]
    pub fn is_pinned(&self, node: usize) -> bool {
        self.pinned[node]
    }

    /// Moves a node by hand. It stays there; its neighbours make room.
    pub fn drag(&mut self, node: usize, delta: Vec2) {
        if self.settled {
            let mut moved = vec![false; self.pos.len()];
            moved[node] = true;
            let mut adjacent = vec![Vec::new(); self.pos.len()];
            for &(a, b) in &self.links {
                adjacent[a].push(b);
                adjacent[b].push(a);
            }
            self.mobility = settling(&moved, &adjacent);
        }
        self.pos[node] = clamp(self.pos[node] + delta);
        self.pinned[node] = true;
        self.temperature = self.temperature.max(LINK * 0.15);
        self.steps = 0;
        self.settled = false;
    }

    /// Every node's box, with room around it.
    pub fn bounds(&self) -> Rect {
        self.pos
            .iter()
            .zip(&self.half)
            .map(|(p, h)| Rect::from_center_size(*p, *h * 2.0))
            .reduce(|a, b| a.union(b))
            .unwrap_or(Rect::from_center_size(Pos2::ZERO, Vec2::splat(LINK)))
    }

    /// Steps until settled or until `budget` is spent; answers whether it settled.
    pub fn settle(&mut self, budget: Duration) -> bool {
        let start = Instant::now();
        while !self.settled && start.elapsed() < budget {
            self.step();
        }
        self.settled
    }

    /// Steps until settled, however long that takes: for tests.
    #[cfg(test)]
    pub fn settle_fully(&mut self) {
        while !self.settled {
            self.step();
        }
    }

    fn step(&mut self) {
        let n = self.pos.len();
        let mut push = vec![Vec2::ZERO; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let mut d = self.pos[i] - self.pos[j];
                if d.length_sq() < 0.01 {
                    // Two nodes on one spot part in a direction their keys decide.
                    d = direction(&self.keys[i]) - direction(&self.keys[j]) + vec2(0.01, 0.0);
                }
                let gap = self.gap(i, j, d);
                if gap > REACH {
                    continue;
                }
                let force = d / d.length() * (LINK * LINK / gap);
                push[i] += force;
                push[j] -= force;
            }
        }
        for &(a, b) in &self.links {
            let d = self.pos[a] - self.pos[b];
            if d.length_sq() < 0.01 {
                continue;
            }
            let gap = self.gap(a, b, d);
            let force = d / d.length() * (gap * gap / LINK);
            push[a] -= force;
            push[b] += force;
        }
        for (i, p) in push.iter_mut().enumerate() {
            *p -= self.pos[i].to_vec2() * GRAVITY;
        }
        for (i, push) in push.iter().enumerate() {
            let length = push.length();
            if self.pinned[i] || !length.is_finite() || length == 0.0 {
                continue;
            }
            let most = self.temperature * self.mobility[i];
            self.pos[i] = clamp(self.pos[i] + *push / length * length.min(most));
        }
        self.separate();
        self.temperature *= COOLING;
        self.steps += 1;
        if self.temperature < SETTLED || self.steps >= MAX_STEPS {
            for _ in 0..60 {
                if !self.separate() {
                    break;
                }
            }
            self.settled = true;
        }
    }

    /// The distance between the edges of two boxes along the line between their centres. Forces
    /// act on it rather than on the centres, so a long name gets as much line as a short one.
    fn gap(&self, i: usize, j: usize, d: Vec2) -> f32 {
        (d.length() - reach(self.half[i], d) - reach(self.half[j], d)).max(1.0)
    }

    /// Moves overlapping boxes apart along the shorter overlap. A pinned box does not move.
    /// Answers whether any two overlapped.
    fn separate(&mut self) -> bool {
        let n = self.pos.len();
        let mut overlapped = false;
        for i in 0..n {
            for j in (i + 1)..n {
                let d = self.pos[j] - self.pos[i];
                let x = self.half[i].x + self.half[j].x + GAP - d.x.abs();
                let y = self.half[i].y + self.half[j].y + GAP - d.y.abs();
                if x <= 0.0 || y <= 0.0 {
                    continue;
                }
                overlapped = true;
                // On one spot, the later node gives way to the right or below.
                let sign = |v: f32| if v < 0.0 { -1.0 } else { 1.0 };
                let apart = if x < y {
                    vec2(x * sign(d.x), 0.0)
                } else {
                    vec2(0.0, y * sign(d.y))
                };
                match (self.pinned[i], self.pinned[j]) {
                    (true, true) => {}
                    (true, false) => self.pos[j] = clamp(self.pos[j] + apart),
                    (false, true) => self.pos[i] = clamp(self.pos[i] - apart),
                    (false, false) => {
                        self.pos[i] = clamp(self.pos[i] - apart * 0.5);
                        self.pos[j] = clamp(self.pos[j] + apart * 0.5);
                    }
                }
            }
        }
        overlapped
    }
}

/// Mobility after a change: what changed and its neighbours move, the rest only make room.
fn settling(changed: &[bool], adjacent: &[Vec<usize>]) -> Vec<f32> {
    (0..changed.len())
        .map(|i| {
            let near = changed[i] || adjacent[i].iter().any(|&j| changed[j]);
            if near {
                1.0
            } else {
                0.0
            }
        })
        .collect()
}

fn clamp(p: Pos2) -> Pos2 {
    Pos2::new(p.x.clamp(-BOUND, BOUND), p.y.clamp(-BOUND, BOUND))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring of 12, a hub with 8 spokes, and two loose nodes.
    fn graph() -> (Vec<String>, Vec<Vec2>, Vec<(usize, usize)>) {
        let keys: Vec<String> = (0..22).map(|i| format!("node:{i}")).collect();
        let half = (0..22)
            .map(|i| vec2(30.0 + (i % 4) as f32 * 12.0, 14.0))
            .collect();
        let mut links: Vec<(usize, usize)> = (0..12).map(|i| (i, (i + 1) % 12)).collect();
        links.extend((13..20).map(|i| (12, i)));
        (keys, half, links)
    }
    fn settled() -> Layout {
        let (keys, half, links) = graph();
        let mut layout = Layout::default();
        layout.sync(&keys, &half, &links);
        layout.settle_fully();
        layout
    }
    fn overlaps(layout: &Layout) -> usize {
        let n = layout.pos.len();
        let boxes: Vec<Rect> = (0..n)
            .map(|i| Rect::from_center_size(layout.pos[i], layout.half[i] * 2.0))
            .collect();
        (0..n)
            .flat_map(|i| ((i + 1)..n).map(move |j| (i, j)))
            .filter(|&(i, j)| boxes[i].intersects(boxes[j]))
            .count()
    }

    #[test]
    fn the_same_graph_gets_the_same_picture() {
        assert_eq!(settled().pos, settled().pos);
    }

    #[test]
    fn a_settled_graph_has_no_overlapping_nodes_and_stays_in_bounds() {
        let layout = settled();
        assert_eq!(overlaps(&layout), 0);
        assert!(layout
            .pos
            .iter()
            .all(|p| p.x.abs() < 5_000.0 && p.y.abs() < 5_000.0));
        let (_, _, links) = graph();
        for (a, b) in links {
            let shown = layout.gap(a, b, layout.pos[a] - layout.pos[b]);
            assert!(
                (GAP..4.0 * LINK).contains(&shown),
                "link {a}-{b} shows {shown}"
            );
        }
    }

    #[test]
    fn a_dragged_node_stays_where_it_was_put() {
        let mut layout = settled();
        let target = layout.position(3) + vec2(240.0, -90.0);
        layout.drag(3, vec2(240.0, -90.0));
        layout.settle_fully();
        assert!(layout.is_pinned(3));
        assert_eq!(layout.position(3), target);
        assert_eq!(overlaps(&layout), 0);
        layout.rearrange();
        layout.settle_fully();
        assert!(!layout.is_pinned(3));
        assert_eq!(layout.pos, settled().pos, "rearranging forgets drags");
    }

    #[test]
    fn a_grown_graph_keeps_what_was_there_where_it_was() {
        let mut layout = settled();
        let before = layout.pos.clone();
        let (mut keys, mut half, mut links) = graph();
        keys.push("node:new".into());
        half.push(vec2(40.0, 14.0));
        links.push((5, 22));
        layout.sync(&keys, &half, &links);
        let start = layout.position(22);
        assert!(
            (start - layout.position(5)).length() <= 3.0 * LINK,
            "starts beside its neighbour"
        );
        layout.settle_fully();
        let mut moved: Vec<f32> = before
            .iter()
            .zip(&layout.pos)
            .map(|(a, b)| (*a - *b).length())
            .collect();
        moved.sort_by(f32::total_cmp);
        let (typical, most) = (moved[moved.len() / 2], moved[moved.len() - 1]);
        eprintln!("old nodes moved: typically {typical}, at most {most}");
        assert!(typical < LINK / 4.0 && most < LINK, "{moved:?}");
        assert_eq!(overlaps(&layout), 0);
    }

    #[test]
    fn a_graph_at_the_node_bound_settles_without_overlaps() {
        // 60 relations of four parties each, sharing some: 300 nodes, as many as are drawn.
        let keys: Vec<String> = (0..300).map(|i| format!("node:{i}")).collect();
        let half = vec![vec2(46.0, 16.0); 300];
        let links: Vec<(usize, usize)> = (0..60)
            .flat_map(|r| (1..5).map(move |p| (r, 60 + (r * 4 + p * 7) % 240)))
            .collect();
        let mut layout = Layout::default();
        layout.sync(&keys, &half, &links);
        let start = Instant::now();
        layout.settle_fully();
        eprintln!(
            "300 nodes settled in {} steps, {:?}",
            layout.steps,
            start.elapsed()
        );
        assert!(layout.steps < MAX_STEPS);
        assert_eq!(overlaps(&layout), 0);
    }

    #[test]
    fn an_empty_graph_is_settled_and_has_bounds() {
        let mut layout = Layout::default();
        layout.sync(&[], &[], &[]);
        assert!(layout.settle(Duration::ZERO));
        assert!(layout.bounds().is_positive());
    }
}
