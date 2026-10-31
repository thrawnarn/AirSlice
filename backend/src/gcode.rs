//! Layer view of sliced G-code: the plate's G-code is copied out of the 3MF
//! once, indexed by layer (byte offsets plus the machine state where each
//! layer starts), and single layers are turned into extrusion paths on demand.

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// Bump when the index format changes.
const INDEX_VERSION: u32 = 3;
/// Arcs are drawn as chords of about this length (mm).
const ARC_STEP: f64 = 0.5;

/// Machine state the parser carries from line to line.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Machine {
    pub x: f64,
    pub y: f64,
    pub tool: u8,
    pub feature: u8,
    /// G91: X/Y are relative.
    pub rel_xy: bool,
    /// M82: E is absolute (Bambu uses M83, relative).
    pub abs_e: bool,
    pub e: f64,
}

impl Default for Machine {
    fn default() -> Self {
        Machine { x: 0.0, y: 0.0, tool: 0, feature: 0, rel_xy: false, abs_e: false, e: 0.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub z: f64,
    pub height: f64,
    #[serde(skip_serializing_if = "is_zero", default)]
    start: u64,
    #[serde(skip_serializing_if = "is_zero", default)]
    end: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    machine: Option<Machine>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    pub layers: Vec<Layer>,
    /// Extruded area: min x, min y, max x, max y (mm).
    pub bounds: [f64; 4],
    /// Feature names (`; FEATURE:` comments); paths refer to them by index.
    pub features: Vec<String>,
}

impl Index {
    /// What the page needs: no offsets or machine state.
    pub fn public(&self) -> Index {
        Index {
            version: self.version,
            layers: self.layers.iter().map(|l| Layer { z: l.z, height: l.height, start: 0, end: 0, machine: None }).collect(),
            bounds: self.bounds,
            features: self.features.clone(),
        }
    }
}

/// One extruded polyline: feature, tool, then x,y pairs in 1/100 mm.
#[derive(Debug, Clone, PartialEq)]
pub struct PathOut {
    pub feature: u8,
    pub tool: u8,
    pub points: Vec<i32>,
}

impl Serialize for PathOut {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(2 + self.points.len()))?;
        seq.serialize_element(&(self.feature as i32))?;
        seq.serialize_element(&(self.tool as i32))?;
        for p in &self.points {
            seq.serialize_element(p)?;
        }
        seq.end()
    }
}

/// Feature names seen so far (index = number used in paths).
struct Features(Vec<String>);

impl Features {
    fn id(&mut self, name: &str) -> u8 {
        match self.0.iter().position(|f| f == name) {
            Some(i) => i as u8,
            None if self.0.len() < 255 => {
                self.0.push(name.to_string());
                (self.0.len() - 1) as u8
            }
            None => 254,
        }
    }
}

/// Words of a G-code line ("G1 X1 Y2 E.3 ; comment" → G1 and X/Y/E values).
struct Words {
    code: (u8, u32),
    x: Option<f64>,
    y: Option<f64>,
    e: Option<f64>,
    i: Option<f64>,
    j: Option<f64>,
}

fn words(line: &str) -> Option<Words> {
    let code_part = line.split(';').next()?.trim();
    let mut parts = code_part.split_ascii_whitespace();
    let head = parts.next()?;
    let letter = head.as_bytes()[0].to_ascii_uppercase();
    let num: u32 = head[1..].parse().ok()?;
    let mut w = Words { code: (letter, num), x: None, y: None, e: None, i: None, j: None };
    for p in parts {
        let (l, v) = (p.as_bytes()[0].to_ascii_uppercase(), p[1..].parse::<f64>().ok());
        match l {
            b'X' => w.x = v,
            b'Y' => w.y = v,
            b'E' => w.e = v,
            b'I' => w.i = v,
            b'J' => w.j = v,
            _ => {}
        }
    }
    Some(w)
}

/// Apply one line. Returns the points of an extruding move (start excluded).
fn step(m: &mut Machine, line: &str, features: &mut Features) -> Option<Vec<(f64, f64)>> {
    let line = line.trim();
    if let Some(name) = line.strip_prefix("; FEATURE:") {
        m.feature = features.id(name.trim());
        return None;
    }
    if line.starts_with(';') || line.is_empty() {
        return None;
    }
    let w = words(line)?;
    match w.code {
        (b'G', 90) => m.rel_xy = false,
        (b'G', 91) => m.rel_xy = true,
        (b'M', 82) => m.abs_e = true,
        (b'M', 83) => m.abs_e = false,
        (b'G', 92) => {
            if let Some(e) = w.e {
                m.e = e;
            }
        }
        // T0–T15 pick a filament; T255/T1000 are macro internals.
        (b'T', n) if n < 16 => m.tool = n as u8,
        (b'G', g @ 0..=3) => {
            let (x0, y0) = (m.x, m.y);
            let (x1, y1) = if m.rel_xy { (x0 + w.x.unwrap_or(0.0), y0 + w.y.unwrap_or(0.0)) } else { (w.x.unwrap_or(x0), w.y.unwrap_or(y0)) };
            let extruding = match w.e {
                Some(e) if m.abs_e => {
                    let d = e - m.e;
                    m.e = e;
                    d > 0.0
                }
                Some(e) => e > 0.0,
                None => false,
            };
            m.x = x1;
            m.y = y1;
            if !extruding || g == 0 {
                return None;
            }
            if g == 1 {
                // Purging in place (at the chute, off the bed) draws nothing.
                return (x1 != x0 || y1 != y0).then(|| vec![(x1, y1)]);
            }
            return Some(arc((x0, y0), (x1, y1), (w.i.unwrap_or(0.0), w.j.unwrap_or(0.0)), g == 2));
        }
        _ => {}
    }
    None
}

/// Points along an arc from `a` to `b` around `a + ij` (G2 clockwise).
fn arc(a: (f64, f64), b: (f64, f64), ij: (f64, f64), clockwise: bool) -> Vec<(f64, f64)> {
    use std::f64::consts::TAU;
    let c = (a.0 + ij.0, a.1 + ij.1);
    let r = ij.0.hypot(ij.1);
    if r < 1e-6 {
        return vec![b];
    }
    let a0 = (a.1 - c.1).atan2(a.0 - c.0);
    let mut a1 = (b.1 - c.1).atan2(b.0 - c.0);
    let same = (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
    if clockwise {
        if same || a1 >= a0 {
            a1 -= TAU;
        }
    } else if same || a1 <= a0 {
        a1 += TAU;
    }
    let sweep = a1 - a0;
    let n = ((sweep.abs() * r / ARC_STEP).ceil() as usize).clamp(1, 360);
    let mut pts: Vec<(f64, f64)> = (1..n).map(|k| a0 + sweep * k as f64 / n as f64).map(|t| (c.0 + r * t.cos(), c.1 + r * t.sin())).collect();
    pts.push(b);
    pts
}

/// Copy the plate's G-code out of the sliced 3MF (once) and index it.
pub fn load_index(sliced: &Path, entry: &str, dir: &Path, plate: u32) -> anyhow::Result<(PathBuf, Index)> {
    let meta = std::fs::metadata(sliced).with_context(|| format!("reading {}", sliced.display()))?;
    let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let key = format!("{plate}-{stamp}-{}", meta.len());
    let gcode = dir.join(format!("layers-{key}.gcode"));
    let index_path = dir.join(format!("layers-v{INDEX_VERSION}-{key}.json"));
    if let Ok(bytes) = std::fs::read(&index_path)
        && let Ok(index) = serde_json::from_slice::<Index>(&bytes)
        && index.version == INDEX_VERSION
        && gcode.exists()
    {
        return Ok((gcode, index));
    }
    // Earlier copies are for an older slice.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("layers-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut zip = zip::ZipArchive::new(File::open(sliced)?).context("the sliced file isn't a 3MF")?;
    let mut src = zip.by_name(entry).with_context(|| format!("{entry} is missing from the sliced file"))?;
    let tmp = gcode.with_extension("part");
    let mut out = std::io::BufWriter::new(File::create(&tmp)?);
    std::io::copy(&mut src, &mut out)?;
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, &gcode)?;

    let index = build_index(BufReader::new(File::open(&gcode)?))?;
    let tmp = index_path.with_extension("part");
    std::fs::write(&tmp, serde_json::to_vec(&index)?)?;
    std::fs::rename(&tmp, &index_path)?;
    Ok((gcode, index))
}

pub fn build_index(mut r: impl BufRead) -> anyhow::Result<Index> {
    let mut m = Machine::default();
    let mut features = Features(vec![]);
    let mut layers: Vec<Layer> = vec![];
    let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut line = String::new();
    let mut offset = 0u64;
    loop {
        line.clear();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let t = line.trim();
        if t == "; CHANGE_LAYER" {
            if let Some(last) = layers.last_mut() {
                last.end = offset;
            }
            layers.push(Layer { z: 0.0, height: 0.0, start: offset, end: 0, machine: Some(m) });
        } else if let Some(v) = t.strip_prefix("; Z_HEIGHT:") {
            if let (Some(l), Ok(z)) = (layers.last_mut(), v.trim().parse()) {
                l.z = z;
            }
        } else if let Some(v) = t.strip_prefix("; LAYER_HEIGHT:") {
            if let (Some(l), Ok(h)) = (layers.last_mut(), v.trim().parse::<f64>())
                && l.height == 0.0
            {
                l.height = h;
            }
        } else if let start = (m.x, m.y)
            && let Some(pts) = step(&mut m, &line, &mut features)
            && !layers.is_empty()
        {
            for (x, y) in std::iter::once(start).chain(pts) {
                bounds = [bounds[0].min(x), bounds[1].min(y), bounds[2].max(x), bounds[3].max(y)];
            }
        }
        offset += n as u64;
    }
    if let Some(last) = layers.last_mut() {
        last.end = offset;
    }
    if layers.is_empty() {
        bail!("the G-code has no layers");
    }
    if bounds[0] > bounds[2] {
        bounds = [0.0, 0.0, 0.0, 0.0];
    }
    Ok(Index { version: INDEX_VERSION, layers, bounds, features: features.0 })
}

/// Extrusion paths of one layer.
pub fn read_layer(gcode: &Path, index: &Index, n: usize) -> anyhow::Result<Vec<PathOut>> {
    let layer = index.layers.get(n).context("no such layer")?;
    let mut f = File::open(gcode)?;
    f.seek(SeekFrom::Start(layer.start))?;
    let r = BufReader::new(f.take(layer.end - layer.start));
    // Feature ids must match the index, so start from its list.
    let mut features = Features(index.features.clone());
    Ok(paths(r, layer.machine.unwrap_or_default(), &mut features))
}

fn paths(r: impl BufRead, mut m: Machine, features: &mut Features) -> Vec<PathOut> {
    let mut out: Vec<PathOut> = vec![];
    let q = |v: f64| (v * 100.0).round() as i32;
    for line in r.lines().map_while(Result::ok) {
        let (x0, y0) = (m.x, m.y);
        let Some(pts) = step(&mut m, &line, features) else { continue };
        let start = [q(x0), q(y0)];
        let joined = out.last().is_some_and(|p| p.feature == m.feature && p.tool == m.tool && p.points[p.points.len() - 2..] == start);
        if !joined {
            out.push(PathOut { feature: m.feature, tool: m.tool, points: start.to_vec() });
        }
        let path = out.last_mut().expect("just pushed");
        for (x, y) in pts {
            path.points.extend([q(x), q(y)]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
; start gcode
G90
M83
G1 X10 Y10 E2
; CHANGE_LAYER
; Z_HEIGHT: 0.2
; LAYER_HEIGHT: 0.2
; FEATURE: Outer wall
G1 X0 Y0 F600
G1 X10 Y0 E.5
G1 X10 Y10 E.5
G1 X20 Y20
T1
; FEATURE: Sparse infill
G3 X30 Y20 I5 J0 E.4
; CHANGE_LAYER
; Z_HEIGHT: 0.4
; LAYER_HEIGHT: 0.2
G2 I1 J0 E.3
";

    #[test]
    fn index_and_layers() {
        let index = build_index(SAMPLE.as_bytes()).unwrap();
        assert_eq!(index.layers.len(), 2);
        assert_eq!(index.layers[1].z, 0.4);
        assert_eq!(index.features, ["Outer wall", "Sparse infill"]);
        // The start G-code's purge isn't part of any layer.
        assert_eq!(index.bounds[0], 0.0);
        // Layer 2 is a circle of radius 1 around (31, 20).
        assert!((index.bounds[2] - 32.0).abs() < 0.05);

        let l0 = &SAMPLE[index.layers[0].start as usize..index.layers[0].end as usize];
        let p = paths(l0.as_bytes(), index.layers[0].machine.unwrap(), &mut Features(index.features.clone()));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].points, [0, 0, 1000, 0, 1000, 1000]);
        assert_eq!((p[1].feature, p[1].tool), (1, 1));
        // Half circle over the top from (20,20) to (30,20) around (25,20).
        let mid = p[1].points.len() / 2;
        assert!(p[1].points[mid + 1] > 2000 || p[1].points[mid - 1] > 2000);
        assert_eq!(p[1].points[p[1].points.len() - 2..], [3000, 2000]);

        // Layer 2 starts where layer 1 ended, with tool 1: a full circle.
        let l1 = &SAMPLE[index.layers[1].start as usize..index.layers[1].end as usize];
        let p = paths(l1.as_bytes(), index.layers[1].machine.unwrap(), &mut Features(index.features.clone()));
        assert_eq!(p[0].tool, 1);
        assert_eq!(p[0].points[..2], [3000, 2000]);
        assert_eq!(p[0].points[p[0].points.len() - 2..], [3000, 2000]);
        assert!(p[0].points.len() > 10);
    }

    #[test]
    fn relative_moves_and_absolute_e() {
        let mut m = Machine::default();
        let mut f = Features(vec![]);
        step(&mut m, "M82", &mut f);
        step(&mut m, "G91", &mut f);
        assert_eq!(step(&mut m, "G1 X5 Y5 E1", &mut f), Some(vec![(5.0, 5.0)]));
        // Absolute E going back is a retract, not extrusion.
        assert_eq!(step(&mut m, "G1 X5 E0.5", &mut f), None);
        // Extruding without moving is a purge, not a line.
        step(&mut m, "M83", &mut f);
        assert_eq!(step(&mut m, "G1 E5", &mut f), None);
        assert_eq!((m.x, m.y), (10.0, 5.0));
    }
}
