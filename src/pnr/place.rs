//! Row-based placement: cells in rows, initial order from the build order
//! (which keeps the gates of one part together), improved by simulated
//! annealing that swaps cells of equal width (so rows stay legal) and
//! minimises the half-perimeter wire length (HPWL).

/// Small deterministic RNG (xorshift), so results are reproducible.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Placement problem: cell widths (grid units) and, per net, the cells it
/// touches with the x offset of the pin inside the cell. Fixed pins (top-level
/// IO) are given by absolute coordinates.
pub struct Problem {
    pub widths: Vec<i32>,
    /// Per net: (cell, pin x offset) and fixed (x, y) points.
    pub nets: Vec<(Vec<(usize, i32)>, Vec<(i32, i32)>)>,
    pub row_height: i32,
}

pub struct Placement {
    /// Lower-left corner of each cell in grid units.
    pub pos: Vec<(i32, i32)>,
    pub rows: i32,
    pub row_length: i32,
    pub hpwl: i64,
}

/// Rows and row length for a roughly square core at `utilization`.
pub fn floorplan(widths: &[i32], row_height: i32, utilization: f64) -> (i32, i32) {
    let total: i64 = widths.iter().map(|&w| w as i64).sum();
    let area = total as f64 * row_height as f64 / utilization;
    let side = area.sqrt();
    let rows = ((side / row_height as f64).round() as i32).max(1);
    let max_w = *widths.iter().max().unwrap_or(&1);
    let row_length = ((total as f64 / utilization / rows as f64).ceil() as i32).max(max_w + 2);
    (rows, row_length)
}

/// Fills rows left to right in the given order, spreading the slack of each
/// row evenly between its cells.
fn pack(order: &[usize], widths: &[i32], rows: i32, row_length: i32, row_height: i32) -> Vec<(i32, i32)> {
    let mut pos = vec![(0, 0); widths.len()];
    let total: i64 = widths.iter().map(|&w| w as i64).sum();
    let per_row = (total as f64 / rows as f64).ceil() as i64;
    let mut row_cells: Vec<Vec<usize>> = vec![Vec::new(); rows as usize];
    let (mut r, mut used) = (0usize, 0i64);
    for &c in order {
        if used + widths[c] as i64 > per_row && r + 1 < rows as usize {
            r += 1;
            used = 0;
        }
        row_cells[r].push(c);
        used += widths[c] as i64;
    }
    for (r, cells) in row_cells.iter().enumerate() {
        let w: i32 = cells.iter().map(|&c| widths[c]).sum();
        let gap = ((row_length - w).max(0) as f64) / (cells.len() + 1) as f64;
        let mut x = gap;
        // Alternate direction per row (snake) so consecutive cells stay close.
        let ordered: Vec<usize> = if r % 2 == 0 { cells.clone() } else { cells.iter().rev().copied().collect() };
        for c in ordered {
            pos[c] = (x.round() as i32, r as i32 * row_height);
            x += widths[c] as f64 + gap;
        }
    }
    pos
}

fn net_hpwl(net: &(Vec<(usize, i32)>, Vec<(i32, i32)>), pos: &[(i32, i32)], row_height: i32) -> i64 {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(c, dx) in &net.0 {
        let (x, y) = (pos[c].0 + dx, pos[c].1 + row_height / 2);
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    for &(x, y) in &net.1 {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    if x0 > x1 {
        return 0;
    }
    (x1 - x0) as i64 + (y1 - y0) as i64
}

/// Places the cells. `effort` scales the number of annealing moves per cell.
pub fn place(p: &Problem, rows: i32, row_length: i32, effort: f64, seed: u64) -> Placement {
    let n = p.widths.len();
    let order: Vec<usize> = (0..n).collect();
    let mut pos = pack(&order, &p.widths, rows, row_length, p.row_height);

    // Nets per cell for incremental cost.
    let mut cell_nets: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, net) in p.nets.iter().enumerate() {
        for &(c, _) in &net.0 {
            if cell_nets[c].last() != Some(&i) {
                cell_nets[c].push(i);
            }
        }
    }
    let mut net_cost: Vec<i64> = p.nets.iter().map(|net| net_hpwl(net, &pos, p.row_height)).collect();
    let mut total: i64 = net_cost.iter().sum();

    // Swap partners: cells of equal width share a fixed set of slots. Each
    // group keeps its slots in row-major order; a move swaps the cells of two
    // slots that are close in that order (same row, or a row or two away),
    // with the window shrinking as the temperature drops.
    let mut by_width: std::collections::HashMap<i32, Vec<usize>> = std::collections::HashMap::new();
    for (c, &w) in p.widths.iter().enumerate() {
        by_width.entry(w).or_default().push(c);
    }
    let mut groups: Vec<Vec<usize>> = by_width.into_values().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| g[0]);
    for g in &mut groups {
        g.sort_by_key(|&c| (pos[c].1, pos[c].0));
    }
    if groups.is_empty() || n < 2 || effort <= 0.0 {
        return Placement { pos, rows, row_length, hpwl: total };
    }
    let start_pos = pos.clone();
    let start_total = total;
    let weights: Vec<usize> = groups.iter().map(|g| g.len()).collect();
    let weight_sum: usize = weights.iter().sum();

    let mut rng = Rng::new(seed);
    let moves = ((n as f64) * 300.0 * effort) as usize;
    let steps = 80;
    let per_step = (moves / steps).max(1);
    // Start warm enough to escape small local minima, not to scramble the
    // build order (which is already a good placement).
    let mut temp = p.row_height as f64 * 1.5;
    let mut touched: Vec<usize> = Vec::new();
    let mut mark = vec![usize::MAX; p.nets.len()];
    let mut stamp = 0usize;
    for step in 0..steps {
        let frac = 1.0 - step as f64 / steps as f64;
        for _ in 0..per_step {
            let mut k = rng.below(weight_sum);
            let mut gi = 0;
            while k >= weights[gi] {
                k -= weights[gi];
                gi += 1;
            }
            let g = &mut groups[gi];
            let len = g.len();
            // Window in slot order: up to ~3 rows' worth of slots, shrinking.
            let per_row = (len as f64 / rows as f64).max(1.0);
            let window = ((per_row * 3.0 * frac).ceil() as usize).clamp(1, len - 1);
            let i = rng.below(len);
            let off = 1 + rng.below(window);
            let j = if rng.below(2) == 0 { (i + off) % len } else { (i + len - off % len) % len };
            if i == j {
                continue;
            }
            let (a, b) = (g[i], g[j]);
            stamp += 1;
            touched.clear();
            for &c in [a, b].iter() {
                for &ni in &cell_nets[c] {
                    if mark[ni] != stamp {
                        mark[ni] = stamp;
                        touched.push(ni);
                    }
                }
            }
            pos.swap(a, b);
            let new_costs: Vec<i64> = touched.iter().map(|&ni| net_hpwl(&p.nets[ni], &pos, p.row_height)).collect();
            let delta: i64 = touched.iter().zip(&new_costs).map(|(&ni, &c)| c - net_cost[ni]).sum();
            if delta <= 0 || rng.unit() < (-(delta as f64) / temp.max(1e-9)).exp() {
                for (k, &ni) in touched.iter().enumerate() {
                    net_cost[ni] = new_costs[k];
                }
                total += delta;
                g.swap(i, j); // slots keep their order; the cells moved
            } else {
                pos.swap(a, b);
            }
        }
        temp *= 0.9;
    }
    if total > start_total {
        return Placement { pos: start_pos, rows, row_length, hpwl: start_total };
    }
    Placement { pos, rows, row_length, hpwl: total }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annealing_shortens_a_chain() {
        // 40 cells in a chain, initially placed in scrambled order.
        let n = 40;
        let mut rng = Rng::new(7);
        let mut perm: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            perm.swap(i, rng.below(i + 1));
        }
        let nets = (0..n - 1).map(|i| (vec![(perm[i], 1), (perm[i + 1], 1)], vec![])).collect();
        let p = Problem { widths: vec![3; n], nets, row_height: 6 };
        let (rows, len) = floorplan(&p.widths, 6, 0.6);
        let start = place(&p, rows, len, 0.0, 1).hpwl;
        let end = place(&p, rows, len, 1.0, 1).hpwl;
        assert!(end < start, "{end} >= {start}");
    }

    #[test]
    fn packing_keeps_cells_apart() {
        let widths = vec![3, 5, 2, 4, 3, 3];
        let pos = pack(&(0..6).collect::<Vec<_>>(), &widths, 2, 14, 6);
        for i in 0..6 {
            for j in i + 1..6 {
                if pos[i].1 == pos[j].1 {
                    let (a0, a1) = (pos[i].0, pos[i].0 + widths[i]);
                    let (b0, b1) = (pos[j].0, pos[j].0 + widths[j]);
                    assert!(a1 <= b0 || b1 <= a0, "cells {i} and {j} overlap");
                }
            }
        }
    }
}
