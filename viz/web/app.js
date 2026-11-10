import init, { Sim } from './pkg/logic_circuit_viz.js';
import { layoutFlat, symbolInputX, SYMBOL_W, CMOS } from './flat.js';

const SVGNS = 'http://www.w3.org/2000/svg';
const HEADER = 34;   // part title area
const ROW = 16;      // vertical spacing between ports
const PORT = 6;      // port square size
const TAP_W = 8;     // splitter / merger bar width
const DIGIT_PART = 'HEX_7SEG';
const DIGIT_W = 40, DIGIT_H = 70;

// Segments a..g of a DIGIT_W x DIGIT_H digit, as [x, y, w, h].
const SEGMENTS = [
  [6, 0, 28, 7], [33, 6, 7, 26], [33, 38, 7, 26], [6, 63, 28, 7],
  [0, 38, 7, 26], [0, 6, 7, 26], [6, 31.5, 28, 7],
];

// Draws one 7-segment digit at (x, y); returns the 7 segment elements.
function drawDigit(parent, x, y) {
  const g = el('g', { transform: `translate(${x},${y})`, class: 'digit' }, parent);
  return SEGMENTS.map(([sx, sy, w, h]) => el('rect', { x: sx, y: sy, width: w, height: h, rx: 3, class: 'seg' }, g));
}

function showSegments(segs, vals) {
  segs.forEach((seg, i) => seg.classList.toggle('on', vals[i] === 1));
}

const $ = (id) => document.getElementById(id);
const statusEl = $('status');

await init();
const sim = new Sim();
const elk = new ELK();
const names = JSON.parse(sim.circuit_names());
const nandCounts = JSON.parse(sim.nand_counts());
const fmtCount = (n) => n.toLocaleString('de-DE');
const nandLabel = (n) => `${fmtCount(n)} NAND`;
const views = new Map();    // type -> CircuitView JSON
const layouts = new Map();  // "type|mode" -> { graph, meta }

// Bus rendering mode: false = one line coloured by summary, true = one
// stripe per bit. Remembered per browser; storage may be unavailable.
const STRIPES_KEY = 'lc-viz-stripes';
let stripes = false;
try { stripes = localStorage.getItem(STRIPES_KEY) === '1'; } catch {}

// View mode: 'parts' (the CLDL hierarchy), 'basic' (flattened to basic
// blocks like NOT, AND, OR_4B) or 'nand' (flattened to NAND gates).
const MODE_KEY = 'lc-viz-mode';
const VIEW_MODES = ['parts', 'basic', 'nand', 'cmos', 'chip'];
// Transistor view: drawing every NAND as a CMOS cell only stays readable
// (and fast) up to this many gates; bigger circuits ask to drill down.
const CMOS_LIMIT = 1500;
let viewMode = 'parts';
try { const m = localStorage.getItem(MODE_KEY); if (VIEW_MODES.includes(m)) viewMode = m; } catch {}
// ?mode=chip (etc.) in the URL wins, e.g. for links opened by `chip view`.
{ const m = new URLSearchParams(location.search).get('mode'); if (VIEW_MODES.includes(m)) viewMode = m; }
const flats = new Map();  // "mode|type" -> flattened netlist JSON
let currentGraph = null;  // { width, height } of what is drawn, for fit()

// Per-bit stripe thickness; narrower for wide buses so a 16-bit bus stays readable.
const stripeWidth = (bits) => Math.max(1.2, Math.min(2.5, 16 / bits));

// Navigation state: stack[0] is the built top-level circuit.
let stack = [];             // [{ instance, type }]
let probes = [];            // [{ keys, apply(vals) }]
let sidebarProbes = [];

function view(type) {
  if (!views.has(type)) views.set(type, JSON.parse(sim.describe(type)));
  return views.get(type);
}

function prefix() {
  return stack.slice(1).map((s) => s.instance).join('.');
}

function fullKey(local) {
  const p = prefix();
  return p ? `${p}.${local}` : local;
}

function pinKeys(base, width) {
  if (width == null) return [base];
  return Array.from({ length: width }, (_, i) => `${base}.${i}`);
}

// ---------------------------------------------------------------- layout

function textWidth(s, px) { return s.length * px * 0.62; }

// Second line of a part node: its type and how many NANDs one instance uses.
function partSubtitle(part) {
  return part.type === 'NAND' ? 'NAND' : `${part.type} · ${nandLabel(part.nands)}`;
}

function pinLabel(p) { return p.width != null ? `${p.name}[${p.width}]` : p.name; }

// Label plus room for the value ("1" or "0x…").
function ioWidth(p) {
  const value = p.width != null ? 2 + Math.ceil(p.width / 4) : 1;
  return textWidth(pinLabel(p), 11) + textWidth('x'.repeat(value), 11) + 36;
}

async function layout(type) {
  const cacheKey = `${type}|${stripes}`;
  if (layouts.has(cacheKey)) return layouts.get(cacheKey);
  const v = view(type);
  const children = [];
  const edges = [];
  const meta = { ports: new Map(), nodes: new Map() };

  const inputs = new Map(v.inputs.map((p) => [p.name, p]));
  const outputs = new Map(v.outputs.map((p) => [p.name, p]));
  const parts = new Map(v.parts.map((p) => [p.name, p]));

  // Striped buses are as wide as all their stripes together. ELK spaces edges
  // as if they were thin lines, so widen the edge channels and the port pitch
  // to the widest bus of this level, otherwise parallel buses overlap.
  const maxBits = Math.max(1, ...[...v.inputs, ...v.outputs, ...v.parts.flatMap((p) => [...p.inputs, ...p.outputs])]
    .map((p) => p.width ?? 1));
  const busWidth = stripes && maxBits > 1 ? Math.ceil(maxBits * stripeWidth(maxBits)) + 2 : 0;
  const row = Math.max(ROW, busWidth + 6);
  const edgeGap = (thin) => String(Math.max(thin, busWidth + 4));

  for (const p of v.inputs) {
    const id = `in:${p.name}`;
    const w = ioWidth(p);
    children.push({
      id, width: w, height: 24,
      layoutOptions: { 'elk.layered.layering.layerConstraint': 'FIRST', 'elk.portConstraints': 'FIXED_POS' },
      ports: [{ id: `${id}#`, x: w, y: 12 - PORT / 2, width: PORT, height: PORT }],
    });
    meta.nodes.set(id, { kind: 'in', pin: p });
    meta.ports.set(`${id}#`, { local: p.name, width: p.width });
  }
  for (const p of v.outputs) {
    const id = `out:${p.name}`;
    const w = ioWidth(p);
    children.push({
      id, width: w, height: 24,
      layoutOptions: { 'elk.layered.layering.layerConstraint': 'LAST', 'elk.portConstraints': 'FIXED_POS' },
      ports: [{ id: `${id}#`, x: -PORT, y: 12 - PORT / 2, width: PORT, height: PORT }],
    });
    meta.nodes.set(id, { kind: 'out', pin: p });
    meta.ports.set(`${id}#`, { local: p.name, width: p.width });
  }
  for (const part of v.parts) {
    const id = `p:${part.name}`;
    const maxIn = Math.max(0, ...part.inputs.map((p) => textWidth(pinLabel(p), 10)));
    const maxOut = Math.max(0, ...part.outputs.map((p) => textWidth(pinLabel(p), 10)));
    const w = Math.max(90, textWidth(part.name, 12) + 24, textWidth(partSubtitle(part), 10) + 24, maxIn + maxOut + 30);
    const rows = Math.max(part.inputs.length, part.outputs.length, 1);
    // A HEX_7SEG part shows its digit below the ports.
    const digitY = part.type === DIGIT_PART ? HEADER + rows * row + 6 : null;
    const h = digitY != null ? digitY + DIGIT_H + 10 : HEADER + rows * row + 4;
    const ports = [];
    part.inputs.forEach((p, i) => {
      const pid = `${id}#${p.name}`;
      ports.push({ id: pid, x: -PORT, y: HEADER + i * row + row / 2 - PORT / 2, width: PORT, height: PORT });
      meta.ports.set(pid, { local: `${part.name}.${p.name}`, width: p.width, label: pinLabel(p), side: 'W', row: i });
    });
    part.outputs.forEach((p, i) => {
      const pid = `${id}#${p.name}`;
      ports.push({ id: pid, x: w, y: HEADER + i * row + row / 2 - PORT / 2, width: PORT, height: PORT });
      meta.ports.set(pid, { local: `${part.name}.${p.name}`, width: p.width, label: pinLabel(p), side: 'E', row: i });
    });
    children.push({ id, width: w, height: h, layoutOptions: { 'elk.portConstraints': 'FIXED_POS' }, ports });
    meta.nodes.set(id, { kind: 'part', part, digitY });
  }

  // Width of an endpoint's pin (bus width or null), used to pick wire keys.
  const endpointWidth = (e) => {
    if (e.index != null) return null;
    if (e.part != null) {
      const part = parts.get(e.part);
      const pin = part && [...part.inputs, ...part.outputs].find((p) => p.name === e.pin);
      return pin ? pin.width : null;
    }
    return (inputs.get(e.pin) || outputs.get(e.pin) || {}).width ?? null;
  };

  const target = (e) => {
    if (e.part != null) return `p:${e.part}#${e.pin}`;
    if (inputs.has(e.pin)) return `in:${e.pin}#`;
    if (outputs.has(e.pin)) return `out:${e.pin}#`;
    const id = `net:${e.pin}`;
    if (!meta.nodes.has(id)) {
      children.push({ id, width: 8, height: 8 });
      meta.nodes.set(id, { kind: 'net', name: e.pin });
    }
    return id;
  };

  const localKey = (e) => {
    const base = e.part != null ? `${e.part}.${e.pin}` : e.pin;
    return e.index != null ? `${base}.${e.index}` : base;
  };

  // Bit-indexed endpoints go through a splitter (source side) or merger
  // (dest side) next to their bus pin. Without it, edges for different bits
  // leave the same port and share their first segments, so the drawn colour
  // of the shared part would be whichever bit was painted last.
  const taps = new Map(); // node id -> { kind, busPort, base, width, bits }
  const tap = (kind, e) => {
    const bus = { ...e, index: null };
    const busPort = target(bus);
    const id = `${kind}:${busPort}`;
    if (!taps.has(id)) {
      taps.set(id, { kind, busPort, base: localKey(bus), width: endpointWidth(bus), bits: new Set() });
    }
    taps.get(id).bits.add(e.index);
    return `${id}#${e.index}`;
  };

  const addEdge = (source, dest, keys, title) => edges.push({
    id: `e${edges.length}`, sources: [source], targets: [dest],
    _keys: keys, _bus: keys.length > 1, _title: title,
    ...(stripes && keys.length > 1
      ? { layoutOptions: { 'elk.edge.thickness': String(keys.length * stripeWidth(keys.length)) } }
      : {}),
  });

  for (const c of v.connections) {
    const srcW = endpointWidth(c.source);
    const local = localKey(c.source);
    const src = c.source.index != null ? tap('split', c.source) : target(c.source);
    const dst = c.dest.index != null ? tap('merge', c.dest) : target(c.dest);
    addEdge(src, dst, pinKeys(local, srcW), `${local}${srcW != null ? `[${srcW}]` : ''} → ${localKey(c.dest)}`);
  }

  for (const [id, t] of taps) {
    const split = t.kind === 'split';
    // MSB on top, matching the stripe order of striped buses.
    const bits = [...t.bits].sort((a, b) => b - a);
    const h = bits.length * ROW;
    const ports = [{ id: `${id}#bus`, x: split ? -PORT : TAP_W, y: h / 2 - PORT / 2, width: PORT, height: PORT }];
    meta.ports.set(`${id}#bus`, { local: t.base, width: t.width });
    bits.forEach((b, i) => {
      const pid = `${id}#${b}`;
      ports.push({ id: pid, x: split ? TAP_W : -PORT, y: i * ROW + ROW / 2 - PORT / 2, width: PORT, height: PORT });
      meta.ports.set(pid, { local: `${t.base}.${b}`, width: null, bit: String(b), side: split ? 'E' : 'W' });
    });
    children.push({ id, width: TAP_W, height: h, layoutOptions: { 'elk.portConstraints': 'FIXED_POS' }, ports });
    meta.nodes.set(id, { kind: 'tap' });
    const busKeys = pinKeys(t.base, t.width);
    const busLabel = `${t.base}${t.width != null ? `[${t.width}]` : ''}`;
    if (split) addEdge(t.busPort, `${id}#bus`, busKeys, `${busLabel} → Splitter`);
    else addEdge(`${id}#bus`, t.busPort, busKeys, `Merger → ${busLabel}`);
  }

  const graph = {
    id: 'root',
    layoutOptions: {
      'elk.algorithm': 'layered',
      'elk.direction': 'RIGHT',
      'elk.edgeRouting': 'ORTHOGONAL',
      'elk.layered.spacing.nodeNodeBetweenLayers': '60',
      'elk.layered.spacing.edgeNodeBetweenLayers': edgeGap(16),
      'elk.layered.spacing.edgeEdgeBetweenLayers': edgeGap(7),
      'elk.spacing.nodeNode': '28',
      'elk.spacing.edgeEdge': edgeGap(6),
      'elk.layered.nodePlacement.strategy': 'NETWORK_SIMPLEX',
      'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
      'elk.padding': '[top=30,left=30,bottom=30,right=30]',
    },
    children,
    edges,
  };
  statusEl.textContent = `Layout ${type} (${children.length} Knoten, ${edges.length} Kanten)…`;
  const t0 = performance.now();
  const result = await elk.layout(graph);
  const out = { graph: result, meta, ms: performance.now() - t0 };
  layouts.set(cacheKey, out);
  return out;
}

// ---------------------------------------------------------------- render

function el(tag, attrs = {}, parent) {
  const n = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  if (parent) parent.appendChild(n);
  return n;
}

// Summarize bit values (LSB first) into a state class and a display string.
function summarize(bits) {
  if (bits.some((b) => b === 2)) return { cls: '2', text: '?' };
  if (bits.length === 1) return { cls: String(bits[0]), text: String(bits[0]) };
  const n = bits.reduce((acc, b, i) => acc + (b << i), 0);
  const all = bits.every((b) => b === bits[0]);
  const digits = Math.ceil(bits.length / 4);
  return { cls: all ? String(bits[0]) : 'm', text: '0x' + n.toString(16).toUpperCase().padStart(digits, '0') };
}

const toPath = (pts) => 'M' + pts.map((p) => `${p.x},${p.y}`).join(' L');

// Offsets a polyline sideways by d (positive = left of the direction of
// travel, i.e. up on a rightward segment). Corners use a miter join, which
// is exact for the orthogonal routes ELK produces.
function offsetPolyline(pts, d) {
  const normals = [];
  for (let i = 0; i < pts.length - 1; i++) {
    const dx = pts[i + 1].x - pts[i].x, dy = pts[i + 1].y - pts[i].y;
    const len = Math.hypot(dx, dy) || 1;
    normals.push({ x: dy / len, y: -dx / len });
  }
  return pts.map((p, i) => {
    const a = normals[Math.max(0, i - 1)], b = normals[Math.min(normals.length - 1, i)];
    const k = d / (1 + a.x * b.x + a.y * b.y || 1);
    return { x: p.x + (a.x + b.x) * k, y: p.y + (a.y + b.y) * k };
  });
}

// One stripe per bit, MSB on the left of the direction of travel (on top for
// the usual left-to-right flow), so 0101 reads grey-green-grey-green.
function renderStripedEdge(e, sec, pts, keys, edgeLayer, labelLayer) {
  const w = keys.length;
  const sw = stripeWidth(w);
  const g = el('g', { class: 'striped' }, edgeLayer);
  el('path', { d: toPath(pts), class: 'stripe-bg', 'stroke-width': w * sw + 2 }, g);
  const paths = keys.map((_, i) => el('path', {
    d: toPath(offsetPolyline(pts, (i - (w - 1) / 2) * sw)),
    class: 'stripe s0', 'stroke-width': sw,
  }, g));
  const title = el('title', {}, g);
  (e.junctionPoints || []).forEach((p) => el('circle', { cx: p.x, cy: p.y, r: (w * sw) / 2 + 1.5, class: 'junction stripe-dot' }, labelLayer));
  const label = el('text', { x: sec.startPoint.x + 6, y: sec.startPoint.y - (w * sw) / 2 - 4, class: 'elabel' }, labelLayer);
  probes.push({
    keys,
    apply(vals) {
      vals.forEach((v, i) => paths[i].setAttribute('class', `stripe e${v}`));
      const s = summarize(vals);
      title.textContent = `${e._title} = ${s.text}`;
      label.textContent = s.text;
    },
  });
}

async function render() {
  const top = stack[stack.length - 1];
  if (viewMode === 'chip') return renderChip();
  leaveChip();
  if (viewMode !== 'parts') return renderFlat(top);
  const { graph, meta, ms } = await layout(top.type);
  currentGraph = graph;
  const vp = $('viewport');
  vp.replaceChildren();
  probes = [];
  const settable = stack.length === 1;

  const edgeLayer = el('g', {}, vp);
  const nodeLayer = el('g', {}, vp);
  const labelLayer = el('g', {}, vp);

  for (const e of graph.edges) {
    const sec = e.sections && e.sections[0];
    if (!sec) continue;
    const pts = [sec.startPoint, ...(sec.bendPoints || []), sec.endPoint];
    const d = toPath(pts);
    const keys = e._keys.map(fullKey);
    if (stripes && e._bus) {
      renderStripedEdge(e, sec, pts, keys, edgeLayer, labelLayer);
      continue;
    }
    const path = el('path', { d, class: `edge${e._bus ? ' bus' : ''}` }, edgeLayer);
    const title = el('title', {}, path);
    const dots = (e.junctionPoints || []).map((p) => el('circle', { cx: p.x, cy: p.y, r: e._bus ? 3.5 : 2.5 }, labelLayer));
    let label = null;
    if (e._bus) label = el('text', { x: sec.startPoint.x + 6, y: sec.startPoint.y - 5, class: 'elabel' }, labelLayer);
    probes.push({
      keys,
      apply(vals) {
        const s = summarize(vals);
        path.setAttribute('class', `edge${e._bus ? ' bus' : ''} e${s.cls}`);
        for (const d of dots) d.setAttribute('class', `junction s${s.cls}`);
        title.textContent = `${e._title} = ${s.text}`;
        if (label) label.textContent = s.text;
      },
    });
  }

  for (const n of graph.children) {
    const m = meta.nodes.get(n.id);
    const g = el('g', { transform: `translate(${n.x},${n.y})` }, nodeLayer);
    if (m.kind === 'net') {
      g.setAttribute('class', 'net');
      el('circle', { cx: 4, cy: 4, r: 3 }, g);
      el('text', { x: 4, y: -4, 'text-anchor': 'middle' }, g).textContent = m.name;
      continue;
    }
    if (m.kind === 'tap') {
      el('rect', { class: 'tapbar', x: TAP_W / 2 - 2, y: -2, width: 4, height: n.height + 4, rx: 2 }, g);
    } else if (m.kind === 'in' || m.kind === 'out') {
      const canSet = m.kind === 'in' && settable && m.pin.width == null;
      g.setAttribute('class', `node io${canSet ? ' settable' : ''}`);
      el('rect', { class: 'body', width: n.width, height: n.height, rx: 12 }, g);
      const txt = el('text', { x: m.kind === 'in' ? 10 : 18, y: 16 }, g);
      txt.textContent = pinLabel(m.pin);
      const val = el('text', { x: m.kind === 'in' ? n.width - 8 : n.width - 8, y: 16, 'text-anchor': 'end' }, g);
      const keys = pinKeys(fullKey(m.pin.name), m.pin.width);
      probes.push({ keys, apply(vals) { val.textContent = summarize(vals).text; } });
      if (canSet) g.addEventListener('click', () => toggleInput(m.pin.name));
    } else {
      const part = m.part;
      g.setAttribute('class', `node${part.expandable ? ' expandable' : ''}`);
      el('rect', { class: 'body', width: n.width, height: n.height, rx: 6 }, g);
      el('text', { x: n.width / 2, y: 15, 'text-anchor': 'middle', class: 'title' }, g).textContent = part.name;
      el('text', { x: n.width / 2, y: 27, 'text-anchor': 'middle', class: 'type' }, g).textContent = partSubtitle(part);
      el('title', {}, g).textContent = part.expandable ? `${part.name}: ${part.type} — Doppelklick zum Öffnen` : `${part.name}: ${part.type}`;
      if (part.expandable) g.addEventListener('dblclick', (ev) => { ev.stopPropagation(); enter(part.name, part.type); });
      if (m.digitY != null) {
        const segs = drawDigit(g, n.width / 2 - DIGIT_W / 2, m.digitY);
        probes.push({ keys: pinKeys(fullKey(`${part.name}.Seg`), 7), apply(vals) { showSegments(segs, vals); } });
      }
    }
    for (const p of n.ports || []) {
      const pm = meta.ports.get(p.id);
      const sq = el('rect', { x: p.x, y: p.y, width: p.width, height: p.height, class: 'port s0' }, g);
      if (pm.bit) {
        const east = pm.side === 'E';
        el('text', { x: east ? p.x + PORT + 3 : p.x - 3, y: p.y - 1, 'text-anchor': east ? 'start' : 'end', class: 'bitlabel' }, g)
          .textContent = pm.bit;
      } else if (pm.label) {
        const left = pm.side === 'W';
        el('text', { x: left ? 5 : n.width - 5, y: p.y + PORT, 'text-anchor': left ? 'start' : 'end', class: 'plabel' }, g)
          .textContent = pm.label;
      }
      probes.push({
        keys: pinKeys(fullKey(pm.local), pm.width),
        apply(vals) { sq.setAttribute('class', `port s${summarize(vals).cls}`); },
      });
    }
  }

  renderCrumbs();
  fit();
  probesChanged();
  refresh();
  const nodes = graph.children.length, edges = graph.edges.length;
  statusEl.textContent = `${top.type} · ${nandLabel(view(top.type).nands)} · ${nodes} Knoten · ${edges} Verbindungen · Layout ${ms.toFixed(0)} ms`;
}

// ---------------------------------------------------------------- flat views

function flatOf(type) {
  const key = `${viewMode}|${type}`;
  if (!flats.has(key)) flats.set(key, JSON.parse(sim.flatten(type, viewMode === 'cmos' ? 'nand' : viewMode)));
  return flats.get(key);
}

async function renderFlat(top) {
  const flat = flatOf(top.type);
  const cmos = viewMode === 'cmos';
  const unit = viewMode === 'nand' || cmos ? 'NANDs' : 'Blöcke';
  if (cmos && flat.blocks.length > CMOS_LIMIT) {
    $('viewport').replaceChildren();
    probes = [];
    probesChanged();
    renderCrumbs();
    statusEl.textContent = `${top.type}: ${fmtCount(flat.blocks.length)} NANDs = ${fmtCount(flat.blocks.length * 4)} Transistoren – zu viele für die Transistoransicht (max. ${fmtCount(CMOS_LIMIT)} NANDs). In der Bauteil-Ansicht ein kleineres Bauteil öffnen.`;
    return;
  }
  const key = `flat|${viewMode}|${top.type}`;
  if (!layouts.has(key)) {
    statusEl.textContent = `Layout ${top.type}: ${fmtCount(flat.blocks.length)} ${unit}…`;
    await new Promise((r) => setTimeout(r, 20)); // let the status paint first
    const t0 = performance.now();
    const lay = await layoutFlat(flat, elk, { cmos });
    layouts.set(key, { lay, ms: performance.now() - t0 });
  }
  const { lay, ms } = layouts.get(key);
  currentGraph = lay;

  const vp = $('viewport');
  vp.replaceChildren();
  probes = [];
  const edgeLayer = el('g', {}, vp);
  const nodeLayer = el('g', {}, vp);
  const settable = stack.length === 1;

  // Everything that shows a net's value: [element, base class, state prefix].
  const onNet = new Map();
  const watch = (net, elem, base, prefix) => {
    if (!onNet.has(net)) onNet.set(net, []);
    onNet.get(net).push([elem, base, prefix]);
  };

  // One path per net (all its driver -> load edges).
  const netPaths = new Map();
  const addPath = (net, d) => netPaths.set(net, (netPaths.get(net) || '') + d + ' ');
  for (const e of lay.edges) {
    if (e.d) addPath(e.net, e.d);
  }
  // Extend input wires from the pin to the symbol's (possibly curved) outline.
  for (const n of lay.nodes) {
    if (!n.symbol) continue;
    for (const p of n.ports) {
      if (p.output) continue;
      const lead = symbolInputX(n.symbol, (p.y * 24) / n.h);
      if (lead > 1) addPath(p.net, `M${n.x},${n.y + p.y} L${n.x + lead},${n.y + p.y}`);
    }
  }
  for (const [net, d] of netPaths) {
    const p = el('path', { d, class: 'edge flat-net e0' }, edgeLayer);
    p.dataset.net = net;
    watch(net, p, 'edge flat-net', 'e');
  }
  for (const e of lay.edges) {
    for (const j of e.junctions) watch(e.net, el('circle', { cx: j.x, cy: j.y, r: 2 }, edgeLayer), 'junction', 's');
  }

  for (const n of lay.nodes) {
    if (n.kind === 'block' && n.cmos) {
      drawCmosNand(n, nodeLayer, watch);
    } else if (n.kind === 'block' && n.symbol) {
      // Logic symbol, coloured by its output.
      const u = el('use', {
        href: `#sym-${n.symbol}`, x: n.x, y: n.y, width: SYMBOL_W, height: n.h,
        preserveAspectRatio: 'none', class: 'gate s0',
      }, nodeLayer);
      u.dataset.block = n.block;
      watch(n.ports.find((p) => p.output).net, u, 'gate', 's');
    } else if (n.kind === 'block') {
      // Multi-bit block (e.g. AND_4B): box with one port per bit.
      const g = el('g', { transform: `translate(${n.x},${n.y})`, class: 'node flat-box' }, nodeLayer);
      g.dataset.block = n.block;
      el('rect', { class: 'body', width: n.w, height: n.h, rx: 4 }, g);
      el('text', { x: n.w / 2, y: 13, 'text-anchor': 'middle', class: 'type' }, g).textContent = n.type;
      for (const p of n.ports) {
        const sq = el('rect', { x: p.output ? p.x - 1 : p.x - 3, y: p.y - 2, width: 4, height: 4, class: 'port s0' }, g);
        watch(p.net, sq, 'port', 's');
        el('text', { x: p.output ? n.w - 5 : 5, y: p.y + 3, 'text-anchor': p.output ? 'end' : 'start', class: 'bitlabel' }, g)
          .textContent = p.label;
      }
    } else {
      const canSet = n.kind === 'in' && settable;
      const g = el('g', { transform: `translate(${n.x},${n.y})`, class: `node io flat-io${canSet ? ' settable' : ''}` }, nodeLayer);
      el('rect', { class: 'body', width: n.w, height: n.h, rx: n.h / 2 }, g);
      el('text', { x: 8, y: n.h / 2 + 4 }, g).textContent = n.label;
      const val = el('text', { x: n.w - 7, y: n.h / 2 + 4, 'text-anchor': 'end' }, g);
      probes.push({ keys: [fullKey(n.key)], apply(v) { val.textContent = v[0] === 2 ? '?' : v[0]; } });
      if (canSet) g.addEventListener('click', () => toggleInput(n.key));
    }
  }

  // One probe per net updates everything attached to it.
  for (const [net, items] of onNet) {
    probes.push({
      keys: [fullKey(flat.nets[net])],
      apply(v) { for (const [elem, base, prefix] of items) elem.setAttribute('class', `${base} ${prefix}${v[0]}`); },
    });
  }

  renderCrumbs();
  fit();
  probesChanged();
  refresh();
  const nands = viewMode === 'basic' ? ` · ${nandLabel(view(top.type).nands)}` : '';
  flatStatus = `${top.type} · ${fmtCount(flat.blocks.length)} ${unit}${nands} · ${fmtCount(netPaths.size)} Netze · Layout ${lay.router} ${ms.toFixed(0)} ms`;
  statusEl.textContent = flatStatus;
}

// One NAND as CMOS cell: PMOS A and B in parallel from VDD to the output,
// NMOS A and B in series from the output to GND. Transistors light up when
// they conduct (CSS: NMOS on at gate 1, PMOS on at gate 0); the output node
// takes the output value.
function drawCmosNand(n, layer, watch) {
  const g = el('g', { transform: `translate(${n.x},${n.y})`, class: 'node cmos' }, layer);
  g.dataset.block = n.block;
  const [a, b, out] = n.ports;
  el('rect', { class: 'body', width: CMOS.w, height: CMOS.h, rx: 4 }, g);
  // Rails
  el('path', { class: 'rail', d: 'M6 6H58 M6 70H58' }, g);
  el('text', { x: 60, y: 8, class: 'rail-label' }, g).textContent = '+';
  el('text', { x: 60, y: 73, class: 'rail-label' }, g).textContent = '⏚';
  // Gate wiring (thin): A to P_A and N_A, B to P_B and N_B.
  const wa = el('path', { class: 'gate-wire s0', d: `M0 ${CMOS.in1}H10V45H26 M10 ${CMOS.in1}V20H14` }, g);
  const wb = el('path', { class: 'gate-wire s0', d: `M0 ${CMOS.in2}H26 M6 ${CMOS.in2}V10H38V20` }, g);
  watch(a.net, wa, 'gate-wire', 's');
  watch(b.net, wb, 'gate-wire', 's');
  // Drains/sources: VDD -> P_A, P_B -> output node; output -> N_A -> N_B -> GND.
  el('path', { class: 'channel-wire', d: 'M20 6V14 M44 6V14 M20 26V32 M44 26V32 M32 32V40 M32 50V56 M32 66V70' }, g);
  const node = el('path', { class: 'out-node s0', d: `M20 32H${CMOS.w}` }, g);
  watch(out.net, node, 'out-node', 's');
  const transistor = (x, y, kind, net, label) => {
    const t = el('rect', { x: x - 3, y, width: 6, height: kind === 'p' ? 12 : 10, rx: 1.5, class: `tr ${kind}0` }, g);
    el('title', {}, t).textContent = `${kind === 'p' ? 'PMOS' : 'NMOS'} ${label}`;
    watch(net, t, 'tr', kind);
    if (kind === 'p') el('circle', { cx: x - 5, cy: y + 6, r: 1.6, class: 'bubble' }, g);
  };
  transistor(20, 14, 'p', a.net, 'A');
  transistor(44, 14, 'p', b.net, 'B');
  transistor(32, 40, 'n', a.net, 'A');
  transistor(32, 56, 'n', b.net, 'B');
}

// Hovering a block or wire in a flat view shows what it is in the status bar.
let flatStatus = '';
$('viewport').addEventListener('mouseover', (e) => {
  if (viewMode === 'parts') return;
  const flat = flats.get(`${viewMode}|${stack[stack.length - 1].type}`);
  if (!flat) return;
  const val = (k) => ['0', '1', '?'][sim.values(fullKey(k))[0]];
  const blockEl = e.target.closest('[data-block]');
  if (blockEl) {
    const b = flat.blocks[Number(blockEl.dataset.block)];
    const outs = b.pins.filter((p) => p.output)
      .map((p) => `${p.name} = ${p.nets.map((n) => val(flat.nets[n])).reverse().join('')}`).join(', ');
    statusEl.textContent = `${b.kind} ${b.id} · ${outs}`;
  } else if (e.target.dataset.net != null) {
    const k = flat.nets[Number(e.target.dataset.net)];
    statusEl.textContent = `Netz ${k} = ${val(k)}`;
  }
});
$('viewport').addEventListener('mouseout', () => { if (viewMode !== 'parts') statusEl.textContent = flatStatus; });

// ---------------------------------------------------------------- simulation

// The probe key list is rebuilt only when probes change (the flat view of a
// big circuit has thousands of them).
let allProbes = null;
let allKeys = '';
let probeOffsets = [];

function probesChanged() {
  allProbes = null;
}

function refresh() {
  if (!allProbes) {
    allProbes = [...probes, ...sidebarProbes];
    probeOffsets = [];
    const keys = [];
    for (const p of allProbes) { probeOffsets.push(keys.length); for (const k of p.keys) keys.push(k); }
    allKeys = keys.join('\n');
  }
  const vals = sim.values(allKeys);
  allProbes.forEach((p, i) => p.apply(p.keys.length === 1 ? [vals[probeOffsets[i]]] : Array.from(vals.subarray(probeOffsets[i], probeOffsets[i] + p.keys.length))));
}

function topInputKeys(name) {
  const pin = view(stack[0].type).inputs.find((p) => p.name === name);
  return pinKeys(name, pin.width);
}

function toggleInput(key) {
  const cur = sim.values(key)[0];
  sim.set_input(key, cur !== 1);
  sim.eval();
  refresh();
}

function setBus(name, value) {
  topInputKeys(name).forEach((k, i) => sim.set_input(k, ((value >> i) & 1) === 1));
  sim.eval();
  refresh();
}

// Inline number entry for a bus input (window.prompt is not available in
// every embedding browser). Enter applies, Escape or leaving the field cancels.
function editBusValue(valEl, pin) {
  if (valEl.querySelector('input')) return;
  const input = document.createElement('input');
  input.className = 'val-edit';
  input.value = valEl.textContent;
  valEl.replaceChildren(input);
  input.focus();
  input.select();
  // refresh() rewrites valEl, which removes the input and fires blur; the
  // flag keeps that blur from closing a second time.
  let done = false;
  const finish = (value) => {
    if (done) return;
    done = true;
    if (value == null) refresh();
    else setBus(pin.name, value);
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      const n = Number(input.value.trim());
      if (Number.isInteger(n) && n >= 0 && n <= 2 ** pin.width - 1) finish(n);
      else input.classList.add('bad');
    } else if (e.key === 'Escape') {
      finish(null);
    }
  });
  input.addEventListener('blur', () => finish(null));
}

function clockName() {
  return view(stack[0].type).inputs.find((p) => p.width == null && /^clk$/i.test(p.name))?.name;
}

$('pulse').addEventListener('click', () => {
  if (sim.is_runnable()) { stopRun(); stepProgram(); return; }
  const clk = clockName();
  sim.set_input(clk, true); sim.eval();
  sim.set_input(clk, false); sim.eval();
  refresh();
});

// Download the current level as structural Verilog (computed in WASM).
$('export-verilog').addEventListener('click', () => {
  const type = stack[stack.length - 1].type;
  const level = viewMode === 'nand' ? 'nand' : 'basic';
  const cells = $('export-cells').value;
  try {
    const text = sim.export_verilog(type, level, cells, false);
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
    a.download = `${type}_${level}_${cells}.v`;
    a.click();
    URL.revokeObjectURL(a.href);
    statusEl.textContent = `${a.download} exportiert (${fmtCount(text.split('\n').length)} Zeilen)`;
  } catch (e) {
    statusEl.textContent = `Export fehlgeschlagen: ${e.message ?? e}`;
  }
});

// ---------------------------------------------------------------- chip 3D

// The 3D view (three.js) is only loaded when "Chip 3D" is chosen. It shows
// the layout extracted for the top-level circuit (layouts/<NAME>.bin, copied
// there by viz/build.sh) or a layout.bin dropped onto the view.
let chipModule = null;
let chipView = null;
let chipLoaded = null; // design name of the loaded layout
let chipStatus = '';

function leaveChip() {
  $('main').classList.remove('chip-mode');
  $('canvas').style.display = '';
  $('chip3d').hidden = true;
  $('chip-section').hidden = true;
}

function chipMessage(html) {
  $('chip-message').innerHTML = html;
  $('chip-message').hidden = !html;
}

async function renderChip() {
  $('main').classList.add('chip-mode');
  $('canvas').style.display = 'none';
  $('chip3d').hidden = false;
  $('chip-section').hidden = false;
  renderCrumbs();
  const name = stack[0].type;
  if (!chipModule) {
    statusEl.textContent = '3D-Ansicht wird geladen…';
    try {
      chipModule = await import('./chip3d.js');
    } catch (e) {
      chipMessage(`three.js konnte nicht geladen werden: ${e.message}`);
      return;
    }
  }
  if (!chipView) {
    try {
      chipView = new chipModule.ChipView($('chip3d'), { onHover: chipHover, onPick: chipPick });
    } catch (e) {
      chipMessage(`Die 3D-Ansicht braucht WebGL, das in diesem Browser nicht verfügbar ist.<br>
        Öffne den Viewer in einem normalen Browser, oder nutze <code>cargo run -- chip render</code>.`);
      statusEl.textContent = 'WebGL nicht verfügbar';
      return;
    }
    chipView.setBackground(getComputedStyle(document.body).backgroundColor);
  }
  // ?own=3: lay out the circuit with our own router instead of loading a file.
  const ownParam = new URLSearchParams(location.search).get('own');
  if (ownParam && !renderChip.ownDone) {
    renderChip.ownDone = true;
    $('own-layers').value = ownParam;
    $('own-route').click();
    return;
  }
  // With viz-serve's API: list the design's runs and load the chosen one
  // (layouts can also be built from here). Without it: layouts/<NAME>.bin.
  const api = await chipApiAvailable();
  $('chip-build').hidden = !api;
  if (api) {
    if (chipRunsFor !== name) await refreshChipRuns(name);
    resumeChipJob();
    const run = $('chip-run').value;
    if (!run) {
      showNoLayout(name, true);
      return;
    }
    if (chipLoaded !== name || chipLoadedRun !== run) {
      await loadChipRun(name, run);
      return;
    }
    hookChipProbes();
    return;
  }
  if (chipLoaded !== name) {
    let res = null;
    try { res = await fetch(`layouts/${name}.bin`); } catch {}
    if (!res || !res.ok) {
      showNoLayout(name, false);
      return;
    }
    loadChipBuffer(await res.arrayBuffer());
  }
  hookChipProbes();
}

function showNoLayout(name, api) {
  chipLoaded = null;
  chipLoadedRun = null;
  chipView.clear();
  chipView.invalidate();
  $('chip-layers').replaceChildren();
  $('chip-metrics').replaceChildren();
  chipMessage(api
    ? `Für <b>${name}</b> gibt es noch kein Layout.<br>
       Links unter <b>Chip bauen</b> mit OpenROAD erzeugen,<br>
       oder den <b>eigenen Router</b> nutzen.`
    : `Für <b>${name}</b> gibt es noch kein Layout.<br>
       <code>cargo run -- chip build ${name}</code><br>
       <code>cargo run -- chip extract ${name}</code><br>
       <code>viz/build.sh</code><br>
       oder eine <code>layout.bin</code> hierher ziehen.`);
  statusEl.textContent = `${name} · kein Layout`;
  probes = [];
  probesChanged();
}

// ---------------------------------------------------------------- chip builds (viz-serve API)

let chipApi = null;        // null = not probed yet
let chipRunsFor = null;    // design whose runs are in #chip-run
let chipLoadedRun = null;
let chipJobTimer = null;

async function chipApiAvailable() {
  if (chipApi === null) {
    try { chipApi = (await fetch('api/health')).ok; } catch { chipApi = false; }
  }
  return chipApi;
}

async function refreshChipRuns(name, select) {
  const sel = $('chip-run');
  let runs = [];
  try { runs = (await (await fetch(`api/chip/runs?name=${encodeURIComponent(name)}`)).json()).runs || []; } catch {}
  chipRunsFor = name;
  const keep = select ?? sel.value;
  sel.replaceChildren();
  for (const r of runs) {
    const o = document.createElement('option');
    o.value = r.run;
    const m = r.metrics;
    o.textContent = m?.fmax_mhz ? `${r.run} · ${m.fmax_mhz.toFixed(0)} MHz` : r.run;
    sel.appendChild(o);
  }
  const names = runs.map((r) => r.run);
  sel.value = names.includes(keep) ? keep : names.includes('basic-met5') ? 'basic-met5' : names[0] ?? '';
}

async function loadChipRun(name, run) {
  statusEl.textContent = `Lade Layout ${name} / ${run}…`;
  const res = await fetch(`api/chip/layout?name=${encodeURIComponent(name)}&run=${encodeURIComponent(run)}`);
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    statusEl.textContent = `Layout ${run}: ${err.error ?? res.status}`;
    return;
  }
  loadChipBuffer(await res.arrayBuffer());
  chipLoadedRun = run;
  hookChipProbes();
}

$('chip-run').addEventListener('change', () => loadChipRun(stack[0].type, $('chip-run').value));

$('chip-build-start').addEventListener('click', async () => {
  const name = stack[0].type;
  const params = new URLSearchParams({
    name,
    max_layer: $('chip-build-layer').value,
    level: $('chip-build-level').value,
    util: $('chip-build-util').value,
    optimize: $('chip-build-opt').checked ? '1' : '0',
    keep_latches: $('chip-build-latches').checked ? '1' : '0',
  });
  const res = await fetch(`api/chip/build?${params}`, { method: 'POST', headers: { 'X-LogicCircuit': '1' } });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) {
    $('chip-job').textContent = body.error ?? `Fehler ${res.status}`;
    return;
  }
  pollChipJob();
});

// Picks up a build that is still running (e.g. after a page reload).
async function resumeChipJob() {
  if (chipJobTimer) return;
  try {
    const job = await (await fetch('api/chip/job')).json();
    if (job.state === 'running') pollChipJob();
  } catch {}
}

function pollChipJob() {
  clearInterval(chipJobTimer);
  $('chip-build-start').disabled = true;
  const tick = async () => {
    let job;
    try { job = await (await fetch('api/chip/job')).json(); } catch { return; }
    const box = $('chip-job');
    if (job.state === 'running') {
      box.textContent = `Baue ${job.name} / ${job.run} … ${job.elapsed_s.toFixed(0)} s\n${job.step}`;
      return;
    }
    clearInterval(chipJobTimer);
    chipJobTimer = null;
    $('chip-build-start').disabled = false;
    if (job.state === 'done') {
      box.textContent = `Fertig: ${job.run} in ${job.elapsed_s.toFixed(0)} s`;
      if (job.name === stack[0].type && viewMode === 'chip') {
        await refreshChipRuns(job.name, job.run);
        await loadChipRun(job.name, job.run);
      }
    } else if (job.state === 'error') {
      box.textContent = `Fehlgeschlagen: ${job.error}`;
    }
  };
  chipJobTimer = setInterval(tick, 1000);
  tick();
}

function loadChipBuffer(buffer) {
  chipLoadedRun = null;
  statusEl.textContent = 'Layout wird aufgebaut…';
  const t0 = performance.now();
  const header = chipView.load(buffer);
  chipView.setShowPower($('chip-power').checked);
  chipView.setZScale(Number($('chip-zscale').value));
  chipView.setExplode(Number($('chip-explode').value));
  chipView.fit();
  { const z = Number(new URLSearchParams(location.search).get('zoom')); if (z > 0) chipView.zoomBy(z); }
  chipLoaded = header.design;
  chipView.keyed = null;
  chipMessage('');
  buildChipPanel(header);
  const shapes = header.sections.reduce((a, s) => a + (s.stride ? s.length / s.stride : 0), 0);
  chipStatus = `${header.design} · ${fmtCount(header.instance_names.length)} Zellen · ${fmtCount(shapes)} Formen · Aufbau ${(performance.now() - t0).toFixed(0)} ms`;
  statusEl.textContent = chipStatus;
}

// Live values: one probe over all nets that map to a simulator wire. Only
// when the layout belongs to the simulated top-level circuit.
function hookChipProbes() {
  probes = [];
  if (chipLoaded && chipLoaded === stack[0].type) {
    const keyed = chipView.keyedNets();
    chipView.keyed = keyed;
    probes.push({ keys: keyed.map(([, k]) => k), apply(v) { chipView.updateValues(v); } });
  } else if (chipLoaded) {
    chipStatus += ' · keine Live-Werte (andere Schaltung)';
    statusEl.textContent = chipStatus;
  }
  probesChanged();
  refresh();
}

function chipHover(hit) {
  if (!hit) { statusEl.textContent = chipStatus; return; }
  const val = (v) => (v === 1 ? '1' : v === 0 ? '0' : '–');
  statusEl.textContent = hit.kind === 'net'
    ? `Netz ${hit.info.name}${hit.info.key ? ` → ${hit.info.key}` : ''} · ${hit.layer} · Wert ${val(hit.value)}`
    : `${hit.master.replace('sky130_fd_sc_hd__', '')} · ${hit.path ?? hit.name} · ${hit.layer}`;
}

function chipPick(hit) {
  if (hit?.kind === 'net' && chipView.highlight === hit.net) {
    statusEl.textContent = `Hervorgehoben: Netz ${hit.info.name}${hit.info.key ? ` → ${hit.info.key}` : ''} (Esc: zurück)`;
  }
}

function buildChipPanel(header) {
  const m = header.metrics;
  const box = $('chip-metrics');
  box.replaceChildren();
  if (m) {
    const rows = [
      ['Lauf', m.run],
      ['Die', `${m.die_um[0].toFixed(0)} × ${m.die_um[1].toFixed(0)} µm`],
      ['Zellen', fmtCount(m.logic_cells)],
      ['Transistoren', fmtCount(m.transistors)],
      ['Leitung', `${(m.wire_total_um / 1000).toLocaleString('de-DE', { maximumFractionDigits: 1 })} mm`],
      ['Vias', fmtCount(m.vias ?? 0)],
      ['fmax', m.fmax_mhz ? `${m.fmax_mhz.toFixed(0)} MHz` : '–'],
    ];
    for (const [k, v] of rows) {
      const row = document.createElement('div');
      row.className = 'kv';
      row.innerHTML = `<span></span><span></span>`;
      row.children[0].textContent = k;
      row.children[1].textContent = v;
      box.appendChild(row);
    }
  }
  const list = $('chip-layers');
  list.replaceChildren();
  for (const l of chipView.layers) {
    if (!l.meshes.length) continue;
    const label = document.createElement('label');
    label.className = 'layer-row';
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = l.visible;
    cb.addEventListener('change', () => chipView.setLayerVisible(l.name, cb.checked));
    const sw = document.createElement('span');
    sw.className = 'swatch';
    sw.style.background = l.color;
    const name = document.createElement('span');
    name.textContent = l.name;
    const kind = document.createElement('span');
    kind.className = 'layer-kind';
    kind.textContent = { well: 'Wanne', device: 'Bauelement', contact: 'Kontakt', routing: 'Leitung', cut: 'Via' }[l.kind] ?? l.kind;
    label.append(cb, sw, name, kind);
    list.appendChild(label);
  }
}

for (const [id, fn] of [['chip-explode', (v) => chipView?.setExplode(v)], ['chip-zscale', (v) => chipView?.setZScale(v)], ['chip-cut', (v) => chipView?.setCut(v)]]) {
  $(id).addEventListener('input', (e) => fn(Number(e.target.value)));
}

$('chip-power').addEventListener('change', (e) => chipView?.setShowPower(e.target.checked));

// Our own placer/router, run in WASM on the simulated top-level circuit.
$('own-route').addEventListener('click', async () => {
  if (!chipView) return;
  const name = stack[0].type;
  const layers = Number($('own-layers').value);
  statusEl.textContent = `Platziere und verdrahte ${name} mit ${layers} Lagen (eigener Router)…`;
  await new Promise((r) => setTimeout(r, 30)); // let the status paint
  const t0 = performance.now();
  try {
    const bytes = sim.route_layout(name, layers, 0.7, 1.0);
    const ms = performance.now() - t0;
    loadChipBuffer(bytes.slice().buffer);
    hookChipProbes();
    chipStatus += ` · eigener Router: ${(ms / 1000).toFixed(1)} s`;
    statusEl.textContent = chipStatus;
  } catch (e) {
    statusEl.textContent = `Eigener Router: ${e.message ?? e}`;
  }
});

// Initial chip settings from the URL (?explode=6&zscale=10&power=1), e.g. from `chip view`.
{
  const q = new URLSearchParams(location.search);
  for (const [param, id] of [['explode', 'chip-explode'], ['zscale', 'chip-zscale'], ['cut', 'chip-cut']]) {
    if (q.has(param)) $(id).value = q.get(param);
  }
  if (q.has('power')) $('chip-power').checked = q.get('power') === '1';
}

// Drop a layout.bin onto the view.
$('main').addEventListener('dragover', (e) => { if (viewMode === 'chip') e.preventDefault(); });
$('main').addEventListener('drop', async (e) => {
  if (viewMode !== 'chip') return;
  e.preventDefault();
  const file = e.dataTransfer.files[0];
  if (!file) return;
  try {
    loadChipBuffer(await file.arrayBuffer());
  } catch (err) {
    chipMessage(`${file.name}: ${err.message}`);
    return;
  }
  if (chipLoaded !== stack[0].type && names.includes(chipLoaded)) {
    select(chipLoaded); // simulate the dropped design for live values
    go();
  } else {
    hookChipProbes();
  }
});

$('viewmode').value = viewMode;
$('viewmode').addEventListener('change', (e) => {
  viewMode = e.target.value;
  try { localStorage.setItem(MODE_KEY, viewMode); } catch {}
  render();
});

$('stripes').checked = stripes;
$('stripes').addEventListener('change', (e) => {
  stripes = e.target.checked;
  try { localStorage.setItem(STRIPES_KEY, stripes ? '1' : '0'); } catch {}
  render();
});

$('reset').addEventListener('click', () => {
  stopRun();
  sim.build(stack[0].type);
  resetStats();
  refresh();
  updateProgramView();
});

function buildSidebar() {
  const v = view(stack[0].type);
  sidebarProbes = [];
  probesChanged();
  const mk = (container, pins, settable) => {
    container.replaceChildren();
    if (!pins.length) container.innerHTML = '<div class="io-row" style="color:var(--muted)">—</div>';
    for (const p of pins) {
      const row = document.createElement('div');
      row.className = 'io-row';
      const name = document.createElement('span');
      name.textContent = pinLabel(p);
      const bits = document.createElement('div');
      bits.className = 'bits';
      const val = document.createElement('span');
      val.className = 'val';
      const keys = pinKeys(p.name, p.width);
      const cells = keys.map((k, i) => {
        // Nibble gap: applied on the MSB side of every 4th bit after reversing.
        const b = document.createElement('div');
        b.className = `bit${settable ? '' : ' ro'}`;
        b.title = k;
        if (settable) b.addEventListener('click', () => toggleInput(k));
        if (p.width != null && i % 4 === 3 && i !== p.width - 1) b.classList.add('nib');
        return b;
      }).reverse(); // MSB on the left
      bits.append(...cells);
      if (settable && p.width != null) {
        val.style.cursor = 'pointer';
        val.title = 'Klicken, um einen Zahlenwert einzugeben (dezimal, 0x… oder 0b…)';
        val.addEventListener('click', () => editBusValue(val, p));
      }
      const head = document.createElement('div');
      head.className = 'io-head';
      head.append(name, val);
      row.append(head, bits);
      container.appendChild(row);
      sidebarProbes.push({
        keys,
        apply(vals) {
          cells.slice().reverse().forEach((c, i) => {
            c.classList.toggle('on', vals[i] === 1);
            c.textContent = vals[i] === 2 ? '?' : vals[i];
          });
          val.textContent = summarize(vals).text;
        },
      });
    }
  };
  const runnable = sim.is_runnable();
  // On a runnable circuit the instruction memory drives Instruction.
  mk($('inputs'), v.inputs.filter((p) => !(runnable && p.name === 'Instruction')), true);
  mk($('outputs'), v.outputs, false);
  $('pulse').hidden = !clockName();
  $('pulse').textContent = runnable ? 'Takt (1 Befehl)' : 'Takt ↑↓';
  $('program-panel').hidden = !runnable;
  $('prog-split').hidden = !runnable;
  buildDisplay(v);
  buildPeripherals(v);
}

// ---------------------------------------------------------------- display

// Top-level outputs named Seg<n>[7] are shown as 7-segment digits, highest
// n on the left.
function buildDisplay(v) {
  const digits = v.outputs
    .filter((p) => p.width === 7 && /^Seg\d+$/.test(p.name))
    .sort((a, b) => Number(b.name.slice(3)) - Number(a.name.slice(3)));
  $('display-section').hidden = !digits.length;
  const svg = $('display');
  svg.replaceChildren();
  const gap = 14;
  svg.setAttribute('viewBox', `-8 -8 ${digits.length * (DIGIT_W + gap) - gap + 16} ${DIGIT_H + 16}`);
  digits.forEach((p, i) => {
    const segs = drawDigit(svg, i * (DIGIT_W + gap), 0);
    sidebarProbes.push({ keys: pinKeys(p.name, 7), apply(vals) { showSegments(segs, vals); } });
  });
}

// GPIO pins (SOC_16B): GpioOut as LEDs, GpioIn as switches, bit 15 on the
// left as on a board. The UART console shows what arrived on Tx.
function buildPeripherals(v) {
  const out = v.outputs.find((p) => p.name === 'GpioOut');
  const inp = v.inputs.find((p) => p.name === 'GpioIn');
  $('gpio-section').hidden = !out && !inp;
  $('gpio-out-label').hidden = $('gpio-out').hidden = !out;
  $('gpio-in-label').hidden = $('gpio-in').hidden = !inp;
  const row = (id, pin, make) => {
    const el = $(id);
    el.replaceChildren();
    if (!pin) return;
    const cells = [];
    for (let i = pin.width - 1; i >= 0; i--) {
      const c = make(i);
      c.title = `${pin.name}[${i}]`;
      el.appendChild(c);
      cells[i] = c;
    }
    sidebarProbes.push({ keys: pinKeys(pin.name, pin.width), apply(vals) { cells.forEach((c, i) => c.classList.toggle('on', vals[i] === 1)); } });
  };
  row('gpio-out', out, () => Object.assign(document.createElement('div'), { className: 'led' }));
  row('gpio-in', inp, (i) => {
    const b = Object.assign(document.createElement('button'), { className: 'gpio-sw', textContent: i.toString(16).toUpperCase() });
    b.addEventListener('click', () => { toggleInput(`GpioIn.${i}`); sim.fetch(); refresh(); });
    return b;
  });
  $('uart-section').hidden = !sim.has_uart();
  updateUart();
}

function updateUart() {
  if ($('uart-section').hidden) return;
  const el = $('uart');
  const text = sim.uart_text();
  if (el.textContent !== text) {
    el.textContent = text;
    el.scrollTop = el.scrollHeight;
  }
}

// ---------------------------------------------------------------- program

let examples = [];    // example programs for the instruction set of the built CPU
let program = null;   // { words, lines, listing, isa } of the loaded program
let runFrameId = null;  // requestAnimationFrame id while running

// Clock rate in instructions per second, remembered per browser.
const HZ_KEY = 'lc-viz-hz';
const MAX_HZ = 100000;
try { const v = localStorage.getItem(HZ_KEY); if (v) $('speed').value = v; } catch {}
function clockHz() {
  const hz = Number($('speed').value);
  return Number.isFinite(hz) && hz > 0 ? Math.min(hz, MAX_HZ) : 1;
}

// Fills the example list for the built CPU (8- or 16-bit instruction set).
function loadExamples() {
  examples = JSON.parse(sim.examples());
  $('examples').replaceChildren();
  for (const ex of examples) {
    const o = document.createElement('option');
    o.value = ex.name;
    o.textContent = ex.name;
    $('examples').appendChild(o);
  }
}

// Address digits in the listing and status: 2 for CPU_8B, 4 for CPU_16B.
const pcDigits = () => Math.max(2, Math.ceil(sim.pc_width() / 4));

$('examples').addEventListener('change', (e) => {
  const ex = examples.find((x) => x.name === e.target.value);
  if (ex) { $('source').value = ex.source; loadProgram(); }
});

function loadProgram() {
  stopRun();
  const r = JSON.parse(sim.assemble($('source').value));
  const errEl = $('asm-error');
  if (!r.ok) {
    errEl.textContent = `Zeile ${r.line}: ${r.message}`;
    errEl.hidden = false;
    return;
  }
  errEl.hidden = true;
  program = { ...r, isa: sim.isa() };
  sim.build(stack[0].type);          // fresh registers, PC = 0
  resetStats();
  sim.set_rom(Uint16Array.from(r.words));
  renderListing();
  refresh();
  updateProgramView();
}

function renderListing() {
  const src = $('source').value.split('\n');
  const list = $('listing');
  list.replaceChildren();
  program.words.forEach((w, addr) => {
    const row = document.createElement('div');
    row.className = 'lrow';
    row.title = src[program.lines[addr] - 1]?.trim() ?? '';
    const hex = (n, d) => n.toString(16).toUpperCase().padStart(d, '0');
    row.innerHTML = `<span class="addr">${hex(addr, pcDigits())}</span><span class="word">${hex(w, 4)}</span><span></span>`;
    row.lastChild.textContent = program.listing[addr];
    list.appendChild(row);
  });
}

function updateProgramView() {
  if (!sim.is_runnable()) return;
  updateUart();
  const pc = sim.pc();
  const halted = sim.halted();
  const rows = $('listing').children;
  for (let i = 0; i < rows.length; i++) rows[i].classList.toggle('cur', i === pc);
  rows[pc]?.scrollIntoView({ block: 'nearest' });
  const pcHex = pc.toString(16).toUpperCase().padStart(pcDigits(), '0');
  $('prog-status').textContent = halted
    ? `Angehalten (HALT) bei PC 0x${pcHex}`
    : `PC 0x${pcHex} · nächster Befehl: ${program ? (program.listing[pc] ?? 'HALT') : '—'}`;
  $('step').disabled = halted;
  $('run').disabled = halted;
}

function stepProgram() {
  const ran = sim.step();
  if (ran) { stats.cycles++; showStats(); }
  refresh();
  updateProgramView();
  return ran;
}

// The run loop is frame-based: each animation frame executes as many
// instructions as the clock rate asks for since the last frame, then redraws
// once. Simulation time per frame is capped, so a rate above what the
// simulator manages just runs as fast as it can instead of freezing the page.
const FRAME_BUDGET_MS = 12;

// Execution statistics since the program was loaded or the CPU rebuilt:
// wall-clock time spent running (pauses excluded), executed instructions, and
// the instructions executed while running (manual steps take no run time, so
// they are left out of the average rate).
const stats = { ms: 0, cycles: 0, runCycles: 0, since: null };  // since = start of current run

function resetStats() {
  stats.ms = 0;
  stats.cycles = 0;
  stats.runCycles = 0;
  stats.since = runFrameId ? performance.now() : null;
  showStats();
}

function showStats(now = performance.now()) {
  const ms = stats.ms + (stats.since != null ? now - stats.since : 0);
  const time = ms >= 1000 ? `${(ms / 1000).toFixed(2)} s` : `${Math.round(ms)} ms`;
  const rate = ms > 0 && stats.runCycles > 0 ? ` · ≈ ${formatHz((stats.runCycles * 1000) / ms)}` : '';
  $('timer').textContent = `⏱ ${time} · ${stats.cycles} ${stats.cycles === 1 ? 'Befehl' : 'Befehle'}${rate}`;
}
let run = null;  // { last, carry }

function runFrame(now) {
  const hz = clockHz();
  run.carry += ((now - run.last) * hz) / 1000;
  run.last = now;
  let due = Math.floor(run.carry);
  run.carry -= due;
  const deadline = performance.now() + FRAME_BUDGET_MS;
  let halted = false;
  while (due-- > 0) {
    if (!sim.step()) { halted = true; break; }
    stats.cycles++;
    stats.runCycles++;
    if (sim.halted()) { halted = true; break; }
    if (performance.now() > deadline) { run.carry = 0; break; }
  }
  refresh();
  updateProgramView();
  // Stop the clock at the moment HALT was reached, not at the end of the frame.
  if (halted) stopRun(performance.now());
  else { showStats(now); runFrameId = requestAnimationFrame(runFrame); }
}

function formatHz(hz) {
  return hz >= 1000 ? `${(hz / 1000).toFixed(1)} kHz` : `${hz.toFixed(hz < 10 ? 1 : 0)} Hz`;
}

function startRun() {
  if (runFrameId || sim.halted()) return;
  const now = performance.now();
  // Start with one instruction due so the first step happens immediately.
  run = { last: now, carry: 1 };
  runFrameId = requestAnimationFrame(runFrame);
  stats.since = now;
  $('run').textContent = '⏸ Stopp';
}

function stopRun(at = performance.now()) {
  if (stats.since != null) {
    stats.ms += at - stats.since;
    stats.since = null;
    showStats();
  }
  if (runFrameId) cancelAnimationFrame(runFrameId);
  runFrameId = null;
  $('run').textContent = '▶ Start';
}

$('load').addEventListener('click', loadProgram);
$('step').addEventListener('click', () => { stopRun(); stepProgram(); });
$('run').addEventListener('click', () => (runFrameId ? stopRun() : startRun()));
// The run loop reads the rate every frame, so changes apply immediately.
$('speed').addEventListener('change', () => {
  $('speed').value = clockHz();
  try { localStorage.setItem(HZ_KEY, $('speed').value); } catch {}
});

// ---------------------------------------------------------------- navigation

function renderCrumbs() {
  const c = $('crumbs');
  c.replaceChildren();
  stack.forEach((s, i) => {
    if (i > 0) c.append(' › ');
    const a = document.createElement('a');
    a.textContent = i === 0 ? s.type : s.instance;
    a.addEventListener('click', () => { stack = stack.slice(0, i + 1); go(); });
    c.append(a);
    if (i > 0) {
      const t = document.createElement('span');
      t.className = 'type';
      t.textContent = ` (${s.type})`;
      c.append(t);
    }
  });
}

function enter(instance, type) {
  stack.push({ instance, type });
  go();
}

function go() {
  location.hash = stack.map((s, i) => (i === 0 ? s.type : s.instance)).join('/');
  render();
}

function select(type) {
  stack = [{ instance: null, type }];
  statusEl.textContent = `Baue ${type}…`;
  stopRun();
  sim.build(type);
  resetStats();
  buildSidebar();
  if (sim.is_runnable()) {
    // The examples depend on the CPU (instruction set, peripherals).
    const selected = $('examples').value;
    loadExamples();
    if (examples.some((x) => x.name === selected)) $('examples').value = selected;
    // A program for the other instruction set does not fit: start over
    // with this CPU's first example.
    if (program?.isa !== sim.isa()) {
      program = null;
      $('listing').replaceChildren();
    }
    if (!program && examples.length) {
      const ex = examples.find((x) => x.name === 'countdown' || x.name === 'fibonacci') ?? examples[0];
      $('examples').value = ex.name;
      $('source').value = ex.source;
      loadProgram();
    } else if (program) {
      renderListing();
      updateProgramView();
    }
  }
  for (const d of $('list').children) d.classList.toggle('sel', d.dataset.name === type);
}

function fromHash() {
  const parts = decodeURIComponent(location.hash.slice(1)).split('/').filter(Boolean);
  const top = names.includes(parts[0]) ? parts[0] : (names.includes('COMPUTER_8B') ? 'COMPUTER_8B' : names[0]);
  if (!stack.length || stack[0].type !== top) select(top);
  stack = [stack[0]];
  for (const inst of parts.slice(1)) {
    const part = view(stack[stack.length - 1].type).parts.find((p) => p.name === inst);
    if (!part || !part.expandable) break;
    stack.push({ instance: inst, type: part.type });
  }
  render();
}

// Circuit list
function renderList() {
  const q = $('filter').value.trim().toLowerCase();
  const list = $('list');
  list.replaceChildren();
  for (const n of names.filter((n) => n.toLowerCase().includes(q))) {
    const d = document.createElement('div');
    d.dataset.name = n;
    d.textContent = n;
    if (n in nandCounts) {
      const c = document.createElement('span');
      c.className = 'count';
      c.textContent = fmtCount(nandCounts[n]);
      c.title = `${nandLabel(nandCounts[n])}`;
      d.appendChild(c);
    }
    if (stack[0] && stack[0].type === n) d.className = 'sel';
    d.addEventListener('click', () => { select(n); go(); });
    list.appendChild(d);
  }
}
$('filter').addEventListener('input', renderList);

// ---------------------------------------------------------------- pan & zoom

const svg = $('canvas');
let tf = { x: 0, y: 0, k: 1 };
const applyTf = () => $('viewport').setAttribute('transform', `translate(${tf.x},${tf.y}) scale(${tf.k})`);

function fit() {
  const graph = currentGraph;
  if (!graph) return;
  const r = svg.getBoundingClientRect();
  const k = Math.min(r.width / graph.width, r.height / graph.height, 1.6);
  tf = { k, x: (r.width - graph.width * k) / 2, y: (r.height - graph.height * k) / 2 };
  applyTf();
}
$('fit').addEventListener('click', () => (viewMode === 'chip' ? chipView?.fit() : fit()));
window.addEventListener('keydown', (e) => {
  if (e.target.tagName === 'INPUT') return;
  if (e.key === 'f' || e.key === 'F') (viewMode === 'chip' ? chipView?.fit() : fit());
  if (e.key === 'Escape' && viewMode === 'chip') chipView?.setHighlight(null);
  if (e.key === 'Backspace' && stack.length > 1) { stack.pop(); go(); }
});

svg.addEventListener('wheel', (e) => {
  e.preventDefault();
  const r = svg.getBoundingClientRect();
  const mx = e.clientX - r.left, my = e.clientY - r.top;
  const f = Math.exp(-e.deltaY * 0.0015);
  const k = Math.min(8, Math.max(0.05, tf.k * f));
  tf.x = mx - (mx - tf.x) * (k / tf.k);
  tf.y = my - (my - tf.y) * (k / tf.k);
  tf.k = k;
  applyTf();
}, { passive: false });

let drag = null;
svg.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  drag = { x: e.clientX, y: e.clientY, tx: tf.x, ty: tf.y };
  svg.classList.add('drag');
});
window.addEventListener('pointermove', (e) => {
  if (!drag) return;
  tf.x = drag.tx + e.clientX - drag.x;
  tf.y = drag.ty + e.clientY - drag.y;
  applyTf();
});
window.addEventListener('pointerup', () => { drag = null; svg.classList.remove('drag'); });
window.addEventListener('resize', fit);
window.addEventListener('hashchange', () => {
  const want = stack.map((s, i) => (i === 0 ? s.type : s.instance)).join('/');
  if (decodeURIComponent(location.hash.slice(1)) !== want) fromHash();
});

// ---------------------------------------------------------------- resizable panels

// Drag handles for the sidebar width and the program panel height. Sizes
// live in CSS variables on :root and are remembered per browser.
const SIZES_KEY = 'lc-viz-sizes';
const sizes = (() => { try { return JSON.parse(localStorage.getItem(SIZES_KEY)) || {}; } catch { return {}; } })();
const applySizes = () => {
  if (sizes.side) document.documentElement.style.setProperty('--side-w', `${sizes.side}px`);
  if (sizes.prog) document.documentElement.style.setProperty('--prog-h', `${sizes.prog}px`);
};
applySizes();

function makeSplitter(handle, axis, measure) {
  handle.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    handle.classList.add('active');
    document.body.classList.add('resizing', axis === 'x' ? 'col' : 'row');
    const move = (ev) => { measure(ev); applySizes(); };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.classList.remove('active');
      document.body.classList.remove('resizing', 'col', 'row');
      try { localStorage.setItem(SIZES_KEY, JSON.stringify(sizes)); } catch {}
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
  });
}

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
makeSplitter($('side-split'), 'x', (e) => {
  sizes.side = Math.round(clamp(e.clientX, 200, Math.min(640, window.innerWidth - 300)));
});
makeSplitter($('prog-split'), 'y', (e) => {
  const bottom = $('workspace').getBoundingClientRect().bottom;
  sizes.prog = Math.round(clamp(bottom - e.clientY, 120, window.innerHeight * 0.75));
});

// ---------------------------------------------------------------- start

fromHash();
renderList();
