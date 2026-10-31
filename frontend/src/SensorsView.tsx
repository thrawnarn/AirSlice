import type { Printer } from "./types";
import { printerState, timeAgo } from "./ui";
import { locale, tr } from "./i18n";

/** `stg_cur` values P1/X1/A1 printers report most; others show their number. */
const STAGES: Record<number, string> = {
  0: tr("Printing"),
  1: tr("Levelling the bed"),
  2: tr("Heating the bed"),
  3: tr("Vibration compensation"),
  4: tr("Changing filament"),
  5: tr("Paused"),
  6: tr("Paused: filament ran out"),
  7: tr("Heating the nozzle"),
  8: tr("Calibrating extrusion"),
  9: tr("Scanning the bed"),
  10: tr("Inspecting the first layer"),
  11: tr("Identifying the build plate"),
  12: tr("Calibrating the micro lidar"),
  13: tr("Homing"),
  14: tr("Cleaning the nozzle tip"),
  15: tr("Checking the nozzle temperature"),
  16: tr("Paused by you"),
  17: tr("Paused: front cover fell off"),
  18: tr("Calibrating the micro lidar"),
  19: tr("Calibrating extrusion flow"),
  20: tr("Paused: nozzle temperature fault"),
  21: tr("Paused: bed temperature fault"),
  22: tr("Unloading filament"),
  23: tr("Paused: skipped step"),
  24: tr("Loading filament"),
  25: tr("Calibrating motor noise"),
  26: tr("Paused: AMS lost"),
  27: tr("Paused: heatbreak fan too slow"),
  29: tr("Cooling the chamber"),
  30: tr("Paused by the G-code"),
  32: tr("Paused: filament on the nozzle"),
  34: tr("Paused: first layer error"),
  35: tr("Paused: nozzle clogged"),
};

const NOZZLES: Record<string, string> = {
  hardened_steel: tr("Hardened steel"),
  stainless_steel: tr("Stainless steel"),
  tungsten_carbide: tr("Tungsten carbide"),
};

function wifiQuality(dbm: number): string {
  return dbm >= -60 ? tr("Good") : dbm >= -70 ? tr("Fair") : tr("Weak");
}

function deg(now: number | null | undefined, target: number | null | undefined) {
  if (now == null) return null;
  const n = `${now.toLocaleString(locale, { maximumFractionDigits: 1 })} °C`;
  return target != null && target > 0 ? tr("{0}, target {1} °C", [n, Math.round(target)]) : n;
}

function pct(v: number | null | undefined) {
  return v == null ? null : v === 0 ? tr("Off") : `${v} %`;
}

type Row = [string, string | null | undefined];

function Group({ title, rows }: { title: string; rows: Row[] }) {
  const shown = rows.filter(([, v]) => v != null && v !== "");
  if (shown.length === 0) return null;
  return (
    <section className="panel sensor-group">
      <h2 className="small">{title}</h2>
      <dl className="sensor-list">
        {shown.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd className="num">{v}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

export function SensorsView({ printers, id }: { printers: Printer[] | null; id: string }) {
  const printer = printers?.find((p) => p.id === id);
  if (!printers) return <main className="page" />;
  if (!printer)
    return (
      <main className="page">
        <a href="#/printers" className="settings-back">
          {tr("‹ Printers")}
        </a>
        <p className="faint">{tr("This printer is gone.")}</p>
      </main>
    );
  const s = printer.status;
  const st = printerState(s, printer);
  const speed = s.speed_level ? [tr("Silent"), tr("Standard"), tr("Sport"), tr("Ludicrous")][s.speed_level - 1] : null;
  return (
    <main className="page">
      <a href="#/printers" className="settings-back">
        {tr("‹ Printers")}
      </a>
      <h1 className="page-title">{tr("{0}: sensors", [printer.name])}</h1>
      <p className="faint small" style={{ marginTop: -6 }}>
        {st.text}
        {s.last_update ? tr(", updated {0}", [timeAgo(s.last_update)]) : ""}
      </p>
      {printer.power && (
        <Group
          title={tr("Power")}
          rows={[
            [tr("Plug"), printer.power.on == null ? tr("Unknown") : printer.power.on ? tr("On") : tr("Off")],
            [tr("Drawing"), printer.power.watts != null ? `${printer.power.watts.toLocaleString(locale, { maximumFractionDigits: 1 })} W` : null],
            [tr("Turns off"), printer.power.off_in != null ? new Date(Date.now() + printer.power.off_in * 1000).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" }) : null],
            [tr("Switch"), printer.power_switch],
          ]}
        />
      )}
      {s.connected && (
        <>
          <Group
            title={tr("Temperatures")}
            rows={[
              [tr("Nozzle"), deg(s.nozzle_temp, s.nozzle_target)],
              [tr("Bed"), deg(s.bed_temp, s.bed_target)],
            ]}
          />
          <Group
            title={tr("Fans")}
            rows={[
              [tr("Part cooling"), pct(s.fan_part)],
              [tr("Auxiliary"), pct(s.fan_aux)],
              [tr("Chamber"), pct(s.fan_chamber)],
              [tr("Heatbreak"), pct(s.fan_heatbreak)],
            ]}
          />
          <Group
            title={tr("Print")}
            rows={[
              [tr("Stage"), s.stage != null ? (STAGES[s.stage] ?? tr("Stage {0}", [s.stage])) : null],
              [tr("Progress"), s.percent != null && s.state !== "IDLE" ? `${s.percent} %` : null],
              [tr("Layer"), s.layer != null && s.total_layers ? tr("{0} of {1}", [s.layer, s.total_layers]) : null],
              [tr("Speed"), speed ? (s.speed_percent ? `${speed} (${s.speed_percent} %)` : speed) : null],
              [tr("File"), s.gcode_file],
              [tr("Sent from"), s.print_type === "local" ? tr("This network") : s.print_type === "cloud" ? tr("Bambu cloud") : null],
            ]}
          />
          {(s.ams_units ?? []).map((u) => (
            <Group
              key={u.id}
              title={u.name}
              rows={[
                [tr("Humidity"), u.humidity_pct != null ? `${u.humidity_pct} %` : null],
                [tr("Dryness"), u.humidity_level != null ? tr("{0} of 5", [u.humidity_level]) : null],
                [tr("Temperature"), u.temp != null ? `${u.temp.toLocaleString(locale, { maximumFractionDigits: 1 })} °C` : null],
                [tr("Drying"), u.dry_minutes ? tr("{0} min left", [u.dry_minutes]) : null],
              ]}
            />
          ))}
          <Group
            title={tr("Hardware")}
            rows={[
              [tr("Nozzle"), [s.nozzle_diameter ? `${s.nozzle_diameter} mm` : null, s.nozzle_type ? (NOZZLES[s.nozzle_type] ?? s.nozzle_type) : null].filter(Boolean).join(", ")],
              [tr("SD card"), s.sdcard == null ? null : s.sdcard ? tr("Inserted") : tr("Missing")],
              ["Wi-Fi", s.wifi_dbm != null ? `${wifiQuality(s.wifi_dbm)} (${s.wifi_dbm} dBm)` : null],
              [tr("Address"), printer.host],
              [tr("Serial number"), printer.serial],
            ]}
          />
        </>
      )}
    </main>
  );
}
