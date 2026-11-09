//! Maze router on a 3D grid with negotiated congestion (PathFinder).
//!
//! Layers alternate their direction (layer 0 horizontal, 1 vertical, ...);
//! a wire only runs in its layer's direction, vias connect neighbouring
//! layers. Every grid node holds at most one net in a legal result. Nets are
//! routed as trees: A* from each further pin to the nearest node already in
//! the tree. Overused nodes get more expensive each iteration (present and
//! history cost) until nets negotiate a solution without overlaps.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub struct Grid {
    pub w: i32,
    pub h: i32,
    pub layers: i32,
    /// Nodes no net may use (cell interiors on layer 0).
    pub blocked: Vec<bool>,
}

impl Grid {
    pub fn new(w: i32, h: i32, layers: i32) -> Self {
        Grid { w, h, layers, blocked: vec![false; (w * h * layers) as usize] }
    }
    #[inline]
    pub fn id(&self, x: i32, y: i32, l: i32) -> u32 {
        ((l * self.h + y) * self.w + x) as u32
    }
    #[inline]
    pub fn xyl(&self, n: u32) -> (i32, i32, i32) {
        let n = n as i32;
        (n % self.w, (n / self.w) % self.h, n / (self.w * self.h))
    }
    pub fn len(&self) -> usize {
        (self.w * self.h * self.layers) as usize
    }
}

pub struct RouteResult {
    /// Per net: grid edges (a, b) of its routing tree.
    pub edges: Vec<Vec<(u32, u32)>>,
    pub iterations: usize,
    /// Nodes still shared by several nets (0 = legal).
    pub overflow: usize,
    /// Nets for which no path was found at all.
    pub failed: Vec<usize>,
}

const VIA_COST: f32 = 3.0;

struct Search {
    cost: Vec<f32>,
    parent: Vec<u32>,
    stamp: Vec<u32>,
    generation: u32,
    in_tree: Vec<u32>,
    tree_gen: u32,
}

/// Routes `nets` (each a list of pin nodes) on `grid`. Pin nodes are reserved
/// for their net.
pub fn route(grid: &Grid, nets: &[Vec<u32>], max_iterations: usize) -> RouteResult {
    let n = grid.len();
    let mut owner: Vec<i32> = vec![-1; n];
    for (i, pins) in nets.iter().enumerate() {
        for &p in pins {
            owner[p as usize] = i as i32;
        }
    }
    let mut occ: Vec<u16> = vec![0; n];
    let mut hist: Vec<f32> = vec![0.0; n];
    let mut nodes_of: Vec<Vec<u32>> = vec![Vec::new(); nets.len()];
    let mut edges: Vec<Vec<(u32, u32)>> = vec![Vec::new(); nets.len()];
    let mut search = Search {
        cost: vec![f32::INFINITY; n],
        parent: vec![u32::MAX; n],
        stamp: vec![0; n],
        generation: 0,
        in_tree: vec![0; n],
        tree_gen: 0,
    };

    // Small nets first: they have the fewest alternatives.
    let mut order: Vec<usize> = (0..nets.len()).filter(|&i| nets[i].len() >= 2).collect();
    let bbox = |pins: &[u32]| {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &p in pins {
            let (x, y, _) = grid.xyl(p);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        (x0, y0, x1, y1)
    };
    order.sort_by_key(|&i| {
        let (x0, y0, x1, y1) = bbox(&nets[i]);
        (x1 - x0 + y1 - y0) as i64 * nets[i].len() as i64
    });

    let mut pres_fac = 0.6f32;
    let mut failed = Vec::new();
    let mut iterations = 0;
    let mut overflow = 0;
    // Give up early when the overflow stops improving (more area is needed).
    let (mut best, mut best_iter) = (usize::MAX, 0);
    for iter in 0..max_iterations {
        iterations = iter + 1;
        failed.clear();
        for &net in &order {
            // After the first pass only nets touching an overused node are rerouted.
            if iter > 0 && !nodes_of[net].iter().any(|&v| occ[v as usize] > 1) {
                continue;
            }
            for &v in &nodes_of[net] {
                occ[v as usize] -= 1;
            }
            nodes_of[net].clear();
            edges[net].clear();
            // Allow wider detours as negotiation goes on.
            let margin = 8 + 3 * iter as i32;
            let ok = route_net(grid, net, &nets[net], &owner, &occ, &hist, pres_fac, margin, &mut search, &mut nodes_of[net], &mut edges[net]);
            if !ok {
                failed.push(net);
            }
            for &v in &nodes_of[net] {
                occ[v as usize] += 1;
            }
        }
        overflow = occ.iter().filter(|&&o| o > 1).count();
        if overflow == 0 {
            break;
        }
        if overflow < best {
            (best, best_iter) = (overflow, iter);
        } else if iter - best_iter >= 6 {
            break;
        }
        for (v, &o) in occ.iter().enumerate() {
            if o > 1 {
                hist[v] += (o - 1) as f32 * 0.6;
            }
        }
        pres_fac *= 1.6;
    }
    RouteResult { edges, iterations, overflow, failed }
}

#[allow(clippy::too_many_arguments)]
fn route_net(
    grid: &Grid,
    net: usize,
    pins: &[u32],
    owner: &[i32],
    occ: &[u16],
    hist: &[f32],
    pres_fac: f32,
    margin: i32,
    s: &mut Search,
    nodes: &mut Vec<u32>,
    edges: &mut Vec<(u32, u32)>,
) -> bool {
    s.tree_gen += 1;
    let tg = s.tree_gen;
    // Start the tree at the first pin; connect the others nearest first.
    let first = pins[0];
    s.in_tree[first as usize] = tg;
    nodes.push(first);
    let (fx, fy, _) = grid.xyl(first);
    let mut rest: Vec<u32> = pins[1..].to_vec();
    rest.sort_by_key(|&p| {
        let (x, y, _) = grid.xyl(p);
        (x - fx).abs() + (y - fy).abs()
    });
    let (mut tx0, mut ty0, mut tx1, mut ty1) = (fx, fy, fx, fy);
    let mut all_ok = true;
    for &pin in &rest {
        if s.in_tree[pin as usize] == tg {
            continue;
        }
        // Search window: tree box and pin, plus a margin that grows on failure.
        let (px, py, _) = grid.xyl(pin);
        let mut found = None;
        for margin in [margin, margin * 3, i32::MAX / 4] {
            let win = (
                tx0.min(px).saturating_sub(margin).max(0),
                ty0.min(py).saturating_sub(margin).max(0),
                tx1.max(px).saturating_add(margin).min(grid.w - 1),
                ty1.max(py).saturating_add(margin).min(grid.h - 1),
            );
            found = astar(grid, net, pin, (tx0, ty0, tx1, ty1), win, owner, occ, hist, pres_fac, s);
            if found.is_some() {
                break;
            }
        }
        let Some(hit) = found else {
            all_ok = false;
            continue;
        };
        // Walk back from the tree node to the pin, adding the path.
        let mut v = hit;
        while v != pin {
            let p = s.parent[v as usize];
            edges.push((p, v));
            if s.in_tree[p as usize] != tg {
                s.in_tree[p as usize] = tg;
                nodes.push(p);
                let (x, y, _) = grid.xyl(p);
                tx0 = tx0.min(x);
                ty0 = ty0.min(y);
                tx1 = tx1.max(x);
                ty1 = ty1.max(y);
            }
            v = p;
        }
    }
    all_ok
}

#[allow(clippy::too_many_arguments)]
fn astar(
    grid: &Grid,
    net: usize,
    source: u32,
    target_box: (i32, i32, i32, i32),
    win: (i32, i32, i32, i32),
    owner: &[i32],
    occ: &[u16],
    hist: &[f32],
    pres_fac: f32,
    s: &mut Search,
) -> Option<u32> {
    s.generation += 1;
    let generation = s.generation;
    let tg = s.tree_gen;
    let h = |x: i32, y: i32| -> f32 {
        let dx = if x < target_box.0 { target_box.0 - x } else if x > target_box.2 { x - target_box.2 } else { 0 };
        let dy = if y < target_box.1 { target_box.1 - y } else if y > target_box.3 { y - target_box.3 } else { 0 };
        (dx + dy) as f32
    };
    let node_cost = |v: u32| -> f32 {
        let o = occ[v as usize] as f32;
        (1.0 + hist[v as usize]) * (1.0 + o * pres_fac)
    };
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    s.cost[source as usize] = 0.0;
    s.stamp[source as usize] = generation;
    s.parent[source as usize] = source;
    let (sx, sy, _) = grid.xyl(source);
    heap.push(Reverse((h(sx, sy).to_bits(), source)));
    while let Some(Reverse((_, v))) = heap.pop() {
        if s.in_tree[v as usize] == tg {
            return Some(v);
        }
        let (x, y, l) = grid.xyl(v);
        let base = s.cost[v as usize];
        let relax = |u: u32, step: f32, s: &mut Search, heap: &mut BinaryHeap<Reverse<(u32, u32)>>| {
            let ui = u as usize;
            if grid.blocked[ui] || (owner[ui] >= 0 && owner[ui] != net as i32) {
                return;
            }
            let c = base + step * node_cost(u);
            if s.stamp[ui] != generation || c < s.cost[ui] {
                s.stamp[ui] = generation;
                s.cost[ui] = c;
                s.parent[ui] = v;
                let (ux, uy, _) = grid.xyl(u);
                heap.push(Reverse(((c + h(ux, uy)).to_bits(), u)));
            }
        };
        if l % 2 == 0 {
            if x > win.0 {
                relax(grid.id(x - 1, y, l), 1.0, s, &mut heap);
            }
            if x < win.2 {
                relax(grid.id(x + 1, y, l), 1.0, s, &mut heap);
            }
        } else {
            if y > win.1 {
                relax(grid.id(x, y - 1, l), 1.0, s, &mut heap);
            }
            if y < win.3 {
                relax(grid.id(x, y + 1, l), 1.0, s, &mut heap);
            }
        }
        if l > 0 {
            relax(grid.id(x, y, l - 1), VIA_COST, s, &mut heap);
        }
        if l + 1 < grid.layers {
            relax(grid.id(x, y, l + 1), VIA_COST, s, &mut heap);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_crossing_nets_without_overlap() {
        // Two nets that must cross: one left-right, one bottom-top.
        let g = Grid::new(10, 10, 2);
        let nets = vec![vec![g.id(0, 5, 0), g.id(9, 5, 0)], vec![g.id(5, 0, 1), g.id(5, 9, 1)]];
        let r = route(&g, &nets, 10);
        assert_eq!(r.overflow, 0);
        assert!(r.failed.is_empty());
        assert_eq!(r.edges[0].len(), 9);
    }

    #[test]
    fn negotiates_a_shared_corridor() {
        // Three nets want the same straight row; congestion pushes two away.
        let g = Grid::new(12, 5, 2);
        let nets: Vec<Vec<u32>> = (0..3).map(|i| vec![g.id(0, 1 + i, 0), g.id(11, 1 + i, 0)]).collect();
        let r = route(&g, &nets, 30);
        assert_eq!(r.overflow, 0);
    }

    #[test]
    fn multi_pin_nets_form_trees() {
        let g = Grid::new(20, 20, 2);
        let pins = vec![g.id(1, 1, 0), g.id(18, 2, 0), g.id(10, 18, 0), g.id(3, 15, 0)];
        let r = route(&g, &[pins.clone()], 5);
        // A tree over 4 pins: every pin is an endpoint of some edge.
        for p in pins {
            assert!(r.edges[0].iter().any(|&(a, b)| a == p || b == p));
        }
    }
}
