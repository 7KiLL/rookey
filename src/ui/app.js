// rookey settings. Every change is saved at once through the local server behind `rookey ui`.
// The page is arrow.js templates over one reactive store: the server's state, and the choices
// that live in the page until there is something to save.

import { html, reactive, nextTick } from "/arrow.js";
import { LOCALES, has, pickLang, setLang, t, tx } from "/i18n.js";

const $ = (selector) => document.querySelector(selector);
const code = (text) => html`<code>${text}</code>`;

const LANGUAGES = ["en", "uk", "ru", "de", "es", "fr", "pl"];

const READERS = {
  ocr: { name: () => t("reader.ocr"), provider: null },
  openai: { name: () => "OpenAI", provider: "openai" },
  anthropic: { name: () => "Claude", provider: "anthropic" },
};

// For desktops whose config rookey doesn't write: the line to add by hand.
const MANUAL = {
  niri: { name: "niri", snippet: 'Mod+Shift+D repeat=false { spawn "rookey" "toggle"; }' },
  hyprland: { name: "Hyprland", snippet: 'hl.bind("SUPER + SHIFT + D", hl.dsp.exec_cmd("rookey toggle"))' },
  macos: { name: "macOS", snippet: "cmd + shift - d : rookey toggle" },
};

// What a key is called in a compositor's config, by the code the browser reports.
const KEY_NAMES = {
  Space: "space", Enter: "Return", Tab: "Tab", Backquote: "grave", Minus: "minus", Equal: "equal",
  BracketLeft: "bracketleft", BracketRight: "bracketright", Backslash: "backslash",
  Semicolon: "semicolon", Quote: "apostrophe", Comma: "comma", Period: "period", Slash: "slash",
  ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down",
  Home: "Home", End: "End", PageUp: "Page_Up", PageDown: "Page_Down",
  Insert: "Insert", Delete: "Delete", Pause: "Pause", ScrollLock: "Scroll_Lock", PrintScreen: "Print",
};

// The link carries a token. Keep it for reloads and take it out of the address bar.
let token = new URLSearchParams(location.search).get("t");
try {
  if (token) sessionStorage.setItem("rookey-token", token);
  else token = sessionStorage.getItem("rookey-token");
} catch {
  // storage is off: the page works until it is reloaded
}
history.replaceState(null, "", "/");

const ui = reactive({
  s: null, // the state from the server
  ready: false, // drawn once the first state is here; after that the parts that read it update
  lang: setLang(pickLang()),
  theme: "system", // both come from the config once the state is here
  stopped: false,
  status: { key: "status.loading", vars: {}, problem: false },
  open: null, // a shut section opened for a moment, to show what a check asks for
  otherLanguage: false,
  desktop: null,
  edit: null, // the rewriting instruction while it is typed, before it is saved
  editCustom: false, // Custom picked, even with no instruction of your own yet
  provider: null, // { id, where: "engine" | "keys", doing: "edit" | "remove" }
  hotkey: { way: null, editing: false, chord: null, file: null, files: false, taken: null, problem: "", pressing: false },
  history: { entries: [], path: "", keep: 0, all: false, clearing: false }, // entries newest first
  pill: { custom: false, at: null }, // Custom picked; the place while it is dragged, before it is saved
});

/** Theme and language go on <html>: CSS picks the tokens, the browser the hyphenation. */
function applyLook() {
  const root = document.documentElement;
  if (ui.theme === "system") delete root.dataset.theme;
  else root.dataset.theme = ui.theme;
  root.lang = setLang(ui.lang);
  document.title = t("title");
}
applyLook();

async function call(path, body) {
  const options = body ? { method: "POST", body: JSON.stringify(body) } : {};
  let response;
  try {
    response = await fetch(`${path}?t=${encodeURIComponent(token || "")}`, options);
  } catch {
    stop();
    throw new Error(t("stopped.short"));
  }
  const reply = await response.json();
  if (!response.ok) throw new Error(reply.error);
  return reply;
}

function stop() {
  if (ui.stopped) return;
  ui.stopped = true;
  document.body.classList.add("is-stopped");
}

/** The line under the heading. Kept as a key, so it follows a change of language. */
function say(key, vars = {}, problem = false) {
  ui.status = { key, vars, problem };
}

/**
 * Takes a new state from the server into ui.s, changing only what differs. Every slot that
 * reads a replaced part is drawn again, and a list drawn again loses focus and what is typed.
 */
function merge(target, source, whole = true) {
  for (const [key, value] of Object.entries(source)) {
    const old = target[key];
    const plain = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
    if (plain(old) && plain(value)) merge(old, value);
    else if (JSON.stringify(old) !== JSON.stringify(value)) target[key] = value;
  }
  // a whole state drops what it no longer has; a part leaves the rest alone
  if (whole) for (const key of Object.keys(target)) if (!(key in source)) delete target[key];
}

/** The new state from the server, into the page. */
function take(state) {
  if (ui.s) merge(ui.s, state);
  else ui.s = state;
}

/** Sends a change. What comes back is the new state, or an answer to look at. */
async function send(path, body, saying = "status.saving", vars = {}) {
  say(saying, vars);
  try {
    const reply = await call(path, body);
    if (reply.values) take(reply);
    else if (reply.trial) merge(ui.s.trial, reply.trial);
    watch();
    return reply;
  } catch (e) {
    say("status.failed", { why: e.message }, true);
    return null;
  }
}

async function save(changes) {
  const saved = await send("/api/save", changes);
  if (saved) say("status.saved", { path: ui.s.path });
  return Boolean(saved);
}

const isOn = (value) => value !== "" && value !== "0" && value !== "false";
const values = () => ui.s.values;
const engine = () => values().ROOKEY_BACKEND || "local";
const cloud = () => engine() !== "local";
const reader = () => (values().ROOKEY_READER in READERS ? values().ROOKEY_READER : "ocr");
const provider = (id) => ui.s.providers.find((p) => p.id === id);
const SOUND_SETS = ["notes", "rook", "pencil"]; // sound::SETS, the first is the default
const PILL_STYLES = ["full", "compact", "dot"]; // overlay::STYLES, the first is the default
// ROOKEY_PILL_AT, in percent of the room the pill has; bottom is overlay::DEFAULT_AT
const PILL_PLACES = { "top-left": [0, 0], top: [50, 0], "top-right": [100, 0], "bottom-left": [0, 100], bottom: [50, 100], "bottom-right": [100, 100] };
const CUES = ["start", "stop", "typed", "failed"];
// ROOKEY_EDIT holds the instruction itself, so a preset is known by its exact text. The text
// goes to the model as it is, in English whatever the page's language.
const EDITS = {
  punctuation: "Fix the punctuation and capitalisation only. Keep every word as it was said, in its language.",
  clean: "Remove filler words, false starts and repeated words, and fix the grammar. Keep the meaning, the tone and the language.",
  formal: "Rewrite this in a clear, formal tone, in full sentences. Keep the meaning and the language.",
  brief: "Make this shorter and more direct. Keep the meaning, the key details and the language.",
};
const editMode = () =>
  ui.editCustom ? "custom" : values().ROOKEY_EDIT ? (Object.keys(EDITS).find((id) => EDITS[id] === values().ROOKEY_EDIT) ?? "custom") : "off";
const soundSet = () => (SOUND_SETS.includes(values().ROOKEY_SOUNDS) ? values().ROOKEY_SOUNDS : SOUND_SETS[0]);
const hasKey = (id) => provider(id).saved || provider(id).env;
const number = (n, digits = 0) => n.toLocaleString(ui.lang, { minimumFractionDigits: digits, maximumFractionDigits: digits });
const size = (mb) => (mb >= 1000 ? t("size.gb", { n: number(mb / 1024, 1) }) : t("size.mb", { n: mb }));
const listOf = (items) => new Intl.ListFormat(ui.lang, { type: "conjunction" }).format(items);

/** A language's name in the page's language: "German", or "німецька". */
function languageName(id, capital = false) {
  let name = id;
  try {
    name = new Intl.DisplayNames([ui.lang], { type: "language" }).of(id) || id;
  } catch {
    // not a code Intl knows: shown as typed
  }
  return capital ? name[0].toLocaleUpperCase(ui.lang) + name.slice(1) : name;
}

function termsMode() {
  const value = values().ROOKEY_CONTEXT;
  if (!isOn(value)) return "off";
  return value === "1" || value === "true" ? "screen" : "command";
}

/** The picked languages, "en,uk" as ["en", "uk"]; none picked means any. */
const languages = () => (values().ROOKEY_LANG || "").split(",").filter((l) => l && l !== "auto");

/** A button that copies, and says so for a moment. `label` names it where several sit together. */
function copyButton(text, label = false) {
  const b = reactive({ label: "copy" });
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text());
      b.label = "copied";
    } catch {
      b.label = "copy.failed";
    }
    setTimeout(() => (b.label = "copy"), 2000);
  };
  return html`<button type="button" class="button" aria-label="${label}" @click="${copy}">${() => t(b.label)}</button>`;
}

const focus = (selector) => nextTick(() => $(selector)?.focus());

// ---- the page ------------------------------------------------------------

function Page() {
  return html`
    <div class="stopped" role="alert" hidden="${() => !ui.stopped}">
      <p>${tx("stopped", { cmd: code("rookey ui") })}</p>
    </div>
    <div class="pad">
    ${Header()}
    <main class="settings" id="settings" inert="${() => ui.stopped}">
      ${() => (ui.ready ? Settings() : "")}
    </main>
    <aside class="specimen" aria-labelledby="specimen-title">
      <div class="specimen-inner" id="specimen" inert="${() => ui.stopped}">
        ${() => (ui.ready ? Specimen() : "")}
      </div>
    </aside>
    </div>`;
}

function Header() {
  const theme = (value) => {
    ui.theme = value;
    applyLook();
    if (ui.s) save({ ROOKEY_UI_THEME: value === "system" ? "" : value });
  };
  // ponytail: a new language reloads the page. Arrow can't swap a whole tree's words in place,
  // and tearing the tree down leaves it running watchers whose slots are gone.
  const language = async (value) => {
    // the token rides along: with sessionStorage off, a bare reload would lock the page
    if (ui.s && (await save({ ROOKEY_UI_LANG: value }))) location.replace(`/?t=${encodeURIComponent(token || "")}`);
  };
  return html`
    <header class="top">
      <h1><img class="logo" src="/icon.svg" alt="" width="36" height="36"><span>${tx("heading", { mark: html`<span class="mark">rookey</span>` })}</span></h1>
      <p class="${() => (ui.status.problem ? "status is-problem" : "status")}" role="status">${() => t(ui.status.key, ui.status.vars)}</p>
      <div class="prefs">
        <label class="pref">
          <span>${t("theme")}</span>
          <select @change="${(e) => theme(e.target.value)}">
            ${["system", "light", "dark"].map((id) => html`<option value="${id}" selected="${() => ui.theme === id}">${t(`theme.${id}`)}</option>`)}
          </select>
        </label>
        <label class="pref">
          <span>${t("language")}</span>
          <select @change="${(e) => language(e.target.value)}">
            ${Object.entries(LOCALES).map(([id, name]) => html`<option value="${id}" lang="${id}" selected="${() => ui.lang === id}">${name}</option>`)}
          </select>
        </label>
      </div>
      ${() => (ui.ready ? html`${shell("ROOKEY_UI_THEME")}${shell("ROOKEY_UI_LANG")}${Version()}` : "")}
    </header>`;
}

/** Which rookey this is, and whether a newer one is out, on its way, or waiting for a restart. */
function Version() {
  const ask = async (what) => {
    try {
      merge(ui.s.update, await call("/api/update", { [what]: true }));
      watch();
    } catch (e) {
      say("status.failed", { why: e.message }, true);
    }
  };
  const button = (key, what) => html`<button type="button" class="button is-quiet" @click="${() => ask(what)}">${t(key)}</button>`;
  const line = () => {
    const { phase, latest, managed, supported, error } = ui.s.update;
    if (!supported) return t("update.unsupported");
    if (phase === "checking") return t("update.checking");
    if (phase === "current") return html`${t("update.current")} · ${button("update.check", "check")}`;
    if (phase === "downloading") return t("update.downloading", { version: latest });
    if (phase === "installed") return t(ui.s.listen.running ? "update.installed.listen" : "update.installed", { version: latest });
    if (phase === "available" && managed) return t("update.managed", { version: latest, who: t(`update.by.${managed}`) });
    if (phase === "available") return html`${t("update.available", { version: latest })} · ${button("update.install", "install")}`;
    if (phase === "failed") return html`<span class="problem-text">${t("update.failed", { why: error })}</span> · ${button("update.again", "check")}`;
    return button("update.check", "check");
  };
  const ready = () => (ui.s.checks.every((c) => c.ok) ? html`<span class="ready">${t("setup.ready")}</span> · ` : "");
  return html`<p class="version" aria-live="polite">${ready}<span>${() => t("update.version", { version: ui.s.update.version })}</span> · ${line}</p>`;
}

function Settings() {
  return html`${Checks()}${Hotkey()}${Engine()}${Typed()}${Talk()}${System()}`;
}

const closed = () => (values().ROOKEY_UI_CLOSED || "").split(",").filter(Boolean);

/** A section that folds: its title, a line saying what is set while it's shut, then the settings.
 * Which are shut is kept in the config: every `rookey ui` is a new origin, with nothing kept. */
function Section(id, title, summary, body) {
  const toggled = (e) => {
    const now = closed().filter((s) => s !== id);
    if (!e.target.open) now.push(id);
    if (ui.open === id && !e.target.open) ui.open = null;
    if (now.join(",") !== closed().join(",")) save({ ROOKEY_UI_CLOSED: now.join(",") });
  };
  return html`
    <details class="group fold" id="${id}" open="${() => !closed().includes(id) || ui.open === id}" @toggle="${toggled}">
      <summary>
        <h2 id="${`${id}-title`}">${title}</h2>
        <span class="summary-value">${summary}</span>
      </summary>
      ${body}
    </details>`;
}

/** What most people never need, folded inside the section it belongs to. */
function More(label, body) {
  return html`<details class="more"><summary>${label}</summary>${body}</details>`;
}

/** Parts of a summary line, the empty ones left out. */
const parts = (...items) => items.filter(Boolean).join(" · ");

/** A setting the shell overrides, noted next to it. */
function shell(name) {
  return html`<p class="note" hidden="${() => !(name in ui.s.env)}">${() =>
    name in ui.s.env ? tx("shell", { setting: code(`${name}=${ui.s.env[name]}`) }) : ""}</p>`;
}

// The helpers below are called with values and with functions. Arrow updates only the slots
// given a function, so every one of theirs gets one.
const live = (v) => (typeof v === "function" ? v : () => v);

/** A switch row: name, what it does, the checkbox. */
function toggle(id, name, about, checked, change) {
  [name, about, checked] = [name, about, checked].map(live);
  return html`
    <label class="switch">
      <span class="switch-text">
        <span class="choice-name">${name}</span>
        <span class="choice-about">${about}</span>
      </span>
      <input type="checkbox" role="switch" id="${id}" checked="${checked}" @change="${change}">
    </label>`;
}

/** A radio choice with a name and a line about it. */
function choice(name, value, checked, title, about, change, disabled = false) {
  [checked, title, about, disabled] = [checked, title, about, disabled].map(live);
  return html`
    <label class="choice">
      <input type="radio" name="${name}" value="${value}" checked="${checked}" disabled="${disabled}" @change="${change}">
      <span class="choice-text">
        <span class="choice-name">${title}</span>
        <span class="choice-about">${about}</span>
      </span>
    </label>`;
}

function chip(type, name, value, checked, text, change, disabled = false) {
  [checked, text, disabled] = [checked, text, disabled].map(live);
  return html`<label class="chip"><input type="${type}" name="${name}" value="${value}" checked="${checked}" disabled="${disabled}" @change="${change}"><span>${text}</span></label>`;
}

// ---- setup check -----------------------------------------------------------

/** A check's words: ours by its id where we have them, else what the server sent. */
function checkWords(c) {
  const engine = html`<a href="#engine-title">${t("engine.title")}</a>`;
  if (!has(`check.${c.id}.title`)) return { title: c.title, missing: [c.missing] };
  let title = t(`check.${c.id}.title`);
  // macOS grants its permissions to the app rookey ui runs in, named where the server could
  const app = ui.s.app || t("check.app.unknown");
  // Rookey, the app that hears the hotkey on macOS, has words of its own where they differ
  const words = ui.s.rookey && has(`check.${c.id}.rookey`) ? `check.${c.id}.rookey` : `check.${c.id}.missing`;
  let missing = tx(words, { engine, app });
  if (c.id === "mic" && c.title.includes(": ")) title = t("check.mic.named", { name: c.title.split(": ").slice(1).join(": ") });
  // which tools, as the server found them with the environment winning over the config
  if (c.id === "screen") {
    const tools = c.tools || [], langs = c.langs || [];
    missing = [];
    if (tools.length) missing.push(...tx("check.screen.missing", { tools: listOf(tools) }));
    if (langs.length) {
      // languages whose tesseract pack isn't installed: their text comes out as junk
      if (tools.length) missing.push(" ");
      missing.push(t("check.screen.langs", { langs: listOf(langs.map((l) => languageName(l))) }));
      if (ui.s.os === "windows") missing.push(" ", t("check.screen.langs.windows"));
    }
  }
  // the default model is here, so the one the settings name is somewhere else
  if (c.id === "model" && ui.s.models.catalog[0].installed) missing = tx("check.model.elsewhere", { engine });
  return { title, missing };
}

/** Languages tesseract has no pack for: optional, so a note under the switch, not a failed check.
 * Drawn once, with every word in a function slot: a redrawn template would keep its old words. */
function ScreenLangs() {
  const screen = () => ui.s.checks.find((c) => c.id === "screen") || {};
  const langs = () => screen().langs || [];
  const words = () => [
    t("check.screen.langs", { langs: listOf(langs().map((l) => languageName(l))) }),
    ...(ui.s.os === "windows" ? [" ", t("check.screen.langs.windows")] : []),
  ];
  // when a tool is missing too, the setup check shows the one command for all of it
  const fix = () => (screen().ok && screen().fix) || "";
  return html`<div hidden="${() => termsMode() === "off" || !langs().length}">
    <p class="note">${() => words()}</p>
    <span class="with-button" hidden="${() => !fix()}"><code class="fix">${() => fix()}</code>${copyButton(fix)}</span>
  </div>`;
}

/** The one thing that fixes a failing check: a command to copy, a settings pane, or the fix itself. */
function checkFix(c) {
  if (c.ok) return "";
  if (c.fix) return html`<span class="with-button"><code class="fix">${c.fix}</code>${copyButton(() => c.fix)}</span>`;
  const button = (key, click) => html`<span class="actions"><button type="button" class="button is-primary" @click="${click}">${t(key)}</button></span>`;
  if (c.open) return button(`open.${c.open}`, () => openPane(c.open));
  if (c.id === "key") {
    return button("key.add", () => {
      ui.open = "engine";
      ui.provider = { id: "elevenlabs", where: "engine", doing: "edit" };
      focus("#key-elevenlabs-engine");
    });
  }
  if (c.id === "model" && !ui.s.models.catalog[0].installed) return ModelFix(ui.s.models.catalog[0]);
  return "";
}

/** The default model's download, started from the check and followed there. */
function ModelFix(m) {
  const download = () => ui.s.models.download;
  const running = () => Boolean(download()?.file === m.file && download().running);
  const part = () => (download()?.total ? download().done / download().total : 0);
  const mb = (bytes) => Math.round(bytes / 1048576);
  return html`
    <span class="actions" hidden="${running}">
      <button type="button" class="button is-primary" disabled="${() => Boolean(download()?.running)}" @click="${() => downloadModel(m)}">${
        t("check.model.download", { name: m.name, size: size(m.mb) })}</button>
    </span>
    <span class="check-progress" hidden="${() => !running()}">
      <progress class="progress" max="100" aria-label="${t("download.label", { name: m.name })}"
        value="${() => Math.round(part() * 1000) / 10}"></progress>
      <span class="progress-text">${() =>
        download()?.total ? t("download.progress", { done: mb(download().done), total: mb(download().total) }) : t("download.connecting")}</span>
    </span>`;
}

async function downloadModel(m) {
  if (await send("/api/model", { download: m.file }, "download.starting", { name: m.name })) say("download.running", { name: m.name });
}

/** Opens the system's settings where the switch is. Coming back to the page checks again. */
async function openPane(what) {
  if (await send("/api/open", { what }, "status.opening")) say("status.opened");
}

function Checks() {
  const failing = () => ui.s.checks.some((c) => !c.ok);
  return html`
    <section class="group" id="setup" aria-labelledby="setup-title" hidden="${() => !failing()}">
      <h2 id="setup-title">${t("setup.title")}</h2>
      <ul class="checks">
        ${() => ui.s.checks.map((c) => {
          const words = checkWords(c);
          return html`
            <li class="${`check ${c.ok ? "is-ok" : "is-missing"}`}">
              <span class="check-mark" role="img" aria-label="${t(c.ok ? "check.ready" : "check.missing")}"></span>
              <span class="row-text">
                <span class="choice-name">${words.title}</span>
                ${c.ok ? "" : html`<span class="choice-about">${words.missing}</span>`}
                ${checkFix(c)}
              </span>
            </li>`.key(JSON.stringify(c)); // a row whose check changed is drawn anew, not patched
        })}
      </ul>
      <div class="actions">
        <button type="button" class="button" @click="${checkAgain}">${t("setup.again")}</button>
      </div>
    </section>`;
}

/** Back from System Settings or a terminal: checks again while something is missing. */
let rechecking = false;
async function recheck() {
  if (rechecking || ui.stopped || !ui.ready || ui.s.checks.every((c) => c.ok) || moving()) return;
  rechecking = true;
  try {
    take(await call("/api/state"));
    if (ui.s.checks.every((c) => c.ok)) say("status.all-here");
  } catch {
    // the line under the heading says it when the server is gone
  }
  rechecking = false;
}

async function checkAgain() {
  say("status.checking");
  try {
    take(await call("/api/state"));
    say(ui.s.checks.every((c) => c.ok) ? "status.all-here" : "status.still-missing");
  } catch (e) {
    say("status.failed", { why: e.message }, true);
  }
}

// ---- engine --------------------------------------------------------------

function Engine() {
  const pickEngine = (e) => {
    const streaming = engine() === "elevenlabs-realtime" || !values().ROOKEY_BACKEND;
    save({ ROOKEY_BACKEND: e.target.value === "local" ? "" : streaming ? "elevenlabs-realtime" : "elevenlabs" });
  };
  const using = () => ui.s.models.installed.find((m) => m.path === ui.s.models.in_use);
  const offer = () => !cloud() && !ui.s.models.found && !ui.s.models.catalog[0].installed;
  const key = () => provider("elevenlabs");
  const summary = () => parts(
    cloud() ? "ElevenLabs" : t("engine.local"),
    cloud() ? engine() === "elevenlabs-realtime" && t("sum.streaming") : using() && (using().name || using().file),
    languages().length ? languages().map((l) => l.toUpperCase()).join(", ") : t("sum.any"),
  );
  return Section("engine", t("section.engine"), summary, html`
      <div class="choices one-about" role="radiogroup" aria-labelledby="engine-title">
        ${choice("engine", "local", () => !cloud(), t("engine.local"), t("engine.local.about"), pickEngine)}
        ${choice("engine", "cloud", cloud, t("engine.cloud"), t("engine.cloud.about"), pickEngine)}
      </div>
      ${shell("ROOKEY_BACKEND")}
      <p class="hint" hidden="${() => !cloud() || !key().saved}">${() => t("engine.key.saved", { name: key().name })}</p>
      <ul class="rows" hidden="${() => !cloud() || key().saved}">${() => (cloud() && !key().saved ? ProviderRow(key(), "engine") : "")}</ul>
      <div hidden="${() => !cloud()}">
        ${toggle("stream", t("engine.stream"), t("engine.stream.about"), () => engine() === "elevenlabs-realtime", (e) =>
          save({ ROOKEY_BACKEND: e.target.checked ? "elevenlabs-realtime" : "elevenlabs" }))}
      </div>
      <p class="hint" hidden="${() => cloud() || !ui.s.models.found || !using()}">${() =>
        using() ? t("engine.uses", { name: using().name || using().file, size: size(using().mb) }) : ""}</p>
      <ul class="rows" hidden="${() => !offer()}">${() => (offer() ? ModelRow(ui.s.models.catalog[0]) : "")}</ul>
      <div hidden="${cloud}">${More(t("more.models"), Models())}</div>
      ${Languages()}`);
}

function Models() {
  const models = () => ui.s.models;
  const listed = () => models().installed.some((m) => m.path === models().in_use);
  const wanted = () => models().catalog.filter((m) => !m.installed);
  return html`
    <div id="models">
      <div class="choices" role="radiogroup" aria-label="${t("models.title")}">
        ${() => models().installed.map((m) =>
          choice("model", m.path, m.path === models().in_use, m.name || m.file,
            t("models.in", { size: size(m.mb), where: m.shown.replace(/[\\/][^\\/]*$/, "") }),
            () => save({ ROOKEY_MODEL: m.default ? "" : m.path })))}
      </div>
      <p class="problem" hidden="${() => models().found}">${() => tx("models.missing", { path: code(models().in_use) })}</p>
      ${shell("ROOKEY_MODEL")}

      <h4 id="models-more-title" hidden="${() => wanted().length === 0}">${t("models.more")}</h4>
      <ul class="rows" aria-labelledby="models-more-title">${() => wanted().map(ModelRow)}</ul>
      <p class="hint" hidden="${() => wanted().length === 0}">${() => t("models.where", { dir: models().dir })}</p>

      <div class="field">
        <label for="model">${t("models.own")}</label>
        <input class="typed-input" id="model" type="text" spellcheck="false" autocomplete="off" placeholder="~/models/ggml-medium.bin"
          .value="${() => (listed() ? "" : values().ROOKEY_MODEL)}" @change="${(e) => save({ ROOKEY_MODEL: e.target.value })}">
        <p class="hint">${t("models.own.hint")}</p>
      </div>
    </div>`;
}

function ModelRow(m) {
  // the part that moves by itself: only these read the download, so the rows stay put
  const download = () => ui.s.models.download;
  const mine = () => download()?.file === m.file;
  const running = () => Boolean(mine() && download().running);
  const failed = () => Boolean(mine() && download().error);
  const part = () => (download()?.total ? download().done / download().total : 0);
  const mb = (bytes) => Math.round(bytes / 1048576);
  const halt = async () => {
    await send("/api/model", { cancel: true }, "download.stopping");
    say("download.stopped", { name: m.name });
  };
  return html`
    <li class="row model-row">
      <span class="row-text">
        <span class="choice-name">${m.name}<span class="choice-meta">, ${size(m.mb)}</span></span>
        <span class="choice-about">${has(`model.${m.file}`) ? t(`model.${m.file}`) : m.about}</span>
      </span>
      <span class="actions">
        <button type="button" class="button" hidden="${running}" disabled="${() => Boolean(download()?.running && !mine())}" @click="${() => downloadModel(m)}">${() =>
          t(failed() ? "download.again" : "download")}</button>
        <button type="button" class="button" hidden="${() => !running()}" @click="${halt}">${t("download.stop")}</button>
      </span>
      <div class="row-wide" hidden="${() => !running()}">
        <progress class="progress" max="100" aria-label="${t("download.label", { name: m.name })}"
          value="${() => Math.round(part() * 1000) / 10}"></progress>
        <p class="progress-text">${() =>
          download()?.total ? t("download.progress", { done: mb(download().done), total: mb(download().total) }) : t("download.connecting")}</p>
      </div>
      <p class="problem row-wide" hidden="${() => !failed()}">${() => (failed() ? t("download.failed", { why: download().error }) : "")}</p>
    </li>`;
}

// ---- languages -------------------------------------------------------------

function Languages() {
  const pickLanguage = (id) => (e) => {
    const picked = languages();
    save({ ROOKEY_LANG: (e.target.checked ? [...picked, id] : picked.filter((l) => l !== id)).join(",") });
  };
  const addCode = (e) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    const typed = e.target.value.trim().toLowerCase();
    if (!typed) return;
    e.target.value = "";
    ui.otherLanguage = false;
    if (!languages().includes(typed)) save({ ROOKEY_LANG: [...languages(), typed].join(",") });
  };
  const about = () => {
    const picked = languages();
    if (picked.length === 0) return t("lang.any");
    const names = picked.map((l) => languageName(l));
    if (picked.length === 1) return t("lang.one", { name: names[0] });
    return t("lang.many", { names: listOf(names) }) + (cloud() ? t("lang.cloud") : "");
  };
  return html`
    <div class="sub">
      <h3 id="language-title">${t("lang.title")}</h3>
      <div class="chips" role="group" aria-labelledby="language-title">
        ${() => {
          const picked = languages();
          const all = [...LANGUAGES, ...picked.filter((l) => !LANGUAGES.includes(l))];
          return all.map((id) => chip("checkbox", "language", id, picked.includes(id), languageName(id, true), pickLanguage(id)));
        }}
        ${chip("checkbox", "language-other", "", () => ui.otherLanguage, t("lang.other"), (e) => {
          ui.otherLanguage = e.target.checked;
          if (e.target.checked) focus("#language-code");
        })}
      </div>
      <div class="field" hidden="${() => !ui.otherLanguage}">
        <label for="language-code">${t("lang.code")}</label>
        <input class="typed-input is-short" id="language-code" type="text" spellcheck="false" autocomplete="off" maxlength="8" placeholder="pt" @keydown="${addCode}">
        <p class="hint">${t("lang.code.hint")}</p>
      </div>
      <p class="hint">${about}</p>
      ${shell("ROOKEY_LANG")}
    </div>`;
}

// ---- what gets typed --------------------------------------------------------

function Typed() {
  const sanitize = () =>
    cloud()
      ? tx("sanitize.cloud", { um: code("um"), uh: code("uh"), yk: code("you know") })
      : tx("sanitize.local", { music: code("[music]") });
  // nothing read, the screen read here or at a provider; a command of your own shows none of them
  const screen = () => (termsMode() === "off" ? "off" : termsMode() === "command" ? "" : reader());
  const pickScreen = (id) => () =>
    save(id === "off" ? { ROOKEY_CONTEXT: "" } : { ROOKEY_CONTEXT: "1", ROOKEY_READER: id === "ocr" ? "" : id });
  const editText = () => ui.edit ?? values().ROOKEY_EDIT;
  // your own text is kept in ROOKEY_EDIT_CUSTOM while a preset or Off is on, and comes back with Custom
  const pickEdit = (id) => async () => {
    // text of your own that isn't kept yet, typed or from before there were presets, is kept now
    const typed = editText();
    const keep = editMode() === "custom" && typed !== values().ROOKEY_EDIT_CUSTOM ? { ROOKEY_EDIT_CUSTOM: typed } : {};
    ui.edit = null;
    ui.editCustom = id === "custom";
    if (id === "custom") return (await save({ ROOKEY_EDIT: values().ROOKEY_EDIT_CUSTOM })) && focus("#edit");
    save({ ROOKEY_EDIT: EDITS[id] ?? "", ...keep });
  };
  const summary = () => parts(
    isOn(values().ROOKEY_SANITIZE) && t("sum.fillers"),
    termsMode() !== "off" && t("sum.terms"),
    words().length && t("sum.words", { n: words().length }),
    cloud() && editMode() !== "off" && t("sum.rewrite", { name: t(`edit.${editMode()}`) }),
  ) || t("sum.as-said");
  return Section("typed", t("section.typed"), summary, html`
      ${toggle("sanitize", t("sanitize"), sanitize, () => isOn(values().ROOKEY_SANITIZE), (e) =>
        save({ ROOKEY_SANITIZE: e.target.checked ? "1" : "" }))}
      ${shell("ROOKEY_SANITIZE")}
      <div class="sub" id="reader-field">
        <h3 id="reader-title">${t("terms")}</h3>
        <div class="chips" role="radiogroup" aria-labelledby="reader-title">
          ${["off", ...Object.keys(READERS)].map((id) =>
            chip("radio", "reader", id, () => screen() === id, id === "off" ? t("reader.off") : READERS[id].name(), pickScreen(id)))}
        </div>
        <p class="hint">${() => (termsMode() === "command" ? t("terms.command") : t(`reader.${screen()}.about`))}</p>
        ${() => {
          const needs = READERS[reader()].provider;
          return needs && termsMode() === "screen" && !hasKey(needs) ? needsKey(needs) : "";
        }}
        ${shell("ROOKEY_CONTEXT")}
        ${shell("ROOKEY_READER")}
        ${ScreenLangs()}
        ${More(t("more.command"), html`
        <div class="field">
          <label for="command">${t("command")}</label>
          <input class="typed-input" id="command" type="text" spellcheck="false" autocomplete="off" placeholder="cat ~/.config/rookey/glossary.txt"
            .value="${() => (termsMode() === "command" ? values().ROOKEY_CONTEXT : "")}"
            @change="${(e) => save({ ROOKEY_CONTEXT: e.target.value.trim() || "1" })}">
          <p class="hint">${t("command.hint")}</p>
        </div>`)}
        <p class="hint" hidden="${() => !cloud() || termsMode() === "off"}">${t("terms.cost")}</p>
      </div>

      ${Words()}

      <div class="${() => (cloud() ? "sub" : "sub is-off")}" id="edit-field">
        <h3 id="edit-title">${t("edit.title")}</h3>
        <div class="chips" role="radiogroup" aria-labelledby="edit-title">
          ${["off", ...Object.keys(EDITS), "custom"].map((id) =>
            chip("radio", "edit-mode", id, () => editMode() === id, t(`edit.${id}`), pickEdit(id), () => !cloud()))}
        </div>
        <p class="hint">${() => t(`edit.${editMode()}.about`)}</p>
        <div class="field" hidden="${() => editMode() === "off"}">
          <label class="visually-hidden" for="edit">${t("edit.custom.label")}</label>
          <textarea id="edit" rows="3" maxlength="2000" placeholder="${t("edit.placeholder")}" disabled="${() => !cloud()}"
            .value="${editText}" @input="${(e) => ((ui.edit = e.target.value), (ui.editCustom = true))}"
            @change="${async (e) => (ui.editCustom = true) && (await save({ ROOKEY_EDIT: e.target.value, ROOKEY_EDIT_CUSTOM: e.target.value })) && (ui.edit = null)}"></textarea>
        </div>
        <p class="hint" hidden="${() => cloud() && editMode() === "off"}">${() => t(cloud() ? "edit.cloud" : "edit.local")}</p>
        ${shell("ROOKEY_EDIT")}
      </div>

`);
}

// ---- your words -------------------------------------------------------------

const words = () => (values().ROOKEY_WORDS || "").split(",").filter(Boolean);

function Words() {
  const draft = reactive({ text: "" });
  const add = async (e) => {
    e.preventDefault();
    const typed = draft.text.split(",").map((w) => w.trim()).filter((w) => w && !words().includes(w));
    if (typed.length === 0) return focus("#word");
    if (await save({ ROOKEY_WORDS: [...words(), ...typed].join(",") })) draft.text = "";
    focus("#word");
  };
  const remove = (word) => () => save({ ROOKEY_WORDS: words().filter((w) => w !== word).join(",") });
  return html`
    <div class="sub">
      <h3 id="words-title">${t("words.title")}</h3>
      <ul class="words" aria-labelledby="words-title" hidden="${() => words().length === 0}">
        ${() => words().map((w) => html`<li><button type="button" class="word" aria-label="${t("words.remove", { word: w })}" @click="${remove(w)}"><span>${w}</span><span class="word-x" aria-hidden="true">×</span></button></li>`)}
      </ul>
      <form class="field" @submit="${add}">
        <label for="word">${t("words.add")}</label>
        <div class="with-button">
          <input class="typed-input" id="word" type="text" spellcheck="false" autocomplete="off" maxlength="200" placeholder="${t("words.placeholder")}"
            .value="${() => draft.text}" @input="${(e) => (draft.text = e.target.value)}">
          <button type="submit" class="button">${t("words.add.button")}</button>
        </div>
        <p class="hint">${t("words.hint")}</p>
      </form>
      <p class="hint">${() => (cloud() ? t("words.cloud") : t("words.local"))}</p>
      ${shell("ROOKEY_WORDS")}
    </div>`;
}

/** "OpenAI needs a key" with the way to the place where keys go. */
function needsKey(id) {
  const open = () => {
    ui.open = "system";
    ui.provider = { id, where: "keys", doing: "edit" };
    focus(`#key-${id}-keys`);
  };
  const link = html`<a href="#providers" @click="${open}">${t("reader.add-key")}</a>`;
  return html`<p class="problem">${tx("reader.needs-key", { name: provider(id).name, link })}</p>`;
}

// ---- system ----------------------------------------------------------------

/** What applies to all of rookey: keys, updates, files, and the macOS clipboard. */
function System() {
  const saved = () => ui.s.providers.filter((p) => p.saved || p.env).length;
  const summary = () => parts(
    saved() ? t("sum.keys", { n: saved() }) : t("sum.no-keys"),
    t(values().ROOKEY_AUTOUPDATE === "0" ? "sum.updates.check" : "sum.updates"),
  );
  return Section("system", t("section.system"), summary, html`
      <div class="sub" id="providers">
        <h3 id="providers-title">${t("keys.title")}</h3>
        <ul class="rows" aria-labelledby="providers-title">${() => ui.s.providers.map((p) => ProviderRow(p, "keys"))}</ul>
      </div>

      <div class="sub" id="updates-field">
        <h3>${t("updates.title")}</h3>
        ${toggle("autoupdate", t("updates.auto"), t("updates.auto.about"), () => values().ROOKEY_AUTOUPDATE !== "0", (e) =>
          save({ ROOKEY_AUTOUPDATE: e.target.checked ? "" : "0" }))}
        ${shell("ROOKEY_AUTOUPDATE")}
      </div>

      <div class="sub">
        <h3>${t("files.title")}</h3>
        <p class="hint">${() => tx("files", { config: code(ui.s.path), keys: code(ui.s.keys_path) })}</p>
      </div>
      ${() => (ui.s.os === "macos" ? Clipboard() : "")}
`);
}

/** Which sounds, a Play button for each, and a file of your own in place of any. */
function Sounds() {
  const setting = (cue) => `ROOKEY_SOUND_${cue.toUpperCase()}`;
  const play = async (cue) => {
    try {
      await call("/api/sound", { cue });
    } catch (e) {
      say("status.failed", { why: e.message }, true);
    }
  };
  return html`
    <div class="sub" id="sounds-field">
      <h3 id="sounds-title">${t("sounds.set")}</h3>
      <div class="chips" role="radiogroup" aria-labelledby="sounds-title">
        ${SOUND_SETS.map((id) =>
          chip("radio", "sounds", id, () => soundSet() === id, t(`sounds.${id}`), () => save({ ROOKEY_SOUNDS: id === SOUND_SETS[0] ? "" : id })))}
      </div>
      <p class="hint">${() => t(`sounds.${soundSet()}.about`)}</p>
      <ul class="rows">
        ${CUES.map((cue) => {
          const id = `sound-${cue}`;
          const label = t("sounds.play-cue", { cue: t(`cue.${cue}`) });
          return html`
          <li class="row">
            <span class="row-text row-wide">
              <label class="choice-name" for="${id}">${t(`cue.${cue}`)}</label>
              <span class="with-button">
                <input class="typed-input" id="${id}" type="text" spellcheck="false" autocomplete="off" placeholder="${t("cue.builtin")}"
                  .value="${() => values()[setting(cue)]}" @change="${(e) => save({ [setting(cue)]: e.target.value.trim() })}">
                <button type="button" class="button" aria-label="${label}" @click="${() => play(cue)}">${t("sounds.play")}</button>
              </span>
            </span>
          </li>`;
        })}
      </ul>
      <p class="hint">${t("sounds.files")}</p>
      ${shell("ROOKEY_SOUNDS")}
    </div>`;
}

/** macOS types by pasting: whether the clipboard is put back afterwards. On unless 0. */
function Clipboard() {
  return html`
    <div class="sub" id="clipboard-field">
      <h3>${t("clipboard.title")}</h3>
      ${toggle("keep-clipboard", t("clipboard"), t("clipboard.about"), () => values().ROOKEY_KEEP_CLIPBOARD !== "0", (e) =>
        save({ ROOKEY_KEEP_CLIPBOARD: e.target.checked ? "" : "0" }))}
      ${shell("ROOKEY_KEEP_CLIPBOARD")}
    </div>`;
}

/** A provider's key, with what can be done with it. The same row shows under Engine and Keys. */
function ProviderRow(p, where) {
  const doing = () => (ui.provider?.id === p.id && ui.provider.where === where ? ui.provider.doing : null);
  const show = (what) => () => {
    ui.provider = what && { id: p.id, where, doing: what };
    if (what === "edit") focus(`#key-${p.id}-${where}`);
    if (what === "remove") focus(`#keep-${p.id}-${where}`);
  };
  const does = has(`provider.${p.id}`) ? t(`provider.${p.id}`) : p.does;
  const saveKey = async (e) => {
    e.preventDefault();
    const input = e.target.querySelector("input");
    if (!input.value.trim()) return input.focus();
    if (await send("/api/key", { provider: p.id, key: input.value.trim() })) {
      ui.provider = null;
      say("key.s.saved", { name: p.name, path: ui.s.keys_path });
    }
  };
  const remove = async () => {
    ui.provider = null;
    if (await send("/api/key", { provider: p.id, key: "" })) say("key.s.removed", { name: p.name, path: ui.s.keys_path });
  };
  const account = html`<a href="${p.site}" target="_blank" rel="noreferrer noopener">${t("key.account", { name: p.name })}</a>`;
  return html`
    <li class="row">
      <span class="row-text">
        <span class="choice-name">${where === "engine" ? t("key.yours", { name: p.name }) : p.name}</span>
        <span class="choice-about">${p.saved
          ? tx("key.saved", { hint: code(p.hint || t("key.short")) })
          : where === "engine" ? t("key.none") : t("key.none.does", { does })}</span>
        ${p.env ? html`<span class="choice-about">${t("key.shell", { var: p.var })}</span>` : ""}
      </span>
      <span class="actions">${() =>
        doing()
          ? ""
          : p.saved
            ? html`<button type="button" class="button" @click="${show("edit")}">${t("key.replace")}</button><button type="button" class="button" @click="${show("remove")}">${t("key.remove")}</button>`
            : html`<button type="button" class="${where === "engine" ? "button is-primary" : "button"}" @click="${show("edit")}">${t("key.add")}</button>`}</span>
      ${() =>
        doing() === "edit"
          ? html`
            <form class="row-wide" @submit="${saveKey}">
              <div class="with-button">
                <input class="typed-input" id="${`key-${p.id}-${where}`}" type="password" spellcheck="false" autocomplete="off"
                  aria-label="${t("key.label", { name: p.name })}" placeholder="${t("key.paste")}">
                <button type="submit" class="button is-primary">${t("key.save")}</button>
                <button type="button" class="button" @click="${show(null)}">${t("key.cancel")}</button>
              </div>
              <p class="hint">${tx("key.where", { link: account })}</p>
            </form>`
          : doing() === "remove"
            ? html`
              <div class="row-wide">
                <p class="confirm-text">${t("key.confirm")}</p>
                <div class="actions">
                  <button type="button" class="button is-danger" @click="${remove}">${t("key.remove.yes")}</button>
                  <button type="button" class="button" id="${`keep-${p.id}-${where}`}" @click="${show(null)}">${t("key.remove.no")}</button>
                </div>
              </div>`
            : ""}
    </li>`;
}

// ---- hotkey --------------------------------------------------------------

/** "listen" when rookey reads the keys itself, "desktop" when a compositor bind runs it. */
function hotkeyWay() {
  if (ui.hotkey.way) return ui.hotkey.way;
  if (ui.s.listen.chord) return "listen";
  return ui.s.hotkey.bound || ui.s.listen.blocked ? "desktop" : "listen";
}

const desktopName = () => (ui.s.hotkey.desktop === "niri" ? "niri" : "Hyprland");

/** Changes what the hotkey part of the page shows. A change clears what was taken or failed. */
function redraw(changes) {
  Object.assign(ui.hotkey, { taken: null, problem: "" }, changes);
}

function Hotkey() {
  const mine = ui.hotkey;
  const listening = () => hotkeyWay() === "listen";
  const writable = () => (listening() ? !ui.s.listen.blocked : ui.s.hotkey.writable);
  const set = () => (listening() ? (ui.s.listen.chord ? { chord: ui.s.listen.chord } : null) : ui.s.hotkey.bound);
  const editing = () => writable() && (mine.editing || !set());
  const manual = () => !listening() && !ui.s.hotkey.writable;

  const pickWay = async (e) => {
    const way = e.target.value;
    Object.assign(mine, { way, editing: false, chord: null, files: false, pressing: false });
    // both at once would start and stop a recording on one press
    if (way === "desktop" && ui.s.listen.chord) {
      if (await send("/api/hotkey", { unlisten: true }, "hotkey.s.stopping")) say("hotkey.s.use-toggle");
    }
    redraw({});
  };
  const unbind = async () => {
    if (listening()) {
      const was = ui.s.listen.chord;
      if (await send("/api/hotkey", { unlisten: true }, "hotkey.s.stopping")) say("hotkey.s.unlistened", { chord: shown(was) });
      return redraw({ way: "listen", editing: false, chord: null });
    }
    const was = ui.s.hotkey.bound;
    if (await send("/api/hotkey", { unbind: true }, "hotkey.s.unbinding")) say("hotkey.s.unbound", { ...was, chord: shown(was.chord) });
    redraw({ editing: false, chord: null });
  };

  const summary = () => {
    const chord = code(keys(set().chord));
    if (!listening()) return tx("hotkey.bound", { chord, file: code(set().file) });
    if (!ui.s.listen.running) return tx("hotkey.not-running", { chord });
    // on macOS Rookey hears the keys itself, from login, and keeps all but a lone modifier
    if (ui.s.os === "macos") return [...tx("hotkey.hold.rookey", { chord }), ui.s.listen.swallowed ? t("hotkey.swallowed.rookey") : ""];
    const more = ui.s.listen.swallowed ? t("hotkey.swallowed", { desktop: desktopName() }) : t("hotkey.passes");
    return [...tx("hotkey.hold", { chord }), more];
  };

  const line = () => {
    if (!set()) return t("sum.unset");
    if (!listening()) return tx("sum.toggle", { chord: keys(set().chord) });
    return tx(ui.s.listen.running ? "sum.hold" : "sum.not-running", { chord: keys(set().chord) });
  };

  return Section("hotkey", t("hotkey.title"), line, html`
      <div class="choices one-about" role="radiogroup" aria-labelledby="hotkey-title">
        ${choice("hotkey-way", "listen", listening, t("hotkey.listen"), t("hotkey.listen.about"), pickWay,
          () => Boolean(ui.s.listen.blocked) && !ui.s.listen.chord)}
        ${choice("hotkey-way", "desktop", () => !listening(), t("hotkey.desktop"),
          tx("hotkey.desktop.about", { cmd: code("rookey toggle") }), pickWay)}
      </div>
      <p class="problem" hidden="${() => !(listening() && ui.s.listen.blocked)}">${() => ui.s.listen.blocked || ""}</p>

      <div hidden="${() => !writable() || editing()}">
        <p class="key-line">${() => (set() && !editing() ? summary() : "")}</p>
        <div class="actions">
          <button type="button" class="button" @click="${() => (redraw({ editing: true }), focus("#chord"))}">${t("hotkey.change")}</button>
          <button type="button" class="button" @click="${unbind}">${() => t(listening() ? "hotkey.unlisten" : "hotkey.unbind")}</button>
        </div>
      </div>

      ${() => (editing() ? HotkeyForm(listening(), set()) : "")}
      <p class="problem" role="alert" hidden="${() => !mine.problem}">${() => mine.problem}</p>

      ${() => (manual() ? ManualHotkey() : "")}

    `);
}

// ---- while you talk ----------------------------------------------------------

/** What rookey does while it records: the sounds, the pill on screen, the notifications. */
function Talk() {
  const pill = () => !isOn(values().ROOKEY_NO_OVERLAY);
  const pillStyle = () => (PILL_STYLES.includes(values().ROOKEY_PILL) ? values().ROOKEY_PILL : PILL_STYLES[0]);
  const summary = () => parts(
    isOn(values().ROOKEY_QUIET) ? t("sum.quiet") : t("sum.sounds", { name: t(`sounds.${soundSet()}`) }),
    pill() ? t("sum.pill", { name: t(`pill.${pillStyle()}`) }) : t("sum.no-pill"),
    !isOn(values().ROOKEY_NO_NOTIFICATIONS) && t("sum.notify"),
  );
  return Section("talk", t("section.talk"), summary, html`
      ${toggle("sounds", t("sounds"), t("sounds.about"), () => !isOn(values().ROOKEY_QUIET), (e) =>
        save({ ROOKEY_QUIET: e.target.checked ? "" : "1" }))}
      <div hidden="${() => isOn(values().ROOKEY_QUIET)}">${More(t("more.sounds"), Sounds())}</div>
      ${toggle("overlay", t("overlay"), tx("overlay.about", { cmd: code("rookey status --follow") }), pill, (e) =>
        save({ ROOKEY_NO_OVERLAY: e.target.checked ? "" : "1" }))}
      ${() => (pill() ? html`${PillStyle()}${More(t("more.place"), PillPlace())}` : "")}
      ${toggle("notifications", t("notifications"), t("notifications.about"), () => !isOn(values().ROOKEY_NO_NOTIFICATIONS), (e) =>
        save({ ROOKEY_NO_NOTIFICATIONS: e.target.checked ? "" : "1" }))}
    `);
}


/** The pill's look: the first is the default and saved as empty. */
function PillStyle() {
  const style = () => (PILL_STYLES.includes(values().ROOKEY_PILL) ? values().ROOKEY_PILL : PILL_STYLES[0]);
  return html`
    <div class="sub" id="pill-field">
      <h3 id="pill-title">${t("pill.style")}</h3>
      <div class="chips" role="radiogroup" aria-labelledby="pill-title">
        ${PILL_STYLES.map((id) =>
          chip("radio", "pill", id, () => style() === id, t(`pill.${id}`), () => save({ ROOKEY_PILL: id === PILL_STYLES[0] ? "" : id })))}
      </div>
      <div class="pill-preview" aria-hidden="true">${PillDrawing(style)}</div>
      <p class="hint">${() => t(`pill.${style()}.about`)}</p>
    </div>`;
}

/** Where the pill goes: ROOKEY_PILL_AT as [x, y] percent, the bottom centre when unset. */
function pillAt() {
  if (ui.pill.at) return ui.pill.at;
  const set = /^\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*$/.exec(values().ROOKEY_PILL_AT || "");
  return set ? [Number(set[1]), Number(set[2])] : PILL_PLACES.bottom;
}

/** The preset the pill is at, or "custom". */
function pillPlace() {
  if (ui.pill.custom) return "custom";
  const [x, y] = pillAt();
  return Object.keys(PILL_PLACES).find((id) => PILL_PLACES[id][0] === x && PILL_PLACES[id][1] === y) ?? "custom";
}

/** Saves a place; the bottom centre is the default, saved as empty. */
async function savePillAt([x, y]) {
  ui.pill.at = [x, y]; // shown while it saves, so a quick second arrow key starts from here
  const [bx, by] = PILL_PLACES.bottom;
  await save({ ROOKEY_PILL_AT: x === bx && y === by ? "" : `${x},${y}` });
  ui.pill.at = null;
}

// The small screen, in its own units: the room the pill's top-left corner has inside the
// margins, as overlay::spot has it, with a menu bar on top and a Dock below.
const MINI = { w: 320, h: 200, left: 8, top: 18, roomW: 236, roomH: 144, pillW: 64, pillH: 10 };
const clamp = (n) => Math.min(100, Math.max(0, n));

function PillPlace() {
  const pick = (id) => () => {
    ui.pill.custom = id === "custom";
    if (id !== "custom") savePillAt(PILL_PLACES[id]);
  };
  const custom = () => pillPlace() === "custom";
  const px = () => MINI.left + (MINI.roomW * pillAt()[0]) / 100;
  const py = () => MINI.top + (MINI.roomH * pillAt()[1]) / 100;
  const words = () => ({ x: Math.round(pillAt()[0]), y: Math.round(pillAt()[1]) });
  // where the pointer is, as the place of the pill centred under it
  const from = (e) => {
    const box = e.currentTarget.getBoundingClientRect();
    const x = ((e.clientX - box.left) / box.width) * MINI.w - MINI.pillW / 2;
    const y = ((e.clientY - box.top) / box.height) * MINI.h - MINI.pillH / 2;
    return [Math.round(clamp(((x - MINI.left) / MINI.roomW) * 100)), Math.round(clamp(((y - MINI.top) / MINI.roomH) * 100))];
  };
  let dragging = false;
  const down = (e) => {
    dragging = true;
    e.currentTarget.setPointerCapture(e.pointerId);
    ui.pill.at = from(e);
  };
  const move = (e) => {
    if (dragging) ui.pill.at = from(e);
  };
  const up = () => {
    if (!dragging) return;
    dragging = false;
    savePillAt(pillAt());
  };
  const key = (e) => {
    const step = e.shiftKey ? 10 : 1;
    const by = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key];
    if (!by) return;
    e.preventDefault();
    const [x, y] = pillAt();
    savePillAt([clamp(Math.round(x) + by[0]), clamp(Math.round(y) + by[1])]);
  };
  return html`
    <div class="sub" id="pill-where-field">
      <h3 id="pill-where">${t("pill.where")}</h3>
      <div class="chips" role="radiogroup" aria-labelledby="pill-where">
        ${[...Object.keys(PILL_PLACES), "custom"].map((id) =>
          chip("radio", "pill-where", id, () => pillPlace() === id, t(`pill.where.${id}`), pick(id)))}
      </div>
      ${() => (custom() ? html`
        <svg class="pill-where" viewBox="0 0 320 200" tabindex="0" role="img" aria-label="${() => t("pill.where.label", words())}"
          @pointerdown="${down}" @pointermove="${move}" @pointerup="${up}" @pointercancel="${up}" @keydown="${key}">
          <rect class="pw-screen" x="1" y="1" width="318" height="198" rx="9"></rect>
          <rect class="pw-bar" x="1" y="1" width="318" height="10"></rect>
          <rect class="pw-dock" x="85" y="182" width="150" height="12" rx="5"></rect>
          <rect class="pw-room" x="8" y="18" width="300" height="154" rx="4"></rect>
          <rect class="pw-pill" x="${px}" y="${py}" width="64" height="10" rx="5"></rect>
          <circle class="pw-dot" cx="${() => px() + 7}" cy="${() => py() + 5}" r="1.8"></circle>
        </svg>
        <p class="pill-where-at">${() => t("pill.where.readout", words())}</p>` : "")}
      <p class="hint">${() => t(custom() ? "pill.where.custom.about" : "pill.where.about")}</p>
    </div>`;
}

/** The pill as it looks while listening, drawn like overlay.rs draws it. The words under it say the same. */
function PillDrawing(style) {
  // one drawing for all three, parts hidden by style: a slot only updates when it is a function
  return html`
    <span class="pill-dot" hidden="${() => style() !== "dot"}"></span>
    <span class="pill" hidden="${() => style() === "dot"}">
      <span class="pill-rec"></span>
      <span class="pill-bars"><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
      <span class="pill-clock" hidden="${() => style() !== "full"}">0:04</span>
    </span>`;
}

function HotkeyForm(listening, set) {
  const mine = ui.hotkey;
  const hotkey = ui.s.hotkey;
  const name = desktopName();

  const bind = async (replace) => {
    const typed = $("#chord").value.trim();
    if (!typed) return $("#chord").focus();
    const chord = stored(typed);
    const asked = listening ? { chord, listen: true, replace } : { chord, file: mine.file, replace };
    const reply = await send("/api/hotkey", asked, listening ? "hotkey.s.listening" : "hotkey.s.binding", { chord: shown(chord) });
    if (!reply) return redraw({ chord: typed, problem: t(ui.status.key, ui.status.vars) });
    if (reply.taken) {
      say("hotkey.s.taken", { chord: shown(reply.taken.chord) });
      return redraw({ chord: typed, taken: reply.taken });
    }
    if (listening) say("hotkey.s.listens", { chord: shown(ui.s.listen.chord) });
    else say("hotkey.s.bound", { ...ui.s.hotkey.bound, chord: shown(ui.s.hotkey.bound.chord) });
    Object.assign(mine, { way: null, editing: false, chord: null, files: false, pressing: false });
    redraw({});
  };

  const press = async () => {
    if (!listening || mine.pressing) return redraw({ pressing: !mine.pressing });
    // from the keyboard itself: keys the compositor keeps from the browser come through too
    redraw({ pressing: true });
    const reply = await send("/api/hotkey", { capture: true }, "hotkey.s.press");
    if (!mine.pressing) return;
    if (!reply) return redraw({ pressing: false, problem: t(ui.status.key, ui.status.vars) });
    if (!reply.captured) {
      say("hotkey.s.none");
      return redraw({ pressing: false });
    }
    say("hotkey.s.got", { chord: shown(reply.captured) });
    redraw({ pressing: false, chord: shown(reply.captured) });
    focus("#hotkey-bind");
  };

  const hint = () =>
    mine.pressing
      ? t(listening ? "hotkey.pressing.listen" : "hotkey.pressing.desktop")
      : listening
        ? hotkey.writable ? tx("hotkey.hint.listen-bound", { desktop: name, mod: keys("Super") }) : tx("hotkey.hint.listen", { mod: keys("Super") })
        : t("hotkey.hint.desktop", { desktop: name });

  const where = () =>
    listening
      ? tx("hotkey.where.listen", { cmd: code("rookey listen") })
      : tx(hotkey.runs ? "hotkey.where.runs" : "hotkey.where", {
          file: code(mine.file || hotkey.file), runs: hotkey.runs ? code(hotkey.runs) : "", desktop: name,
        });

  const taken = () => {
    const { chord, file, line } = mine.taken;
    const outcome = t(listening ? "hotkey.taken.listen" : hotkey.desktop === "niri" ? "hotkey.taken.niri" : "hotkey.taken.other");
    return tx("hotkey.taken", { chord: code(keys(chord)), file: code(file), line, outcome });
  };

  return html`
    <form @submit="${(e) => (e.preventDefault(), bind(false))}">
      <div class="field">
        <label for="chord">${t("hotkey.keys")}</label>
        <div class="with-button">
          <input class="${() => (mine.pressing ? "typed-input is-listening" : "typed-input")}" id="chord" type="text" spellcheck="false" autocomplete="off"
            placeholder="${shown("Super+Shift+D")}" .value="${() => mine.chord ?? shown(set?.chord ?? (listening ? "Control_R" : "Super+Shift+D"))}"
            @input="${(e) => (mine.chord = e.target.value)}">
          <button type="button" class="button" @click="${press}">${() => t(mine.pressing ? "hotkey.pressing" : "hotkey.press")}</button>
        </div>
        <p class="hint">${hint}</p>
      </div>

      <div class="taken" role="alert" hidden="${() => !mine.taken}">
        <p>${() => (mine.taken ? taken() : "")}</p>
        <pre><code>${() => mine.taken?.text ?? ""}</code></pre>
        <div class="actions">
          <button type="button" class="button is-danger" @click="${() => bind(true)}">${t("hotkey.replace")}</button>
          <button type="button" class="button" @click="${() => (redraw({}), focus("#chord"))}">${t("hotkey.keep")}</button>
        </div>
      </div>

      <div class="actions" hidden="${() => Boolean(mine.taken)}">
        <button type="submit" class="button is-primary" id="hotkey-bind">${t(listening ? "hotkey.use" : "hotkey.bind")}</button>
        ${set ? html`<button type="button" class="button" @click="${() => redraw({ editing: false, chord: null, files: false, pressing: false })}">${t("hotkey.cancel")}</button>` : ""}
        ${listening ? "" : html`<button type="button" class="button is-quiet" @click="${() => redraw({ files: !mine.files })}">${t("hotkey.other-file")}</button>`}
      </div>
      <p class="hint">${where}</p>

      <div class="sub" hidden="${() => listening || !mine.files}">
        <h3 id="hotkey-files-title">${t("hotkey.files")}</h3>
        <div class="choices is-compact" role="radiogroup" aria-labelledby="hotkey-files-title">
          ${(hotkey.files || []).map((file) => html`
            <label class="choice">
              <input type="radio" name="hotkey-file" value="${file}" checked="${() => file === (mine.file || hotkey.file)}" @change="${() => (mine.file = file)}">
              <span class="choice-name">${file}</span>
            </label>`)}
        </div>
      </div>
    </form>`;
}

function ManualHotkey() {
  const hotkey = ui.s.hotkey;
  const id = () => (MANUAL[ui.desktop] ? ui.desktop : MANUAL[hotkey.desktop] ? hotkey.desktop : "niri");
  const blocked = hotkey.blocked || t(hotkey.desktop === "macos" ? "hotkey.manual.macos" : "hotkey.manual.other");
  return html`
    <div>
      <p class="hint">${blocked}</p>
      <div class="chips" id="desktops" role="radiogroup" aria-labelledby="hotkey-title">
        ${Object.entries(MANUAL).map(([key, d]) => chip("radio", "desktop", key, () => id() === key, d.name, () => (ui.desktop = key)))}
      </div>
      <div class="snippet">
        <pre><code>${() => MANUAL[id()].snippet}</code></pre>
        ${copyButton(() => MANUAL[id()].snippet)}
      </div>
      <p class="hint">${() => t("hotkey.manual.wtype", { hint: t(`manual.${id()}`) })}</p>
    </div>`;
}

// ---- history ---------------------------------------------------------------

const HISTORY_FIRST = 10; // shown before "Show all"

/** What was said lately, from the server. A list that hasn't changed isn't drawn again. */
async function loadHistory() {
  if (ui.stopped) return;
  try {
    merge(ui.history, await call("/api/history"), false);
  } catch {
    // a stopped server is said at the top already
  }
}

function History() {
  const h = ui.history;
  const on = () => !["0", "false"].includes(values().ROOKEY_HISTORY);
  const shown = () => (h.all ? h.entries : h.entries.slice(0, HISTORY_FIRST));
  const clear = async () => {
    h.clearing = false;
    const reply = await send("/api/history", { clear: true });
    if (reply) {
      merge(h, reply, false);
      say("history.s.cleared");
    }
  };
  return html`
    <details class="recent" id="history">
      <summary><h2 class="label" id="history-title">${t("history.title")}</h2><span class="summary-value">${() =>
        h.entries.length ? t("sum.recent", { n: h.entries.length }) : ""}</span></summary>
      <p class="hint">${() => tx("history.about", { keep: h.keep, path: code(h.path) })}</p>
      ${toggle("history-on", t("history.keep"), t("history.keep.about"), on, (e) => save({ ROOKEY_HISTORY: e.target.checked ? "" : "0" }))}
      ${shell("ROOKEY_HISTORY")}
      <p class="hint" hidden="${() => h.entries.length > 0}">${t("history.empty")}</p>
      <ul class="rows history" aria-labelledby="history-title">${() => shown().map(HistoryRow)}</ul>
      <div class="actions" hidden="${() => h.entries.length === 0 || h.clearing}">
        <button type="button" class="button" hidden="${() => h.all || h.entries.length <= HISTORY_FIRST}" @click="${() => (h.all = true)}">${() =>
          t("history.all", { n: h.entries.length })}</button>
        <button type="button" class="button" @click="${() => ((h.clearing = true), focus("#history-keep"))}">${t("history.clear")}</button>
      </div>
      <div class="actions" hidden="${() => !h.clearing}">
        <p class="confirm-text">${t("history.confirm")}</p>
        <button type="button" class="button is-danger" @click="${clear}">${t("history.clear.yes")}</button>
        <button type="button" class="button" id="history-keep" @click="${() => (h.clearing = false)}">${t("history.clear.no")}</button>
      </div>
    </details>`;
}

function HistoryRow(entry) {
  const at = new Date(entry.at);
  const when = at.toLocaleString(ui.lang, { dateStyle: "medium", timeStyle: "short" });
  return html`
    <li class="row">
      <span class="row-text">
        <time class="choice-about" datetime="${at.toISOString()}">${when}</time>
        <span class="history-text">${entry.text}</span>
      </span>
      <span class="actions">${copyButton(() => entry.text, t("history.copy", { time: when }))}</span>
    </li>`;
}

// ---- the example sentence and the voice test -----------------------------

function Specimen() {
  const clean = () => cloud() && isOn(values().ROOKEY_SANITIZE);
  const terms = () => termsMode() !== "off";
  const cut = (key) => html`<span class="cut">${t(key)}</span>`;
  // a preset rewrites the line; Custom can't be guessed, so the line before it shows, then the instruction
  const rewrite = () => (cloud() && editMode() !== "off" ? editMode() : "");
  const instruction = () => (ui.edit ?? values().ROOKEY_EDIT).trim();
  const typed = () => {
    const name = terms() ? html`<mark>spawn_model_loader</mark>` : html`<span>spawn model loader</span>`;
    const preset = rewrite() && rewrite() !== "custom" ? rewrite() : "";
    return tx(preset ? `typed.${preset}` : clean() ? "typed.clean" : "typed.raw", { name });
  };
  // a new caret, keyed by the line, each time the typed line changes: its blink starts afresh.
  // A redraw with the same line keeps the class it had, so an unrelated change doesn't blink it.
  let shown = null;
  let fresh = false;
  const caret = () => {
    const line = `${clean()}${terms()}${rewrite()}`;
    if (shown !== null && line !== shown) fresh = true;
    shown = line;
    return [html`<span class="${fresh ? "caret is-fresh" : "caret"}" aria-hidden="true"></span>`.key(line)];
  };
  return html`
    <div class="${() => ["specimen-body", clean() && "is-clean", terms() && "is-terms"].filter(Boolean).join(" ")}">
      <h2 class="visually-hidden" id="specimen-title">${t("specimen.title")}</h2>

      <p class="label">${t("specimen.say")}</p>
      <p class="said">${cut("said.um")}${t("said.a")}<span class="term">spawn model loader</span>${t("said.b")}${cut("said.the")}${t("said.c")}${cut("said.yk")}${t("said.end")}</p>

      <p class="label">${t("specimen.types")}<span class="label-tag" hidden="${() => !rewrite()}">${() =>
        rewrite() ? ` · ${t("specimen.rewritten", { name: t(`edit.${rewrite()}`) })}` : ""}</span></p>
      <p class="typed"><span>${typed}</span>${caret}</p>
      <p class="then" hidden="${() => rewrite() !== "custom" || !instruction()}">${() => t("specimen.then", { instruction: instruction() })}</p>

      <p class="caption">${t("specimen.caption")}</p>
      ${Trial()}
      ${History()}
    </div>`;
}

function Trial() {
  const trial = () => ui.s.trial;
  const heard = () => trial().phase === "done" && trial().text.trim() !== "";
  const words = () => {
    const { phase, error } = trial();
    if (phase === "done") return t(heard() ? "trial.heard" : "trial.silent");
    // the first line is the fault, the rest is advice for a terminal
    if (phase === "failed") return t("trial.failed", { why: error.split("\n")[0] });
    return t(`trial.${phase}`);
  };
  const click = async () => {
    const running = trial().phase === "listening";
    const reply = await send("/api/try", running ? { stop: true } : { start: true }, running ? "trial.working" : "trial.s.listening");
    if (reply) say(running ? "trial.s.transcribing" : "trial.s.recording");
  };
  return html`
    <section class="trial" aria-labelledby="trial-title">
      <h2 class="label" id="trial-title">${t("trial.title")}</h2>
      <p class="${() => ({ listening: "trial-state is-listening", failed: "trial-state is-problem" })[trial().phase] || "trial-state"}" role="status">${words}</p>
      <p class="typed" hidden="${() => !heard()}">${() => trial().text}</p>
      <p class="hint" hidden="${() => !heard()}">${() => t("trial.ready", { s: number(trial().waited_ms / 1000, 1) })}</p>
      <div class="actions">
        <button type="button" class="${() => (trial().phase === "listening" ? "button is-danger" : "button is-primary")}"
          disabled="${() => trial().phase === "working"}" @click="${click}">${() =>
          t({ listening: "trial.stop", working: "trial.stop", done: "trial.another" }[trial().phase] || "trial.record")}</button>
      </div>
    </section>`;
}

// ---- moving parts ----------------------------------------------------------

// While a download, a test or an update runs, ask how it is going.
let watching = false;
const moving = () =>
  Boolean(ui.s.models.download?.running) || ["listening", "working"].includes(ui.s.trial.phase) || ["checking", "downloading"].includes(ui.s.update.phase);

async function watch() {
  if (watching || ui.stopped || !moving()) return;
  watching = true;
  while (moving() && !ui.stopped) {
    await new Promise((done) => setTimeout(done, 400));
    let progress;
    try {
      progress = await call("/api/progress");
    } catch {
      break;
    }
    const wasDownloading = ui.s.models.download?.running;
    const wasTrying = ui.s.trial.phase;
    merge(ui.s.models, { download: progress.download }, false);
    merge(ui.s.trial, progress.trial);
    merge(ui.s.update, progress.update);
    if (wasTrying !== progress.trial.phase && progress.trial.phase === "done") say("trial.s.done");
    if (wasTrying !== progress.trial.phase && progress.trial.phase === "failed") say("trial.s.failed", {}, true);
    if (wasDownloading && !progress.download) {
      // it landed: the list of models on disk has changed
      take(await call("/api/state").catch(() => ui.s));
      say(ui.s.models.found ? "download.in-use" : "download.pick");
    }
  }
  watching = false;
}

// A chord is saved in the Linux names everywhere (Super, Alt, Control_R); people see their own keyboard's.
const OWN_KEYS = {
  macos: { Super: "⌘ Cmd", Alt: "⌥ Option", Ctrl: "⌃ Control", Control: "⌃ Control" },
  windows: { Super: "Win", Ctrl: "Ctrl", Control: "Ctrl", Alt: "Alt" },
};
const SIDES = { L: "Left", R: "Right" };

/** A saved chord as this system names its keys: Super+Shift+D is ⌘ Cmd+Shift+D on a Mac. */
function shown(chord) {
  const own = OWN_KEYS[ui.s?.os] || {};
  return String(chord).split("+").map((key) => {
    const [, name, side] = /^(Super|Alt|Ctrl|Control)(?:_([LR]))?$/.exec(key) || [];
    if (!own[name]) return key;
    return side ? `${SIDES[side]} ${own[name]}` : own[name];
  }).join("+");
}

// Windows has no glyph for its key: its logo, from Simple Icons (icons/windows.svg)
const WIN_LOGO = html`<svg class="win-logo" viewBox="0 0 24 24" aria-hidden="true"><path d="M0,0H11.377V11.372H0ZM12.623,0H24V11.372H12.623ZM0,12.623H11.377V24H0Zm12.623,0H24V24H12.623"/></svg>`;

/** A chord to show in a template: shown(), with the Windows logo before each Win. */
function keys(chord) {
  const text = shown(chord);
  if (ui.s?.os !== "windows") return text;
  return text.split(/(?<=^|\+|\s)(?=Win(?:\+|$))/).map((part) => (part.startsWith("Win") ? [WIN_LOGO, part] : part));
}

/** Back from what people see or type (Cmd, Win, Option, Right ⌘ Cmd) to the saved names. */
function stored(text) {
  const names = { cmd: "Super", command: "Super", win: "Super", windows: "Super", super: "Super", option: "Alt", opt: "Alt", alt: "Alt", control: "Ctrl", ctrl: "Ctrl" };
  return text.split("+").map((part) => {
    const words = part.replace(/[⌘⊞⌥⌃]/g, "").trim().split(/\s+/);
    const side = words.length === 2 && /^(left|right)$/i.test(words[0]) ? words.shift()[0].toUpperCase() : "";
    const name = names[words.join(" ").toLowerCase()];
    if (!name) return part.trim();
    // a side makes it a key of its own, which the compositors call Control_R, not Ctrl_R
    return side ? `${name === "Ctrl" ? "Control" : name}_${side}` : name;
  }).join("+");
}

/** Turns a key press into the name a compositor knows it by. */
function chordOf(e) {
  let key = KEY_NAMES[e.code];
  const match = /^(?:Key([A-Z])|Digit(\d)|(F\d{1,2})|Numpad(\d))$/.exec(e.code);
  if (match) key = match[1] || match[2] || match[3] || `KP_${match[4]}`;
  if (!key) return null;
  const mods = [e.metaKey && "Super", e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"];
  return [...mods.filter(Boolean), key].join("+");
}

// on the way down, before the page or the browser acts on the keys
window.addEventListener(
  "keydown",
  (e) => {
    if (!ui.hotkey.pressing) return;
    // rookey reads them from the keyboard, the page only keeps them from acting here
    if (hotkeyWay() === "listen") return e.preventDefault();
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") return redraw({ pressing: false });
    const chord = chordOf(e);
    if (!chord) return redraw({ pressing: false, problem: t("hotkey.unknown-key") });
    redraw({ pressing: false, chord: shown(chord) });
    focus("#hotkey-bind");
  },
  true,
);

async function start() {
  // drawn once, in the language and theme from the config; after that the parts update in place
  // a line to look at while the server gathers the state, which can take a moment
  $("#app").replaceChildren(Object.assign(document.createElement("p"), { className: "loading", textContent: t("status.loading") }));
  let failed = null;
  try {
    ui.s = await call("/api/state");
    // the shell wins over the config, here as everywhere
    const pref = (name) => ui.s.env[name] ?? values()[name];
    ui.theme = pref("ROOKEY_UI_THEME") || "system";
    ui.lang = pickLang(pref("ROOKEY_UI_LANG"));
    ui.ready = true;
  } catch (e) {
    failed = e;
  }
  applyLook();
  $("#app").replaceChildren();
  Page()($("#app"));
  if (failed) {
    say("status.failed", { why: failed.message }, true);
    stop();
    return;
  }
  say("status.saved-as-you-go", { path: ui.s.path });
  watch();
  // dictated in another window: the list is fresh when you come back
  loadHistory();
  window.addEventListener("focus", loadHistory);
  window.addEventListener("focus", recheck);

  // Held open so `rookey ui` can tell when this page is closed, and the page when rookey ui is.
  const line = new EventSource(`/api/alive?t=${encodeURIComponent(token)}`);
  // it reconnects by itself; give up only once rookey ui is really gone
  line.onerror = () => call("/api/state").catch(() => line.close());
}

start();
