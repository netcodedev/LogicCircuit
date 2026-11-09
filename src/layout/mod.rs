//! Chip layout extraction (roadmap phase 3): combines the routed DEF from
//! OpenROAD with the cell geometry from the standard-cell GDS and the LEF
//! technology data into one self-contained layout, ready for 3D display.
//!
//! The layout keeps cell masters once (instances refer to them with a
//! position and orientation), routing as rectangles tagged with their net,
//! and maps every net back to the wire key of our simulator, so a viewer can
//! colour wires by live values.

pub mod def;
pub mod gds;
pub mod lef;
pub mod render;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::{Value, json};

use def::{Def, DefNet, Orient};
use lef::{LayerKind, Lef};

pub const FORMAT_MAGIC: &[u8; 8] = b"LCLAYOUT";
pub const FORMAT_VERSION: u32 = 1;

/// One layer of the process stack as shown in 3D.
#[derive(Debug, Clone)]
pub struct StackLayer {
    pub name: &'static str,
    /// "well", "device", "contact", "routing" or "cut".
    pub kind: &'static str,
    /// GDS layer/datatype of the drawn shapes in the cell library.
    pub gds: (i16, i16),
    /// Bottom and thickness in microns (approximate sky130 stack).
    pub z: f64,
    pub thickness: f64,
    pub color: &'static str,
}

/// sky130 layer stack, bottom to top. Heights follow the published
/// sky130 cross section (rounded); routing thicknesses match the tech LEF.
pub const SKY130_STACK: &[StackLayer] = &[
    StackLayer { name: "nwell", kind: "well", gds: (64, 20), z: -0.4, thickness: 0.4, color: "#4b4f2a" },
    StackLayer { name: "diff", kind: "device", gds: (65, 20), z: 0.0, thickness: 0.12, color: "#3fa34d" },
    StackLayer { name: "tap", kind: "device", gds: (65, 44), z: 0.0, thickness: 0.12, color: "#2f7a8c" },
    StackLayer { name: "poly", kind: "device", gds: (66, 20), z: 0.3262, thickness: 0.18, color: "#d6453d" },
    StackLayer { name: "licon", kind: "contact", gds: (66, 44), z: 0.12, thickness: 0.8161, color: "#9a9a9a" },
    StackLayer { name: "li1", kind: "routing", gds: (67, 20), z: 0.9361, thickness: 0.1, color: "#a46ad6" },
    StackLayer { name: "mcon", kind: "cut", gds: (67, 44), z: 1.0361, thickness: 0.34, color: "#bdbdbd" },
    StackLayer { name: "met1", kind: "routing", gds: (68, 20), z: 1.3761, thickness: 0.35, color: "#3d7fd6" },
    StackLayer { name: "via", kind: "cut", gds: (68, 44), z: 1.7261, thickness: 0.28, color: "#bdbdbd" },
    StackLayer { name: "met2", kind: "routing", gds: (69, 20), z: 2.0061, thickness: 0.35, color: "#d6a33d" },
    StackLayer { name: "via2", kind: "cut", gds: (69, 44), z: 2.3561, thickness: 0.43, color: "#bdbdbd" },
    StackLayer { name: "met3", kind: "routing", gds: (70, 20), z: 2.7861, thickness: 0.8, color: "#3dd6c4" },
    StackLayer { name: "via3", kind: "cut", gds: (70, 44), z: 3.5861, thickness: 0.435, color: "#bdbdbd" },
    StackLayer { name: "met4", kind: "routing", gds: (71, 20), z: 4.0211, thickness: 0.8, color: "#d63da9" },
    StackLayer { name: "via4", kind: "cut", gds: (71, 44), z: 4.8211, thickness: 0.55, color: "#bdbdbd" },
    StackLayer { name: "met5", kind: "routing", gds: (72, 20), z: 5.3711, thickness: 1.2, color: "#e0e0e0" },
];

fn layer_index(name: &str) -> Option<usize> {
    SKY130_STACK.iter().position(|l| l.name == name)
}

/// Layers a cut layer connects (below, above).
fn cut_neighbours(layer: usize) -> Option<(usize, usize)> {
    let n = SKY130_STACK[layer].name;
    let pair = match n {
        "mcon" => ("li1", "met1"),
        "via" => ("met1", "met2"),
        "via2" => ("met2", "met3"),
        "via3" => ("met3", "met4"),
        "via4" => ("met4", "met5"),
        _ => return None,
    };
    Some((layer_index(pair.0)?, layer_index(pair.1)?))
}

#[derive(Debug, Clone)]
pub enum MasterShape {
    Rect { layer: u8, x0: i32, y0: i32, x1: i32, y1: i32 },
    Polygon { layer: u8, points: Vec<(i32, i32)> },
}

#[derive(Debug, Clone)]
pub struct Master {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub shapes: Vec<MasterShape>,
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub name: String,
    /// Hierarchy path in our circuit (None for filler/tap/clock-tree cells).
    pub path: Option<String>,
    pub master: u32,
    pub x: i32,
    pub y: i32,
    pub orient: Orient,
}

#[derive(Debug, Clone)]
pub struct NetInfo {
    pub name: String,
    /// Wire key in our simulator, if the net corresponds to one.
    pub key: Option<String>,
    pub special: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RouteRect {
    pub layer: u8,
    pub net: u32,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub design: String,
    pub dbu_per_micron: i32,
    pub die: (i32, i32, i32, i32),
    pub masters: Vec<Master>,
    pub instances: Vec<Instance>,
    pub nets: Vec<NetInfo>,
    pub routes: Vec<RouteRect>,
    /// Extra information for viewers (e.g. the run's metrics), copied into
    /// the header as "metrics".
    pub metrics: Option<Value>,
}

/// Result of the extraction checks.
#[derive(Debug, Clone, Default)]
pub struct Checks {
    /// Exported cells (names.tsv) that are missing from the layout.
    pub missing_cells: Vec<String>,
    pub exported_cells: usize,
    /// Signal nets with at least two connections that were checked.
    pub nets_checked: usize,
    /// Nets whose pins are not all joined by their routing.
    pub open_nets: Vec<String>,
    pub unknown_vias: Vec<String>,
    pub unknown_layers: Vec<String>,
    pub masters_without_gds: Vec<String>,
}

/// Input files of an extraction.
pub struct Sources<'a> {
    pub def: &'a Path,
    pub tech_lef: &'a Path,
    pub cell_lef: &'a Path,
    pub cell_gds: &'a Path,
    /// names.tsv from the export (Verilog name -> wire key / part path).
    pub names: Option<&'a Path>,
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn extract(src: &Sources) -> Result<(Layout, Checks), String> {
    let mut lef = Lef::default();
    lef::parse_into(&mut lef, &read(src.tech_lef)?);
    lef::parse_into(&mut lef, &read(src.cell_lef)?);
    let widths: HashMap<String, i32> = lef
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Routing)
        .map(|l| (l.name.clone(), l.width))
        .collect();
    let def_text = read(src.def)?;
    let def = def::parse(&def_text, &lef.vias, &widths)?;
    let gds = gds::parse(&std::fs::read(src.cell_gds).map_err(|e| format!("{}: {e}", src.cell_gds.display()))?)?;
    let names: HashMap<String, String> = match src.names {
        Some(p) => read(p)?
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(v, k)| (v.to_string(), k.to_string()))
            .collect(),
        None => HashMap::new(),
    };
    let design = def_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("DESIGN "))
        .map(|s| s.trim_end_matches(';').trim().to_string())
        .unwrap_or_default();

    let mut checks = Checks { unknown_vias: def.unknown_vias.clone(), ..Default::default() };

    // --- Masters: geometry from GDS for every master that is placed.
    let gds_layers: HashMap<(i16, i16), u8> = SKY130_STACK.iter().enumerate().map(|(i, l)| (l.gds, i as u8)).collect();
    let mut master_index: HashMap<String, u32> = HashMap::new();
    let mut masters: Vec<Master> = Vec::new();
    for c in &def.components {
        if master_index.contains_key(&c.master) {
            continue;
        }
        let (w, h) = lef.macros.get(&c.master).map_or((0, 0), |m| (m.width, m.height));
        let mut shapes = Vec::new();
        match gds.structures.get(&c.master) {
            Some(gs) => {
                for s in gs {
                    let Some(&layer) = gds_layers.get(&(s.layer, s.datatype)) else { continue };
                    shapes.push(shape_from_points(layer, &s.points));
                }
            }
            None => checks.masters_without_gds.push(c.master.clone()),
        }
        master_index.insert(c.master.clone(), masters.len() as u32);
        masters.push(Master { name: c.master.clone(), width: w, height: h, shapes });
    }

    let instances: Vec<Instance> = def
        .components
        .iter()
        .map(|c| Instance {
            name: c.name.clone(),
            path: names.get(&c.name).cloned(),
            master: master_index[&c.master],
            x: c.x,
            y: c.y,
            orient: c.orient,
        })
        .collect();

    // --- Nets and their routing.
    let clock_port = def
        .nets
        .iter()
        .find(|n| !n.special && n.connections.iter().any(|(i, _)| i.starts_with("clkbuf_0")))
        .map(|n| n.name.clone());
    let pin_nets: HashMap<&str, &str> = def.pins.iter().map(|p| (p.name.as_str(), p.net.as_str())).collect();
    let mut nets = Vec::new();
    let mut routes = Vec::new();
    let mut unknown_layers = Vec::new();
    for (idx, n) in def.nets.iter().enumerate() {
        let clean = n.name.replace('\\', "");
        let key = if n.special {
            None
        } else if let Some(k) = names.get(&clean) {
            Some(k.clone())
        } else if clean.starts_with("clknet") {
            clock_port.as_ref().map(|c| port_key(c))
        } else if pin_nets.values().any(|v| v.replace('\\', "") == clean) {
            Some(port_key(&clean))
        } else {
            None
        };
        nets.push(NetInfo { name: clean, key, special: n.special });
        for (layer, x0, y0, x1, y1) in &n.shapes {
            match layer_index(layer) {
                Some(l) => routes.push(RouteRect { layer: l as u8, net: idx as u32, x0: *x0, y0: *y0, x1: *x1, y1: *y1 }),
                None => unknown_layers.push(layer.clone()),
            }
        }
    }
    // Top-level pin shapes belong to their net.
    let net_by_name: HashMap<&str, usize> = def.nets.iter().enumerate().map(|(i, n)| (n.name.as_str(), i)).collect();
    for p in &def.pins {
        if let Some(&net) = net_by_name.get(p.net.as_str()) {
            for (layer, x0, y0, x1, y1) in &p.rects {
                if let Some(l) = layer_index(layer) {
                    routes.push(RouteRect { layer: l as u8, net: net as u32, x0: *x0, y0: *y0, x1: *x1, y1: *y1 });
                }
            }
        }
    }
    unknown_layers.sort();
    unknown_layers.dedup();
    checks.unknown_layers = unknown_layers;

    // --- Checks: every exported cell placed, every signal net connected.
    let placed: std::collections::HashSet<&str> = def.components.iter().map(|c| c.name.as_str()).collect();
    let mut exported: Vec<&String> = names.keys().filter(|v| v.starts_with("u_")).collect();
    exported.sort();
    checks.exported_cells = exported.len();
    checks.missing_cells = exported.into_iter().filter(|v| !placed.contains(v.as_str())).cloned().collect();
    let components: HashMap<&str, &def::DefComponent> = def.components.iter().map(|c| (c.name.as_str(), c)).collect();
    for n in def.nets.iter().filter(|n| !n.special && n.connections.len() >= 2) {
        checks.nets_checked += 1;
        if !net_connected(n, &def, &lef, &components) {
            checks.open_nets.push(n.name.replace('\\', ""));
        }
    }

    Ok((Layout { design, dbu_per_micron: def.dbu_per_micron, die: def.die, masters, instances, nets, routes, metrics: None }, checks))
}

/// "Instruction[3]" -> "Instruction.3", "CLK" -> "CLK" (our key format).
fn port_key(net: &str) -> String {
    match net.strip_suffix(']').and_then(|s| s.split_once('[')) {
        Some((name, bit)) => format!("{name}.{bit}"),
        None => net.to_string(),
    }
}

fn shape_from_points(layer: u8, pts: &[(i32, i32)]) -> MasterShape {
    let xs: Vec<i32> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<i32> = pts.iter().map(|p| p.1).collect();
    let (x0, x1) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
    let (y0, y1) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
    let is_rect = pts.len() == 4 && pts.iter().all(|&(x, y)| (x == x0 || x == x1) && (y == y0 || y == y1));
    if is_rect {
        MasterShape::Rect { layer, x0, y0, x1, y1 }
    } else {
        MasterShape::Polygon { layer, points: pts.to_vec() }
    }
}

/// True if the routing of `net` joins all pins it connects. Shapes connect
/// when they overlap on the same layer, or through a cut layer (via) to the
/// layers below and above it.
fn net_connected(net: &DefNet, def: &Def, lef: &Lef, components: &HashMap<&str, &def::DefComponent>) -> bool {
    // (layer index, rect, owner): owner = connection index for pin shapes.
    let mut shapes: Vec<(usize, (i32, i32, i32, i32), Option<usize>)> = Vec::new();
    for (layer, x0, y0, x1, y1) in &net.shapes {
        if let Some(l) = layer_index(layer) {
            shapes.push((l, (*x0, *y0, *x1, *y1), None));
        }
    }
    for (ci, (inst, pin)) in net.connections.iter().enumerate() {
        if inst == "PIN" {
            for p in def.pins.iter().filter(|p| &p.name == pin) {
                for (layer, x0, y0, x1, y1) in &p.rects {
                    if let Some(l) = layer_index(layer) {
                        shapes.push((l, (*x0, *y0, *x1, *y1), Some(ci)));
                    }
                }
            }
            continue;
        }
        let Some(c) = components.get(inst.as_str()) else { return false };
        let Some(m) = lef.macros.get(&c.master) else { return false };
        for p in m.pins.iter().filter(|p| &p.name == pin) {
            for (layer, x0, y0, x1, y1) in &p.rects {
                if let Some(l) = layer_index(layer) {
                    let (a, b, cc, d) = c.orient.apply_rect((*x0, *y0, *x1, *y1), m.width, m.height);
                    shapes.push((l, (c.x + a, c.y + b, c.x + cc, c.y + d), Some(ci)));
                }
            }
        }
    }

    let mut parent: Vec<usize> = (0..shapes.len()).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut x = x;
        while p[x] != r {
            let next = p[x];
            p[x] = r;
            x = next;
        }
        r
    }
    let touches = |a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)| a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3;
    let joins = |la: usize, lb: usize| {
        la == lb
            || cut_neighbours(la).is_some_and(|(d, u)| lb == d || lb == u)
            || cut_neighbours(lb).is_some_and(|(d, u)| la == d || la == u)
    };
    for i in 0..shapes.len() {
        for j in i + 1..shapes.len() {
            if joins(shapes[i].0, shapes[j].0) && touches(shapes[i].1, shapes[j].1) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    // One component must contain a shape of every connection.
    let mut owners_by_root: HashMap<usize, std::collections::HashSet<usize>> = HashMap::new();
    for i in 0..shapes.len() {
        if let Some(o) = shapes[i].2 {
            let r = find(&mut parent, i);
            owners_by_root.entry(r).or_default().insert(o);
        }
    }
    owners_by_root.values().any(|o| o.len() == net.connections.len())
}

impl Layout {
    /// Header (JSON) plus Int32 sections, see `write_binary`.
    fn header_and_sections(&self) -> (Value, Vec<(&'static str, usize, Vec<i32>)>) {
        let mut master_rects = Vec::new();
        let mut master_polys = Vec::new();
        for (mi, m) in self.masters.iter().enumerate() {
            for s in &m.shapes {
                match s {
                    MasterShape::Rect { layer, x0, y0, x1, y1 } => {
                        master_rects.extend([mi as i32, *layer as i32, *x0, *y0, *x1, *y1]);
                    }
                    MasterShape::Polygon { layer, points } => {
                        master_polys.extend([mi as i32, *layer as i32, points.len() as i32]);
                        master_polys.extend(points.iter().flat_map(|&(x, y)| [x, y]));
                    }
                }
            }
        }
        let instances: Vec<i32> = self.instances.iter().flat_map(|i| [i.master as i32, i.x, i.y, i.orient.code()]).collect();
        let routes: Vec<i32> = self
            .routes
            .iter()
            .flat_map(|r| [r.layer as i32, r.net as i32, r.x0, r.y0, r.x1, r.y1])
            .collect();
        let sections = vec![("master_rects", 6, master_rects), ("master_polys", 0, master_polys), ("instances", 4, instances), ("routes", 6, routes)];

        let header = json!({
            "format": "LogicCircuit layout",
            "version": FORMAT_VERSION,
            "design": self.design,
            "dbu_per_micron": self.dbu_per_micron,
            "die": [self.die.0, self.die.1, self.die.2, self.die.3],
            "orientations": ["N", "S", "E", "W", "FN", "FS", "FE", "FW"],
            "layers": SKY130_STACK.iter().map(|l| json!({
                "name": l.name, "kind": l.kind, "z": l.z, "thickness": l.thickness, "color": l.color,
            })).collect::<Vec<_>>(),
            "masters": self.masters.iter().map(|m| json!({ "name": m.name, "width": m.width, "height": m.height })).collect::<Vec<_>>(),
            "instance_names": self.instances.iter().map(|i| &i.name).collect::<Vec<_>>(),
            "instance_paths": self.instances.iter().map(|i| &i.path).collect::<Vec<_>>(),
            "nets": self.nets.iter().map(|n| json!({ "name": n.name, "key": n.key, "special": n.special })).collect::<Vec<_>>(),
            "metrics": self.metrics,
        });
        (header, sections)
    }

    /// Binary layout: magic "LCLAYOUT", u32 version, u32 header length, the
    /// JSON header (padded to 4 bytes), then the Int32 (little endian)
    /// sections listed in the header's "sections" (offsets in Int32 units
    /// from the start of the section data):
    /// - master_rects: [master, layer, x0, y0, x1, y1]
    /// - master_polys: [master, layer, n, x0, y0, ..., xn-1, yn-1]
    /// - instances:    [master, x, y, orientation index]
    /// - routes:       [layer, net, x0, y0, x1, y1]
    ///
    /// All coordinates are database units (`dbu_per_micron`).
    pub fn write_binary(&self) -> Vec<u8> {
        let (mut header, sections) = self.header_and_sections();
        let mut offset = 0;
        let mut index = Vec::new();
        for (name, stride, data) in &sections {
            index.push(json!({ "name": name, "offset": offset, "length": data.len(), "stride": stride }));
            offset += data.len();
        }
        header["sections"] = Value::Array(index);
        let mut header_bytes = header.to_string().into_bytes();
        while header_bytes.len() % 4 != 0 {
            header_bytes.push(b' ');
        }
        let mut out = Vec::with_capacity(16 + header_bytes.len() + offset * 4);
        out.extend_from_slice(FORMAT_MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&header_bytes);
        for (_, _, data) in &sections {
            for v in data {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    /// The same content as one JSON document (for debugging; large).
    pub fn to_json(&self) -> String {
        let (mut header, sections) = self.header_and_sections();
        for (name, _, data) in sections {
            header[name] = json!(data);
        }
        header.to_string()
    }

    /// Shape counts per layer (masters counted per placed instance).
    pub fn shapes_per_layer(&self) -> BTreeMap<&'static str, usize> {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut per_master = vec![BTreeMap::<u8, usize>::new(); self.masters.len()];
        for (i, m) in self.masters.iter().enumerate() {
            for s in &m.shapes {
                let l = match s {
                    MasterShape::Rect { layer, .. } | MasterShape::Polygon { layer, .. } => *layer,
                };
                *per_master[i].entry(l).or_default() += 1;
            }
        }
        for inst in &self.instances {
            for (l, n) in &per_master[inst.master as usize] {
                *counts.entry(SKY130_STACK[*l as usize].name).or_default() += n;
            }
        }
        for r in &self.routes {
            *counts.entry(SKY130_STACK[r.layer as usize].name).or_default() += 1;
        }
        counts
    }
}

/// Reads a binary layout back (header JSON and sections), for tests and tools.
pub fn read_binary(data: &[u8]) -> Result<(Value, Vec<i32>), String> {
    if data.len() < 16 || &data[0..8] != FORMAT_MAGIC {
        return Err("not a LogicCircuit layout file".into());
    }
    let header_len = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let header: Value = serde_json::from_slice(&data[16..16 + header_len]).map_err(|e| e.to_string())?;
    let ints = data[16 + header_len..]
        .chunks_exact(4)
        .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    Ok((header, ints))
}

impl Layout {
    /// Reads a binary layout (see `write_binary`) back into a `Layout`.
    pub fn from_binary(data: &[u8]) -> Result<Layout, String> {
        let (h, ints) = read_binary(data)?;
        let section = |name: &str| -> &[i32] {
            h["sections"]
                .as_array()
                .and_then(|a| a.iter().find(|s| s["name"] == name))
                .map(|s| {
                    let o = s["offset"].as_u64().unwrap_or(0) as usize;
                    let l = s["length"].as_u64().unwrap_or(0) as usize;
                    &ints[o..o + l]
                })
                .unwrap_or(&[])
        };
        let str_of = |v: &Value| v.as_str().unwrap_or("").to_string();
        let mut masters: Vec<Master> = h["masters"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|m| Master {
                        name: str_of(&m["name"]),
                        width: m["width"].as_i64().unwrap_or(0) as i32,
                        height: m["height"].as_i64().unwrap_or(0) as i32,
                        shapes: Vec::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        for r in section("master_rects").chunks_exact(6) {
            if let Some(m) = masters.get_mut(r[0] as usize) {
                m.shapes.push(MasterShape::Rect { layer: r[1] as u8, x0: r[2], y0: r[3], x1: r[4], y1: r[5] });
            }
        }
        let polys = section("master_polys");
        let mut i = 0;
        while i + 3 <= polys.len() {
            let n = polys[i + 2] as usize;
            let points = polys[i + 3..i + 3 + 2 * n].chunks_exact(2).map(|p| (p[0], p[1])).collect();
            if let Some(m) = masters.get_mut(polys[i] as usize) {
                m.shapes.push(MasterShape::Polygon { layer: polys[i + 1] as u8, points });
            }
            i += 3 + 2 * n;
        }
        let orients: Vec<String> = h["orientations"].as_array().map(|a| a.iter().map(str_of).collect()).unwrap_or_default();
        let names = h["instance_names"].as_array().cloned().unwrap_or_default();
        let paths = h["instance_paths"].as_array().cloned().unwrap_or_default();
        let instances = section("instances")
            .chunks_exact(4)
            .enumerate()
            .map(|(k, r)| Instance {
                name: names.get(k).map(str_of).unwrap_or_default(),
                path: paths.get(k).and_then(|p| p.as_str()).map(str::to_string),
                master: r[0] as u32,
                x: r[1],
                y: r[2],
                orient: Orient::parse(orients.get(r[3] as usize).map(String::as_str).unwrap_or("N")),
            })
            .collect();
        let nets = h["nets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|n| NetInfo { name: str_of(&n["name"]), key: n["key"].as_str().map(str::to_string), special: n["special"].as_bool().unwrap_or(false) })
                    .collect()
            })
            .unwrap_or_default();
        let routes = section("routes")
            .chunks_exact(6)
            .map(|r| RouteRect { layer: r[0] as u8, net: r[1] as u32, x0: r[2], y0: r[3], x1: r[4], y1: r[5] })
            .collect();
        let die = h["die"].as_array().map(|d| {
            let v = |i: usize| d.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
            (v(0), v(1), v(2), v(3))
        });
        Ok(Layout {
            design: str_of(&h["design"]),
            dbu_per_micron: h["dbu_per_micron"].as_i64().unwrap_or(1000) as i32,
            die: die.unwrap_or_default(),
            masters,
            instances,
            nets,
            routes,
            metrics: (!h["metrics"].is_null()).then(|| h["metrics"].clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_roundtrip() {
        let layout = Layout {
            design: "T".into(),
            dbu_per_micron: 1000,
            die: (0, 0, 100, 100),
            masters: vec![Master {
                name: "m".into(),
                width: 10,
                height: 20,
                shapes: vec![
                    MasterShape::Rect { layer: 7, x0: 0, y0: 0, x1: 5, y1: 5 },
                    MasterShape::Polygon { layer: 5, points: vec![(0, 0), (4, 0), (4, 2), (2, 2), (2, 4), (0, 4)] },
                ],
            }],
            instances: vec![Instance { name: "u_a".into(), path: Some("A".into()), master: 0, x: 3, y: 4, orient: Orient::FS }],
            nets: vec![NetInfo { name: "n_x".into(), key: Some("X".into()), special: false }],
            routes: vec![RouteRect { layer: 7, net: 0, x0: 1, y0: 2, x1: 3, y1: 4 }],
            metrics: None,
        };
        let (header, ints) = read_binary(&layout.write_binary()).unwrap();
        let section = |name: &str| -> Vec<i32> {
            let s = header["sections"].as_array().unwrap().iter().find(|s| s["name"] == name).unwrap();
            let (o, l) = (s["offset"].as_u64().unwrap() as usize, s["length"].as_u64().unwrap() as usize);
            ints[o..o + l].to_vec()
        };
        assert_eq!(section("master_rects"), vec![0, 7, 0, 0, 5, 5]);
        assert_eq!(section("master_polys")[..3], [0, 5, 6]);
        assert_eq!(section("instances"), vec![0, 3, 4, Orient::FS.code()]);
        assert_eq!(section("routes"), vec![7, 0, 1, 2, 3, 4]);
        assert_eq!(header["nets"][0]["key"], "X");
        assert_eq!(header["instance_paths"][0], "A");
        let back = Layout::from_binary(&layout.write_binary()).unwrap();
        assert_eq!(back.write_binary(), layout.write_binary());
    }

    #[test]
    fn port_keys() {
        assert_eq!(port_key("Instruction[3]"), "Instruction.3");
        assert_eq!(port_key("CLK"), "CLK");
    }

    #[test]
    fn stack_is_ordered_and_cuts_connect_neighbours() {
        let routing: Vec<f64> = SKY130_STACK.iter().filter(|l| l.kind == "routing").map(|l| l.z).collect();
        assert!(routing.windows(2).all(|w| w[0] < w[1]));
        let via2 = layer_index("via2").unwrap();
        assert_eq!(cut_neighbours(via2), Some((layer_index("met2").unwrap(), layer_index("met3").unwrap())));
    }
}
