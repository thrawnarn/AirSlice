export type Source = "makerworld" | "printables" | "thingiverse" | "upload" | "external";

export interface FilamentInfo {
  type: string;
  color: string;
}

export interface ModelFile {
  id: string;
  name: string;
  kind: string;
  size_bytes: number | null;
  thumbnail: string | null;
  printer: string | null;
  compatible: string[];
  plates: number | null;
  need_ams: boolean | null;
  filaments: FilamentInfo[];
  folder?: string | null;
  /** What the source says the profile takes. */
  prediction_secs?: number | null;
  weight_g?: number | null;
}

export interface ResolvedModel {
  source: Source;
  model_id: string;
  url: string;
  title: string;
  author: string | null;
  cover: string | null;
  files: ModelFile[];
  preselected_file_id: string | null;
  notes: string[];
  details?: ModelDetails | null;
}

/** A model page's extras; the description is plain text blocks. */
export interface ModelDetails {
  summary: string | null;
  description: { kind: "p" | "h" | "li"; text: string }[];
  images: string[];
  tags: string[];
  categories: string[];
  likes: number | null;
  downloads: number | null;
  prints: number | null;
  collections: number | null;
  license: string | null;
  published_at: number | null;
}

export type JobState =
  | "downloading"
  | "ready"
  | "slicing"
  | "sliced"
  | "uploading"
  | "starting"
  | "queued"
  | "started"
  | "failed"
  | "waiting"
  | "finished";

/** A Spoolman spool. */
export interface Spool {
  id: number;
  name: string;
  material: string;
  color: string;
  remaining_g: number | null;
}

export interface Cost {
  filament: number | null;
  machine: number | null;
  total: number;
  currency: string;
  actual: boolean;
}

export interface CostSettings {
  currency: string;
  price_per_kg: number | null;
  per_hour: number | null;
}

/** Push notifications through Home Assistant (Settings). */
export interface NotifySettings {
  enabled: boolean;
  ha_url: string;
  /** Only sent when changing it; the server never returns it. */
  token?: string;
  has_token?: boolean;
  /** Running as the Home Assistant add-on: no address or token needed. */
  in_addon?: boolean;
  open_url: string;
  targets: NotifyTarget[];
}

/** One notification: where it goes and which events it carries. */
export interface NotifyTarget {
  id: string;
  name: string;
  enabled: boolean;
  /** "ha" (Home Assistant), "telegram", "ntfy" or "webpush" (a browser). */
  channel: "ha" | "telegram" | "ntfy" | "webpush";
  service: string;
  /** Only sent when changing it; the server never returns it. */
  telegram_token?: string;
  has_telegram_token?: boolean;
  telegram_chat: string;
  ntfy_url: string;
  ntfy_token?: string;
  has_ntfy_token?: boolean;
  /** Web Push: the browser's push address (set by the browser subscribing). */
  push_endpoint?: string;
  /** Add a camera picture when the printer has a camera. */
  photos: boolean;
  /** Language of the notifications; "" = Home Assistant's. */
  language?: "" | "en" | "sv";
  sliced: boolean;
  printing: boolean;
  finished: boolean;
  problems: boolean;
  queue: boolean;
  first_layer: boolean;
  attention: boolean;
}

/** Retries and waiting for offline printers (Settings). */
export interface ConnectionSettings {
  wait_minutes: number;
  upload_attempts: number;
  retry_delay_secs: number;
  start_timeout_secs: number;
  reconnect_max_secs: number;
}

export interface PlateInfo {
  /** Filament slots this plate uses, when the file tells. */
  filaments?: number[] | null;
  index: number;
  name: string | null;
  has_thumbnail: boolean;
  object_count: number;
  /** The objects on the plate (3MF ids), in the project's order. */
  objects?: { id: string; name: string }[];
  /** Print time and filament, from the file or an earlier slice. */
  prediction_secs?: number | null;
  weight_g?: number | null;
}

export interface Inspection {
  kind: "project" | "sliced" | "mesh";
  plates: PlateInfo[];
  printer_model: string | null;
  printer_profile: string | null;
  filaments: FilamentInfo[];
}

export interface SlicedFilament {
  id: number;
  type: string;
  color: string;
  used_g: number;
  used_m: number;
}

export interface SliceOutcome {
  /** The files on the plate, when models were added. */
  files?: SlicedFile[];
  /** What was asked for; added models are in here. */
  options?: { copies?: number | null; also_plates?: number[]; extras?: { id: string; copies?: number | null; plate?: number | null; tray_id?: number | null }[] } | null;
  printer_id: string;
  /** Setting changes made for this slice. */
  tweaks?: SliceTweaks;
  plate: number;
  gcode_entry: string;
  filament_slots: number;
  filaments: SlicedFilament[];
  prediction_secs: number | null;
  weight_g: number | null;
  settings: "project" | "printer" | "presliced";
  transform?: MeshTransform | null;
  filament_profile?: string | null;
  preferred_trays?: number[] | null;
  /** Build plate the G-code was made for (BuildPlate value). */
  bed_type?: string | null;
}

export interface MeshTransform {
  scale_percent?: number;
  copies?: number;
  rotate_z?: number;
  auto_orient?: boolean;
}

export interface QueueEntry {
  id: string;
  job_id: string;
  printer_id: string;
  /** The printer profile it's sliced for; any printer with it can take it. */
  group: string;
  added_at: number;
  position: number;
  title: string;
  cover: string | null;
  file_name: string;
  plate: number | null;
  prediction_secs: number | null;
  weight_g: number | null;
  filaments: SlicedFilament[];
  has_thumbnail: boolean;
  plate_label?: string | null;
  /** Came sliced: can't be sliced again for another printer type. */
  presliced: boolean;
  filament_ok: boolean;
  filament_warnings?: string[];
}

export interface Tray {
  id: number;
  ams: number | null;
  slot: number | null;
  type: string;
  color: string;
  name: string | null;
  remain: number | null;
  /** Full spool weight in grams, from the RFID tag. */
  spool_g?: number | null;
  info_idx?: string | null;
  /** RFID tag id of the spool. */
  uuid?: string | null;
  active: boolean;
  /** Drying values from the spool's RFID tag. */
  dry_temp?: number | null;
  dry_hours?: number | null;
}

export interface TrayFilament {
  tray_id: number;
  type: string;
  color: string;
  profile: string | null;
  exact: boolean;
}

export interface Suggestion {
  ams_mapping: number[];
  use_ams: boolean;
  warnings: string[];
  complete: boolean;
}

/** One file in a slice and the filament slots it prints with. */
export interface SlicedFile {
  id?: string | null;
  title: string;
  cover?: string | null;
  slots: number[];
  copies: number;
}

/** A model added to a job's plate. */
export interface PlateExtra {
  id: string;
  title: string;
  source: Source;
  model_url: string;
  cover: string | null;
  kind: "mesh" | "project" | "sliced";
  plates: PlateInfo[];
  multicolor: boolean;
}

export interface Job {
  extras?: PlateExtra[];
  id: string;
  created_at: number;
  updated_at: number;
  state: JobState;
  /** While slicing: how far OrcaSlicer has come. */
  progress?: { percent: number; message: string } | null;
  /** While waiting for an offline printer: when it gives up (unix seconds). */
  waiting_until?: number | null;
  error: string | null;
  log: string[];
  source: Source;
  model_url: string;
  model_id: string;
  model_title: string;
  model_cover: string | null;
  file: ModelFile;
  inspection: Inspection | null;
  slice: SliceOutcome | null;
  print: {
    printer_id: string;
    remote_file: string;
    ams_mapping: number[];
    use_ams: boolean;
    started_at: number;
    outcome?: "finished" | "failed" | "cancelled" | "unknown" | null;
    ended_at?: number | null;
    progress?: number | null;
    /** What the plug's energy meter counted during the print. */
    energy_kwh?: number | null;
  } | null;
  /** Spoolman spools in the printer's trays (tray id -> spool). */
  spools?: Record<string, Spool> | null;
  cost_estimate?: Cost | null;
  cost?: Cost | null;
  /** Downloaded and sliced files were removed to save space. */
  files_removed?: boolean;
  /** A camera picture of the result was kept (`api/jobs/<id>/photo.jpg`). */
  photo?: boolean;
  /** While printing: the printer's live progress for this print. */
  live?: { state: string | null; percent: number | null; remaining_minutes: number | null; layer: number | null; total_layers: number | null } | null;
  auto: boolean;
  favorite: boolean;
  copied_from?: string | null;
  plate_label?: string | null;
  /** Sent to the printer: the next plate of the file nobody printed or queued. */
  next_plate?: PlateInfo | null;
  suggestion?: Suggestion;
  trays?: Tray[];
}

export interface Status {
  connected: boolean;
  last_error: string | null;
  last_update: number | null;
  state: string | null;
  task_name: string | null;
  percent: number | null;
  remaining_minutes: number | null;
  layer: number | null;
  total_layers: number | null;
  nozzle_temp: number | null;
  nozzle_target: number | null;
  bed_temp: number | null;
  bed_target: number | null;
  chamber_light: boolean | null;
  /** Print speed: 1 silent, 2 standard, 3 sport, 4 ludicrous. */
  speed_level?: number | null;
  print_error: number | null;
  /** Active HMS messages. */
  /** Active HMS messages: `text` in English, `texts` per language ("en", "sv"). */
  problems?: { code: string; severity: "fatal" | "serious" | "common" | "info"; text: string | null; texts?: Record<string, string>; url: string }[];
  has_ams: boolean;
  trays: Tray[];
  external: Tray | null;
  ams_units?: AmsUnit[];
  /** Wi-Fi signal in dBm. */
  wifi_dbm?: number | null;
  /** Fan speeds in percent. */
  fan_part?: number | null;
  fan_aux?: number | null;
  fan_chamber?: number | null;
  nozzle_diameter?: string | null;
  fan_heatbreak?: number | null;
  nozzle_type?: string | null;
  speed_percent?: number | null;
  /** `stg_cur`: what the printer is doing within a print. */
  stage?: number | null;
  print_type?: string | null;
  gcode_file?: string | null;
  sdcard?: boolean | null;
}

export interface AmsUnit {
  id: number;
  name: string;
  humidity_pct: number | null;
  humidity_level: number | null;
  temp: number | null;
  /** Drying time left in minutes, while drying. */
  dry_minutes?: number | null;
  model?: string | null;
  /** Highest drying temperature in °C; none when the unit can't dry. */
  dry_max_temp?: number | null;
  /** Feeds the running print, so the printer won't dry it. */
  in_print?: boolean;
}

/** A job as the printer card shows it. */
export interface JobRef {
  id: string;
  title: string;
  plate_label: string | null;
  thumbnail_plate: number | null;
  cover: string | null;
  photo: boolean;
  outcome: "finished" | "failed" | "cancelled" | "unknown" | null;
  started_at: number | null;
  ended_at: number | null;
}

export type FtpDataMode = "auto" | "private" | "clear";

export interface Printer {
  id: string;
  name: string;
  host: string;
  serial: string;
  has_access_code: boolean;
  printer_model: string;
  machine_profile: string;
  process_profile: string | null;
  filament_profile: string | null;
  ftp_data_mode: FtpDataMode;
  /** Build plates the owner has (BuildPlate values); empty = not set. */
  plates: string[];
  default_plate: string | null;
  /** "" (none), "printer" (its own camera) or "ha" (a Home Assistant camera). */
  camera: "" | "printer" | "ha";
  camera_entity: string | null;
  /** Power plug (Home Assistant switch) and the sensors found on its device. */
  power_switch?: string | null;
  power_sensor?: string | null;
  energy_sensor?: string | null;
  /** Turn the plug off this long after a print; 0 = never. */
  power_off_after_min?: number;
  /** False for models known to ignore the command: drying only from their screen. */
  can_start_drying?: boolean;
  /** `on_secs`: seconds since the plug went on while the printer hasn't answered; `off_in`: seconds to auto-off. */
  power?: { on: boolean | null; watts: number | null; off_in: number | null; on_secs?: number | null } | null;
  enabled: boolean;
  is_default: boolean;
  queue_length: number;
  status: Status;
  current_job?: JobRef | null;
  last_job?: JobRef | null;
}

export interface MachineProfile {
  name: string;
  printer_model: string;
  nozzle: string;
}

/** A build plate OrcaSlicer knows; `value` is what the slicer and AirSlice store. */
export interface BuildPlate {
  value: string;
  label: string;
}

export interface NamedProfile {
  name: string;
  filament_type?: string;
  /** Bambu filament id, e.g. "GFA00"; matches the spool's RFID. */
  filament_id?: string;
}

export interface ProfileChoices {
  processes: { name: string }[];
  filaments: NamedProfile[];
  default_process: string | null;
}

export interface ApiKey {
  id: string;
  name: string;
  prefix: string;
  created_at: number;
  last_used_at: number | null;
}

export interface Info {
  version: string;
  slicer: {
    available: boolean;
    path: string;
    version: string | null;
    profiles_loaded: boolean;
    profiles_error: string | null;
  };
  bambu: { linked: boolean; email: string | null; region: string };
  base_path: string;
}

/** Quick changes to the print settings; unset keeps the profile's own. */
export interface SliceTweaks {
  layer_height?: number;
  walls?: number;
  infill?: number;
  supports?: "off" | "normal" | "tree";
  brim?: boolean;
}

export interface SlicePreset extends SliceTweaks {
  name: string;
}

/** Start page hint: the next plate of the last print, when nothing is queued. */
export interface NextPlateSuggestion {
  job_id: string;
  model_title: string;
  model_cover: string | null;
  has_photo: boolean;
  plate: PlateInfo;
  plates: number;
}
