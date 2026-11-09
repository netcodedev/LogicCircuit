//! Our own placement and routing (roadmap phase 6), independent of OpenROAD
//! and the PDK: runs in the CLI and, compiled to WASM, in the browser.
//!
//! It is a teaching-grade flow, not a sign-off tool: cells are simple boxes
//! with one grid column per pin, layers alternate direction, and the result
//! is legal on its grid (no two nets share a node) rather than against real
//! design rules. The output uses the same `Layout` as the OpenROAD flow, so
//! the 3D viewer, `chip render` and live signals work unchanged.

pub mod place;
pub mod route;

use std::collections::{BTreeMap, HashMap};

use serde_json::json;

use crate::cldl::{CircuitBuilder, FlatLevel};
use crate::layout::def::Orient;
use crate::layout::{Instance, Layout, Master, MasterShape, NetInfo, RouteRect, SKY130_STACK};
use place::{Problem, floorplan, place};
use route::{Grid, route};

/// Grid pitch in database units (nm); roughly a sky130 metal pitch.
const PITCH: i32 = 460;
/// Cell row height in grid tracks.
const ROW_TRACKS: i32 = 8;

#[derive(Debug, Clone)]
pub struct Options {
    pub level: FlatLevel,
    /// Keep our latch flip-flops as gates instead of one block per flip-flop.
    pub keep_latches: bool,
    /// Routing layers (2..5), mapped to met1..met5.
    pub layers: i32,
    pub utilization: f64,
    /// Placement effort (annealing moves per cell, relative).
    pub effort: f64,
    pub max_iterations: usize,
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options { level: FlatLevel::Basic, keep_latches: false, layers: 3, utilization: 0.7, effort: 1.0, max_iterations: 40, seed: 1 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub cells: usize,
    pub nets: usize,
    pub grid: (i32, i32, i32),
    pub channel_tracks: i32,
    pub hpwl_before: i64,
    pub hpwl_after: i64,
    pub wire_per_layer: BTreeMap<String, f64>,
    pub vias: usize,
    pub iterations: usize,
    pub overflow: usize,
    pub failed_nets: Vec<String>,
    pub place_ms: f64,
    pub route_ms: f64,
    /// Core utilisation used (after retries).
    pub utilization: f64,
    /// Place-and-route attempts until legal (or giving up).
    pub attempts: usize,
}

/// Transistors per block type (static CMOS), for the viewer's metrics.
fn transistors(kind: &str, bits: usize) -> usize {
    bits * match kind {
        "NAND" | "NOR" => 4,
        "NOT" => 2,
        "AND" | "OR" => 6,
        "AND3" | "NOR4" => 8,
        "XOR" | "XNOR" => 10,
        "DFLIP_FLOP" => 24,
        k if k.starts_with("AND_") || k.starts_with("OR_") => 6,
        k if k.starts_with("XOR_") => 10,
        _ => 4,
    }
}

/// Wall clock in ms (std::time::Instant is not available in WASM).
fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }
}

/// Channel tracks between rows: with few layers layer 0 (blocked over the
/// cells) needs room for horizontal wiring; with more layers the routing
/// goes over the cells.
fn channel_tracks(layers: i32) -> i32 {
    match layers {
        ..=2 => 6,
        3 => 3,
        _ => 1,
    }
}

/// Places and routes; if the routing does not become legal, retries with
/// wider channels and lower utilisation (fewer layers need more area), up to
/// `MAX_ATTEMPTS` times. The report says what it took.
pub fn place_and_route(builder: &CircuitBuilder, name: &str, opts: &Options) -> Result<(Layout, Report), String> {
    if !(2..=5).contains(&opts.layers) {
        return Err(format!("layers must be 2..5, got {}", opts.layers));
    }
    const MAX_ATTEMPTS: usize = 5;
    let mut channel = channel_tracks(opts.layers);
    let mut util = opts.utilization;
    let mut last = None;
    for attempt_no in 1..=MAX_ATTEMPTS {
        let (layout, mut report) = attempt(builder, name, opts, channel, util)?;
        report.attempts = attempt_no;
        let legal = report.overflow == 0 && report.failed_nets.is_empty();
        if legal {
            return Ok((layout, report));
        }
        last = Some((layout, report));
        channel += 1 + channel / 2;
        util *= 0.85;
    }
    Ok(last.unwrap())
}

fn attempt(builder: &CircuitBuilder, name: &str, opts: &Options, channel: i32, utilization: f64) -> Result<(Layout, Report), String> {
    let keep_ff = !opts.keep_latches;
    let flat = builder
        .flatten_until(name, &|t: &str| (opts.level == FlatLevel::Basic && builder.is_basic(t)) || (keep_ff && t == "DFLIP_FLOP"))
        .ok_or_else(|| format!("Circuit '{name}' not found"))?;

    // --- Cells: one grid column per pin bit, inputs first.
    struct CellPin {
        net: usize,
        output: bool,
    }
    let cells: Vec<(String, String, Vec<CellPin>)> = flat
        .blocks
        .iter()
        .map(|b| {
            let mut pins = Vec::new();
            for output in [false, true] {
                for p in b.pins.iter().filter(|p| p.output == output) {
                    pins.extend(p.nets.iter().map(|&net| CellPin { net, output }));
                }
            }
            (b.id.clone(), b.kind.clone(), pins)
        })
        .collect();
    let widths: Vec<i32> = cells.iter().map(|c| c.2.len() as i32 + 1).collect();

    // --- Nets: drivers and loads among cell pins and top-level IO.
    let mut net_cells: HashMap<usize, Vec<(usize, i32)>> = HashMap::new();
    let mut driven: std::collections::HashSet<usize> = flat.inputs.iter().map(|(_, n)| *n).collect();
    for (ci, c) in cells.iter().enumerate() {
        for (pi, p) in c.2.iter().enumerate() {
            net_cells.entry(p.net).or_default().push((ci, pi as i32 + 1));
            if p.output {
                driven.insert(p.net);
            }
        }
    }
    let io_nets: Vec<(usize, bool, String)> = flat
        .inputs
        .iter()
        .map(|(k, n)| (*n, false, k.clone()))
        .chain(flat.outputs.iter().map(|(k, n)| (*n, true, k.clone())))
        .collect();
    let mut net_ids: Vec<usize> = net_cells
        .keys()
        .copied()
        .chain(io_nets.iter().map(|(n, _, _)| *n))
        .filter(|n| driven.contains(n))
        .collect();
    net_ids.sort_unstable();
    net_ids.dedup();

    // --- Floorplan and placement.
    let pitch_y = ROW_TRACKS + channel;
    let (rows, row_length) = floorplan(&widths, pitch_y, utilization);
    let margin = 3; // tracks around the core for IO pins
    let grid_w = row_length + 2 * margin;
    let grid_h = rows * pitch_y + channel + 2 * margin;
    let io_layer = if opts.layers >= 3 { 2 } else { 0 };
    // IO pins: inputs on the left edge, outputs on the right edge, spread
    // over the rows (in the channels when they sit on layer 0).
    let io_ys = |count: usize| -> Vec<i32> {
        let slots: Vec<i32> = if io_layer == 0 {
            (0..=rows).flat_map(|r| (0..channel).map(move |t| margin + r * pitch_y + t)).collect()
        } else {
            (margin..grid_h - margin).collect()
        };
        (0..count).map(|i| slots[(i * slots.len()) / count.max(1) + slots.len() / (2 * count.max(1))]).collect()
    };
    let ins: Vec<&(usize, bool, String)> = io_nets.iter().filter(|io| !io.1).collect();
    let outs: Vec<&(usize, bool, String)> = io_nets.iter().filter(|io| io.1).collect();
    let mut io_pos: Vec<(usize, String, i32, i32)> = Vec::new(); // net, key, x, y
    for (list, x) in [(&ins, 0), (&outs, grid_w - 1)] {
        for (io, y) in list.iter().zip(io_ys(list.len())) {
            io_pos.push((io.0, io.2.clone(), x, y));
        }
    }

    let problem = Problem {
        widths: widths.clone(),
        nets: net_ids
            .iter()
            .map(|n| {
                let fixed = io_pos.iter().filter(|p| p.0 == *n).map(|p| (p.2 - margin, p.3 - margin)).collect();
                (net_cells.get(n).cloned().unwrap_or_default(), fixed)
            })
            .collect(),
        row_height: pitch_y,
    };
    let t0 = now_ms();
    let start = place(&problem, rows, row_length, 0.0, opts.seed);
    let placed = place(&problem, rows, row_length, opts.effort, opts.seed);
    let place_ms = now_ms() - t0;
    // Grid position of each cell: rows sit above a bottom channel.
    let cell_xy: Vec<(i32, i32)> = placed.pos.iter().map(|&(x, y)| (x + margin, y + channel + margin)).collect();

    // --- Routing grid: layer 0 blocked over cells, except at pins.
    let mut grid = Grid::new(grid_w, grid_h, opts.layers);
    let pin_y = |cell: usize| cell_xy[cell].1 + ROW_TRACKS / 2;
    for (ci, &(x, y)) in cell_xy.iter().enumerate() {
        for dx in 0..widths[ci] {
            for dy in 0..ROW_TRACKS {
                if x + dx < grid_w && y + dy < grid_h {
                    let id = grid.id(x + dx, y + dy, 0) as usize;
                    grid.blocked[id] = true;
                }
            }
        }
    }
    let mut route_pins: Vec<Vec<u32>> = Vec::new();
    for &n in &net_ids {
        let mut pins = Vec::new();
        for &(c, dx) in net_cells.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
            let id = grid.id(cell_xy[c].0 + dx, pin_y(c), 0);
            grid.blocked[id as usize] = false;
            pins.push(id);
        }
        for p in io_pos.iter().filter(|p| p.0 == n) {
            pins.push(grid.id(p.2, p.3, io_layer));
        }
        pins.sort_unstable();
        pins.dedup();
        route_pins.push(pins);
    }
    let t1 = now_ms();
    let routed = route(&grid, &route_pins, opts.max_iterations);
    let route_ms = now_ms() - t1;

    // --- Layout.
    let dbu = 1000;
    let to_dbu = |t: i32| t * PITCH;
    let layer_of = |name: &str| SKY130_STACK.iter().position(|l| l.name == name).unwrap() as u8;
    let metal = |l: i32| layer_of(["met1", "met2", "met3", "met4", "met5"][l as usize]);
    let cut = |l: i32| layer_of(["via", "via2", "via3", "via4"][l as usize]);
    let (diff, li1, mcon) = (layer_of("diff"), layer_of("li1"), layer_of("mcon"));

    // Masters per (kind, width).
    let mut master_index: HashMap<(String, i32), u32> = HashMap::new();
    let mut masters: Vec<Master> = Vec::new();
    let mut instances = Vec::new();
    for (ci, (id, kind, pins)) in cells.iter().enumerate() {
        let w = widths[ci];
        let key = (kind.clone(), w);
        let m = *master_index.entry(key).or_insert_with(|| {
            let mut shapes = vec![MasterShape::Rect { layer: diff, x0: 40, y0: 40, x1: to_dbu(w) - 40, y1: to_dbu(ROW_TRACKS) - 40 }];
            for k in 0..pins.len() as i32 {
                let (cx, cy) = (to_dbu(k + 1), to_dbu(ROW_TRACKS / 2));
                shapes.push(MasterShape::Rect { layer: li1, x0: cx - 90, y0: cy - 90, x1: cx + 90, y1: cy + 90 });
                shapes.push(MasterShape::Rect { layer: mcon, x0: cx - 60, y0: cy - 60, x1: cx + 60, y1: cy + 60 });
            }
            masters.push(Master { name: format!("{kind}_{w}"), width: to_dbu(w), height: to_dbu(ROW_TRACKS), shapes });
            masters.len() as u32 - 1
        });
        instances.push(Instance { name: id.clone(), path: Some(id.clone()), master: m, x: to_dbu(cell_xy[ci].0), y: to_dbu(cell_xy[ci].1), orient: Orient::N });
    }

    // Wires: merge collinear unit edges per net and layer into rectangles.
    let half_w = |l: i32| if l < 2 { 70 } else { 150 };
    let mut routes = Vec::new();
    let mut nets = Vec::new();
    let mut wire_per_layer: BTreeMap<String, f64> = BTreeMap::new();
    let mut vias = 0;
    for (ni, &n) in net_ids.iter().enumerate() {
        let key = flat.nets[n].clone();
        nets.push(NetInfo { name: key.clone(), key: Some(key), special: false });
        let net_idx = ni as u32;
        // Group straight edges by line: (layer, fixed coordinate) -> spans.
        let mut lines: BTreeMap<(i32, bool, i32), Vec<(i32, i32)>> = BTreeMap::new();
        for &(a, b) in &routed.edges[ni] {
            let (ax, ay, al) = grid.xyl(a);
            let (bx, by, bl) = grid.xyl(b);
            if al != bl {
                let l = al.min(bl);
                let (cx, cy) = (to_dbu(ax), to_dbu(ay));
                routes.push(RouteRect { layer: cut(l), net: net_idx, x0: cx - 75, y0: cy - 75, x1: cx + 75, y1: cy + 75 });
                for ml in [al, bl] {
                    let h = half_w(ml).max(100);
                    routes.push(RouteRect { layer: metal(ml), net: net_idx, x0: cx - h, y0: cy - h, x1: cx + h, y1: cy + h });
                }
                vias += 1;
            } else if ay == by {
                lines.entry((al, true, ay)).or_default().push((ax.min(bx), ax.max(bx)));
            } else {
                lines.entry((al, false, ax)).or_default().push((ay.min(by), ay.max(by)));
            }
        }
        for ((l, horizontal, fixed), mut spans) in lines {
            spans.sort_unstable();
            let mut merged: Vec<(i32, i32)> = Vec::new();
            for (s, e) in spans {
                match merged.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => merged.push((s, e)),
                }
            }
            let hw = half_w(l);
            for (s, e) in merged {
                *wire_per_layer.entry(SKY130_STACK[metal(l) as usize].name.to_string()).or_default() += ((e - s) * PITCH) as f64 / 1000.0;
                let r = if horizontal {
                    (to_dbu(s) - hw, to_dbu(fixed) - hw, to_dbu(e) + hw, to_dbu(fixed) + hw)
                } else {
                    (to_dbu(fixed) - hw, to_dbu(s) - hw, to_dbu(fixed) + hw, to_dbu(e) + hw)
                };
                routes.push(RouteRect { layer: metal(l), net: net_idx, x0: r.0, y0: r.1, x1: r.2, y1: r.3 });
            }
        }
        // Pin landing pads on met1 and top-level IO pins.
        for &p in &route_pins[ni] {
            let (x, y, l) = grid.xyl(p);
            let (cx, cy) = (to_dbu(x), to_dbu(y));
            routes.push(RouteRect { layer: metal(l), net: net_idx, x0: cx - 120, y0: cy - 120, x1: cx + 120, y1: cy + 120 });
        }
    }

    let logic_cells = instances.len();
    let transistor_count: usize = cells.iter().map(|(_, k, pins)| transistors(k, pins.iter().filter(|p| p.output).count().max(1))).sum();
    let wire_total: f64 = wire_per_layer.values().sum();
    let die = (0, 0, to_dbu(grid_w), to_dbu(grid_h));
    let run = format!("own-L{}", opts.layers);
    let metrics = json!({
        "run": run,
        "max_layer": format!("met{}", opts.layers),
        "die_um": [die.2 as f64 / 1000.0, die.3 as f64 / 1000.0],
        "logic_cells": logic_cells,
        "transistors": transistor_count,
        "wire_total_um": wire_total,
        "wire_um": wire_per_layer,
        "vias": vias,
        "fmax_mhz": null,
        "router": { "iterations": routed.iterations, "overflow": routed.overflow, "failed_nets": routed.failed.len(), "utilization": utilization, "channel_tracks": channel },
    });
    let layout = Layout { design: name.to_string(), dbu_per_micron: dbu, die, masters, instances, nets, routes, metrics: Some(metrics) };
    let report = Report {
        cells: logic_cells,
        nets: net_ids.len(),
        grid: (grid_w, grid_h, opts.layers),
        channel_tracks: channel,
        hpwl_before: start.hpwl,
        hpwl_after: placed.hpwl,
        wire_per_layer,
        vias,
        iterations: routed.iterations,
        overflow: routed.overflow,
        failed_nets: routed.failed.iter().map(|&i| flat.nets[net_ids[i]].clone()).collect(),
        place_ms,
        route_ms,
        utilization,
        attempts: 1,
    };
    Ok((layout, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn routes_small_circuits_legally() {
        let mut b = CircuitBuilder::new();
        b.load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits"))).unwrap();
        for (name, layers) in [("FULL_ADDER", 2), ("ALU_4B", 3), ("REGISTER_8B", 4)] {
            let opts = Options { layers, ..Default::default() };
            let (layout, report) = place_and_route(&b, name, &opts).unwrap();
            assert_eq!(report.overflow, 0, "{name}");
            assert!(report.failed_nets.is_empty(), "{name}: {:?}", report.failed_nets);
            assert!(report.hpwl_after <= report.hpwl_before, "{name}");
            assert!(layout.nets.iter().all(|n| n.key.is_some()));
            // Readable by the viewer.
            let (header, _) = crate::layout::read_binary(&layout.write_binary()).unwrap();
            assert_eq!(header["design"], name);
        }
    }
}
