import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import { Notice } from "./ui";
import { tr } from "./i18n";

interface LayerIndex {
  layers: { z: number; height: number }[];
  /** min x, min y, max x, max y in mm. */
  bounds: [number, number, number, number];
  features: string[];
}

/** `[feature, tool, x, y, x, y, …]`, coordinates in 1/100 mm. */
type Path = number[];

// Colours for line types, close to what slicers use.
const FEATURE_COLORS: Record<string, string> = {
  "Outer wall": "#ff8a3d",
  "Inner wall": "#ffd23f",
  "Overhang wall": "#3d7bff",
  "Sparse infill": "#b8323a",
  "Internal solid infill": "#a64dcf",
  "Solid infill": "#a64dcf",
  "Top surface": "#e8404f",
  "Bottom surface": "#7a8cff",
  Bridge: "#4aa3df",
  "Internal Bridge": "#2f7fb5",
  "Gap infill": "#f2f2f2",
  Skirt: "#59c3a6",
  Brim: "#59c3a6",
  Support: "#3fbf4f",
  "Support interface": "#2a8f39",
  "Support transition": "#2a8f39",
  "Prime tower": "#8c96a3",
  Ironing: "#ff6fb5",
  Custom: "#5f6a75",
};

/**
 * The sliced G-code one layer at a time, seen from above. The layer below is
 * drawn faintly underneath so the current one stands out.
 */
export default function LayerView({ jobId, colorFor }: { jobId: string; colorFor: (filament: number, fileColor: string | null) => string | null }) {
  const [index, setIndex] = useState<LayerIndex | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [n, setN] = useState(0);
  const [mode, setMode] = useState<"filament" | "type">("filament");
  const [layers, setLayers] = useState<Map<number, Path[]>>(new Map());
  /** Tap to look closer: canvas point (CSS px) shown 3× in the middle. */
  const [focus, setFocus] = useState<{ x: number; y: number } | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const loading = useRef(new Set<number>());

  useEffect(() => {
    api
      .get<LayerIndex>(`jobs/${jobId}/layers`)
      .then((ix) => {
        setIndex(ix);
        setN(Math.max(0, Math.floor(ix.layers.length / 2)));
      })
      .catch((e) => setError((e as Error).message));
  }, [jobId]);

  // Fetch the shown layer and the one below (kept in a small cache).
  useEffect(() => {
    if (!index) return;
    for (const k of [n, n - 1]) {
      if (k < 0 || layers.has(k) || loading.current.has(k)) continue;
      loading.current.add(k);
      api
        .get<{ z: number; paths: Path[] }>(`jobs/${jobId}/layers/${k}`)
        .then((l) =>
          setLayers((m) => {
            const next = new Map(m);
            next.set(k, l.paths);
            // Keep memory bounded on tall prints.
            for (const key of next.keys()) if (next.size > 40 && Math.abs(key - k) > 20) next.delete(key);
            return next;
          }),
        )
        .catch((e) => setError((e as Error).message))
        .finally(() => loading.current.delete(k));
    }
  }, [index, n, jobId, layers]);

  const current = layers.get(n);
  const present = useMemo(() => {
    const seen = new Set<number>();
    for (const p of current ?? []) seen.add(p[0]);
    return [...seen];
  }, [current]);

  useEffect(() => {
    const c = canvas.current;
    if (!c || !index || !current) return;
    const dpr = window.devicePixelRatio || 1;
    const size = c.clientWidth;
    c.width = size * dpr;
    c.height = size * dpr;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    const [x0, y0, x1, y1] = index.bounds;
    const span = Math.max(x1 - x0, y1 - y0, 1);
    const pad = 0.04 * span;
    const scale = (size * dpr) / (span + 2 * pad);
    // Centre the model; Y points up on the bed.
    const ox = (size * dpr - (x1 - x0) * scale) / 2;
    const oy = (size * dpr - (y1 - y0) * scale) / 2;
    const X = (v: number) => ox + (v / 100 - x0) * scale;
    const Y = (v: number) => size * dpr - (oy + (v / 100 - y0) * scale);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, c.width, c.height);
    if (focus) ctx.setTransform(3, 0, 0, 3, (size / 2 - focus.x * 3) * dpr, (size / 2 - focus.y * 3) * dpr);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    const width = Math.max(1, 0.42 * scale);
    const draw = (paths: Path[], color: (p: Path) => string, alpha: number) => {
      ctx.globalAlpha = alpha;
      ctx.lineWidth = width;
      for (const p of paths) {
        ctx.strokeStyle = color(p);
        ctx.beginPath();
        ctx.moveTo(X(p[2]), Y(p[3]));
        for (let i = 4; i < p.length; i += 2) ctx.lineTo(X(p[i]), Y(p[i + 1]));
        ctx.stroke();
      }
    };
    const below = layers.get(n - 1);
    if (below) draw(below, () => "#8a8f96", 0.18);
    draw(
      current,
      mode === "type" ? (p) => FEATURE_COLORS[index.features[p[0]]] ?? "#c9a54a" : (p) => colorFor(p[1] + 1, null) ?? "#c9a54a",
      1,
    );
    ctx.globalAlpha = 1;
  }, [index, current, layers, n, mode, colorFor, focus]);

  if (error) return <Notice tone="err">{tr("Couldn't show the layers: {0}", [error])}</Notice>;
  if (!index) return <p className="faint">{tr("Reading the sliced file…")}</p>;
  const layer = index.layers[n];
  const last = index.layers.length - 1;
  return (
    <div className="layer-view">
      <canvas
        ref={canvas}
        className={`layer-canvas ${focus ? "zoomed" : ""}`}
        aria-label={tr("Layer {0} of {1}", [n + 1, last + 1])}
        title={focus ? tr("Tap to see the whole plate") : tr("Tap to look closer")}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setFocus(focus ? null : { x: e.clientX - r.left, y: e.clientY - r.top });
        }}
      />
      <div className="row small" style={{ marginTop: 10 }}>
        <span className="grow num">
          {tr("Layer {0} of {1}, {2} mm", [n + 1, last + 1, layer.z.toFixed(2)])}
        </span>
        <div className="segmented" role="group" aria-label={tr("Colour by")}>
          <button type="button" aria-pressed={mode === "filament"} onClick={() => setMode("filament")}>
            {tr("Filament")}
          </button>
          <button type="button" aria-pressed={mode === "type"} onClick={() => setMode("type")}>
            {tr("Line type")}
          </button>
        </div>
      </div>
      <div className="row" style={{ gap: 8, marginTop: 8 }}>
        <button className="btn small" onClick={() => setN(Math.max(0, n - 1))} disabled={n === 0} aria-label={tr("Layer down")}>
          −
        </button>
        <input
          className="grow layer-slider"
          type="range"
          min={0}
          max={last}
          value={n}
          onChange={(e) => setN(Number(e.target.value))}
          aria-label={tr("Layer")}
        />
        <button className="btn small" onClick={() => setN(Math.min(last, n + 1))} disabled={n === last} aria-label={tr("Layer up")}>
          +
        </button>
      </div>
      {mode === "type" && (
        <ul className="layer-legend small">
          {present.map((f) => (
            <li key={f}>
              <span style={{ background: FEATURE_COLORS[index.features[f]] ?? "#c9a54a" }} />
              {tr(index.features[f])}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
