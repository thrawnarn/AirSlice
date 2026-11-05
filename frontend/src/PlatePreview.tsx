// 3D preview of the build plate: models at real size, in their filament
// colours, positioned as they will print. Loaded lazily (three.js is big).

import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { api } from "./api";
import { creasedGeometry } from "./meshNormals";
import type { MeshTransform } from "./types";
import { tr } from "./i18n";

export interface PreviewData {
  kind: "layout" | "model";
  bed: { shape: [number, number][]; width: number; depth: number; height: number };
  objects: {
    name: string;
    /** 3MF object id; empty for bare meshes. */
    object_id: string;
    filament: number;
    instance: number;
    color: string | null;
    /** u16 xyz triples; position = origin + q * step. */
    positions: string;
    origin: [number, number, number];
    step: [number, number, number];
    /** Triangle indices, `index_bits` wide. */
    indices: string;
    index_bits: 16 | 32;
    triangles: number;
  }[];
  bounds: [number, number, number, number, number, number];
  triangles: number;
}

type Props = {
  jobId: string;
  plate?: number;
  printerId?: string;
  source?: "input" | "sliced";
  /** Colour for a 1-based filament slot; return null to use the file's own. */
  colorFor?: (filament: number, fileColor: string | null) => string | null;
  /** Bare meshes: size/copies/rotation to apply before display. */
  transform?: MeshTransform;
  /** Shown under the size readout. */
  note?: string;
  /** Object ids left out of the print, not drawn. */
  hidden?: string[];
};

const FALLBACK = ["#d9dde0", "#3a6ea5", "#c0392b", "#2f7d5b", "#d9a441", "#7d4fa6", "#4d575e"];
const GAP = 6; // mm between copies, roughly what the arranger leaves

function bytesOf(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes.buffer;
}

/** Packed u16 positions back to millimetres. */
function decodePositions(o: PreviewData["objects"][number]): Float32Array {
  const q = new Uint16Array(bytesOf(o.positions));
  const out = new Float32Array(q.length);
  for (let i = 0; i < q.length; i++) out[i] = o.origin[i % 3] + q[i] * o.step[i % 3];
  return out;
}

/** PEI-gold texture with a 10 mm grid, matching the app's plate motif. */
function plateTexture(width: number, depth: number): THREE.CanvasTexture {
  const px = 4; // pixels per mm, capped below
  const scale = Math.min(px, 2048 / Math.max(width, depth));
  const c = document.createElement("canvas");
  c.width = Math.round(width * scale);
  c.height = Math.round(depth * scale);
  const g = c.getContext("2d")!;
  g.fillStyle = "#b8873b";
  g.fillRect(0, 0, c.width, c.height);
  // speckle of the textured PEI
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  g.fillStyle = "rgba(255, 226, 170, 0.35)";
  for (let i = 0; i < (c.width * c.height) / 60; i++) g.fillRect(rnd() * c.width, rnd() * c.height, 1.2, 1.2);
  g.strokeStyle = "rgba(60, 40, 10, 0.18)";
  g.lineWidth = 1;
  for (let mm = 10; mm < width; mm += 10) {
    g.beginPath();
    g.moveTo(mm * scale, 0);
    g.lineTo(mm * scale, c.height);
    g.stroke();
  }
  for (let mm = 10; mm < depth; mm += 10) {
    g.beginPath();
    g.moveTo(0, mm * scale);
    g.lineTo(c.width, mm * scale);
    g.stroke();
  }
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = 4;
  return t;
}

export default function PlatePreview({ jobId, plate, printerId, source, colorFor, transform, note, hidden }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const [data, setData] = useState<PreviewData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const three = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    controls: OrbitControls;
    models: THREE.Group;
    render: () => void;
    fit: (top?: boolean) => void;
  } | null>(null);

  // ---------------------------------------------------------------- fetch
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    const q = new URLSearchParams();
    if (plate) q.set("plate", String(plate));
    if (printerId) q.set("printer_id", printerId);
    if (source) q.set("source", source);
    api
      .get<PreviewData>(`jobs/${jobId}/preview?${q}`)
      .then((d) => !cancelled && setData(d))
      .catch((e) => !cancelled && setError((e as Error).message))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [jobId, plate, printerId, source]);

  // Geometry is decoded once per response.
  const geometries = useMemo(() => {
    if (!data) return [];
    return data.objects.map((o) => {
      const buf = bytesOf(o.indices);
      const g = creasedGeometry(decodePositions(o), o.index_bits === 16 ? new Uint16Array(buf) : new Uint32Array(buf));
      g.computeBoundingBox();
      return g;
    });
  }, [data]);

  // ---------------------------------------------------------------- scene setup (per bed)
  useEffect(() => {
    if (!data || !host.current) return;
    const el = host.current;
    const bed = data.bed;
    // Without WebGL (old phones, locked-down browsers) three.js throws; show
    // a message instead of taking the whole page down.
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    } catch {
      setError(tr("this browser can't show 3D (WebGL is off or unsupported)"));
      return;
    }
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setSize(el.clientWidth, el.clientHeight);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    el.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.add(new THREE.HemisphereLight(0xffffff, 0x6b5a3a, 1.6));
    const sun = new THREE.DirectionalLight(0xffffff, 1.6);
    sun.position.set(bed.width * 0.2, -bed.depth * 0.6, bed.height * 1.6);
    scene.add(sun);
    const fill = new THREE.DirectionalLight(0xffffff, 0.5);
    fill.position.set(-bed.width, bed.depth * 1.5, bed.height);
    scene.add(fill);

    // Build plate
    const shape = new THREE.Shape(bed.shape.map(([x, y]) => new THREE.Vector2(x, y)));
    const plateGeo = new THREE.ShapeGeometry(shape);
    // UVs across the bed rectangle
    const uv = plateGeo.attributes.uv as THREE.BufferAttribute;
    const pos = plateGeo.attributes.position as THREE.BufferAttribute;
    for (let i = 0; i < uv.count; i++) uv.setXY(i, pos.getX(i) / bed.width, pos.getY(i) / bed.depth);
    const plateMat = new THREE.MeshStandardMaterial({ map: plateTexture(bed.width, bed.depth), roughness: 0.9, metalness: 0.05 });
    const plateMesh = new THREE.Mesh(plateGeo, plateMat);
    plateMesh.position.z = -0.2;
    scene.add(plateMesh);
    const frame = new THREE.Mesh(
      new THREE.BoxGeometry(bed.width + 12, bed.depth + 12, 4),
      new THREE.MeshStandardMaterial({ color: 0x23292e, roughness: 0.7 }),
    );
    frame.position.set(bed.width / 2, bed.depth / 2, -2.3);
    scene.add(frame);
    // Faint build volume outline
    const vol = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.BoxGeometry(bed.width, bed.depth, bed.height)),
      new THREE.LineBasicMaterial({ color: 0x8a949b, transparent: true, opacity: 0.28 }),
    );
    vol.position.set(bed.width / 2, bed.depth / 2, bed.height / 2);
    scene.add(vol);

    const models = new THREE.Group();
    scene.add(models);

    const camera = new THREE.PerspectiveCamera(35, el.clientWidth / el.clientHeight, 1, 10000);
    camera.up.set(0, 0, 1);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = false;
    controls.maxPolarAngle = Math.PI * 0.49;
    controls.screenSpacePanning = true;

    const render = () => renderer.render(scene, camera);
    const fit = (top = false) => {
      const box = new THREE.Box3().setFromObject(models);
      const bedCenter = new THREE.Vector3(bed.width / 2, bed.depth / 2, 0);
      const span = Math.max(bed.width, bed.depth);
      // Frame the whole plate, nudged towards the models; small models get
      // a closer view so they don't disappear on a big plate.
      let target = bedCenter.clone();
      let dist = span * 1.3;
      if (!box.isEmpty()) {
        const c = box.getCenter(new THREE.Vector3());
        const size = box.getSize(new THREE.Vector3());
        const extent = Math.max(size.x, size.y, size.z * 1.4);
        target = c.clone().setZ(size.z * 0.35).lerp(bedCenter, extent < span * 0.35 ? 0.15 : 0.5);
        dist = Math.min(span * 1.3, Math.max(extent * 2.6, span * 0.55));
      }
      controls.target.copy(target);
      if (top) camera.position.set(target.x, target.y - 0.01, dist * 1.25);
      else camera.position.set(target.x + dist * 0.12, target.y - dist * 0.82, dist * 0.6);
      camera.lookAt(target);
      controls.update();
      render();
    };
    controls.addEventListener("change", render);

    const ro = new ResizeObserver(() => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (!w || !h) return;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      render();
    });
    ro.observe(el);

    three.current = { renderer, scene, camera, controls, models, render, fit };
    return () => {
      ro.disconnect();
      controls.dispose();
      scene.traverse((o) => {
        const m = o as THREE.Mesh;
        if (m.isMesh) {
          (Array.isArray(m.material) ? m.material : [m.material]).forEach((mat) => {
            (mat as THREE.MeshStandardMaterial).map?.dispose();
            mat.dispose();
          });
          if (m.geometry && !geometries.includes(m.geometry)) m.geometry.dispose();
        }
      });
      renderer.dispose();
      renderer.domElement.remove();
      three.current = null;
    };
    // geometries are tied to data
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  useEffect(() => () => geometries.forEach((g) => g.dispose()), [geometries]);

  // ---------------------------------------------------------------- models (colours, copies, transforms)
  const layout = useMemo(() => {
    if (!data) return null;
    if (data.kind === "layout") {
      const [x0, y0, z0, x1, y1, z1] = data.bounds;
      return { copies: [] as [number, number][], size: [x1 - x0, y1 - y0, z1 - z0], extent: [x1 - x0, y1 - y0], fits: true, s: 1, rot: 0 };
    }
    const s = (transform?.scale_percent ?? 100) / 100;
    const rot = (((transform?.rotate_z ?? 0) % 360) + 360) % 360;
    const n = Math.max(1, transform?.copies ?? 1);
    const [x0, y0, z0, x1, y1, z1] = data.bounds;
    const w0 = (x1 - x0) * s;
    const d0 = (y1 - y0) * s;
    const h = (z1 - z0) * s;
    const swap = rot === 90 || rot === 270;
    const [w, d] = swap ? [d0, w0] : [w0, d0];
    const cols = Math.ceil(Math.sqrt(n));
    const rows = Math.ceil(n / cols);
    const ext: [number, number] = [cols * w + (cols - 1) * GAP, rows * d + (rows - 1) * GAP];
    const cx = data.bed.width / 2;
    const cy = data.bed.depth / 2;
    const copies: [number, number][] = [];
    for (let i = 0; i < n; i++) {
      const r = Math.floor(i / cols);
      const c = i % cols;
      const inRow = r === rows - 1 ? n - r * cols : cols;
      const rowW = inRow * w + (inRow - 1) * GAP;
      copies.push([cx - rowW / 2 + w / 2 + c * (w + GAP), cy + ext[1] / 2 - d / 2 - r * (d + GAP)]);
    }
    const fits = ext[0] <= data.bed.width && ext[1] <= data.bed.depth && h <= data.bed.height;
    return { copies, size: [w, d, h], extent: ext, fits, s, rot };
  }, [data, transform?.scale_percent, transform?.rotate_z, transform?.copies]);

  useEffect(() => {
    const t = three.current;
    if (!t || !data || !layout) return;
    t.models.clear();
    const material = (o: PreviewData["objects"][number], i: number) =>
      new THREE.MeshStandardMaterial({
        color: new THREE.Color(colorFor?.(o.filament, o.color) ?? o.color ?? FALLBACK[(o.filament - 1 + i) % FALLBACK.length]),
        roughness: 0.55,
        metalness: 0.02,
      });
    if (data.kind === "layout") {
      data.objects.forEach((o, i) => {
        if (!hidden?.includes(o.object_id)) t.models.add(new THREE.Mesh(geometries[i], material(o, i)));
      });
    } else {
      const mats = data.objects.map((o, i) => material(o, i));
      for (const [x, y] of layout.copies) {
        const g = new THREE.Group();
        data.objects.forEach((_, i) => g.add(new THREE.Mesh(geometries[i], mats[i])));
        g.scale.setScalar(layout.s);
        g.rotation.z = THREE.MathUtils.degToRad(layout.rot);
        g.position.set(x, y, 0);
        t.models.add(g);
      }
    }
    t.fit();
  }, [data, layout, geometries, colorFor, hidden?.join()]);

  // ---------------------------------------------------------------- UI
  const size = layout?.size;
  const count = data ? (data.kind === "model" ? (layout?.copies.length ?? 1) : new Set(data.objects.map((o) => o.instance)).size) : 0;

  return (
    <figure className="preview">
      <div className="preview-canvas" ref={host} aria-label={tr("3D preview of the build plate. Drag to rotate, pinch to zoom.")}>
        {loading && <div className="preview-msg">{tr("Loading preview…")}</div>}
        {error && !loading && <div className="preview-msg">{tr("Preview not available: {0}", [error])}</div>}
        {data && !error && (
          <div className="preview-tools">
            <button type="button" className="btn small" onClick={() => three.current?.fit(true)}>
              {tr("Top")}
            </button>
            <button type="button" className="btn small" onClick={() => three.current?.fit(false)}>
              {tr("Reset")}
            </button>
          </div>
        )}
      </div>
      {data && size && (
        <figcaption className="preview-caption">
          <span className="num">
            {data.kind === "layout" && count > 1 ? tr("{0} objects, together ", [count]) : ""}
            {Math.round(size[0])} × {Math.round(size[1])} × {Math.round(size[2])} mm
            {data.kind === "model" && count > 1 ? tr(" each, {0} copies", [count]) : ""}
          </span>
          <span className="faint num">
            {tr(" on a {0} × {1} mm plate", [Math.round(data.bed.width), Math.round(data.bed.depth)])}
          </span>
          {layout && !layout.fits && <span className="preview-warn">{tr("Too big for the plate at this size.")}</span>}
          {note && <span className="faint preview-note">{note}</span>}
        </figcaption>
      )}
    </figure>
  );
}
