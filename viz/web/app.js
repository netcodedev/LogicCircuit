import init, { Sim } from './pkg/logic_circuit_viz.js';

const SVGNS = 'http://www.w3.org/2000/svg';
const HEADER = 34;   // part title area
const ROW = 16;      // vertical spacing between ports
const PORT = 6;      // port square size
const TAP_W = 8;     // splitter / merger bar width

const $ = (id) => document.getElementById(id);
const statusEl = $('status');

await init();
const sim = new Sim();
const elk = new ELK();
const names = JSON.parse(sim.circuit_names());
const views = new Map();    // type -> CircuitView JSON
const layouts = new Map();  // "type|mode" -> { graph, meta }

// Bus rendering mode: false = one line coloured by summary, true = one
// stripe per bit. Remembered per browser; storage may be unavailable.
const STRIPES_KEY = 'lc-viz-stripes';
let stripes = false;
try { stripes = localStorage.getItem(STRIPES_KEY) === '1'; } catch {}

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
    const w = Math.max(90, textWidth(part.name, 12) + 24, textWidth(part.type, 10) + 24, maxIn + maxOut + 30);
    const rows = Math.max(part.inputs.length, part.outputs.length, 1);
    const h = HEADER + rows * row + 4;
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
    meta.nodes.set(id, { kind: 'part', part });
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
  const { graph, meta, ms } = await layout(top.type);
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
      el('text', { x: n.width / 2, y: 27, 'text-anchor': 'middle', class: 'type' }, g).textContent = part.type;
      el('title', {}, g).textContent = part.expandable ? `${part.name}: ${part.type} — Doppelklick zum Öffnen` : `${part.name}: ${part.type}`;
      if (part.expandable) g.addEventListener('dblclick', (ev) => { ev.stopPropagation(); enter(part.name, part.type); });
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
  refresh();
  const nodes = graph.children.length, edges = graph.edges.length;
  statusEl.textContent = `${top.type} · ${nodes} Knoten · ${edges} Verbindungen · Layout ${ms.toFixed(0)} ms`;
}

// ---------------------------------------------------------------- simulation

let allKeys = '';
let probeOffsets = [];

function refresh() {
  const all = [...probes, ...sidebarProbes];
  probeOffsets = [];
  const keys = [];
  for (const p of all) { probeOffsets.push(keys.length); keys.push(...p.keys); }
  allKeys = keys.join('\n');
  const vals = sim.values(allKeys);
  all.forEach((p, i) => p.apply(Array.from(vals.subarray(probeOffsets[i], probeOffsets[i] + p.keys.length))));
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

function clockName() {
  return view(stack[0].type).inputs.find((p) => p.width == null && /^clk$/i.test(p.name))?.name;
}

$('pulse').addEventListener('click', () => {
  const clk = clockName();
  sim.set_input(clk, true); sim.eval();
  sim.set_input(clk, false); sim.eval();
  refresh();
});

$('stripes').checked = stripes;
$('stripes').addEventListener('change', (e) => {
  stripes = e.target.checked;
  try { localStorage.setItem(STRIPES_KEY, stripes ? '1' : '0'); } catch {}
  render();
});

$('reset').addEventListener('click', () => {
  sim.build(stack[0].type);
  refresh();
});

function buildSidebar() {
  const v = view(stack[0].type);
  sidebarProbes = [];
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
        val.title = 'Klicken, um einen Zahlenwert einzugeben';
        val.addEventListener('click', () => {
          const s = prompt(`Wert für ${p.name} (dezimal oder 0x…):`);
          if (s != null && s.trim() !== '') setBus(p.name, Number(s.trim()));
        });
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
  mk($('inputs'), v.inputs, true);
  mk($('outputs'), v.outputs, false);
  $('pulse').hidden = !clockName();
}

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
  sim.build(type);
  buildSidebar();
  for (const d of $('list').children) d.classList.toggle('sel', d.textContent === type);
}

function fromHash() {
  const parts = decodeURIComponent(location.hash.slice(1)).split('/').filter(Boolean);
  const top = names.includes(parts[0]) ? parts[0] : (names.includes('CPU_8B') ? 'CPU_8B' : names[0]);
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
    d.textContent = n;
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
  const entry = stack.length && layouts.get(`${stack[stack.length - 1].type}|${stripes}`);
  if (!entry) return;
  const { graph } = entry;
  const r = svg.getBoundingClientRect();
  const k = Math.min(r.width / graph.width, r.height / graph.height, 1.6);
  tf = { k, x: (r.width - graph.width * k) / 2, y: (r.height - graph.height * k) / 2 };
  applyTf();
}
$('fit').addEventListener('click', fit);
window.addEventListener('keydown', (e) => {
  if (e.target.tagName === 'INPUT') return;
  if (e.key === 'f' || e.key === 'F') fit();
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

// ---------------------------------------------------------------- start

fromHash();
renderList();
