// Floorplan map of a chip layout: which parts of the design sit where.
//
// Groups come from the hierarchy paths of the cells (`Cpu.ICache.Line_3.Not`
// -> Cpu -> ICache -> Line): starting with the top-level parts, the largest
// group is split into its children until there are about `target` groups;
// children too small to show become part of "<group> · Rest". Then the die is
// cut into square bins, each bin gets the group with the most cell area
// around it (a small blur, so rows and gaps do not fray the regions), and
// every larger connected region gets a label.
//
// Pure functions, no DOM except drawMap (a canvas).

// Children smaller than this share of the design are merged into "· Rest";
// it shrinks with the number of groups asked for.
const minShareFor = (target) => 1 / (target * 2.5);
const BINS = 150;          // bins along the longer die side
const BLUR = 3;            // box blur radius in bins
const LABEL_MIN_BINS = 40; // smaller regions get no label

/** Path segment without its array index ("Line_14" -> "Line"). */
function baseName(seg) {
  return seg.replace(/_\d+$/, '');
}

/**
 * Groups and map of a loaded layout.
 * `header`: layout header (instance_paths, masters, die, dbu_per_micron);
 * `instances`: [{master, x, y}] in microns (ChipView.instances).
 */
export function computeRegions(header, instances, { target = 14 } = {}) {
  const dbu = header.dbu_per_micron;
  const paths = header.instance_paths;
  // Hierarchy tree with cell area per node; leaves (the gates) are dropped.
  const root = { key: '', name: 'SoC', area: 0, cells: 0, own: 0, ownCells: 0, children: new Map(), depth: 0 };
  const leafOf = new Array(instances.length).fill(null);
  instances.forEach((inst, i) => {
    const p = paths[i];
    if (!p) return;
    const m = header.masters[inst.master];
    const area = (m.width / dbu) * (m.height / dbu);
    // Tie and buffer cells ("(tie-low) Cpu.Alu.X") belong to their part.
    const raw = p.replace(/^\([^)]*\)\s*/, '').split('.').slice(0, -1);
    let node = root;
    node.area += area;
    node.cells++;
    for (const r of raw) {
      const s = baseName(r);
      if (!node.children.has(s)) {
        node.children.set(s, {
          key: node.key ? `${node.key}.${s}` : s, name: s, area: 0, cells: 0, own: 0, ownCells: 0, children: new Map(), depth: node.depth + 1,
          // An array of parts (Line_0..Line_15): shown as one group, never
          // split into the insides of its elements.
          array: s !== r,
        });
      }
      node = node.children.get(s);
      node.area += area;
      node.cells++;
    }
    node.own += area;
    node.ownCells++;
    leafOf[i] = node;
  });
  if (!root.cells) return null;

  // Split the largest groups. A group is {node, rest}: rest = the node's own
  // gates plus children too small to be groups of their own. A rest that is
  // itself too small goes to the node's largest child (see restOf below).
  const total = root.area;
  const minShare = minShareFor(target);
  const big = (node) => [...node.children.values()].filter((c) => c.area / total >= minShare);
  const expand = (node) => {
    const out = big(node).map((c) => ({ node: c, rest: false }));
    const restArea = node.area - out.reduce((a, g) => a + g.node.area, 0);
    const restCells = node.cells - out.reduce((a, g) => a + g.node.cells, 0);
    if (restCells && (restArea / total >= minShare || !out.length)) out.push({ node, rest: true, area: restArea, cells: restCells });
    return out;
  };
  const areaOf = (g) => (g.rest ? g.area : g.node.area);
  let groups = expand(root);
  for (;;) {
    if (groups.length >= target) break;
    const splittable = groups
      .filter((g) => !g.rest && !g.node.array && big(g.node).length >= 2)
      .sort((a, b) => areaOf(b) - areaOf(a));
    if (!splittable.length) break;
    const g = splittable[0];
    groups = groups.filter((x) => x !== g).concat(expand(g.node));
  }
  groups.sort((a, b) => areaOf(b) - areaOf(a));

  // Instance -> group: walk up from the instance's node to a group node.
  const groupOfNode = new Map();
  groups.forEach((g, gi) => {
    if (!g.rest) groupOfNode.set(g.node, gi);
  });
  const restOf = new Map();
  groups.forEach((g, gi) => {
    if (g.rest) restOf.set(g.node, gi);
  });
  const parentOf = new Map();
  const walk = (n) => n.children.forEach((c) => { parentOf.set(c, n); walk(c); });
  walk(root);
  // Split nodes without a rest group of their own: their small parts join
  // the group of their largest child.
  const splitNodes = new Set();
  for (const g of groups) {
    for (let n = parentOf.get(g.node) ?? (g.rest ? null : root); n; n = parentOf.get(n)) splitNodes.add(n);
    if (g.rest) splitNodes.add(g.node);
  }
  for (const n of splitNodes) {
    if (restOf.has(n) || groupOfNode.has(n)) continue;
    let c = n;
    while (c && !groupOfNode.has(c)) {
      c = [...c.children.values()].sort((a, b) => b.area - a.area)[0];
    }
    if (c) restOf.set(n, groupOfNode.get(c));
  }
  // Group nodes are disjoint subtrees: a node inside one belongs to it.
  // Otherwise it is part of a small subtree (or the own gates) of a split
  // node, i.e. the rest of the nearest split ancestor.
  const memo = new Map();
  const resolve = (node) => {
    if (memo.has(node)) return memo.get(node);
    let gi = -1;
    for (let n = node; n; n = parentOf.get(n)) {
      if (groupOfNode.has(n)) { gi = groupOfNode.get(n); break; }
    }
    if (gi < 0) {
      for (let n = node; n; n = parentOf.get(n)) {
        if (restOf.has(n)) { gi = restOf.get(n); break; }
      }
    }
    memo.set(node, gi);
    return gi;
  };
  const groupOf = new Int16Array(instances.length).fill(-1);
  leafOf.forEach((node, i) => { if (node) groupOf[i] = resolve(node); });

  const out = groups.map((g, gi) => {
    const key = g.rest ? (g.node.key || '') : g.node.key;
    // An array is named with its parent ("ICache.Line"): "Line" alone says little.
    const parent = parentOf.get(g.node);
    const own = g.node.array && parent && parent !== root ? `${parent.name}.${g.node.name}` : g.node.name;
    const name = g.rest ? (g.node === root ? 'Verbindungslogik' : `${g.node.name} · Rest`) : own;
    return {
      key, name,
      path: g.rest ? (g.node === root ? 'SoC' : `${g.node.key.replaceAll('.', ' › ')} › Rest`) : g.node.key.replaceAll('.', ' › '),
      area: 0, cells: 0,
      color: colorFor(gi, groups.length),
    };
  });

  // Bins.
  const [dx0, dy0, dx1, dy1] = header.die.map((v) => v / dbu);
  const w = dx1 - dx0, h = dy1 - dy0;
  const bin = Math.max(w, h) / BINS;
  const gw = Math.ceil(w / bin), gh = Math.ceil(h / bin);
  const G = out.length;
  const votes = new Float32Array(G * gw * gh);
  instances.forEach((inst, i) => {
    const gi = groupOf[i];
    if (gi < 0) return;
    const m = header.masters[inst.master];
    const cw = m.width / dbu, ch = m.height / dbu;
    out[gi].area += cw * ch;
    out[gi].cells++;
    const bx = Math.min(gw - 1, Math.max(0, Math.floor((inst.x + cw / 2 - dx0) / bin)));
    const by = Math.min(gh - 1, Math.max(0, Math.floor((inst.y + ch / 2 - dy0) / bin)));
    votes[gi * gw * gh + by * gw + bx] += cw * ch;
  });
  // Separable box blur per group.
  const tmp = new Float32Array(gw * gh);
  for (let g = 0; g < G; g++) {
    const v = votes.subarray(g * gw * gh, (g + 1) * gw * gh);
    for (let y = 0; y < gh; y++) {
      let s = 0;
      for (let x = -BLUR; x < gw + BLUR; x++) {
        if (x + BLUR < gw) s += v[y * gw + x + BLUR];
        if (x - BLUR - 1 >= 0) s -= v[y * gw + x - BLUR - 1];
        if (x >= 0 && x < gw) tmp[y * gw + x] = s;
      }
    }
    for (let x = 0; x < gw; x++) {
      let s = 0;
      for (let y = -BLUR; y < gh + BLUR; y++) {
        if (y + BLUR < gh) s += tmp[(y + BLUR) * gw + x];
        if (y - BLUR - 1 >= 0) s -= tmp[(y - BLUR - 1) * gw + x];
        if (y >= 0 && y < gh) v[y * gw + x] = s;
      }
    }
  }
  const owner = new Int16Array(gw * gh).fill(-1);
  const density = new Float32Array(gw * gh);
  const window = (2 * BLUR + 1) ** 2 * bin * bin;
  for (let b = 0; b < gw * gh; b++) {
    let best = -1, bestV = 0, sum = 0;
    for (let g = 0; g < G; g++) {
      const x = votes[g * gw * gh + b];
      sum += x;
      if (x > bestV) { bestV = x; best = g; }
    }
    density[b] = Math.min(1, sum / window);
    owner[b] = density[b] > 0.04 ? best : -1;
  }

  // Connected regions (4-neighbourhood) for labels.
  const comp = new Int32Array(gw * gh).fill(-1);
  const labels = [];
  const stack = [];
  let nComp = 0;
  for (let b = 0; b < gw * gh; b++) {
    if (owner[b] < 0 || comp[b] >= 0) continue;
    const g = owner[b];
    let n = 0, sx = 0, sy = 0;
    const members = [];
    stack.push(b);
    comp[b] = nComp;
    while (stack.length) {
      const c = stack.pop();
      members.push(c);
      const x = c % gw, y = (c - x) / gw;
      n++; sx += x; sy += y;
      for (const [nx, ny] of [[x + 1, y], [x - 1, y], [x, y + 1], [x, y - 1]]) {
        if (nx < 0 || ny < 0 || nx >= gw || ny >= gh) continue;
        const k = ny * gw + nx;
        if (comp[k] < 0 && owner[k] === g) { comp[k] = nComp; stack.push(k); }
      }
    }
    if (n >= LABEL_MIN_BINS) {
      // The member nearest to the centroid, so the label sits inside.
      const cx = sx / n, cy = sy / n;
      let best = members[0], bd = Infinity;
      for (const m of members) {
        const x = m % gw, y = (m - x) / gw;
        const d = (x - cx) ** 2 + (y - cy) ** 2;
        if (d < bd) { bd = d; best = m; }
      }
      const x = best % gw, y = (best - x) / gw;
      labels.push({ group: g, bins: n, x: dx0 + (x + 0.5) * bin, y: dy0 + (y + 0.5) * bin });
    }
    nComp++;
  }
  // Per group only its larger regions: the largest, and others at least
  // half as large.
  labels.sort((a, b) => b.bins - a.bins);
  const largest = new Map();
  const shown = labels.filter((l) => {
    if (!largest.has(l.group)) { largest.set(l.group, l.bins); return true; }
    return l.bins >= largest.get(l.group) / 2;
  });

  const totalArea = out.reduce((a, g) => a + g.area, 0);
  out.forEach((g) => { g.share = g.area / totalArea; });
  return { groups: out, groupOf, owner, density, gw, gh, bin, die: [dx0, dy0, dx1, dy1], labels: shown };
}

/** Distinct colours: golden-angle hues, alternating lightness. */
export function colorFor(i, n) {
  const hue = (i * 137.508 + 200) % 360;
  const light = i % 2 ? 62 : 52;
  return `hsl(${hue.toFixed(0)} 70% ${light}%)`;
}

/**
 * Draws the map into `canvas` (sized to the bins, `scale` pixels per bin).
 * `focus`: group index to emphasise (others dimmed), or null.
 */
export function drawMap(canvas, r, { scale = 4, focus = null, labels = true, background = null } = {}) {
  canvas.width = r.gw * scale;
  canvas.height = r.gh * scale;
  const ctx = canvas.getContext('2d');
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (background) {
    ctx.fillStyle = background;
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }
  // Bins (canvas y grows downwards, die y upwards).
  for (let y = 0; y < r.gh; y++) {
    for (let x = 0; x < r.gw; x++) {
      const b = y * r.gw + x;
      const g = r.owner[b];
      if (g < 0) continue;
      const dim = focus != null && g !== focus;
      ctx.globalAlpha = dim ? 0.12 : 0.45 + 0.55 * r.density[b];
      ctx.fillStyle = r.groups[g].color;
      ctx.fillRect(x * scale, (r.gh - 1 - y) * scale, scale, scale);
    }
  }
  ctx.globalAlpha = 1;
  // Borders between regions.
  ctx.strokeStyle = 'rgb(0 0 0 / 55%)';
  ctx.lineWidth = Math.max(1, scale / 3);
  ctx.beginPath();
  for (let y = 0; y < r.gh; y++) {
    for (let x = 0; x < r.gw; x++) {
      const g = r.owner[y * r.gw + x];
      const cy = (r.gh - 1 - y) * scale;
      if (x + 1 < r.gw && r.owner[y * r.gw + x + 1] !== g) {
        ctx.moveTo((x + 1) * scale, cy);
        ctx.lineTo((x + 1) * scale, cy + scale);
      }
      if (y + 1 < r.gh && r.owner[(y + 1) * r.gw + x] !== g) {
        ctx.moveTo(x * scale, cy);
        ctx.lineTo(x * scale + scale, cy);
      }
    }
  }
  ctx.stroke();
  if (!labels) return;
  // Labels, larger for larger regions.
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.lineJoin = 'round';
  for (const l of r.labels) {
    if (focus != null && l.group !== focus) continue;
    const g = r.groups[l.group];
    const px = ((l.x - r.die[0]) / r.bin) * scale;
    const py = (r.gh - (l.y - r.die[1]) / r.bin) * scale;
    const size = Math.round(Math.min(26, Math.max(11, Math.sqrt(l.bins) * scale * 0.18)));
    ctx.font = `600 ${size}px system-ui, sans-serif`;
    ctx.lineWidth = Math.max(3, size / 4);
    ctx.strokeStyle = 'rgb(10 12 16 / 85%)';
    ctx.strokeText(g.name, px, py);
    ctx.fillStyle = '#f3f5f7';
    ctx.fillText(g.name, px, py);
  }
}

/** Group index at a point of the die (microns), or -1. */
export function groupAt(r, x, y) {
  const bx = Math.floor((x - r.die[0]) / r.bin), by = Math.floor((y - r.die[1]) / r.bin);
  if (bx < 0 || by < 0 || bx >= r.gw || by >= r.gh) return -1;
  return r.owner[by * r.gw + bx];
}
