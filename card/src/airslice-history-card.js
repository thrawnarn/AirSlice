import { LitElement, css, html, nothing } from "lit";
import { langOf, t } from "./i18n.js";

const CARD = "airslice-history-card";
const PAGE = 15;

const OUTCOMES = {
  finished: { icon: "mdi:check-circle", tone: "ok", label: "Finished" },
  failed: { icon: "mdi:close-circle", tone: "bad", label: "Failed" },
  cancelled: { icon: "mdi:stop-circle", tone: "warn", label: "Stopped" },
  unknown: { icon: "mdi:help-circle", tone: "", label: "Unknown result" },
};

const number = (lang, value, digits = 0) => new Intl.NumberFormat(lang, { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(value);

export function grams(lang, g) {
  return g >= 1000 ? `${number(lang, g / 1000, 2)} kg` : `${number(lang, g)} g`;
}

const money = (lang, value, currency) => `${number(lang, value, 2)}${currency ? ` ${currency}` : ""}`;

export function minutes(m) {
  if (!Number.isFinite(m) || m <= 0) return "";
  const rounded = Math.max(1, Math.round(m));
  const h = Math.floor(rounded / 60);
  return h ? `${h} h ${rounded % 60} min` : `${rounded} min`;
}

export function duration(print) {
  return print.started && print.ended ? (new Date(print.ended) - new Date(print.started)) / 60000 : NaN;
}

function when(lang, iso) {
  const d = iso ? new Date(iso) : null;
  return d && !Number.isNaN(d.getTime()) ? d.toLocaleString(lang, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) : "";
}

/** Every word has to be in the name, as the integration's own search. */
export function matching(prints, search) {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  return prints.filter((p) => words.every((w) => p.name.toLowerCase().includes(w)));
}

const safeColor = (c) => (/^#[0-9a-f]{3,8}$/i.test(c || "") ? c : "var(--disabled-color)");
const httpUrl = (u) => (/^https?:\/\//i.test(u || "") ? u : null);

export class AirSliceHistoryCard extends LitElement {
  static properties = {
    hass: { attribute: false },
    _config: { state: true },
    _prints: { state: true },
    _error: { state: true },
    _search: { state: true },
    _shown: { state: true },
    _open: { state: true },
    _photos: { state: true },
    _broken: { state: true },
  };

  constructor() {
    super();
    this._prints = null;
    this._search = "";
    this._photos = {};
    this._broken = {};
    this._asked = new Set();
    this._request = 0;
  }

  static getStubConfig(hass) {
    const entity = Object.values(hass?.entities ?? {}).find((e) => e.platform === "airslice" && e.entity_id.startsWith("sensor."));
    return { entity: entity?.entity_id ?? "" };
  }

  static getConfigForm() {
    const lang = document.documentElement.lang?.startsWith("sv") ? "sv" : "en";
    const labels = { entity: "Print sensor", title: "Title", page_size: "Prints shown at first", show_summary: "Show the totals" };
    return {
      schema: [
        { name: "entity", required: true, selector: { entity: { domain: "sensor", filter: [{ integration: "airslice" }] } } },
        { name: "title", selector: { text: {} } },
        { name: "page_size", selector: { number: { min: 1, max: 200, mode: "box" } } },
        { name: "show_summary", selector: { boolean: {} } },
      ],
      computeLabel: (s) => t(lang, labels[s.name] ?? s.name),
      computeHelper: (s) => (s.name === "entity" ? t(lang, "The sensor AirSlice has for each printer.") : undefined),
    };
  }

  setConfig(config) {
    this._config = { show_summary: true, ...config };
    this._shown = this._pageSize;
    this._seen = undefined;
  }

  get _pageSize() {
    return Number(this._config?.page_size) > 0 ? Number(this._config.page_size) : PAGE;
  }

  getCardSize() {
    return 6;
  }

  getGridOptions() {
    return { columns: 12, min_columns: 6, rows: "auto" };
  }

  // `hass` changes with every state in the house; only this sensor matters.
  shouldUpdate(changed) {
    if (changed.size !== 1 || !changed.has("hass")) return true;
    const before = changed.get("hass");
    const entity = this._config?.entity;
    return !before || before.states[entity] !== this.hass.states[entity] || langOf(before) !== langOf(this.hass);
  }

  willUpdate() {
    const state = this.hass?.states[this._config?.entity];
    if (!state) return;
    // A new print, or one that ended, changes these.
    const seen = `${state.state}|${state.attributes.latest_id}|${state.attributes.latest_outcome}`;
    if (seen !== this._seen) {
      this._seen = seen;
      this._load();
    }
  }

  updated() {
    this._askPhotos();
  }

  async _load() {
    const request = ++this._request;
    try {
      const res = await this.hass.callWS({
        type: "call_service",
        domain: "airslice",
        service: "list_prints",
        service_data: { entity_id: this._config.entity },
        return_response: true,
      });
      if (request !== this._request) return;
      this._prints = res.response?.prints ?? [];
      this._error = undefined;
    } catch (e) {
      if (request === this._request) this._error = e.message || String(e);
    }
  }

  get _found() {
    return matching(this._prints ?? [], this._search);
  }

  /** Photos are signed links, asked for only when a row is on screen. */
  async _askPhotos() {
    for (const p of this._found.slice(0, this._shown)) {
      if (!p.photo || this._asked.has(p.photo)) continue;
      this._asked.add(p.photo);
      try {
        const r = await this.hass.callWS({ type: "media_source/resolve_media", media_content_id: p.photo });
        this._photos = { ...this._photos, [p.photo]: r.url };
      } catch {
        // No photo after all: the row keeps its picture or icon.
      }
    }
  }

  _setSearch(e) {
    this._search = e.target.value;
    this._shown = this._pageSize;
  }

  _picture(print) {
    const url = (print.photo && this._photos[print.photo]) || httpUrl(print.cover);
    return url && !this._broken[url] ? url : null;
  }

  _imgFailed(url) {
    this._broken = { ...this._broken, [url]: true };
  }

  render() {
    if (!this._config) return nothing;
    const lang = langOf(this.hass);
    const state = this.hass?.states[this._config.entity];
    return html`<ha-card>
      <div class="head">
        <div class="title">${this._config.title || t(lang, "Print history")}</div>
        ${state && this._config.show_summary ? this._renderSummary(lang, state) : nothing}
      </div>
      ${this._renderBody(lang, state)}
    </ha-card>`;
  }

  _renderSummary(lang, state) {
    const a = state.attributes;
    const count = Number(state.state);
    if (!count) return nothing;
    const currency = this._prints?.find((p) => p.cost)?.cost.currency ?? "";
    const parts = [
      count === 1 ? t(lang, "1 print") : t(lang, "{0} prints", [number(lang, count)]),
      a.hours ? `${number(lang, a.hours)} h` : "",
      a.grams ? grams(lang, a.grams) : "",
      a.cost ? money(lang, a.cost, currency) : "",
    ];
    return html`<div class="muted summary">${parts.filter(Boolean).join(" · ")}</div>`;
  }

  _renderBody(lang, state) {
    if (!this._config.entity) return html`<div class="message muted">${t(lang, "Choose an AirSlice print sensor in the card's settings.")}</div>`;
    if (!state) return html`<div class="message muted">${t(lang, "{0} isn't available. Is the AirSlice integration set up?", [this._config.entity])}</div>`;
    if (this._error) return html`<div class="message error">${t(lang, "Couldn't read the history: {0}", [this._error])}</div>`;
    if (this._prints === null) return html`<div class="message muted">${t(lang, "Loading…")}</div>`;
    if (!this._prints.length) return html`<div class="message muted">${t(lang, "No prints yet.")}</div>`;
    const found = this._found;
    return html`
      ${this._prints.length > this._pageSize
        ? html`<div class="search">
            <input type="search" .value=${this._search} placeholder=${t(lang, "Search prints")} aria-label=${t(lang, "Search prints")} @input=${this._setSearch} />
          </div>`
        : nothing}
      ${found.length ? nothing : html`<div class="message muted">${t(lang, "No prints match.")}</div>`}
      <ul class="list">
        ${found.slice(0, this._shown).map((p) => this._renderPrint(lang, p))}
      </ul>
      ${found.length > this._shown
        ? html`<button class="more" @click=${() => (this._shown += this._pageSize)}>${t(lang, "Show more")}</button>`
        : nothing}
    `;
  }

  _renderPrint(lang, p) {
    const outcome = OUTCOMES[p.outcome] ?? OUTCOMES.unknown;
    const open = this._open === p.id;
    const picture = this._picture(p);
    const usage = (p.usage ?? []).filter((u) => u.grams > 0);
    return html`<li>
      <button class="row" aria-expanded=${open ? "true" : "false"} aria-label=${`${p.name}, ${t(lang, "Show details")}`} @click=${() => (this._open = open ? undefined : p.id)}>
        <span class="thumb">
          ${picture ? html`<img src=${picture} alt="" loading="lazy" @error=${() => this._imgFailed(picture)} />` : html`<ha-icon icon="mdi:printer-3d"></ha-icon>`}
        </span>
        <span class="main">
          <span class="name">${p.name}</span>
          <span class="muted small">${[when(lang, p.ended), minutes(duration(p))].filter(Boolean).join(" · ")}</span>
          <span class="meta small">
            <span class="outcome ${outcome.tone}"><ha-icon icon=${outcome.icon}></ha-icon>${t(lang, outcome.label)}</span>
            ${p.weight ? html`<span>${grams(lang, p.weight)}</span>` : nothing}
            ${p.cost ? html`<span>${money(lang, p.cost.total, p.cost.currency)}</span>` : nothing}
            ${usage.map((u) => html`<span class="swatch" style="background:${safeColor(u.color)}" title=${`${u.slot ?? ""} ${u.type ?? ""}`.trim()}></span>`)}
          </span>
        </span>
      </button>
      ${open ? this._renderDetails(lang, p, picture, usage) : nothing}
    </li>`;
  }

  _renderDetails(lang, p, picture, usage) {
    const stopped = p.outcome === "failed" || p.outcome === "cancelled";
    const waste = p.waste && p.waste.flush_g + p.waste.tower_g > 0 ? p.waste : null;
    const model = httpUrl(p.model_url);
    const row = (label, value) => html`<div class="fact"><span class="muted">${label}</span><span>${value}</span></div>`;
    const currency = p.cost?.currency;
    return html`<div class="details">
      ${picture ? html`<img class="photo" src=${picture} alt="" @error=${() => this._imgFailed(picture)} />` : nothing}
      ${p.error ? html`<div class="error">${p.error}</div>` : nothing}
      ${p.external ? html`<div class="muted">${t(lang, "Started outside AirSlice")}</div>` : nothing}
      ${stopped && p.progress != null ? row(t(lang, "Stopped at {0} %", [p.progress]), p.total_layers ? t(lang, "Layer {0} of {1}", [p.layer ?? 0, p.total_layers]) : "") : nothing}
      ${p.started ? row(t(lang, "Started"), when(lang, p.started)) : nothing}
      ${p.ended ? row(t(lang, "Ended"), when(lang, p.ended)) : nothing}
      ${usage.map((u) => row(html`<span class="swatch" style="background:${safeColor(u.color)}"></span> ${[u.slot, u.type].filter(Boolean).join(" ")}`, grams(lang, u.grams)))}
      ${waste
        ? row(
            html`${t(lang, "Purge and prime tower")}${waste.changes ? html`, ${waste.changes === 1 ? t(lang, "1 filament change") : t(lang, "{0} filament changes", [waste.changes])}` : nothing}`,
            `${grams(lang, waste.flush_g + waste.tower_g)}${waste.cost != null ? ` · ${money(lang, waste.cost, currency)}` : ""}`,
          )
        : nothing}
      ${p.cost?.filament != null ? row(t(lang, "Filament"), money(lang, p.cost.filament, currency)) : nothing}
      ${p.cost?.machine != null ? row(t(lang, "Machine and power"), money(lang, p.cost.machine, currency)) : nothing}
      ${p.cost ? row(t(lang, p.cost.actual ? "Total" : "Estimated cost"), money(lang, p.cost.total, currency)) : nothing}
      ${p.energy_kwh != null ? row(t(lang, "Energy"), `${number(lang, p.energy_kwh, 2)} kWh`) : nothing}
      ${model ? html`<a class="link" href=${model} target="_blank" rel="noopener noreferrer">${t(lang, "Open the model page")}</a>` : nothing}
    </div>`;
  }

  static styles = css`
    ha-card { padding: 16px 0 8px; }
    .head { padding: 0 16px 8px; }
    .title { font-size: var(--ha-font-size-l, 18px); font-weight: 500; color: var(--primary-text-color); }
    .muted { color: var(--secondary-text-color); }
    .small, .summary { font-size: var(--ha-font-size-s, 12px); }
    .error { color: var(--error-color); }
    .message { padding: 8px 16px 16px; }
    .search { padding: 0 16px 8px; }
    input {
      width: 100%; box-sizing: border-box; padding: 10px 12px;
      background: var(--mdc-text-field-fill-color, rgba(127, 127, 127, 0.12));
      color: var(--primary-text-color); border: none;
      border-bottom: 1px solid var(--divider-color); border-radius: 4px 4px 0 0;
      font: inherit; font-size: 16px; /* smaller makes iOS zoom in on focus */
    }
    input:focus { outline: none; border-bottom-color: var(--primary-color); }
    .list { list-style: none; margin: 0; padding: 0; }
    li + li { border-top: 1px solid var(--divider-color); }
    button { font: inherit; color: inherit; background: none; border: none; cursor: pointer; }
    button:focus-visible, a:focus-visible { outline: 2px solid var(--primary-color); outline-offset: -2px; }
    .row { display: flex; gap: 12px; align-items: center; width: 100%; padding: 10px 16px; text-align: left; }
    .row:hover { background: var(--secondary-background-color); }
    .thumb {
      flex: none; width: 56px; height: 56px; border-radius: 8px; overflow: hidden;
      display: flex; align-items: center; justify-content: center;
      background: var(--secondary-background-color); color: var(--secondary-text-color);
    }
    .thumb img { width: 100%; height: 100%; object-fit: cover; }
    .main { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
    .name { color: var(--primary-text-color); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; color: var(--secondary-text-color); }
    .outcome { display: inline-flex; align-items: center; gap: 3px; --mdc-icon-size: 16px; }
    .outcome.ok { color: var(--success-color, var(--green-color)); }
    .outcome.bad { color: var(--error-color); }
    .outcome.warn { color: var(--warning-color); }
    .swatch { display: inline-block; width: 11px; height: 11px; border-radius: 50%; box-shadow: inset 0 0 0 1px var(--divider-color); vertical-align: -1px; }
    .details { display: flex; flex-direction: column; gap: 6px; padding: 2px 16px 14px; }
    .photo { width: 100%; max-height: 320px; object-fit: contain; border-radius: 8px; background: var(--secondary-background-color); }
    .fact { display: flex; justify-content: space-between; gap: 12px; }
    .fact > :last-child { text-align: right; color: var(--primary-text-color); white-space: nowrap; }
    .link { color: var(--primary-color); padding-top: 4px; }
    .more { display: block; width: 100%; padding: 12px; color: var(--primary-color); font-weight: 500; }
  `;
}

if (!customElements.get(CARD)) customElements.define(CARD, AirSliceHistoryCard);

window.customCards = window.customCards || [];
if (!window.customCards.some((c) => c.type === CARD)) {
  window.customCards.push({ type: CARD, name: "AirSlice history", description: "Every print AirSlice has followed, with photos, filament and cost.", documentationURL: "https://github.com/thrawnarn/AirSlice" });
}
