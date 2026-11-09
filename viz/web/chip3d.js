// 3D view of an extracted chip layout (layout.bin from `chip extract`).
//
// Geometry is built once per cell master and layer and drawn with
// InstancedMesh for every placement; routing is one instanced unit box per
// rectangle and layer, with a per-instance colour so nets can light up with
// live simulation values. World units are microns, z points up.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';

const MAGIC = 'LCLAYOUT';
const HI = new THREE.Color('#3ddc84');
const MARK = new THREE.Color('#ffd84a');
const GHOST = new THREE.Color('#2a2f37');

/** Reads layout.bin: header JSON and Int32 sections. */
export function parseLayout(buffer) {
  const bytes = new Uint8Array(buffer);
  if (new TextDecoder().decode(bytes.subarray(0, 8)) !== MAGIC) throw new Error('Keine LogicCircuit-Layoutdatei');
  const headerLen = new DataView(buffer).getUint32(12, true);
  const header = JSON.parse(new TextDecoder().decode(bytes.subarray(16, 16 + headerLen)));
  const ints = new Int32Array(buffer, 16 + headerLen);
  const section = (name) => {
    const s = header.sections.find((x) => x.name === name);
    return s ? ints.subarray(s.offset, s.offset + s.length) : new Int32Array(0);
  };
  return { header, section };
}

// DEF orientations (header "orientations" order) as x' = a x + b y + tx,
// y' = c x + d y + ty for a cell of size (w, h).
function orientCoeffs(o, w, h) {
  switch (o) {
    case 'S': return [-1, 0, 0, -1, w, h];
    case 'E': return [0, 1, -1, 0, 0, w];
    case 'W': return [0, -1, 1, 0, h, 0];
    case 'FN': return [-1, 0, 0, 1, w, 0];
    case 'FS': return [1, 0, 0, -1, 0, h];
    case 'FE': return [0, -1, -1, 0, h, w];
    case 'FW': return [0, 1, 1, 0, 0, 0];
    default: return [1, 0, 0, 1, 0, 0];
  }
}

export class ChipView {
  constructor(container, { onHover, onPick } = {}) {
    this.container = container;
    this.onHover = onHover;
    this.onPick = onPick;
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.localClippingEnabled = true;
    container.appendChild(this.renderer.domElement);
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100000);
    this.camera.up.set(0, 0, 1);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.addEventListener('change', () => this.invalidate());
    this.scene.add(new THREE.AmbientLight(0xffffff, 0.65));
    const sun = new THREE.DirectionalLight(0xffffff, 1.6);
    sun.position.set(-0.6, -1, 2.2);
    this.scene.add(sun);
    this.clip = new THREE.Plane(new THREE.Vector3(0, 0, -1), 1e9);

    this.explode = 0;      // extra microns between consecutive layers
    this.zscale = 5;       // vertical exaggeration
    this.showPower = false; // power grid (special nets VDD/VSS)
    this.highlight = null; // net index
    this.values = null;    // per net: 0, 1, 2 (unknown)
    this.dirty = true;

    this.raycaster = new THREE.Raycaster();
    this.lastHover = 0;
    const el = this.renderer.domElement;
    el.addEventListener('pointermove', (e) => this.hover(e));
    el.addEventListener('pointerdown', (e) => { this.downAt = [e.clientX, e.clientY]; });
    el.addEventListener('pointerup', (e) => {
      if (this.downAt && Math.hypot(e.clientX - this.downAt[0], e.clientY - this.downAt[1]) < 4) this.click(e);
    });
    new ResizeObserver(() => this.resize()).observe(container);
    this.resize();
    const loop = () => {
      this.frame = requestAnimationFrame(loop);
      if (this.controls.update() || this.dirty) {
        this.dirty = false;
        this.renderer.render(this.scene, this.camera);
      }
    };
    loop();
  }

  invalidate() { this.dirty = true; }

  resize() {
    const r = this.container.getBoundingClientRect();
    if (!r.width || !r.height) return;
    this.renderer.setSize(r.width, r.height);
    this.camera.aspect = r.width / r.height;
    this.camera.updateProjectionMatrix();
    this.invalidate();
  }

  setBackground(css) {
    this.scene.background = new THREE.Color(css);
    this.invalidate();
  }

  // ------------------------------------------------------------- loading

  load(buffer) {
    this.clear();
    const { header, section } = parseLayout(buffer);
    this.header = header;
    const dbu = header.dbu_per_micron;
    this.layers = header.layers.map((l, i) => ({
      ...l, index: i, visible: l.kind !== 'well', meshes: [],
      material: new THREE.MeshLambertMaterial({ color: l.color, side: THREE.DoubleSide, clippingPlanes: [this.clip] }),
      // Routing shares its layer's height with the cells' own shapes on that
      // layer (e.g. li1 pads and met1 over cell met1). A depth offset lets the
      // routing win instead of both flickering (z-fighting).
      routeMaterial: new THREE.MeshLambertMaterial({
        color: l.color, side: THREE.DoubleSide, clippingPlanes: [this.clip],
        polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -4,
      }),
    }));

    // --- Cell masters: merged geometry per (master, layer), z from 0 to 1.
    const perMaster = header.masters.map(() => new Map()); // layer -> geometries
    const add = (m, layer, geom) => {
      const map = perMaster[m];
      if (!map.has(layer)) map.set(layer, []);
      map.get(layer).push(geom);
    };
    const rects = section('master_rects');
    for (let i = 0; i < rects.length; i += 6) {
      const [m, layer, x0, y0, x1, y1] = rects.subarray(i, i + 6);
      const w = (x1 - x0) / dbu, h = (y1 - y0) / dbu;
      if (w <= 0 || h <= 0) continue;
      const g = new THREE.BoxGeometry(w, h, 1).toNonIndexed();
      g.translate((x0 + x1) / 2 / dbu, (y0 + y1) / 2 / dbu, 0.5);
      add(m, layer, g);
    }
    const polys = section('master_polys');
    for (let i = 0; i < polys.length;) {
      const m = polys[i], layer = polys[i + 1], n = polys[i + 2];
      const pts = [];
      for (let k = 0; k < n; k++) pts.push(new THREE.Vector2(polys[i + 3 + 2 * k] / dbu, polys[i + 4 + 2 * k] / dbu));
      i += 3 + 2 * n;
      const g = new THREE.ExtrudeGeometry(new THREE.Shape(pts), { depth: 1, bevelEnabled: false });
      g.deleteAttribute('uv');
      add(m, layer, g.index ? g.toNonIndexed() : g);
    }

    // Placements per master.
    const inst = section('instances');
    this.instances = [];
    const placements = header.masters.map(() => []);
    for (let i = 0; i < inst.length; i += 4) {
      const idx = i / 4;
      this.instances.push({ master: inst[i], x: inst[i + 1] / dbu, y: inst[i + 2] / dbu, orient: header.orientations[inst[i + 3]] });
      placements[inst[i]].push(idx);
    }
    this.cellMeshes = [];
    perMaster.forEach((map, m) => {
      const master = header.masters[m];
      for (const [layer, geoms] of map) {
        geoms.forEach((g) => { if (g.attributes.uv) g.deleteAttribute('uv'); });
        const geom = mergeGeometries(geoms, false);
        geoms.forEach((g) => g.dispose());
        const mesh = new THREE.InstancedMesh(geom, this.layers[layer].material, placements[m].length);
        mesh.userData = { kind: 'cell', layer, master: m, placements: placements[m], w: master.width / dbu, h: master.height / dbu };
        mesh.frustumCulled = false;
        this.scene.add(mesh);
        this.layers[layer].meshes.push(mesh);
        this.cellMeshes.push(mesh);
      }
    });

    // --- Routing: one instanced unit box per layer.
    const routes = section('routes');
    const byLayer = this.layers.map(() => []);
    for (let i = 0; i < routes.length; i += 6) byLayer[routes[i]].push(i);
    this.netRoutes = new Map(); // net -> [[mesh, instance]]
    const unit = new THREE.BoxGeometry(1, 1, 1);
    unit.translate(0, 0, 0.5);
    this.routeMeshes = [];
    byLayer.forEach((offsets, layer) => {
      if (!offsets.length) return;
      const mesh = new THREE.InstancedMesh(unit, this.layers[layer].routeMaterial, offsets.length);
      const boxes = new Float32Array(offsets.length * 4);
      const nets = new Int32Array(offsets.length);
      offsets.forEach((o, k) => {
        boxes.set([routes[o + 2] / dbu, routes[o + 3] / dbu, routes[o + 4] / dbu, routes[o + 5] / dbu], k * 4);
        nets[k] = routes[o + 1];
        if (!this.netRoutes.has(nets[k])) this.netRoutes.set(nets[k], []);
        this.netRoutes.get(nets[k]).push([mesh, k]);
      });
      const special = Uint8Array.from(nets, (n) => (header.nets[n].special ? 1 : 0));
      mesh.userData = { kind: 'route', layer, boxes, nets, special };
      mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(offsets.length * 3), 3);
      mesh.frustumCulled = false;
      this.scene.add(mesh);
      this.layers[layer].meshes.push(mesh);
      this.routeMeshes.push(mesh);
    });

    this.applyLayout();
    this.recolor();
    this.layers.forEach((l) => this.setLayerVisible(l.name, l.visible));
    this.fit();
    return header;
  }

  clear() {
    for (const m of [...(this.cellMeshes || []), ...(this.routeMeshes || [])]) {
      this.scene.remove(m);
      if (m.userData.kind === 'cell') m.geometry.dispose();
      m.dispose();
    }
    (this.layers || []).forEach((l) => { l.material.dispose(); l.routeMaterial.dispose(); });
    this.cellMeshes = [];
    this.routeMeshes = [];
    this.highlight = null;
    this.values = null;
  }

  // ------------------------------------------------------------- geometry

  // Bottom z and height of a layer in world units.
  zOf(layer) {
    const l = this.layers[layer];
    let z = l.z * this.zscale + this.explode * layer;
    let t = Math.max(l.thickness * this.zscale, 0.02);
    // Contacts and vias end exactly where the metal above and below starts;
    // pull them in slightly so their faces do not coincide with the metal's.
    if (l.kind === 'cut' || l.kind === 'contact') {
      const inset = t * 0.03;
      z += inset;
      t -= 2 * inset;
    }
    return [z, t];
  }

  topZ() {
    const last = this.layers.length - 1;
    const [z, h] = this.zOf(last);
    return z + h;
  }

  applyLayout() {
    const m4 = new THREE.Matrix4();
    for (const mesh of this.cellMeshes) {
      const { layer, placements, w, h } = mesh.userData;
      const [z, t] = this.zOf(layer);
      placements.forEach((idx, k) => {
        const p = this.instances[idx];
        const [a, b, c, d, tx, ty] = orientCoeffs(p.orient, w, h);
        m4.set(a, b, 0, p.x + tx, c, d, 0, p.y + ty, 0, 0, t, z, 0, 0, 0, 1);
        mesh.setMatrixAt(k, m4);
      });
      mesh.instanceMatrix.needsUpdate = true;
    }
    for (const mesh of this.routeMeshes) {
      const { layer, boxes, special } = mesh.userData;
      const [z, t0] = this.zOf(layer);
      // 2 % taller than the cells' own shapes on this layer: their top faces
      // must not lie in the same plane (z-fighting), the depth offset of
      // routeMaterial covers the sides.
      const t = t0 * 1.02;
      for (let k = 0; k < boxes.length / 4; k++) {
        const x0 = boxes[k * 4], y0 = boxes[k * 4 + 1], x1 = boxes[k * 4 + 2], y1 = boxes[k * 4 + 3];
        // Hidden power grid: collapse to nothing (keeps instance indices stable).
        const s = special[k] && !this.showPower ? 0 : 1;
        m4.set((x1 - x0) * s, 0, 0, (x0 + x1) / 2, 0, (y1 - y0) * s, 0, (y0 + y1) / 2, 0, 0, t * s, z, 0, 0, 0, 1);
        mesh.setMatrixAt(k, m4);
      }
      mesh.instanceMatrix.needsUpdate = true;
    }
    this.invalidate();
  }

  setShowPower(on) { this.showPower = on; this.applyLayout(); }

  setExplode(v) { this.explode = v; this.applyLayout(); this.setCut(this.cutFraction ?? 1); }
  // (fit() after changing explode/zscale is left to the user: F / Einpassen)
  setZScale(v) { this.zscale = v; this.applyLayout(); this.setCut(this.cutFraction ?? 1); }

  /** Hides everything above a fraction (0..1) of the stack height. */
  setCut(fraction) {
    this.cutFraction = fraction;
    this.clip.constant = fraction >= 1 ? 1e9 : fraction * this.topZ();
    this.invalidate();
  }

  setLayerVisible(name, visible) {
    const l = this.layers?.find((x) => x.name === name);
    if (!l) return;
    l.visible = visible;
    l.meshes.forEach((m) => { m.visible = visible; });
    this.invalidate();
  }

  fit() {
    const [x0, y0, x1, y1] = this.header.die.map((v) => v / this.header.dbu_per_micron);
    const cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
    // Take the (exploded) stack height into account as well.
    const top = this.topZ();
    const size = Math.max(x1 - x0, y1 - y0, top * 1.4);
    this.controls.target.set(cx, cy, top / 2);
    this.camera.position.set(cx - size * 0.15, cy - size * 1.05, top / 2 + size * 0.85);
    this.camera.near = size / 1000;
    this.camera.far = size * 20;
    this.camera.updateProjectionMatrix();
    this.controls.update();
    this.invalidate();
  }

  /** Moves the camera towards its target (factor > 1 = closer). */
  zoomBy(factor) {
    const t = this.controls.target;
    this.camera.position.sub(t).multiplyScalar(1 / factor).add(t);
    this.controls.update();
    this.invalidate();
  }

  // ------------------------------------------------------------- colours

  /** Nets with a simulator key: [[net index, key], ...]. */
  keyedNets() {
    return this.header.nets.map((n, i) => [i, n.key]).filter(([, k]) => k);
  }

  /** Values (0/1/2) for the nets of keyedNets(), in that order. */
  updateValues(vals) {
    const keyed = this.keyed || (this.keyed = this.keyedNets());
    if (!this.values) this.values = new Uint8Array(this.header.nets.length).fill(2);
    let changed = false;
    keyed.forEach(([net], i) => {
      if (this.values[net] !== vals[i]) { this.values[net] = vals[i]; changed = true; }
    });
    if (changed) this.recolor();
  }

  setHighlight(net) {
    this.highlight = net;
    this.recolor();
  }

  recolor() {
    const c = new THREE.Color();
    for (const mesh of this.routeMeshes) {
      const base = this.layers[mesh.userData.layer].material.color;
      const { nets } = mesh.userData;
      for (let k = 0; k < nets.length; k++) {
        const net = nets[k];
        if (this.highlight != null) {
          c.copy(net === this.highlight ? MARK : GHOST);
        } else if (this.values && this.values[net] === 1) {
          c.copy(HI);
        } else {
          c.setRGB(0.62, 0.62, 0.62); // multiplied with the layer colour
          mesh.setColorAt(k, c);
          continue;
        }
        // Instance colour multiplies the material colour; divide it out so
        // highlight colours show as they are.
        c.setRGB(c.r / Math.max(base.r, 0.05), c.g / Math.max(base.g, 0.05), c.b / Math.max(base.b, 0.05));
        mesh.setColorAt(k, c);
      }
      mesh.instanceColor.needsUpdate = true;
    }
    this.invalidate();
  }

  // ------------------------------------------------------------- picking

  pick(e) {
    const r = this.renderer.domElement.getBoundingClientRect();
    const p = new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    this.raycaster.setFromCamera(p, this.camera);
    const targets = [...this.routeMeshes, ...this.cellMeshes].filter((m) => m.visible);
    const hits = this.raycaster.intersectObjects(targets, false).filter((h) => this.clip.distanceToPoint(h.point) >= 0);
    const h = hits[0];
    if (!h) return null;
    const ud = h.object.userData;
    if (ud.kind === 'route') {
      const net = ud.nets[h.instanceId];
      return { kind: 'net', net, info: this.header.nets[net], layer: this.layers[ud.layer].name, value: this.values?.[net] };
    }
    const idx = ud.placements[h.instanceId];
    return {
      kind: 'cell',
      instance: idx,
      name: this.header.instance_names[idx],
      path: this.header.instance_paths[idx],
      master: this.header.masters[ud.master].name,
      layer: this.layers[ud.layer].name,
    };
  }

  hover(e) {
    const now = performance.now();
    if (!this.header || now - this.lastHover < 80 || e.buttons) return;
    this.lastHover = now;
    this.onHover?.(this.pick(e));
  }

  click(e) {
    if (!this.header) return;
    const hit = this.pick(e);
    this.setHighlight(hit?.kind === 'net' && hit.net !== this.highlight ? hit.net : null);
    this.onPick?.(hit);
  }

  dispose() {
    cancelAnimationFrame(this.frame);
    this.clear();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}
