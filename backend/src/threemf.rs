//! Reading Bambu/Orca 3MF projects: plates, thumbnails, settings, and the
//! slice results written by the slicer.

use crate::models::{FilamentInfo, Inspection, PlateInfo, PlateObject, SlicedFilament};
use anyhow::{Context, anyhow};
use quick_xml::events::{BytesStart, Event};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

// ------------------------------------------------------------------ tiny DOM

#[derive(Debug, Default, Clone)]
pub struct Node {
    pub name: String,
    pub attrs: HashMap<String, String>,
    pub children: Vec<Node>,
}

impl Node {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(String::as_str)
    }
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// `<metadata key="k" value="v"/>` children as a map.
    fn metadata(&self) -> HashMap<String, String> {
        self.children_named("metadata")
            .filter_map(|m| Some((m.attr("key")?.to_string(), m.attr("value").unwrap_or("").to_string())))
            .collect()
    }
}

fn element(e: &BytesStart<'_>) -> anyhow::Result<Node> {
    let mut node = Node { name: e.local_name().as_ref().to_string(), ..Default::default() };
    for a in e.attributes().flatten() {
        let key = a.key.local_name().as_ref().to_string();
        let value = unescape(&a.value);
        node.attrs.insert(key, value);
    }
    Ok(node)
}

fn unescape(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    raw.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn entry_names(zip: &zip::ZipArchive<std::fs::File>) -> Vec<String> {
    zip.file_names().filter_map(|n| n.ok().map(|c| c.into_owned())).collect()
}

pub fn parse_xml(text: &str) -> anyhow::Result<Node> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Node> = vec![Node { name: "#root".into(), ..Default::default() }];
    loop {
        match reader.read_event().context("invalid XML")? {
            Event::Start(e) => stack.push(element(&e)?),
            Event::Empty(e) => {
                let n = element(&e)?;
                stack.last_mut().unwrap().children.push(n);
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let n = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(n);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        let n = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(n);
    }
    Ok(stack.pop().unwrap())
}

// ------------------------------------------------------------------ archive

fn open(path: &Path) -> anyhow::Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    zip::ZipArchive::new(f).context("not a valid 3MF (zip) file")
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, limit: u64) -> Option<Vec<u8>> {
    let entry = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    entry.take(limit).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn read_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    read_entry(zip, name, 64 * 1024 * 1024).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn plate_gcode_index(name: &str) -> Option<u32> {
    name.strip_prefix("Metadata/plate_")?.strip_suffix(".gcode")?.parse().ok()
}

/// Normalize "FF0000", "#ff0000ff" -> "#FF0000".
pub fn hex_color(raw: &str) -> String {
    let hex: String = raw.trim().trim_start_matches('#').chars().take(6).collect();
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) { format!("#{}", hex.to_uppercase()) } else { String::new() }
}

fn json_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(serde_json::Value::Array(a)) => a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect(),
        Some(serde_json::Value::String(s)) => s.split(';').map(str::to_string).collect(),
        _ => vec![],
    }
}

/// Inspect a downloaded file. Meshes (STL/OBJ/STEP) are a single "plate".
pub fn inspect(path: &Path) -> anyhow::Result<Inspection> {
    let lower = path.to_string_lossy().to_lowercase();
    if !lower.ends_with(".3mf") {
        return Ok(Inspection {
            kind: "mesh".into(),
            plates: vec![PlateInfo { index: 1, name: None, has_thumbnail: false, object_count: 1, objects: vec![], prediction_secs: None, weight_g: None, filaments: None }],
            ..Default::default()
        });
    }
    let mut zip = open(path)?;
    let names: Vec<String> = entry_names(&zip);
    let sliced_plates: Vec<u32> = names.iter().filter_map(|n| plate_gcode_index(n)).collect();

    let mut plates: Vec<PlateInfo> = Vec::new();
    if let Some(xml) = read_text(&mut zip, "Metadata/model_settings.config")
        && let Ok(root) = parse_xml(&xml) {
            for config in root.children_named("config") {
                for plate in config.children_named("plate") {
                    let meta = plate.metadata();
                    let Some(index) = meta.get("plater_id").and_then(|v| v.parse::<u32>().ok()) else { continue };
                    let names: HashMap<String, String> = config
                        .children_named("object")
                        .filter_map(|o| Some((o.attr("id")?.to_string(), o.metadata().get("name")?.clone())))
                        .collect();
                    let objects: Vec<PlateObject> = plate
                        .children_named("model_instance")
                        .filter_map(|i| i.metadata().get("object_id").cloned())
                        .map(|id| PlateObject { name: names.get(&id).cloned().unwrap_or_else(|| format!("Object {id}")), id })
                        .collect();
                    plates.push(PlateInfo {
                        index,
                        name: meta.get("plater_name").filter(|n| !n.trim().is_empty()).cloned(),
                        has_thumbnail: false,
                        object_count: objects.len() as u32,
                        objects,
                        prediction_secs: None,
                        weight_g: None,
                        filaments: None,
                    });
                }
            }
        }
    if plates.is_empty() {
        // Plain 3MF from other tools, or a sliced file: derive from contents.
        let mut idx: Vec<u32> = if sliced_plates.is_empty() { vec![1] } else { sliced_plates.clone() };
        idx.sort_unstable();
        idx.dedup();
        plates = idx.into_iter().map(|i| PlateInfo { index: i, name: None, has_thumbnail: false, object_count: 0, objects: vec![], prediction_secs: None, weight_g: None, filaments: None }).collect();
    }
    // A sliced file can only print the plates that have G-code.
    if !sliced_plates.is_empty() {
        plates.retain(|p| sliced_plates.contains(&p.index));
    }
    // Plates without objects can't be printed.
    if plates.iter().any(|p| p.object_count > 0) {
        plates.retain(|p| p.object_count > 0 || sliced_plates.contains(&p.index));
    }
    for p in &mut plates {
        p.has_thumbnail = thumbnail_entry(&names, p.index).is_some();
    }
    // Estimates the file brings along (projects saved after slicing, and
    // every pre-sliced file).
    if let Some(xml) = read_text(&mut zip, "Metadata/slice_info.config")
        && let Ok(root) = parse_xml(&xml)
    {
        for plate in root.children_named("config").flat_map(|c| c.children_named("plate")) {
            let meta = plate.metadata();
            let Some(index) = meta.get("index").and_then(|v| v.parse::<u32>().ok()) else { continue };
            if let Some(p) = plates.iter_mut().find(|p| p.index == index) {
                p.prediction_secs = meta.get("prediction").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0).map(|v| v as u64);
                p.weight_g = meta.get("weight").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
            }
        }
    }
    plates.sort_by_key(|p| p.index);

    let mut out = Inspection {
        kind: if sliced_plates.is_empty() { "project".into() } else { "sliced".into() },
        plates,
        ..Default::default()
    };
    if let Some(text) = read_text(&mut zip, "Metadata/project_settings.config")
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            out.printer_model = v.get("printer_model").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
            out.printer_profile =
                v.get("printer_settings_id").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
            let types = json_list(&v, "filament_type");
            let colors = json_list(&v, "filament_colour");
            out.filaments = types
                .iter()
                .enumerate()
                .map(|(i, t)| FilamentInfo { kind: t.clone(), color: colors.get(i).map(|c| hex_color(c)).unwrap_or_default() })
                .collect();
            set_plate_filaments(&mut zip, &names, &mut out, &v);
        }
    Ok(out)
}

/// Settings that send a feature to a filament of its own, 0 meaning the object's.
const FEATURE_FILAMENTS: &[&str] = &["support_filament", "support_interface_filament", "wall_filament", "sparse_infill_filament", "solid_infill_filament"];

/// Work out each plate's filament slots from the objects on it. Left unknown
/// when something else can bring in a filament: painting, or a change per layer.
fn set_plate_filaments(zip: &mut zip::ZipArchive<std::fs::File>, names: &[String], out: &mut Inspection, project: &serde_json::Value) {
    let count = out.filaments.len() as u32;
    if count == 0 {
        return;
    }
    if read_text(zip, CUSTOM_GCODE).is_some_and(|t| regex::Regex::new(r#"\bextruder="[1-9]"#).is_ok_and(|r| r.is_match(&t))) {
        return;
    }
    if names.iter().filter(|n| n.ends_with(".model")).any(|n| has_painting(zip, n)) {
        return;
    }
    let Some(root) = read_text(zip, MODEL_SETTINGS).and_then(|x| parse_xml(&x).ok()) else { return };
    let number = |m: &HashMap<String, String>, key: &str| m.get(key).and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0);
    let mut by_object: HashMap<String, Vec<u32>> = HashMap::new();
    for object in root.children_named("config").flat_map(|c| c.children_named("object")) {
        let Some(id) = object.attr("id") else { continue };
        let meta = object.metadata();
        let own = number(&meta, "extruder");
        let mut slots = Vec::new();
        let mut parts = 0;
        for part in object.children_named("part") {
            parts += 1;
            let m = part.metadata();
            slots.extend(number(&m, "extruder").or(own));
            slots.extend(FEATURE_FILAMENTS.iter().filter_map(|k| number(&m, k)));
        }
        if parts == 0 {
            slots.extend(own);
        }
        slots.extend(FEATURE_FILAMENTS.iter().filter_map(|k| number(&meta, k)));
        by_object.insert(id.to_string(), slots);
    }
    let global: Vec<u32> = FEATURE_FILAMENTS.iter().flat_map(|k| json_list(project, k)).filter_map(|v| v.parse().ok()).filter(|n| *n > 0).collect();
    for plate in &mut out.plates {
        let mut slots: Vec<u32> = plate.objects.iter().filter_map(|o| by_object.get(&o.id)).flatten().chain(&global).copied().collect();
        // An object without a filament of its own takes the first.
        if plate.objects.iter().any(|o| by_object.get(&o.id).is_none_or(Vec::is_empty)) {
            slots.push(1);
        }
        slots.sort_unstable();
        slots.dedup();
        if !plate.objects.is_empty() && !slots.is_empty() && slots.iter().all(|s| *s <= count) {
            plate.filaments = Some(slots);
        }
    }
}

/// Whether a model file holds painted triangles, read in pieces since these files can be huge.
fn has_painting(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> bool {
    const MARKS: [&[u8]; 2] = [b"paint_color=", b"mmu_segmentation="];
    let Ok(mut file) = zip.by_name(name) else { return true };
    let mut buf = vec![0u8; 1 << 20];
    let mut kept = 0;
    loop {
        let Ok(read) = file.read(&mut buf[kept..]) else { return true };
        if read == 0 {
            return false;
        }
        let end = kept + read;
        if MARKS.iter().any(|m| buf[..end].windows(m.len()).any(|w| w == *m)) {
            return true;
        }
        // Keep the tail, in case a mark falls between two pieces.
        kept = 32.min(end);
        buf.copy_within(end - kept..end, 0);
    }
}

fn thumbnail_entry(names: &[String], plate: u32) -> Option<String> {
    [
        format!("Metadata/plate_{plate}.png"),
        format!("Metadata/top_{plate}.png"),
        format!("Metadata/plate_{plate}_small.png"),
        format!("Metadata/pick_{plate}.png"),
    ]
    .into_iter()
    .find(|c| names.contains(c))
    .or_else(|| (plate == 1).then(|| "Metadata/thumbnail.png".to_string()).filter(|c| names.contains(c)))
}

pub fn thumbnail(path: &Path, plate: u32) -> anyhow::Result<Vec<u8>> {
    let mut zip = open(path)?;
    let names: Vec<String> = entry_names(&zip);
    let entry = thumbnail_entry(&names, plate).ok_or_else(|| anyhow!("no thumbnail for plate {plate}"))?;
    read_entry(&mut zip, &entry, 16 * 1024 * 1024).ok_or_else(|| anyhow!("could not read thumbnail"))
}

#[derive(Debug, Clone, Default)]
pub struct PlateSlice {
    pub index: u32,
    pub gcode_entry: Option<String>,
    pub prediction_secs: Option<u64>,
    pub weight_g: Option<f64>,
    pub filaments: Vec<SlicedFilament>,
    /// Objects on the plate (every copy counts).
    pub object_count: u32,
}

/// Read the slicer's per-plate results from a sliced 3MF.
pub fn slice_results(path: &Path) -> anyhow::Result<(Vec<PlateSlice>, u32)> {
    let mut zip = open(path)?;
    let names: Vec<String> = entry_names(&zip);
    let gcode_entries: Vec<(u32, String)> = names.iter().filter_map(|n| Some((plate_gcode_index(n)?, n.clone()))).collect();
    let mut plates: Vec<PlateSlice> = Vec::new();

    if let Some(xml) = read_text(&mut zip, "Metadata/slice_info.config") {
        let root = parse_xml(&xml)?;
        for config in root.children_named("config") {
            for plate in config.children_named("plate") {
                let meta = plate.metadata();
                let Some(index) = meta.get("index").and_then(|v| v.parse().ok()) else { continue };
                let filaments = plate
                    .children_named("filament")
                    .filter_map(|f| {
                        Some(SlicedFilament {
                            id: f.attr("id")?.parse().ok()?,
                            kind: f.attr("type").unwrap_or("").to_string(),
                            color: hex_color(f.attr("color").unwrap_or("")),
                            used_g: f.attr("used_g").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                            used_m: f.attr("used_m").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                        })
                    })
                    .collect();
                plates.push(PlateSlice {
                    index,
                    gcode_entry: gcode_entries.iter().find(|(i, _)| *i == index).map(|(_, n)| n.clone()),
                    prediction_secs: meta.get("prediction").and_then(|v| v.parse::<f64>().ok()).map(|v| v as u64),
                    weight_g: meta.get("weight").and_then(|v| v.parse().ok()),
                    filaments,
                    object_count: plate.children_named("object").count() as u32,
                });
            }
        }
    }
    // Fallback when slice_info is missing: G-code entries only.
    for (i, name) in &gcode_entries {
        if !plates.iter().any(|p| p.index == *i) {
            plates.push(PlateSlice { index: *i, gcode_entry: Some(name.clone()), ..Default::default() });
        }
    }
    plates.sort_by_key(|p| p.index);

    // Number of filament slots in the project = length of the AMS mapping.
    let mut slots = 0u32;
    if let Some(text) = read_text(&mut zip, "Metadata/project_settings.config")
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            slots = json_list(&v, "filament_type").len() as u32;
        }
    let max_used = plates.iter().flat_map(|p| p.filaments.iter().map(|f| f.id)).max().unwrap_or(1);
    Ok((plates, slots.max(max_used)))
}

/// One project setting, e.g. `curr_bed_type`.
pub fn project_setting(path: &Path, key: &str) -> Option<String> {
    let mut zip = open(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&read_text(&mut zip, "Metadata/project_settings.config")?).ok()?;
    json_list(&v, key).into_iter().next().filter(|s| !s.is_empty())
}

/// Copies the project at `src` to `dest` with project settings changed:
/// `drop` keys are removed (also from per-object overrides) so the slicer
/// falls back to its own defaults, `set` keys get a new value. Every other
/// entry is copied unchanged.
pub fn edit_settings(src: &Path, dest: &Path, drop: &[String], set: &[(String, String)]) -> anyhow::Result<()> {
    rewrite(src, dest, &[PROJECT_SETTINGS, MODEL_SETTINGS], |name, text| {
        Ok(if name == PROJECT_SETTINGS {
            let mut v: serde_json::Value = serde_json::from_str(&text).context("reading project settings")?;
            if let Some(map) = v.as_object_mut() {
                for k in drop {
                    map.remove(k);
                }
                for (k, value) in set {
                    map.insert(k.clone(), serde_json::Value::String(value.clone()));
                }
            }
            serde_json::to_vec_pretty(&v)?
        } else {
            // One `<metadata key=".." value=".."/>` per line.
            let kept: Vec<&str> = text
                .lines()
                .filter(|l| !drop.iter().any(|k| l.trim_start().starts_with(&format!("<metadata key=\"{k}\""))))
                .collect();
            kept.join("\n").into_bytes()
        })
    })
}

/// Copies the project with only the `keep` objects (ids) left on `plate`.
/// The others stay in the file but no longer belong to the plate, which is
/// how OrcaSlicer leaves them out (removing their build item breaks loading).
pub fn keep_plate_objects(src: &Path, dest: &Path, plate: u32, keep: &[String]) -> anyhow::Result<()> {
    let instance = regex::Regex::new(r#"(?s)\s*<model_instance>.*?</model_instance>"#)?;
    let object_id = regex::Regex::new(r#"key="object_id" value="([^"]*)""#)?;
    let plate_block = regex::Regex::new(r#"(?s)<plate>.*?</plate>"#)?;
    let plate_id = format!(r#"key="plater_id" value="{plate}""#);
    rewrite(src, dest, &[MODEL_SETTINGS], |_, text| {
        let edited = plate_block.replace_all(&text, |p: &regex::Captures| {
            let block = &p[0];
            if !block.contains(&plate_id) {
                return block.to_string();
            }
            instance
                .replace_all(block, |m: &regex::Captures| {
                    let id = object_id.captures(&m[0]).map(|c| c[1].to_string()).unwrap_or_default();
                    if keep.contains(&id) { m[0].to_string() } else { String::new() }
                })
                .into_owned()
        });
        Ok(edited.into_owned().into_bytes())
    })
}

/// How many filaments a project has.
pub fn filament_count(path: &Path) -> Option<usize> {
    let mut zip = open(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&read_text(&mut zip, PROJECT_SETTINGS)?).ok()?;
    v.get("filament_colour")?.as_array().map(Vec::len)
}

/// Copies the project with the objects of `plates` (and only those) on plate 1,
/// `copies` times, and room for `extra_slots` more filaments. OrcaSlicer's arrange
/// gathers every plate's objects onto one, it has no way to repeat a project
/// without slicing it, and it crashes when given more filaments than the project
/// has. Other objects keep their mesh files but lose their build item, settings
/// and assembly entries; removing only some of those fails to load.
pub fn single_plate(src: &Path, dest: &Path, plates: &[u32], copies: u32, extra_slots: usize) -> anyhow::Result<()> {
    let copies = copies.max(1) as usize;
    let plate_block = regex::Regex::new(r#"(?s)\s*<plate>.*?</plate>"#)?;
    let plater_id = regex::Regex::new(r#"key="plater_id" value="(\d+)""#)?;
    let instance_block = regex::Regex::new(r#"(?s)\s*<model_instance>.*?</model_instance>"#)?;
    let instance_id = regex::Regex::new(r#"(key="instance_id" value=")\d+"#)?;
    let object_id = regex::Regex::new(r#"key="object_id" value="([^"]*)""#)?;
    let number_of = |block: &str| plater_id.captures(block).and_then(|c| c[1].parse::<u32>().ok());
    let settings = {
        let mut zip = open(src)?;
        read_text(&mut zip, MODEL_SETTINGS).unwrap_or_default()
    };
    // The plate's instances, repeated. Without plates in the file, everything is on the one plate.
    let chosen: Vec<&str> = plates.iter().filter_map(|p| plate_block.find_iter(&settings).map(|b| b.as_str()).find(|b| number_of(b) == Some(*p))).collect();
    let instances: Vec<&str> = chosen.iter().flat_map(|b| instance_block.find_iter(b).map(|m| m.as_str())).collect();
    let keep: Option<Vec<String>> = (!chosen.is_empty()).then(|| instances.iter().filter_map(|i| object_id.captures(i).map(|c| c[1].to_string())).collect());
    let merged_plate = chosen.first().map(|first| {
        let bare = instance_block.replace_all(first, "");
        let numbered = plater_id.replace(&bare, r#"key="plater_id" value="1""#);
        let mut all = String::new();
        for copy in 0..copies {
            for i in &instances {
                all.push_str(&instance_id.replace(i, |c: &regex::Captures| format!("{}{copy}", &c[1])));
            }
        }
        numbered.replace("</plate>", &format!("{all}\n  </plate>"))
    });
    let object = regex::Regex::new(r#"(?s)\s*<object id="([^"]*)">.*?</object>"#)?;
    let assembled = regex::Regex::new(r#"\s*<assemble_item object_id="([^"]*)"[^>]*/>"#)?;
    let item = regex::Regex::new(r#"\s*<item objectid="([^"]*)"[^>]*/>"#)?;
    let kept = |c: &regex::Captures, keep: &[String]| if keep.contains(&c[1].to_string()) { c[0].to_string() } else { String::new() };
    rewrite(src, dest, &[MODEL_SETTINGS, "3D/3dmodel.model", PROJECT_SETTINGS], |name, text| {
        Ok(match (name, &keep) {
            (PROJECT_SETTINGS, _) => {
                let mut v: serde_json::Value = serde_json::from_str(&text).context("reading project settings")?;
                pad_filaments(&mut v, extra_slots);
                serde_json::to_vec_pretty(&v)?
            }
            (MODEL_SETTINGS, Some(keep)) => {
                let mut placed = false;
                let text = plate_block.replace_all(&text, |p: &regex::Captures| {
                    if placed || !plates.contains(&number_of(&p[0]).unwrap_or(0)) {
                        return String::new();
                    }
                    placed = true;
                    merged_plate.clone().unwrap_or_default()
                });
                let text = object.replace_all(&text, |c: &regex::Captures| kept(c, keep));
                assembled
                    .replace_all(&text, |c: &regex::Captures| {
                        let line = kept(c, keep);
                        // Each copy needs the object's own assembly entry; a part's has no instance.
                        if line.contains(r#"instance_id="0""#) { (0..copies).map(|n| line.replace(r#"instance_id="0""#, &format!(r#"instance_id="{n}""#))).collect::<String>() } else { line }
                    })
                    .into_owned()
                    .into_bytes()
            }
            (_, Some(keep)) => item.replace_all(&text, |c: &regex::Captures| kept(c, keep).repeat(copies)).into_owned().into_bytes(),
            (_, None) => text.into_bytes(),
        })
    })
}

/// How many objects each plate of a project holds, in plate order.
pub fn plate_instance_counts(path: &Path) -> Vec<u32> {
    let Ok(mut zip) = open(path) else { return vec![] };
    let settings = read_text(&mut zip, MODEL_SETTINGS).unwrap_or_default();
    let Ok(block) = regex::Regex::new(r#"(?s)<plate>.*?</plate>"#) else { return vec![] };
    block.find_iter(&settings).map(|b| b.as_str().matches("<model_instance>").count() as u32).collect()
}

/// Make room for more filaments in project settings: per-filament values copy
/// the last filament's, and the flush tables grow.
fn pad_filaments(v: &mut serde_json::Value, extra: usize) {
    use serde_json::Value;
    let Some(map) = v.as_object_mut() else { return };
    let n = map.get("filament_colour").and_then(|c| c.as_array()).map_or(0, Vec::len);
    if extra == 0 || n == 0 {
        return;
    }
    let total = n + extra;
    let text = |s: String| Value::String(s);
    for (key, value) in map.iter_mut() {
        let Some(list) = value.as_array_mut() else { continue };
        match key.as_str() {
            "filament_self_index" => *list = (1..=total).map(|i| text(i.to_string())).collect(),
            // The process's entry, one per filament, the printer's.
            "different_settings_to_system" if !list.is_empty() => {
                let at = list.len() - 1;
                list.splice(at..at, std::iter::repeat_n(text(String::new()), extra));
            }
            "flush_volumes_matrix" if list.len() == n * n => {
                let old = std::mem::take(list);
                for row in 0..total {
                    for col in 0..total {
                        list.push(match (row == col, row < n && col < n) {
                            (true, _) => text("0".into()),
                            (_, true) => old[row * n + col].clone(),
                            _ => text(DEFAULT_FLUSH.into()),
                        });
                    }
                }
            }
            "flush_volumes_vector" if list.len() == 2 * n => {
                list.extend(std::iter::repeat_n(text(DEFAULT_FLUSH_MULTIPLIER.into()), 2 * extra));
            }
            key if (crate::filament_keys::PER_FILAMENT.contains(&key) || key.starts_with("filament_")) && !list.is_empty() && list.len() % n == 0 => {
                let group = list.len() / n;
                let last: Vec<Value> = list[list.len() - group..].to_vec();
                for _ in 0..extra {
                    list.extend(last.iter().cloned());
                }
            }
            _ => {}
        }
    }
}

/// Copies the project with filament slots merged, `map[slot - 1]` being the slot
/// each one becomes: for several slots printing from one tray. Objects, parts and
/// layer changes move to the merged slot, and nothing is purged between merged
/// slots. Painted triangles keep their slot, so those still change filament, but
/// without purging.
pub fn merge_filaments(src: &Path, dest: &Path, map: &[u32]) -> anyhow::Result<()> {
    let slot_of = |old: &str| old.parse::<usize>().ok().and_then(|s| s.checked_sub(1)).and_then(|i| map.get(i)).map(u32::to_string).unwrap_or_else(|| old.to_string());
    let extruder = regex::Regex::new(r#"(<metadata key="extruder" value=")(\d+)"#)?;
    let layer_change = regex::Regex::new(r#"(\bextruder=")(\d+)"#)?;
    rewrite(src, dest, &[MODEL_SETTINGS, PROJECT_SETTINGS, CUSTOM_GCODE], |name, text| {
        Ok(match name {
            MODEL_SETTINGS => extruder.replace_all(&text, |c: &regex::Captures| format!("{}{}", &c[1], slot_of(&c[2]))).into_owned().into_bytes(),
            CUSTOM_GCODE => layer_change.replace_all(&text, |c: &regex::Captures| format!("{}{}", &c[1], slot_of(&c[2]))).into_owned().into_bytes(),
            _ => {
                let mut v: serde_json::Value = serde_json::from_str(&text).context("reading project settings")?;
                if let Some(list) = v.get_mut("flush_volumes_matrix").and_then(|m| m.as_array_mut())
                    && list.len() == map.len() * map.len() {
                        let n = map.len();
                        for row in 0..n {
                            for col in 0..n {
                                if map[row] == map[col] {
                                    list[row * n + col] = serde_json::Value::String("0".into());
                                }
                            }
                        }
                    }
                serde_json::to_vec_pretty(&v)?
            }
        })
    })
}

/// Flush volume in mm³ between a new filament and any other.
const DEFAULT_FLUSH: &str = "280";
const DEFAULT_FLUSH_MULTIPLIER: &str = "140";

/// Copies the project, setting the filament slot of each object `slot_of`
/// knows by name.
pub fn set_object_slots(src: &Path, dest: &Path, slot_of: impl Fn(&str) -> Option<u32>) -> anyhow::Result<()> {
    let object = regex::Regex::new(r#"(?s)<object id="[^"]*">.*?</object>"#)?;
    let name = regex::Regex::new(r#"<metadata key="name" value="([^"]*)""#)?;
    let extruder = regex::Regex::new(r#"(<metadata key="extruder" value=")[^"]*"#)?;
    rewrite(src, dest, &[MODEL_SETTINGS], |_, text| {
        let edited = object.replace_all(&text, |o: &regex::Captures| {
            match name.captures(&o[0]).and_then(|c| slot_of(&c[1])) {
                Some(slot) => extruder.replacen(&o[0], 1, |e: &regex::Captures| format!("{}{slot}", &e[1])).into_owned(),
                None => o[0].to_string(),
            }
        });
        Ok(edited.into_owned().into_bytes())
    })
}

/// A plate's objects as numbers in the order of the file's build items, which
/// is how OrcaSlicer numbers the files of `--export-stl` (`obj_1_…`). `None`
/// when the file doesn't say which objects are on which plate.
pub fn plate_object_numbers(path: &Path, plate: u32) -> Option<Vec<usize>> {
    let mut zip = open(path).ok()?;
    let settings = read_text(&mut zip, MODEL_SETTINGS)?;
    let model = read_text(&mut zip, "3D/3dmodel.model")?;
    let block = regex::Regex::new(r#"(?s)<plate>.*?</plate>"#).ok()?;
    let plate_id = format!(r#"key="plater_id" value="{plate}""#);
    let found = block.find_iter(&settings).find(|b| b.as_str().contains(&plate_id))?;
    let object_id = regex::Regex::new(r#"key="object_id" value="([^"]*)""#).ok()?;
    let on_plate: Vec<String> = object_id.captures_iter(found.as_str()).map(|c| c[1].to_string()).collect();
    let item = regex::Regex::new(r#"<item objectid="([^"]*)""#).ok()?;
    Some(
        item.captures_iter(&model)
            .enumerate()
            .filter(|(_, c)| on_plate.contains(&c[1].to_string()))
            .map(|(i, _)| i + 1)
            .collect(),
    )
}

const PROJECT_SETTINGS: &str = "Metadata/project_settings.config";
const CUSTOM_GCODE: &str = "Metadata/custom_gcode_per_layer.xml";
const MODEL_SETTINGS: &str = "Metadata/model_settings.config";

/// Copies a 3MF, passing the named text entries through `edit`; everything
/// else is copied as it is.
fn rewrite(src: &Path, dest: &Path, names: &[&str], mut edit: impl FnMut(&str, String) -> anyhow::Result<Vec<u8>>) -> anyhow::Result<()> {
    let mut zip = open(src)?;
    let out = std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut w = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for i in 0..zip.len() {
        let name = zip.by_index_raw(i)?.name()?.into_owned();
        if names.contains(&name.as_str()) {
            let text = read_text(&mut zip, &name).unwrap_or_default();
            let data = edit(&name, text)?;
            w.start_file(name, opts)?;
            std::io::Write::write_all(&mut w, &data)?;
        } else {
            w.raw_copy_file(zip.by_index_raw(i)?)?;
        }
    }
    w.finish()?;
    Ok(())
}

/// The settings OrcaSlicer refused, from lines like
/// `raft_first_layer_expansion: -1 not in range [0,2147483647]`.
pub fn rejected_settings(slicer_output: &str) -> Vec<String> {
    let mut keys: Vec<String> = slicer_output
        .lines()
        .filter(|l| l.contains(" not in range ["))
        .filter_map(|l| l.split_once(':').map(|(k, _)| k.trim()))
        .filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(str::to_string)
        .collect();
    keys.dedup();
    keys
}

/// The slicer's output file (first .3mf in `dir`).
pub fn find_output(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.extension().map(|e| e.eq_ignore_ascii_case("3mf")).unwrap_or(false)
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;

    fn two_plate_project(path: &Path) {
        let settings = r#"<config>
  <object id="4">
    <metadata key="name" value="one"/>
    <metadata key="extruder" value="1"/>
  </object>
  <object id="7">
    <metadata key="name" value="airslice-e0_1.stl"/>
    <metadata key="extruder" value="1"/>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
    <model_instance><metadata key="object_id" value="4"/></model_instance>
  </plate>
  <plate>
    <metadata key="plater_id" value="2"/>
    <model_instance><metadata key="object_id" value="7"/></model_instance>
  </plate>
  <assemble>
   <assemble_item object_id="4" instance_id="0"/>
   <assemble_item object_id="7" instance_id="0"/>
  </assemble>
</config>"#;
        let model = r#"<build><item objectid="7" printable="1"/><item objectid="4" printable="1"/></build>"#;
        let project = serde_json::json!({
            "filament_colour": ["#111111", "#222222"],
            "nozzle_temperature": ["220", "230"],
            "filament_dev_ams_drying_time": ["1", "2", "3", "4"],
            "filament_self_index": ["1", "2"],
            "different_settings_to_system": ["process", "a", "b", "printer"],
            "flush_volumes_matrix": ["0", "5", "6", "0"],
            "flush_volumes_vector": ["1", "2", "3", "4"],
            "nozzle_diameter": ["0.4", "0.4"],
            "printable_area": ["0x0", "256x0"],
        });
        make_3mf(path, &[
            ("Metadata/model_settings.config", settings.as_bytes()),
            ("3D/3dmodel.model", model.as_bytes()),
            ("Metadata/project_settings.config", project.to_string().as_bytes()),
        ]);
    }

    fn entry(path: &Path, name: &str) -> String {
        read_text(&mut open(path).unwrap(), name).unwrap()
    }

    #[test]
    fn single_plate_keeps_one_plate_and_adds_filaments() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dest) = (dir.join("a.3mf"), dir.join("b.3mf"));
        two_plate_project(&src);
        single_plate(&src, &dest, &[2], 1, 1).unwrap();

        let settings = entry(&dest, super::MODEL_SETTINGS);
        assert_eq!(settings.matches("<plate>").count(), 1);
        assert!(settings.contains(r#"key="plater_id" value="1""#));
        assert!(settings.contains(r#"<object id="7">"#) && !settings.contains(r#"<object id="4">"#));
        assert!(settings.contains(r#"assemble_item object_id="7""#) && !settings.contains(r#"assemble_item object_id="4""#));
        let model = entry(&dest, "3D/3dmodel.model");
        assert!(model.contains(r#"objectid="7""#) && !model.contains(r#"objectid="4""#));

        let v: serde_json::Value = serde_json::from_str(&entry(&dest, PROJECT_SETTINGS)).unwrap();
        let list = |k: &str| v[k].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(list("filament_colour"), ["#111111", "#222222", "#222222"]);
        assert_eq!(list("nozzle_temperature"), ["220", "230", "230"]);
        assert_eq!(list("filament_dev_ams_drying_time"), ["1", "2", "3", "4", "3", "4"]);
        assert_eq!(list("filament_self_index"), ["1", "2", "3"]);
        assert_eq!(list("different_settings_to_system"), ["process", "a", "b", "", "printer"]);
        assert_eq!(list("flush_volumes_matrix"), ["0", "5", "280", "6", "0", "280", "280", "280", "0"]);
        assert_eq!(list("flush_volumes_vector").len(), 6);
        // A printer's settings aren't per filament, even at the same length.
        assert_eq!(list("nozzle_diameter").len(), 2);
        assert_eq!(list("printable_area").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn with_plates(settings: &str, model: &[u8], project: serde_json::Value) -> Inspection {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.3mf");
        make_3mf(&path, &[
            ("Metadata/model_settings.config", settings.as_bytes()),
            ("3D/Objects/o.model", model),
            ("Metadata/project_settings.config", project.to_string().as_bytes()),
        ]);
        let out = inspect(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    const PLATES: &str = r#"<config>
  <object id="1"><metadata key="name" value="a"/><metadata key="extruder" value="1"/></object>
  <object id="2"><metadata key="name" value="b"/><metadata key="extruder" value="2"/>
    <part id="1"><metadata key="name" value="p"/></part>
    <part id="2"><metadata key="name" value="q"/><metadata key="extruder" value="3"/></part>
  </object>
  <plate><metadata key="plater_id" value="1"/><model_instance><metadata key="object_id" value="1"/></model_instance></plate>
  <plate><metadata key="plater_id" value="2"/><model_instance><metadata key="object_id" value="2"/></model_instance></plate>
</config>"#;

    #[test]
    fn knows_each_plates_filaments() {
        let project = serde_json::json!({"filament_type": ["PLA", "PLA", "PLA", "PLA"], "filament_colour": ["#1", "#2", "#3", "#4"]});
        let out = with_plates(PLATES, b"<model/>", project.clone());
        assert_eq!(out.plates[0].filaments, Some(vec![1]));
        assert_eq!(out.plates[1].filaments, Some(vec![2, 3]));
        // A support filament counts on every plate.
        let mut with_support = project.clone();
        with_support["support_filament"] = serde_json::json!("4");
        assert_eq!(with_plates(PLATES, b"<model/>", with_support).plates[0].filaments, Some(vec![1, 4]));
        // Painting can use any filament, so it stays unknown.
        let out = with_plates(PLATES, br#"<triangle v1="0" paint_color="8"/>"#, project);
        assert_eq!(out.plates[0].filaments, None);
    }

    #[test]
    fn merges_filaments() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dest) = (dir.join("a.3mf"), dir.join("b.3mf"));
        let settings = r#"<object id="1"><metadata key="extruder" value="2"/><part id="1"><metadata key="extruder" value="3"/></part></object>"#;
        let layers = r##"<layer top_z="1" type="2" extruder="3" color="#111111"/>"##;
        let project = serde_json::json!({"flush_volumes_matrix": ["0","1","2", "3","0","4", "5","6","0"]});
        make_3mf(&src, &[
            ("Metadata/model_settings.config", settings.as_bytes()),
            ("Metadata/custom_gcode_per_layer.xml", layers.as_bytes()),
            ("Metadata/project_settings.config", project.to_string().as_bytes()),
        ]);
        merge_filaments(&src, &dest, &[1, 1, 3]).unwrap();
        let s = entry(&dest, super::MODEL_SETTINGS);
        assert!(s.contains(r#"<object id="1"><metadata key="extruder" value="1"/>"#) && s.contains(r#"<part id="1"><metadata key="extruder" value="3"/>"#));
        assert!(entry(&dest, CUSTOM_GCODE).contains(r#"extruder="3""#));
        let v: serde_json::Value = serde_json::from_str(&entry(&dest, PROJECT_SETTINGS)).unwrap();
        // Slots 1 and 2 are one now; 3 still purges from and to them.
        assert_eq!(v["flush_volumes_matrix"], serde_json::json!(["0","0","2", "0","0","4", "5","6","0"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_plate_combines_plates_and_repeats_them() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dest) = (dir.join("a.3mf"), dir.join("b.3mf"));
        two_plate_project(&src);
        single_plate(&src, &dest, &[2, 1], 2, 0).unwrap();
        let settings = entry(&dest, super::MODEL_SETTINGS);
        assert_eq!(settings.matches("<plate>").count(), 1);
        assert!(settings.contains(r#"key="plater_id" value="1""#) && !settings.contains(r#"key="plater_id" value="2""#));
        // Both plates' objects, twice each.
        assert_eq!(settings.matches("<model_instance>").count(), 4);
        assert_eq!(settings.matches(r#"assemble_item object_id="7" instance_id="1""#).count(), 1);
        assert_eq!(entry(&dest, "3D/3dmodel.model").matches("<item ").count(), 4);
        assert_eq!(plate_instance_counts(&dest), [4]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sets_the_filament_of_named_objects() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dest) = (dir.join("a.3mf"), dir.join("b.3mf"));
        two_plate_project(&src);
        set_object_slots(&src, &dest, |n| n.starts_with("airslice-e0_").then_some(3)).unwrap();
        let settings = entry(&dest, super::MODEL_SETTINGS);
        assert!(settings.contains("value=\"airslice-e0_1.stl\"/>\n    <metadata key=\"extruder\" value=\"3\""));
        assert!(settings.contains("value=\"one\"/>\n    <metadata key=\"extruder\" value=\"1\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn numbers_a_plates_objects_by_build_order() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.3mf");
        two_plate_project(&src);
        assert_eq!(plate_object_numbers(&src, 1), Some(vec![2]));
        assert_eq!(plate_object_numbers(&src, 2), Some(vec![1]));
        assert_eq!(plate_object_numbers(&src, 3), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    pub fn make_3mf(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    const MODEL_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="2"><metadata key="name" value="Benchy"/></object>
  <plate>
    <metadata key="plater_id" value="1"/>
    <metadata key="plater_name" value="Hull"/>
    <metadata key="thumbnail_file" value="Metadata/plate_1.png"/>
    <model_instance><metadata key="object_id" value="2"/></model_instance>
  </plate>
  <plate>
    <metadata key="plater_id" value="2"/>
    <metadata key="plater_name" value=""/>
    <model_instance><metadata key="object_id" value="3"/></model_instance>
    <model_instance><metadata key="object_id" value="4"/></model_instance>
  </plate>
  <plate><metadata key="plater_id" value="3"/></plate>
</config>"#;

    const SLICE_INFO: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <header><header_item key="X-BBL-Client-Type" value="slicer"/></header>
  <plate>
    <metadata key="index" value="2"/>
    <metadata key="prediction" value="3045"/>
    <metadata key="weight" value="12.50"/>
    <object identify_id="1" name="a" skipped="false"/>
    <filament id="1" tray_info_idx="GFA00" type="PLA" color="#ffffff" used_m="3.98" used_g="11.86"/>
    <filament id="3" type="PETG" color="#00FF00FF" used_m="0.2" used_g="0.64"/>
  </plate>
</config>"##;

    #[test]
    fn inspects_project() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("model.3mf");
        let settings = br##"{"printer_model":"Bambu Lab A1","printer_settings_id":"Bambu Lab A1 0.4 nozzle","filament_type":["PLA","PETG"],"filament_colour":["#FF0000","#00ff00"]}"##;
        make_3mf(&p, &[
            ("Metadata/model_settings.config", MODEL_SETTINGS.as_bytes()),
            ("Metadata/project_settings.config", settings),
            ("Metadata/plate_1.png", b"png"),
        ]);
        let i = inspect(&p).unwrap();
        assert_eq!(i.kind, "project");
        assert_eq!(i.plates.len(), 2, "empty plate 3 is dropped");
        assert_eq!(i.plates[0].name.as_deref(), Some("Hull"));
        assert!(i.plates[0].has_thumbnail);
        assert_eq!(i.plates[1].object_count, 2);
        assert!(!i.plates[1].has_thumbnail);
        assert_eq!(i.printer_model.as_deref(), Some("Bambu Lab A1"));
        assert_eq!(i.filaments[1], FilamentInfo { kind: "PETG".into(), color: "#00FF00".into() });
        assert_eq!(thumbnail(&p, 1).unwrap(), b"png");
        assert!(thumbnail(&p, 2).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_slice_results() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("out.3mf");
        make_3mf(&p, &[
            ("Metadata/slice_info.config", SLICE_INFO.as_bytes()),
            ("Metadata/plate_2.gcode", b"G28"),
            ("Metadata/project_settings.config", br#"{"filament_type":["PLA","PLA","PETG","PLA"]}"#),
        ]);
        let (plates, slots) = slice_results(&p).unwrap();
        assert_eq!(slots, 4);
        assert_eq!(plates.len(), 1);
        let pl = &plates[0];
        assert_eq!(pl.index, 2);
        assert_eq!(pl.gcode_entry.as_deref(), Some("Metadata/plate_2.gcode"));
        assert_eq!(pl.prediction_secs, Some(3045));
        assert_eq!(pl.object_count, 1);
        assert_eq!(pl.filaments.len(), 2);
        assert_eq!(pl.filaments[0].color, "#FFFFFF");
        assert_eq!(pl.filaments[1].id, 3);
        assert_eq!(pl.filaments[1].color, "#00FF00");
        let i = inspect(&p).unwrap();
        assert_eq!(i.kind, "sliced");
        assert_eq!(i.plates.iter().map(|p| p.index).collect::<Vec<_>>(), vec![2]);
        assert_eq!((i.plates[0].prediction_secs, i.plates[0].weight_g), (Some(3045), Some(12.5)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn edits_project_settings() {
        // As printed by OrcaSlicer for a Bambu Studio 2.8 project.
        let out = "Param values in 3mf/config error: \n\
            raft_first_layer_expansion: -1 not in range [0,2147483647]\n\
            wall_filament: 0 not in range [1,2147483647]\n\
            run found error, return -18, exit...";
        let keys = rejected_settings(out);
        assert_eq!(keys, vec!["raft_first_layer_expansion", "wall_filament"]);

        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dest) = (dir.join("in.3mf"), dir.join("out.3mf"));
        let objects = "<config>\n  <object id=\"2\">\n    <metadata key=\"name\" value=\"Rex\"/>\n    <metadata key=\"wall_filament\" value=\"0\"/>\n  </object>\n</config>";
        make_3mf(&src, &[
            ("3D/3dmodel.model", b"<model/>"),
            ("Metadata/project_settings.config", br#"{"raft_first_layer_expansion":"-1","wall_filament":"0","layer_height":"0.2"}"#),
            ("Metadata/model_settings.config", objects.as_bytes()),
        ]);
        edit_settings(&src, &dest, &keys, &[("curr_bed_type".into(), "Textured PEI Plate".into())]).unwrap();
        let mut z = open(&dest).unwrap();
        let project: serde_json::Value = serde_json::from_str(&read_text(&mut z, "Metadata/project_settings.config").unwrap()).unwrap();
        assert_eq!(project, serde_json::json!({ "layer_height": "0.2", "curr_bed_type": "Textured PEI Plate" }));
        assert_eq!(project_setting(&dest, "curr_bed_type").as_deref(), Some("Textured PEI Plate"));
        assert_eq!(project_setting(&dest, "nope"), None);
        let objects = read_text(&mut z, "Metadata/model_settings.config").unwrap();
        assert!(objects.contains("value=\"Rex\"") && !objects.contains("wall_filament"));
        assert_eq!(read_text(&mut z, "3D/3dmodel.model").unwrap(), "<model/>");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_chosen_objects_on_a_plate() {
        let dir = std::env::temp_dir().join(format!("as-3mf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dest) = (dir.join("in.3mf"), dir.join("out.3mf"));
        make_3mf(&src, &[("Metadata/model_settings.config", MODEL_SETTINGS.as_bytes())]);
        let i = inspect(&src).unwrap();
        let ids: Vec<&str> = i.plates[1].objects.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["3", "4"]);
        assert_eq!(i.plates[0].objects[0].name, "Benchy");
        keep_plate_objects(&src, &dest, 2, &["4".to_string()]).unwrap();
        let after = inspect(&dest).unwrap();
        assert_eq!(after.plates[1].objects.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), vec!["4"]);
        // Other plates are left alone.
        assert_eq!(after.plates[0].object_count, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mesh_is_one_plate() {
        let i = inspect(Path::new("/nope/part.stl")).unwrap();
        assert_eq!(i.kind, "mesh");
        assert_eq!(i.plates.len(), 1);
    }
}
