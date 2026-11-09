// Layout for the flat views: a circuit reduced to NAND gates or to basic
// blocks (NOT, AND, OR, XOR_8B, ...).
//
// Small netlists are laid out by ELK (orthogonal routing, like the normal
// view). ELK becomes far too slow for thousands of blocks, so larger
// netlists use a fast layered layout of our own: breadth-first layering,
// a few barycenter sweeps for ordering, and curved edges.

export const ELK_LIMIT = 400;

// Single-bit gates drawn with a logic symbol (see the <symbol>s in
// index.html); everything else is drawn as a box with one port per bit.
export const SYMBOLS = {
  NAND: 'nand', AND: 'and', AND3: 'and', OR: 'or', NOR: 'nor', NOR4: 'nor',
  XOR: 'xor', XNOR: 'xnor', NOT: 'not',
};
export const SYMBOL_W = 36;

// Transistor view: a NAND drawn as CMOS cell (see app.js drawCmosNand).
export const CMOS = { w: 64, h: 76, in1: 24, in2: 61, out: 32 };

// x of a symbol's input outline at height y (both in the 36x24 viewBox of the
// <symbol>s in index.html). OR-family symbols have a curved back
// (quadratic from (x0,1) via (x0+6,12) to (x0,23)); input wires must reach it.
export function symbolInputX(symbol, y) {
  const t = Math.min(1, Math.max(0, (y - 1) / 22));
  const bulge = 12 * t * (1 - t);
  switch (symbol) {
    case 'or': case 'nor': return 1 + bulge;
    case 'xor': case 'xnor': return 5 + bulge;
    default: return 1;
  }
}
const BOX_HEADER = 20;
const BOX_W = 70;
const BOX_ROW = 11;
const IO_H = 18;

// Fast layout spacing.
const LAYER_GAP = 70;
const ROW_PITCH = 34;
const SUB_GAP = 26;     // between the sub-columns of a wrapped layer
const SWEEPS = 6;

const bitLabel = (key) => key.replace(/\.(\d+)$/, '[$1]');

/**
 * Builds the graph: nodes for top-level input bits, blocks and output bits,
 * each with ports at fixed positions, and one edge per (net driver -> load).
 * Nets without a driver (unconnected, reading 0) have no edges.
 */
function buildGraph(flat, cmos) {
  const nodes = [];
  const ioWidth = (label) => Math.max(44, label.length * 6.6 + 14);

  flat.inputs.forEach(([key, net], i) => {
    const label = bitLabel(key);
    const w = ioWidth(label);
    nodes.push({ id: `i${i}`, kind: 'in', key, label, w, h: IO_H, ports: [{ net, output: true, x: w, y: IO_H / 2 }] });
  });

  flat.blocks.forEach((b, i) => {
    // One port per bit, inputs left, outputs right.
    const bitsOf = (output) => b.pins.filter((p) => p.output === output).flatMap((p) =>
      p.nets.map((net, bit) => ({ net, output, label: p.width != null ? `${p.name}${bit}` : p.name })));
    const ins = bitsOf(false);
    const outs = bitsOf(true);
    const symbol = SYMBOLS[b.kind] && outs.length === 1 ? SYMBOLS[b.kind] : null;
    let w, h;
    if (cmos && b.kind === 'NAND') {
      w = CMOS.w;
      h = CMOS.h;
      Object.assign(ins[0], { x: 0, y: CMOS.in1 });
      Object.assign(ins[1], { x: 0, y: CMOS.in2 });
      Object.assign(outs[0], { x: w, y: CMOS.out });
      nodes.push({ id: `b${i}`, kind: 'block', block: i, gid: b.id, type: b.kind, cmos: true, w, h, ports: [...ins, ...outs] });
      return;
    }
    if (symbol) {
      w = SYMBOL_W;
      h = ins.length <= 2 ? 24 : ins.length * 10;
      ins.forEach((p, k) => { p.x = 0; p.y = (h * (2 * k + 1)) / (2 * ins.length); });
      outs[0].x = w;
      outs[0].y = h / 2;
    } else {
      w = BOX_W;
      h = BOX_HEADER + Math.max(ins.length, outs.length, 1) * BOX_ROW + 4;
      ins.forEach((p, k) => { p.x = 0; p.y = BOX_HEADER + k * BOX_ROW + BOX_ROW / 2; });
      outs.forEach((p, k) => { p.x = w; p.y = BOX_HEADER + k * BOX_ROW + BOX_ROW / 2; });
    }
    nodes.push({ id: `b${i}`, kind: 'block', block: i, gid: b.id, type: b.kind, symbol, w, h, ports: [...ins, ...outs] });
  });

  flat.outputs.forEach(([key, net], i) => {
    const label = bitLabel(key);
    nodes.push({ id: `o${i}`, kind: 'out', key, label, w: ioWidth(label), h: IO_H, ports: [{ net, output: false, x: 0, y: IO_H / 2 }] });
  });

  for (const n of nodes) n.ports.forEach((p, k) => { p.id = `${n.id}:${k}`; p.node = n; });

  // Block outputs drive their nets; top-level inputs drive nets nothing else drives.
  const driver = new Map(); // net -> port
  for (const n of nodes) {
    if (n.kind === 'block') for (const p of n.ports) if (p.output && !driver.has(p.net)) driver.set(p.net, p);
  }
  for (const n of nodes) {
    if (n.kind === 'in' && !driver.has(n.ports[0].net)) driver.set(n.ports[0].net, n.ports[0]);
  }

  const edges = [];
  for (const n of nodes) {
    for (const p of n.ports) {
      if (p.output) continue;
      const from = driver.get(p.net);
      if (from) edges.push({ net: p.net, from, to: p });
    }
  }
  return { nodes, edges };
}

const abs = (port) => ({ x: port.node.x + port.x, y: port.node.y + port.y });

export async function layoutFlat(flat, elk, { cmos = false } = {}) {
  const graph = buildGraph(flat, cmos);
  return flat.blocks.length <= ELK_LIMIT ? layoutElk(graph, elk) : layoutFast(graph);
}

// ---------------------------------------------------------------- ELK

async function layoutElk({ nodes, edges }, elk) {
  const P = 2;
  const children = nodes.map((n) => {
    const layoutOptions = { 'elk.portConstraints': 'FIXED_POS' };
    if (n.kind === 'in') layoutOptions['elk.layered.layering.layerConstraint'] = 'FIRST';
    if (n.kind === 'out') layoutOptions['elk.layered.layering.layerConstraint'] = 'LAST';
    return {
      id: n.id, width: n.w, height: n.h, layoutOptions,
      ports: n.ports.map((p) => ({ id: p.id, x: p.output ? p.x : -P, y: p.y - P / 2, width: P, height: P })),
    };
  });
  const result = await elk.layout({
    id: 'root',
    layoutOptions: {
      'elk.algorithm': 'layered',
      'elk.direction': 'RIGHT',
      'elk.edgeRouting': 'ORTHOGONAL',
      'elk.layered.spacing.nodeNodeBetweenLayers': '40',
      'elk.layered.spacing.edgeNodeBetweenLayers': '10',
      'elk.layered.spacing.edgeEdgeBetweenLayers': '5',
      'elk.spacing.nodeNode': '16',
      'elk.spacing.edgeEdge': '5',
      'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
      'elk.padding': '[top=30,left=30,bottom=30,right=30]',
    },
    children,
    edges: edges.map((e, i) => ({ id: `e${i}`, sources: [e.from.id], targets: [e.to.id] })),
  });
  const pos = new Map(result.children.map((c) => [c.id, c]));
  for (const n of nodes) { n.x = pos.get(n.id).x; n.y = pos.get(n.id).y; }
  const out = result.edges.map((re, i) => {
    const sec = re.sections?.[0];
    const pts = sec ? [sec.startPoint, ...(sec.bendPoints || []), sec.endPoint] : [];
    // ELK ends edges on the outer border of the (2px) ports. Snap both ends
    // onto the actual pin; the end segments are horizontal, so only x moves.
    if (pts.length) {
      pts[0] = { x: abs(edges[i].from).x, y: pts[0].y };
      pts[pts.length - 1] = { x: abs(edges[i].to).x, y: pts[pts.length - 1].y };
    }
    return {
      net: edges[i].net,
      d: pts.length ? 'M' + pts.map((p) => `${p.x},${p.y}`).join(' L') : '',
      junctions: re.junctionPoints || [],
    };
  });
  return { width: result.width, height: result.height, nodes, edges: out, router: 'ELK' };
}

// ---------------------------------------------------------------- fast layered layout

function layoutFast({ nodes, edges }) {
  const index = new Map(nodes.map((n, i) => [n, i]));
  const succ = nodes.map(() => []);
  for (const e of edges) {
    const u = index.get(e.from.node), v = index.get(e.to.node);
    if (u !== v) succ[u].push(v);
  }

  // 1. Layering by breadth-first distance from the inputs. Unlike
  //    longest-path layering this needs no cycle breaking (latches and
  //    flip-flops are full of feedback) and keeps the drawing compact:
  //    longest paths through feedback loops chain whole register files into
  //    hundreds of nearly empty layers. Edges may point backwards; they are
  //    drawn as loops.
  const layer = new Int32Array(nodes.length).fill(-1);
  const bfs = (sources, startLayer) => {
    const queue = [];
    for (const s of sources) { layer[s] = startLayer; queue.push(s); }
    for (let qi = 0; qi < queue.length; qi++) {
      const u = queue[qi];
      for (const v of succ[u]) {
        if (layer[v] === -1 && nodes[v].kind !== 'out') { layer[v] = layer[u] + 1; queue.push(v); }
      }
    }
  };
  bfs(nodes.flatMap((n, i) => (n.kind === 'in' ? [i] : [])), 0);
  // Blocks not reachable from any input (e.g. fed only by constants or by
  // their own feedback) start their own search.
  nodes.forEach((n, i) => { if (layer[i] === -1 && n.kind === 'block') bfs([i], 1); });
  let maxLayer = 0;
  nodes.forEach((n, i) => { if (n.kind !== 'out') maxLayer = Math.max(maxLayer, layer[i]); });
  nodes.forEach((n, i) => { if (n.kind === 'out') layer[i] = maxLayer + 1; });

  // 2. Order within layers: start in build order (which keeps a part's
  //    blocks together), then barycenter sweeps against all neighbours.
  const layers = Array.from({ length: maxLayer + 2 }, () => []);
  nodes.forEach((_, i) => layers[layer[i]].push(i));
  const pred = nodes.map(() => []);
  succ.forEach((vs, u) => vs.forEach((v) => pred[v].push(u)));
  const rel = new Float64Array(nodes.length); // relative position in its layer, 0..1
  const updateRel = (l) => l.forEach((n, k) => { rel[n] = l.length > 1 ? k / (l.length - 1) : 0.5; });
  layers.forEach(updateRel);
  for (let s = 0; s < SWEEPS; s++) {
    const down = s % 2 === 0;
    const order = down ? layers : [...layers].reverse();
    for (const l of order) {
      const key = new Map();
      for (const n of l) {
        const nb = down ? pred[n] : succ[n];
        key.set(n, nb.length ? nb.reduce((a, m) => a + rel[m], 0) / nb.length : rel[n]);
      }
      l.sort((a, b) => key.get(a) - key.get(b));
      updateRel(l);
    }
  }

  // 3. Coordinates: layers left to right, each centred vertically. Layers
  //    taller than the height budget wrap into several sub-columns, so that big
  //    circuits (thousands of blocks in a few BFS layers) end up roughly
  //    square instead of a single 40 000 px high strip. Rows are as tall as
  //    the tallest block of their layer.
  const pitchOf = (l) => Math.max(ROW_PITCH, ...l.map((n) => nodes[n].h + 10));
  const widthOf = (l) => Math.max(SYMBOL_W, ...l.map((n) => nodes[n].w));
  // Size of the drawing for a given column height budget.
  const extent = (budget) => {
    let w = 0, h = 0;
    for (const l of layers) {
      const rows = Math.max(1, Math.floor(budget / pitchOf(l)));
      const columns = Math.ceil(l.length / rows) || 1;
      w += columns * widthOf(l) + (columns - 1) * SUB_GAP + LAYER_GAP;
      h = Math.max(h, Math.min(rows, l.length) * pitchOf(l));
    }
    return { w, h };
  };
  // Pick the budget whose drawing is closest to landscape 1.6 : 1 (width
  // shrinks as the budget grows, so bisect).
  let lo = 12 * ROW_PITCH, hi = Math.max(lo, ...layers.map((l) => l.length * pitchOf(l)));
  for (let i = 0; i < 40; i++) {
    const mid = (lo + hi) / 2;
    const { w, h } = extent(mid);
    if (w / h > 1.6) lo = mid; else hi = mid;
  }
  const targetHeight = hi;
  const maxRowsOf = (l) => Math.max(1, Math.floor(targetHeight / pitchOf(l)));
  const columnHeight = (l) => Math.min(maxRowsOf(l), l.length) * pitchOf(l);
  const tallest = Math.max(...layers.map(columnHeight));
  let x = 30;
  for (const l of layers) {
    const w = widthOf(l);
    const pitch = pitchOf(l);
    const columns = Math.max(1, Math.ceil(l.length / maxRowsOf(l)));
    const perColumn = Math.ceil(l.length / columns);
    l.forEach((n, k) => {
      const col = Math.floor(k / perColumn);
      const row = k % perColumn;
      const inColumn = Math.min(perColumn, l.length - col * perColumn);
      const node = nodes[n];
      node.x = x + col * (w + SUB_GAP) + (w - node.w);
      node.y = 30 + (tallest - inColumn * pitch) / 2 + row * pitch + (pitch - node.h) / 2;
    });
    x += columns * w + (columns - 1) * SUB_GAP + LAYER_GAP;
  }

  // 4. Edges as cubic curves; backward edges loop out and back in.
  const out = edges.map((e) => {
    const a = abs(e.from);
    const b = abs(e.to);
    const dx = b.x > a.x ? Math.max(20, (b.x - a.x) / 2) : 60;
    return {
      net: e.net,
      d: `M${a.x},${a.y} C${a.x + dx},${a.y} ${b.x - dx},${b.y} ${b.x},${b.y}`,
      junctions: [],
    };
  });
  return { width: x - LAYER_GAP + 30, height: tallest + 60, nodes, edges: out, router: 'schnell' };
}
