//! Geometry for the 3D plate preview.
//!
//! Turns STL/OBJ files and Bambu/Orca 3MF projects (including sliced
//! output) into triangle meshes in plate coordinates (millimetres), with
//! the filament slot of each part so the app can colour it.

use anyhow::{Context, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use quick_xml::events::{BytesStart, Event};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Read,
    path::Path,
};

/// Previews beyond this get refused rather than freezing a phone.
/// Most triangles read for a preview (per model file and per plate); they
/// are simplified for display, this only bounds time for absurd files.
pub const MAX_TRIANGLES: usize = 60_000_000;
/// Meshes with more points than this are merged into a grid while reading.
const COARSE_VERTICES: usize = 1_500_000;
/// Grid cells along the longest side for that merge (~1 mm on a 256 mm plate
/// would be 256; this keeps far more detail than a phone can show).
const COARSE_CELLS: f32 = 1200.0;
/// Triangles sent for display; more detail is simplified away (a phone over
/// VPN gets a few MB instead of tens).
const PREVIEW_TRIANGLES: usize = 250_000;
/// Small parts keep at least this much shape.
const MIN_OBJECT_TRIANGLES: usize = 2_000;
/// Allowed surface error when simplifying, mm: start here, grow until it fits.
/// The last round allows about 0.8 mm, past that a preview looks wrong.
const FIRST_ERROR: f32 = 0.005;
const ERROR_GROWTH: f32 = 1.5;
const ERROR_ROUNDS: usize = 13;
/// Bambu lays plates out in a grid with a 1/5 plate gap (PartPlate.cpp).
const PLATE_GAP: f32 = 0.2;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Bed {
    /// Printable area outline, mm.
    pub shape: Vec<[f32; 2]>,
    pub width: f32,
    pub depth: f32,
    pub height: f32,
}

impl Bed {
    pub fn rect(width: f32, depth: f32, height: f32) -> Self {
        Bed { shape: vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]], width, depth, height }
    }

    /// From OrcaSlicer settings: `printable_area` = ["0x0","256x0",…].
    pub fn from_settings(area: &[String], height: Option<f32>) -> Option<Self> {
        let shape: Vec<[f32; 2]> = area
            .iter()
            .filter_map(|p| {
                let (x, y) = p.split_once('x')?;
                Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
            })
            .collect();
        if shape.len() < 3 {
            return None;
        }
        let (minx, maxx) = shape.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
        let (miny, maxy) = shape.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
        Some(Bed { width: maxx - minx, depth: maxy - miny, height: height.unwrap_or(250.0), shape })
    }

    /// Fallback when no profile is available.
    pub fn for_model(printer_model: &str) -> Self {
        let m = printer_model.to_lowercase();
        if m.contains("a1 mini") {
            Bed::rect(180.0, 180.0, 180.0)
        } else if m.contains("h2d") || m.contains("h2c") {
            Bed::rect(350.0, 320.0, 325.0)
        } else if m.contains("h2s") {
            Bed::rect(340.0, 320.0, 340.0)
        } else {
            Bed::rect(256.0, 256.0, 256.0)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewObject {
    pub name: String,
    /// The 3MF object this came from (empty for bare meshes).
    pub object_id: String,
    /// 1-based filament slot.
    pub filament: u32,
    /// Which object on the plate this belongs to (objects with several
    /// filaments are split into one entry per filament).
    pub instance: usize,
    /// Colour from the project (`#RRGGBB`), if known.
    pub color: Option<String>,
    /// Little-endian u16 xyz triples, base64; position = origin + q * step.
    pub positions: String,
    pub origin: [f32; 3],
    pub step: [f32; 3],
    /// Little-endian triangle indices, base64, `index_bits` wide (16 or 32);
    /// empty = non-indexed.
    pub indices: String,
    pub index_bits: u8,
    pub triangles: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// `layout`: objects are in plate coordinates.
    /// `model`: a single mesh centred on the origin, resting on z = 0;
    /// the app places copies and applies scale/rotation itself.
    pub kind: &'static str,
    pub bed: Bed,
    pub objects: Vec<PreviewObject>,
    /// [minx, miny, minz, maxx, maxy, maxz] over all objects.
    pub bounds: [f32; 6],
    pub triangles: usize,
}

/// A mesh being collected (plate coordinates).
#[derive(Default, Clone)]
struct Mesh {
    positions: Vec<f32>,
    indices: Vec<u32>,
    /// Painted filament per triangle (0 = the part's own); empty when
    /// nothing is painted.
    paint: Vec<u8>,
}

impl Mesh {
    fn triangles(&self) -> usize {
        if self.indices.is_empty() { self.positions.len() / 9 } else { self.indices.len() / 3 }
    }
}

/// Positions as u16 per axis over the mesh's own box: under 0.01 mm steps
/// for anything that fits on a plate, at half the size of f32.
fn encode_positions(v: &[f32]) -> (String, [f32; 3], [f32; 3]) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in v.as_chunks::<3>().0 {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if v.is_empty() {
        return (String::new(), [0.0; 3], [0.0; 3]);
    }
    let step = [0, 1, 2].map(|k| ((hi[k] - lo[k]) / 65535.0).max(f32::MIN_POSITIVE));
    let mut b = Vec::with_capacity(v.len() * 2);
    for p in v.as_chunks::<3>().0 {
        for k in 0..3 {
            let q = ((p[k] - lo[k]) / step[k]).round().clamp(0.0, 65535.0) as u16;
            b.extend_from_slice(&q.to_le_bytes());
        }
    }
    (STANDARD.encode(b), lo, step)
}

/// Indices as u16 when the mesh has few enough vertices, else u32.
fn encode_indices(v: &[u32], vertices: usize) -> (String, u8) {
    if vertices <= u16::MAX as usize + 1 {
        let b: Vec<u8> = v.iter().flat_map(|x| (*x as u16).to_le_bytes()).collect();
        (STANDARD.encode(b), 16)
    } else {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        (STANDARD.encode(b), 32)
    }
}

/// Vertices with the same position become one and triangles get indices, as
/// STL files (and some 3MF exports) repeat each point for every triangle. The
/// simplifier and the app's normals both need to see which faces touch.
fn weld(m: &mut Mesh) {
    let (vertices, tris) = (m.positions.len() / 3, m.triangles());
    if !m.indices.is_empty() && vertices <= tris {
        return;
    }
    let mut ids: HashMap<[u32; 3], u32> = HashMap::with_capacity(tris);
    let mut positions = Vec::new();
    let map: Vec<u32> = m
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            // +0.0 for -0.0, so the two count as the same point.
            *ids.entry(p.map(|v| (v + 0.0).to_bits())).or_insert_with(|| {
                positions.extend(p);
                (positions.len() / 3 - 1) as u32
            })
        })
        .collect();
    m.indices = if m.indices.is_empty() { map } else { m.indices.iter().map(|&i| map[i as usize]).collect() };
    m.positions = positions;
}

/// Safe input for the simplifier, which trusts its indices: a damaged file's
/// triangles pointing outside the mesh go, unreadable numbers become 0.
fn sanitize(m: &mut Mesh) {
    for v in &mut m.positions {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    let n = (m.positions.len() / 3) as u32;
    if m.indices.iter().any(|&i| i >= n) {
        m.indices = m.indices.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|&i| i < n)).flatten().copied().collect();
    }
    m.indices.truncate(m.indices.len() / 3 * 3);
}

/// The mesh with `indices`, dropping points no triangle uses any more.
fn with_indices(m: &Mesh, indices: Vec<u32>) -> Mesh {
    const UNUSED: u32 = u32::MAX;
    let mut map = vec![UNUSED; m.positions.len() / 3];
    let mut positions = Vec::new();
    let indices = indices
        .iter()
        .map(|&i| {
            let slot = &mut map[i as usize];
            if *slot == UNUSED {
                *slot = (positions.len() / 3) as u32;
                positions.extend_from_slice(&m.positions[i as usize * 3..i as usize * 3 + 3]);
            }
            *slot
        })
        .collect();
    Mesh { positions, indices, ..Default::default() }
}

fn adapter(m: &Mesh) -> meshopt::VertexDataAdapter<'_> {
    meshopt::VertexDataAdapter::new(meshopt::typed_to_bytes(&m.positions), 12, 0).expect("positions are xyz triples")
}

/// Collapse edges for as long as the surface stays within `error` mm. Sharp
/// edges and thin walls survive, since points only ever move along the surface.
fn collapse(m: &Mesh, error: f32, lock_border: bool) -> Mesh {
    let mut options = meshopt::SimplifyOptions::ErrorAbsolute;
    if lock_border {
        options |= meshopt::SimplifyOptions::LockBorder;
    }
    let indices = meshopt::simplify(&m.indices, &adapter(m), MIN_OBJECT_TRIANGLES * 3, error, options, None);
    with_indices(m, indices)
}

/// Fewer triangles for display, the same care for every part: one allowed
/// error for the plate, raised until all of it fits the budget. Sharing the
/// budget by triangle count instead let a dense little thread starve a big
/// ribbed wall next to it.
fn simplify_plate(meshes: &mut [(&mut Mesh, bool)], budget: usize) {
    let total = |meshes: &[(&mut Mesh, bool)]| meshes.iter().map(|(m, _)| m.triangles()).sum::<usize>();
    let mut error = FIRST_ERROR;
    for _ in 0..ERROR_ROUNDS {
        if total(meshes) <= budget {
            return;
        }
        for (m, lock) in meshes.iter_mut() {
            if m.triangles() > MIN_OBJECT_TRIANGLES {
                **m = collapse(m, error, *lock);
            }
        }
        error *= ERROR_GROWTH;
    }
    // Still too much at the largest error worth keeping (locked borders of
    // heavily painted models, or just huge plates): cut by share, shape first.
    let tris = total(meshes);
    if tris <= budget + budget / 10 {
        return;
    }
    for (m, _) in meshes.iter_mut() {
        let share = (budget as f64 * m.triangles() as f64 / tris as f64).ceil() as usize;
        if m.triangles() > share.max(MIN_OBJECT_TRIANGLES) {
            let indices = meshopt::simplify_sloppy(&m.indices, &adapter(m), share.max(MIN_OBJECT_TRIANGLES) * 3, f32::MAX, None);
            **m = with_indices(m, indices);
        }
    }
}

fn bounds_of(meshes: &[&Mesh]) -> [f32; 6] {
    let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
    for m in meshes {
        for p in m.positions.as_chunks::<3>().0 {
            for i in 0..3 {
                b[i] = b[i].min(p[i]);
                b[i + 3] = b[i + 3].max(p[i]);
            }
        }
    }
    if b[0] > b[3] { [0.0; 6] } else { b }
}

/// (name, filament, colour, mesh, instance, object id)
type Part = (String, u32, Option<String>, Mesh, usize, String);

fn finish(kind: &'static str, bed: Bed, parts: Vec<Part>) -> anyhow::Result<Preview> {
    let triangles: usize = parts.iter().map(|p| p.3.triangles()).sum();
    if triangles == 0 {
        bail!("nothing to preview on this plate");
    }
    if triangles > MAX_TRIANGLES {
        bail!("this model is too detailed to preview ({} million triangles)", triangles / 1_000_000);
    }
    let bounds = bounds_of(&parts.iter().map(|p| &p.3).collect::<Vec<_>>());
    let mut parts = parts;
    for part in &mut parts {
        sanitize(&mut part.3);
        weld(&mut part.3);
    }
    // Parts of one object touch where the filament changes: their shared
    // edge stays put, or gaps open up between the colours.
    let mut per_instance: HashMap<usize, usize> = HashMap::new();
    for p in &parts {
        *per_instance.entry(p.4).or_default() += 1;
    }
    let locks: Vec<bool> = parts.iter().map(|p| per_instance[&p.4] > 1).collect();
    let mut meshes: Vec<(&mut Mesh, bool)> = parts.iter_mut().zip(locks).map(|(p, lock)| (&mut p.3, lock)).collect();
    simplify_plate(&mut meshes, PREVIEW_TRIANGLES);
    let objects = parts
        .into_iter()
        .map(|(name, filament, color, m, instance, object_id)| {
            let (positions, origin, step) = encode_positions(&m.positions);
            let (indices, index_bits) = encode_indices(&m.indices, m.positions.len() / 3);
            PreviewObject { instance, triangles: m.triangles(), positions, origin, step, indices, index_bits, name, object_id, filament, color }
        })
        .collect::<Vec<_>>();
    let shown = objects.iter().map(|o| o.triangles).sum();
    Ok(Preview { kind, bed, objects, bounds, triangles: shown })
}

// ------------------------------------------------------------------ STL / OBJ

fn read_stl(bytes: &[u8]) -> anyhow::Result<Mesh> {
    if bytes.len() >= 84 {
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if 84 + n * 50 == bytes.len() {
            if n > MAX_TRIANGLES {
                bail!("this model is too detailed to preview");
            }
            let mut positions = Vec::with_capacity(n * 9);
            for t in 0..n {
                let base = 84 + t * 50 + 12; // skip normal
                for k in 0..9 {
                    let o = base + k * 4;
                    positions.push(f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]));
                }
            }
            return Ok(Mesh { positions, ..Default::default() });
        }
    }
    let text = String::from_utf8_lossy(bytes);
    let mut positions = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("vertex") {
            let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 {
                positions.extend(v);
            }
        }
    }
    if positions.len() < 9 {
        bail!("not a readable STL file");
    }
    positions.truncate(positions.len() / 9 * 9);
    Ok(Mesh { positions, ..Default::default() })
}

fn read_obj(text: &str) -> anyhow::Result<Mesh> {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    positions.extend(v);
                }
            }
            Some("f") => {
                let count = (positions.len() / 3) as i64;
                let idx: Vec<u32> = it
                    .filter_map(|tok| tok.split('/').next()?.parse::<i64>().ok())
                    .map(|i| if i < 0 { count + i } else { i - 1 })
                    .filter(|i| *i >= 0 && *i < count)
                    .map(|i| i as u32)
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    indices.extend([idx[0], idx[k], idx[k + 1]]);
                }
            }
            _ => {}
        }
    }
    if indices.is_empty() {
        bail!("not a readable OBJ file");
    }
    Ok(Mesh { positions, indices, ..Default::default() })
}

/// Centre in x/y on the origin and rest on z = 0.
fn center_on_origin(m: &mut Mesh) {
    let b = bounds_of(&[m]);
    let (cx, cy, z0) = ((b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0, b[2]);
    for p in m.positions.as_chunks_mut::<3>().0 {
        p[0] -= cx;
        p[1] -= cy;
        p[2] -= z0;
    }
}

pub fn mesh_preview(path: &Path, bed: Bed) -> anyhow::Result<Preview> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let mut mesh = match ext.as_str() {
        "stl" => read_stl(&bytes)?,
        "obj" => read_obj(&String::from_utf8_lossy(&bytes))?,
        _ => bail!("a preview of this file type is available after slicing"),
    };
    center_on_origin(&mut mesh);
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    finish("model", bed, vec![(name, 1, None, mesh, 0, String::new())])
}

// ------------------------------------------------------------------ 3MF

/// Row-major 3x4 affine transform as written in 3MF ("m00 m01 m02 m10 … m32"),
/// applied as p' = p * M.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tf([f32; 12]);

impl Tf {
    const ID: Tf = Tf([1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.]);

    fn parse(s: Option<&str>) -> Tf {
        let Some(s) = s else { return Tf::ID };
        let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if v.len() == 12 { Tf(v.try_into().unwrap()) } else { Tf::ID }
    }

    fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [
            p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
            p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
            p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
        ]
    }

    /// self, then other.
    fn then(&self, o: &Tf) -> Tf {
        let a = &self.0;
        let b = &o.0;
        let mut r = [0f32; 12];
        for i in 0..3 {
            for j in 0..3 {
                r[i * 3 + j] = a[i * 3] * b[j] + a[i * 3 + 1] * b[3 + j] + a[i * 3 + 2] * b[6 + j];
            }
        }
        for j in 0..3 {
            r[9 + j] = a[9] * b[j] + a[10] * b[3 + j] + a[11] * b[6 + j] + b[9 + j];
        }
        Tf(r)
    }
}

#[derive(Default)]
struct ObjectDef {
    name: Option<String>,
    mesh: Option<Mesh>,
    /// (path or "" for this file, object id, transform)
    components: Vec<(String, String, Tf)>,
}

#[derive(Default)]
struct ModelFile {
    objects: HashMap<String, ObjectDef>,
    /// Build items in order: (object id, transform).
    items: Vec<(String, Tf)>,
}

/// Merge points into a grid of `cells` along the longest side: the merged
/// points (cell averages) and, for every original point, its merged index.
fn merge_points(positions: &[f32], cells: f32) -> (Vec<f32>, Vec<u32>) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in positions.as_chunks::<3>().0 {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let longest = (0..3).map(|k| hi[k] - lo[k]).fold(0f32, f32::max).max(1e-3);
    let cell = longest / cells;
    let mut ids: HashMap<(i32, i32, i32), u32> = HashMap::new();
    let mut sums: Vec<[f64; 4]> = Vec::new();
    let mut map = Vec::with_capacity(positions.len() / 3);
    for p in positions.as_chunks::<3>().0 {
        let key = (((p[0] - lo[0]) / cell) as i32, ((p[1] - lo[1]) / cell) as i32, ((p[2] - lo[2]) / cell) as i32);
        let id = *ids.entry(key).or_insert_with(|| {
            sums.push([0.0; 4]);
            (sums.len() - 1) as u32
        });
        let s = &mut sums[id as usize];
        s[0] += p[0] as f64;
        s[1] += p[1] as f64;
        s[2] += p[2] as f64;
        s[3] += 1.0;
        map.push(id);
    }
    let merged = sums.iter().flat_map(|s| [(s[0] / s[3]) as f32, (s[1] / s[3]) as f32, (s[2] / s[3]) as f32]).collect();
    (merged, map)
}

/// The filament painted on a triangle, from a TriangleSelector string (hex
/// nibbles read from the end; a triangle can be split into painted pieces).
/// The piece colour that occurs most wins; 0 = not painted.
fn painted_filament(code: &str) -> u8 {
    let mut nibbles = code.bytes().rev().filter_map(|c| (c as char).to_digit(16).map(|d| d as u8));
    let mut counts = [0u32; 32];
    // Depth-first over the split tree, without recursion.
    let mut pending = 1u32;
    while pending > 0 {
        pending -= 1;
        let Some(n) = nibbles.next() else { break };
        let split = n & 0b11;
        if split == 0 {
            let state = match n >> 2 {
                3 => nibbles.next().unwrap_or(0) + 3,
                s => s,
            };
            if let Some(c) = counts.get_mut(state as usize) {
                *c += 1;
            }
        } else {
            pending += split as u32 + 1;
        }
    }
    (1..counts.len()).filter(|&i| counts[i] > 0).max_by_key(|&i| counts[i]).unwrap_or(0) as u8
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .map(|a| a.value.replace("&amp;", "&").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&apos;", "'"))
}

#[cfg(test)]
fn parse_model(xml: &str) -> anyhow::Result<ModelFile> {
    read_model(xml.as_bytes())
}

/// Parse a 3MF model part as a stream: detailed models are hundreds of MB
/// of XML, too much to hold as text next to the mesh.
fn read_model<R: std::io::BufRead>(src: R) -> anyhow::Result<ModelFile> {
    let mut reader = quick_xml::Reader::from_reader(src);
    let mut buf = Vec::new();
    let mut out = ModelFile::default();
    let mut current: Option<(String, ObjectDef)> = None;
    let mut in_build = false;
    let mut vertices = 0usize;
    let mut triangles = 0usize;
    // Very detailed meshes: their points merged into a grid before the
    // triangles come, so memory doesn't grow with the detail.
    let mut remap: Option<Vec<u32>> = None;
    loop {
        buf.clear();
        let ev = reader.read_event_into(&mut buf).context("invalid 3MF model XML")?;
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e), false),
            Event::Empty(e) => (Some(e), true),
            Event::End(e) => {
                match e.local_name().as_ref() {
                    "object" => {
                        if let Some((id, def)) = current.take() {
                            out.objects.insert(id, def);
                        }
                    }
                    "build" => in_build = false,
                    _ => {}
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let e = e.unwrap();
        match e.local_name().as_ref() {
            "mesh" => remap = None,
            "triangles" => {
                remap = None;
                if let Some(m) = current.as_mut().and_then(|(_, d)| d.mesh.as_mut())
                    && m.positions.len() / 3 > COARSE_VERTICES
                {
                    let (merged, map) = merge_points(&m.positions, COARSE_CELLS);
                    m.positions = merged;
                    remap = Some(map);
                }
            }
            "object" => {
                let id = attr(e, "id").unwrap_or_default();
                let def = ObjectDef { name: attr(e, "name"), ..Default::default() };
                if empty {
                    out.objects.insert(id, def);
                } else {
                    current = Some((id, def));
                }
            }
            "vertex" => {
                if let Some((_, def)) = current.as_mut() {
                    let m = def.mesh.get_or_insert_with(Mesh::default);
                    // Millions of these: read the attributes once, without copies.
                    let mut xyz = [0f32; 3];
                    for a in e.attributes().with_checks(false).flatten() {
                        let k = match a.key.local_name().as_ref() {
                            "x" => 0,
                            "y" => 1,
                            "z" => 2,
                            _ => continue,
                        };
                        xyz[k] = a.value.trim().parse().unwrap_or(0.0);
                    }
                    m.positions.extend(xyz);
                    vertices += 1;
                }
            }
            "triangle" => {
                if let Some((_, def)) = current.as_mut() {
                    let m = def.mesh.get_or_insert_with(Mesh::default);
                    let mut tri = [None::<u32>; 3];
                    let mut painted = 0u8;
                    for a in e.attributes().with_checks(false).flatten() {
                        let text = a.value.trim();
                        match a.key.local_name().as_ref() {
                            "v1" => tri[0] = text.parse().ok(),
                            "v2" => tri[1] = text.parse().ok(),
                            "v3" => tri[2] = text.parse().ok(),
                            // Colour painting: Bambu Studio's paint_color, PrusaSlicer's mmu_segmentation.
                            "paint_color" | "mmu_segmentation" => painted = painted_filament(text),
                            _ => {}
                        }
                    }
                    let tri = match (&remap, tri) {
                        (Some(map), [Some(a), Some(b), Some(c)]) => {
                            let t = [a, b, c].map(|v| map.get(v as usize).copied());
                            match t {
                                // Triangles that collapsed into a line or a point disappear.
                                [Some(a), Some(b), Some(c)] if a != b && b != c && a != c => [Some(a), Some(b), Some(c)],
                                _ => [None; 3],
                            }
                        }
                        (_, t) => t,
                    };
                    if let [Some(a), Some(b), Some(c)] = tri {
                        m.indices.extend([a, b, c]);
                        if painted > 0 && m.paint.is_empty() {
                            m.paint.resize(m.indices.len() / 3 - 1, 0);
                        }
                        if !m.paint.is_empty() {
                            m.paint.push(painted);
                        }
                    }
                    triangles += 1;
                    if triangles > MAX_TRIANGLES {
                        bail!("this model is too detailed to preview (over {} million triangles)", MAX_TRIANGLES / 1_000_000);
                    }
                }
            }
            "component" => {
                if let Some((_, def)) = current.as_mut() {
                    let path = attr(e, "path").unwrap_or_default();
                    def.components.push((path, attr(e, "objectid").unwrap_or_default(), Tf::parse(attr(e, "transform").as_deref())));
                }
            }
            "build" => in_build = !empty,
            "item" if in_build => {
                out.items.push((attr(e, "objectid").unwrap_or_default(), Tf::parse(attr(e, "transform").as_deref())));
            }
            _ => {}
        }
    }
    let _ = vertices;
    Ok(out)
}

/// Per-object and per-part settings from Metadata/model_settings.config.
#[derive(Default, Debug)]
struct Settings {
    object_extruder: HashMap<String, u32>,
    object_name: HashMap<String, String>,
    /// (object id, part id) -> (extruder, is a printable part)
    parts: HashMap<(String, String), (Option<u32>, bool)>,
    /// plate index -> [(object id, instance index)]
    plates: Vec<(u32, Vec<(String, usize)>)>,
}

fn parse_settings(xml: &str) -> Settings {
    let mut s = Settings::default();
    let Ok(root) = crate::threemf::parse_xml(xml) else { return s };
    for config in root.children.iter().filter(|c| c.name == "config") {
        for obj in config.children.iter().filter(|c| c.name == "object") {
            let Some(id) = obj.attrs.get("id").cloned() else { continue };
            for m in obj.children.iter().filter(|c| c.name == "metadata") {
                match (m.attrs.get("key").map(String::as_str), m.attrs.get("value")) {
                    (Some("extruder"), Some(v)) => {
                        if let Ok(x) = v.parse() {
                            s.object_extruder.insert(id.clone(), x);
                        }
                    }
                    (Some("name"), Some(v)) => {
                        s.object_name.insert(id.clone(), v.clone());
                    }
                    _ => {}
                }
            }
            for part in obj.children.iter().filter(|c| c.name == "part") {
                let Some(pid) = part.attrs.get("id").cloned() else { continue };
                let printable = part.attrs.get("subtype").map(|t| t == "normal_part").unwrap_or(true);
                let extruder = part
                    .children
                    .iter()
                    .find(|m| m.name == "metadata" && m.attrs.get("key").map(String::as_str) == Some("extruder"))
                    .and_then(|m| m.attrs.get("value")?.parse().ok());
                s.parts.insert((id.clone(), pid), (extruder, printable));
            }
        }
        for plate in config.children.iter().filter(|c| c.name == "plate") {
            let meta = |node: &crate::threemf::Node, key: &str| {
                node.children
                    .iter()
                    .find(|m| m.name == "metadata" && m.attrs.get("key").map(String::as_str) == Some(key))
                    .and_then(|m| m.attrs.get("value").cloned())
            };
            let Some(index) = meta(plate, "plater_id").and_then(|v| v.parse().ok()) else { continue };
            let instances = plate
                .children
                .iter()
                .filter(|c| c.name == "model_instance")
                .filter_map(|mi| Some((meta(mi, "object_id")?, meta(mi, "instance_id").and_then(|v| v.parse().ok()).unwrap_or(0))))
                .collect();
            s.plates.push((index, instances));
        }
    }
    s
}

fn json_strings(v: &serde_json::Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(serde_json::Value::Array(a)) => a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect(),
        Some(serde_json::Value::String(s)) => s.split(',').map(str::to_string).collect(),
        _ => vec![],
    }
}

/// A model part of the 3MF, streamed out of the zip.
fn read_zip_model(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> anyhow::Result<Option<ModelFile>> {
    let Ok(entry) = zip.by_name(name.trim_start_matches('/')) else { return Ok(None) };
    read_model(std::io::BufReader::with_capacity(1 << 20, entry)).map(Some)
}

fn read_zip_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let entry = zip.by_name(name.trim_start_matches('/')).ok()?;
    let mut s = String::new();
    entry.take(1024 * 1024 * 1024).read_to_string(&mut s).ok()?;
    Some(s)
}

/// Plate layout from a 3MF project or sliced file.
/// `fallback_bed` is used when the file has no printable area.
pub fn project_preview(path: &Path, plate: u32, fallback_bed: Bed) -> anyhow::Result<Preview> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(f).context("not a valid 3MF file")?;

    // Colours and bed from the project settings.
    let mut colors: Vec<String> = vec![];
    let mut bed = fallback_bed;
    if let Some(text) = read_zip_text(&mut zip, "Metadata/project_settings.config")
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            colors = json_strings(&v, "filament_colour").iter().map(|c| crate::threemf::hex_color(c)).collect();
            let height = v.get("printable_height").and_then(|h| h.as_str().and_then(|s| s.parse().ok()).or(h.as_f64().map(|f| f as f32)));
            if let Some(b) = Bed::from_settings(&json_strings(&v, "printable_area"), height) {
                bed = b;
            }
        }
    let settings = read_zip_text(&mut zip, "Metadata/model_settings.config").map(|t| parse_settings(&t)).unwrap_or_default();

    let root = read_zip_model(&mut zip, "3D/3dmodel.model")?.ok_or_else(|| anyhow!("the 3MF has no 3D model"))?;
    let mut files: HashMap<String, ModelFile> = HashMap::new();
    files.insert(String::new(), root);

    // Which build items belong to the plate.
    let plate_count = settings.plates.len().max(1);
    let selected: Vec<(String, Tf)> = {
        let items = &files[""].items;
        match settings.plates.iter().find(|(i, _)| *i == plate) {
            Some((_, instances)) => instances
                .iter()
                .filter_map(|(obj, n)| items.iter().filter(|(id, _)| id == obj).nth(*n).cloned())
                .collect(),
            None if settings.plates.is_empty() => items.clone(),
            None => bail!("plate {plate} not found in the file"),
        }
    };
    // Load only the sub-models those objects use (Bambu keeps meshes in
    // 3D/Objects/*.model; the other plates' meshes can be large).
    let mut wanted: Vec<String> = selected
        .iter()
        .filter_map(|(obj, _)| files[""].objects.get(obj))
        .flat_map(|o| o.components.iter().map(|c| c.0.clone()))
        .filter(|p| !p.is_empty())
        .collect();
    wanted.sort();
    wanted.dedup();
    for p in wanted {
        if let Some(model) = read_zip_model(&mut zip, &p)? {
            files.insert(p, model);
        }
    }

    let mut parts: Vec<Part> = Vec::new();
    for (instance, (obj_id, item_tf)) in selected.iter().enumerate() {
        let root_model = &files[""];
        let Some(obj) = root_model.objects.get(obj_id) else { continue };
        let obj_extruder = settings.object_extruder.get(obj_id).copied().unwrap_or(1);
        let name = settings.object_name.get(obj_id).cloned().or_else(|| obj.name.clone()).unwrap_or_else(|| format!("Object {obj_id}"));
        // (mesh, transform, extruder)
        let mut pieces: Vec<(&Mesh, Tf, u32)> = Vec::new();
        if let Some(m) = &obj.mesh {
            pieces.push((m, *item_tf, obj_extruder));
        }
        for (path, cid, ctf) in &obj.components {
            let (extruder, printable) = settings.parts.get(&(obj_id.clone(), cid.clone())).copied().unwrap_or((None, true));
            if !printable {
                continue;
            }
            let file = files.get(path.as_str()).unwrap_or(root_model);
            if let Some(m) = file.objects.get(cid).and_then(|o| o.mesh.as_ref()) {
                pieces.push((m, ctf.then(item_tf), extruder.unwrap_or(obj_extruder)));
            }
        }
        // One preview object per filament within this object.
        let mut by_filament: HashMap<u32, Mesh> = HashMap::new();
        for (m, tf, ext) in pieces {
            if m.paint.is_empty() {
                let out = by_filament.entry(ext).or_default();
                let base = (out.positions.len() / 3) as u32;
                for p in m.positions.as_chunks::<3>().0 {
                    out.positions.extend(tf.apply([p[0], p[1], p[2]]));
                }
                out.indices.extend(m.indices.iter().map(|i| i + base));
                continue;
            }
            // Painted: each triangle goes to its painted filament, with its
            // own copy of the vertices it uses.
            let mut remap: HashMap<(u32, u32), u32> = HashMap::new();
            for (t, tri) in m.indices.as_chunks::<3>().0.iter().enumerate() {
                let f = match m.paint.get(t).copied().unwrap_or(0) {
                    0 => ext,
                    p => p as u32,
                };
                let out = by_filament.entry(f).or_default();
                for &v in tri {
                    let idx = *remap.entry((f, v)).or_insert_with(|| {
                        let i = v as usize * 3;
                        let p = m.positions.get(i..i + 3).unwrap_or(&[0.0, 0.0, 0.0]);
                        out.positions.extend(tf.apply([p[0], p[1], p[2]]));
                        (out.positions.len() / 3 - 1) as u32
                    });
                    out.indices.push(idx);
                }
            }
        }
        let mut keys: Vec<u32> = by_filament.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let mesh = by_filament.remove(&k).unwrap();
            parts.push((name.clone(), k, colors.get(k as usize - 1).cloned().filter(|c| !c.is_empty()), mesh, instance, obj_id.clone()));
        }
    }

    // Plates after the first sit to the side in the project's world space.
    if plate > 1 || plate_count > 1 {
        let cols = plate_columns(plate_count);
        let i = plate.saturating_sub(1) as usize;
        let (row, col) = (i / cols, i % cols);
        let off_x = col as f32 * bed.width * (1.0 + PLATE_GAP);
        let off_y = -(row as f32) * bed.depth * (1.0 + PLATE_GAP);
        shift(&mut parts, -off_x, -off_y);
    }
    // Safety net: if the layout still isn't on the bed (unknown conventions),
    // centre it so the preview is useful.
    let b = bounds_of(&parts.iter().map(|p| &p.3).collect::<Vec<_>>());
    let (cx, cy) = ((b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0);
    if cx < -bed.width * 0.1 || cx > bed.width * 1.1 || cy < -bed.depth * 0.1 || cy > bed.depth * 1.1 {
        shift(&mut parts, bed.width / 2.0 - cx, bed.depth / 2.0 - cy);
    }
    finish("layout", bed, parts)
}

fn plate_columns(count: usize) -> usize {
    let v = (count as f32).sqrt();
    let r = v.round();
    (if v > r { r + 1.0 } else { r }).max(1.0) as usize
}

fn shift(parts: &mut [Part], dx: f32, dy: f32) {
    for p in parts {
        for v in p.3.positions.as_chunks_mut::<3>().0 {
            v[0] += dx;
            v[1] += dy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_positions(o: &PreviewObject) -> Vec<f32> {
        STANDARD
            .decode(&o.positions)
            .unwrap()
            .as_chunks::<2>().0.iter()
            .enumerate()
            .map(|(i, c)| o.origin[i % 3] + u16::from_le_bytes([c[0], c[1]]) as f32 * o.step[i % 3])
            .collect()
    }

    /// A flat square of `n` x `n` cells, two triangles each, 100 mm wide.
    fn grid(n: u32) -> Mesh {
        let mut m = Mesh::default();
        for y in 0..=n {
            for x in 0..=n {
                m.positions.extend([x as f32 * 100.0 / n as f32, y as f32 * 100.0 / n as f32, (x as f32 * 0.37).sin()]);
            }
        }
        for y in 0..n {
            for x in 0..n {
                let i = y * (n + 1) + x;
                m.indices.extend([i, i + 1, i + n + 1, i + 1, i + n + 2, i + n + 1]);
            }
        }
        m
    }

    /// A closed ribbed tube: `ribs` around, `layers` up, wall on the outside only.
    fn ribbed(ribs: u32, radius: f32, depth: f32, segments: u32, layers: u32) -> Mesh {
        let mut m = Mesh::default();
        for l in 0..=layers {
            for s in 0..segments {
                let a = s as f32 / segments as f32 * std::f32::consts::TAU;
                let r = radius + depth * (0.5 + 0.5 * (ribs as f32 * a).cos());
                m.positions.extend([r * a.cos(), r * a.sin(), l as f32 * 50.0 / layers as f32]);
            }
        }
        for l in 0..layers {
            for s in 0..segments {
                let u = (s + 1) % segments;
                let (a, b, c, d) = (l * segments + s, l * segments + u, (l + 1) * segments + u, (l + 1) * segments + s);
                m.indices.extend([a, b, c, a, c, d]);
            }
        }
        m
    }

    fn radial_range(m: &Mesh) -> (f32, f32) {
        m.positions.as_chunks::<3>().0.iter().map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt()).fold((f32::MAX, 0.0), |(a, b), r| (a.min(r), b.max(r)))
    }

    #[test]
    fn simplifies_to_the_budget() {
        let mut m = grid(200); // 80 000 triangles
        let full = m.clone();
        simplify_plate(&mut [(&mut m, false)], 5_000);
        assert!(m.triangles() <= 5_500 && m.triangles() >= 2_000, "{} triangles", m.triangles());
        let (a, b) = (bounds_of(&[&full]), bounds_of(&[&m]));
        assert!((a[3] - b[3]).abs() < 0.5 && (a[4] - b[4]).abs() < 0.5, "outline kept: {a:?} vs {b:?}");
        let mut small = full.clone();
        simplify_plate(&mut [(&mut small, false)], 100_000);
        assert_eq!(small.triangles(), full.triangles(), "small enough meshes are untouched");
    }

    /// Averaging points in a grid put them between the two sides of a thin
    /// wall and tore holes in it (a printed egg shell, 2026-11-05).
    #[test]
    fn thin_walls_stay_apart() {
        let mut m = grid(300);
        let top = grid(300);
        let base = (m.positions.len() / 3) as u32;
        m.positions.extend(top.positions.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2] + 0.3]));
        m.indices.extend(top.indices.iter().map(|i| i + base));
        simplify_plate(&mut [(&mut m, false)], 20_000);
        // Every point is still on one of the two sides, and both sides remain.
        let wave = |p: &[f32; 3]| (p[0] * 3.0 * 0.37).sin();
        let lower = m.positions.as_chunks::<3>().0.iter().filter(|p| (p[2] - wave(p)).abs() < 0.01).count();
        let upper = m.positions.as_chunks::<3>().0.iter().filter(|p| (p[2] - wave(p) - 0.3).abs() < 0.01).count();
        assert_eq!(lower + upper, m.positions.len() / 3, "nothing between the sides");
        assert!(lower > 100 && upper > 100, "{lower} + {upper}");
        assert!(m.triangles() <= 22_000, "{} triangles", m.triangles());
    }

    /// A dense little part must not take the budget from a big ribbed one.
    #[test]
    fn dense_small_parts_leave_the_ribs_alone() {
        let mut wall = ribbed(56, 28.0, 0.75, 1100, 110);
        let mut thread = ribbed(3, 9.0, 1.0, 1400, 500);
        simplify_plate(&mut [(&mut wall, false), (&mut thread, false)], 30_000);
        let (low, high) = radial_range(&wall);
        assert!(high - low > 0.7, "rib depth kept: {low}..{high}");
        assert!(wall.triangles() > thread.triangles() / 4, "{} vs {}", wall.triangles(), thread.triangles());
        assert!(wall.triangles() + thread.triangles() <= 33_000);
    }

    #[test]
    fn repeated_points_become_one() {
        let mut m = Mesh { positions: vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.], ..Default::default() };
        weld(&mut m);
        assert_eq!(m.positions.len() / 3, 4);
        assert_eq!(m.indices, vec![0, 1, 2, 1, 3, 2]);
    }

    #[test]
    fn damaged_files_cant_reach_the_simplifier() {
        let mut m = Mesh { positions: vec![0., 0., 0., f32::NAN, 0., 0., 0., 1., 0.], indices: vec![0, 1, 2, 0, 1, 99], ..Default::default() };
        sanitize(&mut m);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(m.positions[3], 0.0);
        // Whole files with such triangles get through finish() too.
        let bad = Mesh { positions: vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 1.], indices: vec![0, 1, 2, 0, 1, 99], ..Default::default() };
        let pv = finish("model", Bed::rect(1., 1., 1.), vec![(String::new(), 1, None, bad, 0, String::new())]).unwrap();
        assert_eq!(pv.triangles, 1);
    }

    #[test]
    fn painted_parts_keep_their_shared_edge() {
        let mut m = grid(200);
        let edge = |m: &Mesh| m.positions.as_chunks::<3>().0.iter().filter(|p| p[0] == 0.0).count();
        let before = edge(&m);
        simplify_plate(&mut [(&mut m, true)], 5_000);
        assert_eq!(edge(&m), before, "border points stay when the part shares them");
    }

    #[test]
    fn packs_positions_precisely() {
        let m = grid(3);
        let (b64, origin, step) = encode_positions(&m.positions);
        let o = PreviewObject {
            name: String::new(),
            object_id: String::new(), filament: 1, instance: 0, color: None, positions: b64, origin, step,
            indices: String::new(), index_bits: 16, triangles: 0,
        };
        for (a, b) in decode_positions(&o).iter().zip(&m.positions) {
            assert!((a - b).abs() < 0.01, "{a} vs {b}");
        }
        assert_eq!(encode_indices(&[0, 1, 2], 3).1, 16);
        assert_eq!(encode_indices(&[0, 1, 70_000], 70_001).1, 32);
    }

    #[test]
    fn bed_from_settings() {
        let b = Bed::from_settings(&["0x0".into(), "256x0".into(), "256x256".into(), "0x256".into()], Some(250.0)).unwrap();
        assert_eq!((b.width, b.depth, b.height), (256.0, 256.0, 250.0));
        assert!(Bed::from_settings(&["bad".into()], None).is_none());
        assert_eq!(Bed::for_model("Bambu Lab A1 mini").width, 180.0);
    }

    #[test]
    fn stl_ascii_and_binary() {
        let ascii = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 10 0 0\nvertex 0 10 5\nendloop\nendfacet\nendsolid";
        let m = read_stl(ascii.as_bytes()).unwrap();
        assert_eq!(m.triangles(), 1);
        let mut bin = vec![0u8; 80];
        bin.extend(1u32.to_le_bytes());
        bin.extend([0u8; 12]);
        for v in [0f32, 0., 0., 10., 0., 0., 0., 10., 5.] {
            bin.extend(v.to_le_bytes());
        }
        bin.extend([0u8; 2]);
        let m2 = read_stl(&bin).unwrap();
        assert_eq!(m2.positions, m.positions);
    }

    #[test]
    fn obj_quads_and_negative_indices() {
        let m = read_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1 2/2 3/3 4/4\nf -4 -3 -2\n").unwrap();
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3, 0, 1, 2]);
    }

    #[test]
    fn transforms_compose() {
        let rot = Tf([0., 1., 0., -1., 0., 0., 0., 0., 1., 0., 0., 0.]); // 90° about Z
        let mv = Tf([1., 0., 0., 0., 1., 0., 0., 0., 1., 100., 50., 0.]);
        assert_eq!(rot.apply([1., 0., 0.]), [0., 1., 0.]);
        assert_eq!(rot.then(&mv).apply([1., 0., 0.]), [100., 51., 0.]);
    }

    #[test]
    fn reads_paint_per_triangle() {
        let xml = r#"<model><resources><object id="1"><mesh><vertices>
          <vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="1" y="1" z="0"/></vertices>
          <triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="1" v2="3" v3="2" paint_color="8"/></triangles></mesh></object></resources></model>"#;
        let m = parse_model(xml).unwrap();
        assert_eq!(m.objects["1"].mesh.as_ref().unwrap().paint, vec![0, 2]);
    }

    #[test]
    fn merges_points_into_a_grid() {
        // Two points closer than a cell share one; the far one keeps its own.
        let (merged, map) = merge_points(&[0.0, 0.0, 0.0, 0.01, 0.0, 0.0, 100.0, 0.0, 0.0], 100.0);
        assert_eq!(map, vec![0, 0, 1]);
        assert_eq!(merged.len(), 6);
        assert!((merged[0] - 0.005).abs() < 1e-4);
    }

    #[test]
    fn decodes_painted_triangles() {
        assert_eq!(painted_filament(""), 0);
        assert_eq!(painted_filament("4"), 1);
        assert_eq!(painted_filament("8"), 2);
        // State 3 and up take a second nibble: 3 + 0, 3 + 1.
        assert_eq!(painted_filament("0C"), 3);
        assert_eq!(painted_filament("1C"), 4);
        // Split into two pieces (code 1): filament 2 and filament 2 beat nothing.
        assert_eq!(painted_filament("881"), 2);
        // Split into three (code 2): one unpainted, two filament 1.
        assert_eq!(painted_filament("4402"), 1);
    }

    #[test]
    fn plate_columns_match_orca() {
        assert_eq!(plate_columns(1), 1);
        assert_eq!(plate_columns(2), 2);
        assert_eq!(plate_columns(4), 2);
        assert_eq!(plate_columns(5), 3);
        assert_eq!(plate_columns(10), 4);
    }

    #[test]
    fn bambu_style_project() {
        let dir = std::env::temp_dir().join(format!("as-prev-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("p.3mf");
        // Unit triangle sub-mesh; object 2 has two parts (filament 1 and 2) plus a modifier.
        let sub = r#"<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>
          <object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="0"/></vertices>
          <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>
          <object id="3" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="5"/></vertices>
          <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>
          <object id="4" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
          <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources></model>"#;
        let root = r#"<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources>
          <object id="2" type="model"><components>
            <component p:path="/3D/Objects/object_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>
            <component p:path="/3D/Objects/object_1.model" objectid="3" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>
            <component p:path="/3D/Objects/object_1.model" objectid="4" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>
          </components></object></resources>
          <build>
            <item objectid="2" transform="1 0 0 0 1 0 0 0 1 100 100 0" printable="1"/>
            <item objectid="2" transform="1 0 0 0 1 0 0 0 1 120 100 0" printable="1"/>
            <item objectid="2" transform="1 0 0 0 1 0 0 0 1 407.2 100 0" printable="1"/>
          </build></model>"#;
        let settings = r#"<config>
          <object id="2"><metadata key="name" value="Gear"/><metadata key="extruder" value="1"/>
            <part id="1" subtype="normal_part"><metadata key="extruder" value="1"/></part>
            <part id="3" subtype="normal_part"><metadata key="extruder" value="2"/></part>
            <part id="4" subtype="modifier_part"></part></object>
          <plate><metadata key="plater_id" value="1"/>
            <model_instance><metadata key="object_id" value="2"/><metadata key="instance_id" value="0"/></model_instance>
            <model_instance><metadata key="object_id" value="2"/><metadata key="instance_id" value="1"/></model_instance></plate>
          <plate><metadata key="plater_id" value="2"/>
            <model_instance><metadata key="object_id" value="2"/><metadata key="instance_id" value="2"/></model_instance></plate>
        </config>"#;
        let project = r##"{"printable_area":["0x0","256x0","256x256","0x256"],"printable_height":"250","filament_colour":["#FF0000","#00FF00"]}"##;
        crate::threemf::tests::make_3mf(&p, &[
            ("3D/3dmodel.model", root.as_bytes()),
            ("3D/Objects/object_1.model", sub.as_bytes()),
            ("Metadata/model_settings.config", settings.as_bytes()),
            ("Metadata/project_settings.config", project.as_bytes()),
        ]);
        let pv = project_preview(&p, 1, Bed::rect(1., 1., 1.)).unwrap();
        assert_eq!(pv.kind, "layout");
        assert_eq!(pv.bed.width, 256.0);
        // two instances x two printable filaments, modifier skipped
        assert_eq!(pv.objects.len(), 4);
        assert_eq!(pv.triangles, 4);
        assert_eq!(pv.objects[0].name, "Gear");
        assert_eq!(pv.objects[1].filament, 2);
        assert_eq!((pv.objects[1].instance, pv.objects[2].instance), (0, 1));
        assert_eq!(pv.objects[1].color.as_deref(), Some("#00FF00"));
        let pos = decode_positions(&pv.objects[0]);
        assert!(pos[..3].iter().zip([100.0, 100.0, 0.0]).all(|(a, b)| (a - b).abs() < 0.01), "{:?}", &pos[..3]);
        assert_eq!(pv.bounds[3], 130.0);

        // Plate 2 sits one plate stride (256 * 1.2) to the right in the project.
        let pv2 = project_preview(&p, 2, Bed::rect(1., 1., 1.)).unwrap();
        let pos2 = decode_positions(&pv2.objects[0]);
        assert!((pos2[0] - 100.0).abs() < 0.01, "plate 2 shifted back onto the bed: {}", pos2[0]);
        assert!(project_preview(&p, 3, Bed::rect(1., 1., 1.)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mesh_preview_is_centred() {
        let dir = std::env::temp_dir().join(format!("as-prev-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("cube.stl");
        std::fs::write(&p, "solid\nvertex 10 10 5\nvertex 30 10 5\nvertex 10 50 25\nendsolid").unwrap();
        let pv = mesh_preview(&p, Bed::rect(256., 256., 256.)).unwrap();
        assert_eq!(pv.kind, "model");
        assert_eq!(pv.bounds, [-10.0, -20.0, 0.0, 10.0, 20.0, 20.0]);
        assert!(mesh_preview(&dir.join("x.step"), Bed::rect(1., 1., 1.)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
