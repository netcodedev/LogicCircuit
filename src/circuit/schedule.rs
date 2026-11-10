//! Evaluation order and change tracking for `Circuit::evaluate_stable`.
//!
//! The gates are grouped into units: the strongly connected components of
//! the gate graph (an edge runs from a gate to every gate reading its
//! output; flip-flops cut it). Almost every unit is a single gate; a unit of
//! several gates (or one gate reading its own output) is a feedback loop,
//! e.g. an SR latch, and is iterated until it settles. The units are sorted
//! topologically, so one pass in that order settles all gates. A wire change
//! marks the units reading it (a bit set over the units); a pass evaluates
//! only marked units. Flip-flops whose clock changed are checked after each
//! pass: all of them sample D first, then their outputs change, and the next
//! pass propagates that.

use std::cell::{Cell, RefCell};

use super::{Circuit, WireId};
use super::circuit::MAX_ITERATIONS;

#[derive(Debug)]
pub(crate) struct Schedule {
    /// Unit u consists of the gates `order[unit_start[u]..unit_start[u + 1]]`.
    unit_start: Vec<u32>,
    order: Vec<u32>,
    /// Units that are feedback loops.
    cyclic: Vec<bool>,
    /// Units reading wire w: `fan_units[fan_start[w]..fan_start[w + 1]]`.
    fan_start: Vec<u32>,
    fan_units: Vec<u32>,
    /// Flip-flops clocked by wire w: `clk_flops[clk_start[w]..clk_start[w + 1]]`.
    clk_start: Vec<u32>,
    clk_flops: Vec<u32>,
    /// Marked units, one bit each.
    dirty: Vec<Cell<u64>>,
    /// Flip-flops whose clock changed since their last check.
    pending: RefCell<Vec<u32>>,
    is_pending: Vec<Cell<bool>>,
}

/// Compressed adjacency: `items[start[k]..start[k + 1]]` for each key k.
fn csr(keys: usize, pairs: &[(u32, u32)]) -> (Vec<u32>, Vec<u32>) {
    let mut start = vec![0u32; keys + 1];
    for &(k, _) in pairs {
        start[k as usize + 1] += 1;
    }
    for k in 0..keys {
        start[k + 1] += start[k];
    }
    let mut fill = start.clone();
    let mut items = vec![0u32; pairs.len()];
    for &(k, v) in pairs {
        items[fill[k as usize] as usize] = v;
        fill[k as usize] += 1;
    }
    (start, items)
}

impl Schedule {
    pub(crate) fn new(c: &Circuit) -> Schedule {
        let gates = c.sim_gates();
        let wires = c.wire_count();

        // Gates reading each wire.
        let mut reads: Vec<(u32, u32)> = Vec::new();
        for (i, g) in gates.iter().enumerate() {
            let mut ins = g.inputs[..g.n as usize].to_vec();
            ins.sort_unstable();
            ins.dedup();
            reads.extend(ins.into_iter().map(|w| (w, i as u32)));
        }
        let (read_start, readers) = csr(wires, &reads);
        let succ = |g: usize| {
            let w = gates[g].out as usize;
            &readers[read_start[w] as usize..read_start[w + 1] as usize]
        };

        // Strongly connected components (Tarjan, iterative). They come out
        // in reverse topological order.
        let n = gates.len();
        const NONE: u32 = u32::MAX;
        let mut index = vec![NONE; n];
        let mut low = vec![0u32; n];
        let mut on_stack = vec![false; n];
        let mut stack: Vec<u32> = Vec::new();
        let mut calls: Vec<(u32, u32)> = Vec::new();
        let mut next = 0u32;
        let mut sccs: Vec<Vec<u32>> = Vec::new();
        for root in 0..n {
            if index[root] != NONE {
                continue;
            }
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root as u32);
            on_stack[root] = true;
            calls.push((root as u32, 0));
            while let Some(&mut (v, ref mut edge)) = calls.last_mut() {
                let s = succ(v as usize);
                if (*edge as usize) < s.len() {
                    let w = s[*edge as usize] as usize;
                    *edge += 1;
                    if index[w] == NONE {
                        index[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w as u32);
                        on_stack[w] = true;
                        calls.push((w as u32, 0));
                    } else if on_stack[w] {
                        low[v as usize] = low[v as usize].min(index[w]);
                    }
                } else {
                    calls.pop();
                    let v = v as usize;
                    if low[v] == index[v] {
                        let mut scc = Vec::new();
                        loop {
                            let w = stack.pop().unwrap();
                            on_stack[w as usize] = false;
                            scc.push(w);
                            if w as usize == v {
                                break;
                            }
                        }
                        scc.sort_unstable(); // build order within a loop
                        sccs.push(scc);
                    }
                    if let Some(&(p, _)) = calls.last() {
                        low[p as usize] = low[p as usize].min(low[v]);
                    }
                }
            }
        }
        sccs.reverse();

        let mut unit_start = Vec::with_capacity(sccs.len() + 1);
        let mut order = Vec::with_capacity(n);
        let mut cyclic = Vec::with_capacity(sccs.len());
        let mut unit_of = vec![0u32; n];
        for (u, scc) in sccs.iter().enumerate() {
            unit_start.push(order.len() as u32);
            let self_loop = scc.len() == 1 && succ(scc[0] as usize).contains(&scc[0]);
            cyclic.push(scc.len() > 1 || self_loop);
            for &g in scc {
                unit_of[g as usize] = u as u32;
                order.push(g);
            }
        }
        unit_start.push(order.len() as u32);

        // Units reading each wire.
        let mut unit_reads: Vec<(u32, u32)> = reads.iter().map(|&(w, g)| (w, unit_of[g as usize])).collect();
        unit_reads.sort_unstable();
        unit_reads.dedup();
        let (fan_start, fan_units) = csr(wires, &unit_reads);

        let clocks: Vec<(u32, u32)> = c.sim_flops().iter().enumerate().map(|(i, f)| (f.clk, i as u32)).collect();
        let (clk_start, clk_flops) = csr(wires, &clocks);

        let units = sccs.len();
        let flops = c.sim_flops().len();
        // Everything is marked for the first evaluation.
        let dirty = (0..units.div_ceil(64))
            .map(|w| {
                let bits = (units - w * 64).min(64);
                Cell::new(if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 })
            })
            .collect();
        Schedule {
            unit_start,
            order,
            cyclic,
            fan_start,
            fan_units,
            clk_start,
            clk_flops,
            dirty,
            pending: RefCell::new((0..flops as u32).collect()),
            is_pending: (0..flops).map(|_| Cell::new(true)).collect(),
        }
    }

    /// Marks the units and flip-flops that read wire `w`.
    #[inline]
    pub(crate) fn mark(&self, w: WireId) {
        let w = w as usize;
        for &u in &self.fan_units[self.fan_start[w] as usize..self.fan_start[w + 1] as usize] {
            let cell = &self.dirty[(u / 64) as usize];
            cell.set(cell.get() | 1 << (u % 64));
        }
        let flops = &self.clk_flops[self.clk_start[w] as usize..self.clk_start[w + 1] as usize];
        if !flops.is_empty() {
            let mut pending = self.pending.borrow_mut();
            for &f in flops {
                if !self.is_pending[f as usize].replace(true) {
                    pending.push(f);
                }
            }
        }
    }

    /// Evaluates the marked units in topological order. A unit only marks
    /// units after it (or itself, for a loop), so one pass settles all.
    fn pass(&self, c: &Circuit) {
        for word in 0..self.dirty.len() {
            loop {
                let bits = self.dirty[word].get();
                if bits == 0 {
                    break;
                }
                let bit = bits.trailing_zeros();
                let u = word * 64 + bit as usize;
                let gates = &self.order[self.unit_start[u] as usize..self.unit_start[u + 1] as usize];
                self.dirty[word].set(bits & !(1 << bit));
                if self.cyclic[u] {
                    self.settle_loop(c, gates);
                    // Its own marks are settled already.
                    let cell = &self.dirty[word];
                    cell.set(cell.get() & !(1 << bit));
                } else {
                    let g = c.sim_gate(gates[0]);
                    c.write(g.out, c.eval_gate(g));
                }
            }
        }
    }

    /// Iterates the gates of a feedback loop until none changes.
    fn settle_loop(&self, c: &Circuit, gates: &[u32]) {
        for _ in 0..MAX_ITERATIONS {
            let mut changed = false;
            for &g in gates {
                let g = c.sim_gate(g);
                changed |= c.write(g.out, c.eval_gate(g));
            }
            if !changed {
                return;
            }
        }
        eprintln!(
            "Warning: Circuit {} did not stabilize after {} iterations. Possible oscillation detected.",
            c.get_name(),
            MAX_ITERATIONS
        );
    }

    /// Evaluates until nothing changes (see the module comment).
    pub(crate) fn run(&self, c: &Circuit) {
        let mut updates: Vec<(u32, bool)> = Vec::new();
        for _ in 0..MAX_ITERATIONS {
            self.pass(c);
            let pending = self.pending.take();
            if pending.is_empty() {
                return;
            }
            // All flip-flops clocked now sample D before any output changes.
            updates.clear();
            for &f in &pending {
                self.is_pending[f as usize].set(false);
                if let Some(q) = c.clock_flop(f) {
                    updates.push((f, q));
                }
            }
            for &(f, q) in &updates {
                c.set_flop(f, q);
            }
            let mut pending = pending;
            pending.clear();
            // Reuse the allocation unless new clock changes came in.
            let mut current = self.pending.borrow_mut();
            if current.is_empty() {
                *current = pending;
            }
        }
        eprintln!(
            "Warning: Circuit {} did not stabilize after {} iterations. Possible oscillation detected.",
            c.get_name(),
            MAX_ITERATIONS
        );
    }
}
