import { test } from "node:test";
import assert from "node:assert/strict";
import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register();
const { AirSliceHistoryCard, duration, grams, matching, minutes } = await import("../src/airslice-history-card.js");

const ENTITY = "sensor.airslice_pandan_prints";
const prints = (n = 2) => [
  {
    id: "a", name: "Vase", outcome: "finished", started: "2026-11-09T18:00:00+00:00", ended: "2026-11-09T22:15:00+00:00", progress: 100,
    weight: 89.3, usage: [{ slot: "A1", type: "PLA", color: "#FFFFFF", grams: 89.3 }], cost: { filament: 12, machine: 7.78, total: 19.78, currency: "kr", actual: true },
    waste: { changes: 2, flush_g: 4, tower_g: 1, cost: 0.75 }, energy_kwh: 0.24, photo: "media-source://airslice/e/a", model_url: "https://makerworld.com/models/1",
  },
  { id: "b", name: "Egg, plate 2", outcome: "failed", started: "2026-11-08T10:00:00+00:00", ended: "2026-11-08T11:00:00+00:00", progress: 25, layer: 10, total_layers: 40, error: "Nozzle clog", usage: [{ slot: "A2", type: "PLA", color: "red;background:url(x)", grams: 5 }], photo: null, external: true, model_url: "javascript:alert(1)" },
  ...Array.from({ length: Math.max(0, n - 2) }, (_, i) => ({ id: `x${i}`, name: `Print ${i}`, outcome: "unknown", usage: [], photo: null })),
];

function hassWith(list, language = "sv") {
  const hass = {
    locale: { language },
    calls: [],
    states: { [ENTITY]: { state: String(list.length), attributes: { hours: 5.3, grams: 94.3, cost: 19.78, latest_id: list[0]?.id, latest_outcome: list[0]?.outcome } } },
    callWS: async (msg) => {
      hass.calls.push(msg);
      if (msg.type === "media_source/resolve_media") return { url: "/api/airslice/e/photo/a?authSig=x" };
      return { response: { prints: list, count: list.length } };
    },
  };
  return hass;
}

async function mount(hass, config = {}) {
  const card = new AirSliceHistoryCard();
  card.setConfig({ entity: ENTITY, ...config });
  card.hass = hass;
  document.body.append(card);
  await settle(card);
  return card;
}

async function settle(card) {
  for (let i = 0; i < 4; i++) {
    await card.updateComplete;
    await new Promise((r) => setTimeout(r));
  }
}

const $ = (card, s) => card.shadowRoot.querySelector(s);
const $$ = (card, s) => [...card.shadowRoot.querySelectorAll(s)];
const text = (el) => el.textContent.replace(/\s+/g, " ").trim();
const asked = (hass) => hass.calls.filter((c) => c.type === "call_service");

test("reads the history through the integration's service", async () => {
  const hass = hassWith(prints());
  const card = await mount(hass);
  assert.deepEqual(asked(hass)[0], { type: "call_service", domain: "airslice", service: "list_prints", service_data: { entity_id: ENTITY }, return_response: true });
  assert.deepEqual($$(card, ".name").map(text), ["Vase", "Egg, plate 2"]);
  assert.match(text($(card, ".summary")), /2 utskrifter · 5 h · 94 g · 19,78 kr/);
  card.remove();
});

test("shows the result in the reader's language", async () => {
  const sv = await mount(hassWith(prints()));
  assert.deepEqual($$(sv, ".outcome").map(text), ["Klar", "Misslyckades"]);
  sv.remove();
  const en = await mount(hassWith(prints(), "en-GB"));
  assert.equal(text($(en, ".title")), "Print history");
  assert.deepEqual($$(en, ".outcome").map(text), ["Finished", "Failed"]);
  en.remove();
});

test("a row opens to its details, and closes again", async () => {
  const card = await mount(hassWith(prints()));
  const row = $$(card, ".row")[0];
  row.click();
  await settle(card);
  const details = text($(card, ".details"));
  assert.match(details, /Utspolning och prime tower, 2 filamentbyten/);
  assert.match(details, /Totalt/);
  assert.match(details, /0,24 kWh/);
  assert.equal($(card, ".link").getAttribute("href"), "https://makerworld.com/models/1");
  assert.equal($$(card, ".row")[0].getAttribute("aria-expanded"), "true");
  $$(card, ".row")[0].click();
  await settle(card);
  assert.equal($(card, ".details"), null);
  card.remove();
});

test("a failed print says why and how far it got", async () => {
  const card = await mount(hassWith(prints()));
  $$(card, ".row")[1].click();
  await settle(card);
  const details = text($(card, ".details"));
  assert.match(details, /Nozzle clog/);
  assert.match(details, /Stoppad vid 25 %/);
  assert.match(details, /Lager 10 av 40/);
  assert.match(details, /Startad utanför AirSlice/);
  card.remove();
});

test("remote text can't become a link or a style", async () => {
  const card = await mount(hassWith(prints()));
  $$(card, ".row")[1].click();
  await settle(card);
  assert.equal($(card, ".link"), null, "javascript: isn't a link");
  assert.ok(!$$(card, ".swatch").some((s) => s.getAttribute("style").includes("url(")));
  card.remove();
});

test("photos are resolved for what's on screen, once", async () => {
  const hass = hassWith(prints());
  const card = await mount(hass);
  await settle(card);
  const photos = hass.calls.filter((c) => c.type === "media_source/resolve_media");
  assert.equal(photos.length, 1);
  assert.equal($(card, ".thumb img").getAttribute("src"), "/api/airslice/e/photo/a?authSig=x");
  card.remove();
});

test("a picture that won't load falls back to the icon", async () => {
  const card = await mount(hassWith(prints()));
  await settle(card);
  $(card, ".thumb img").dispatchEvent(new Event("error"));
  await settle(card);
  assert.equal($(card, ".thumb img"), null);
  assert.ok($(card, ".thumb ha-icon"));
  card.remove();
});

test("search and show more", async () => {
  const card = await mount(hassWith(prints(20)), { page_size: 5 });
  assert.equal($$(card, ".row").length, 5);
  $(card, ".more").click();
  await settle(card);
  assert.equal($$(card, ".row").length, 10);
  const input = $(card, "input");
  input.value = "egg plate";
  input.dispatchEvent(new Event("input"));
  await settle(card);
  assert.deepEqual($$(card, ".name").map(text), ["Egg, plate 2"]);
  assert.equal($(card, ".more"), null);
  card.remove();
});

test("reads again only when this sensor changes", async () => {
  const hass = hassWith(prints());
  const card = await mount(hass);
  assert.equal(asked(hass).length, 1);
  card.hass = { ...hass, states: { ...hass.states, "light.other": { state: "on", attributes: {} } } };
  await settle(card);
  assert.equal(asked(hass).length, 1, "something else in the house changed");
  card.hass = { ...hass, states: { [ENTITY]: { state: "3", attributes: { latest_id: "new", latest_outcome: "finished" } } } };
  await settle(card);
  assert.equal(asked(hass).length, 2);
  card.remove();
});

test("says what's wrong rather than staying blank", async () => {
  const none = await mount(hassWith(prints()), { entity: "sensor.gone" });
  assert.match(text($(none, ".message")), /sensor\.gone är inte tillgänglig/);
  none.remove();
  const empty = await mount(hassWith([]));
  assert.match(text($(empty, ".message")), /Inga utskrifter ännu/);
  empty.remove();
  const hass = hassWith(prints());
  hass.callWS = async () => {
    throw new Error("Service not found");
  };
  const broken = await mount(hass);
  assert.match(text($(broken, ".error")), /Service not found/);
  broken.remove();
});

test("the card picker finds the sensor by itself", () => {
  const stub = AirSliceHistoryCard.getStubConfig({ entities: { a: { entity_id: "light.x", platform: "hue" }, b: { entity_id: ENTITY, platform: "airslice" } } });
  assert.deepEqual(stub, { entity: ENTITY });
});

test("number helpers", () => {
  assert.equal(grams("sv", 1250), "1,25 kg");
  assert.equal(minutes(255), "4 h 15 min");
  assert.equal(minutes(NaN), "");
  assert.equal(duration({ started: "2026-01-01T10:00:00Z", ended: "2026-01-01T10:30:00Z" }), 30);
  assert.equal(matching([{ name: "Egg, plate 2" }], "plate egg").length, 1);
});
