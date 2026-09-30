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
  advanced: false, // from the config too: every `rookey ui` is a new origin, with nothing kept
  otherLanguage: false,
  desktop: null,
  edit: null, // the rewriting instruction while it is typed, before it is saved
  provider: null, // { id, where: "engine" | "keys", doing: "edit" | "remove" }
  hotkey: { way: null, editing: false, chord: null, file: null, files: false, taken: null, problem: "", pressing: false },
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
const SOUND_SETS = ["rook", "notes", "pencil"]; // sound::SETS, the first is the default
const PILL_STYLES = ["full", "compact", "dot"]; // overlay::STYLES, the first is the default
const CUES = ["start", "stop", "typed", "failed"];
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

/** A button that copies, and says so for a moment. */
function copyButton(text) {
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
  return html`<button type="button" class="button" @click="${copy}">${() => t(b.label)}</button>`;
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
      ${() => (ui.ready ? html`${shell("ROOKEY_UI_THEME")}${shell("ROOKEY_UI_LANG")}` : "")}
    </header>`;
}

function Settings() {
  return html`${Checks()}${Engine()}${Languages()}${Cleanup()}${Words()}${Hotkey()}${Advanced()}`;
}

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

function chip(type, name, value, checked, text, change) {
  [checked, text] = [checked, text].map(live);
  return html`<label class="chip"><input type="${type}" name="${name}" value="${value}" checked="${checked}" @change="${change}"><span>${text}</span></label>`;
}

// ---- setup check -----------------------------------------------------------

/** A check's words: ours by its id where we have them, else what the server sent. */
function checkWords(c) {
  const engine = html`<a href="#engine-title">${t("engine.title")}</a>`;
  if (!has(`check.${c.id}.title`)) return { title: c.title, missing: [c.missing] };
  let title = t(`check.${c.id}.title`);
  let missing = tx(`check.${c.id}.missing`, { engine });
  if (c.id === "mic" && c.title.includes(": ")) title = t("check.mic.named", { name: c.title.split(": ").slice(1).join(": ") });
  // which tools, as the server found them with the environment winning over the config
  if (c.id === "screen") missing = tx("check.screen.missing", { tools: listOf(c.tools || []) });
  return { title, missing };
}

function Checks() {
  const failing = () => ui.s.checks.some((c) => !c.ok);
  return html`
    <section class="group" id="setup" aria-labelledby="setup-title" hidden="${() => !failing()}">
      <h2 id="setup-title">${t("setup.title")}</h2>
      <p class="about">${t("setup.about")}</p>
      <ul class="checks">
        ${() => ui.s.checks.map((c) => {
          const words = checkWords(c);
          return html`
            <li class="${`check ${c.ok ? "is-ok" : "is-missing"}`}">
              <span class="check-mark" role="img" aria-label="${t(c.ok ? "check.ready" : "check.missing")}"></span>
              <span class="row-text">
                <span class="choice-name">${words.title}</span>
                ${c.ok ? "" : html`<span class="choice-about">${words.missing}</span>`}
                ${c.fix ? html`<span class="with-button"><code class="fix">${c.fix}</code>${copyButton(() => c.fix)}</span>` : ""}
              </span>
            </li>`;
        })}
      </ul>
      <div class="actions">
        <button type="button" class="button" @click="${checkAgain}">${t("setup.again")}</button>
      </div>
    </section>
    <p class="ready" hidden="${failing}">${t("setup.ready")}</p>`;
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
  return html`
    <section class="group" aria-labelledby="engine-title">
      <h2 id="engine-title">${t("engine.title")}</h2>
      <p class="about">${t("engine.about")}</p>
      <div class="choices" role="radiogroup" aria-labelledby="engine-title">
        ${choice("engine", "local", () => !cloud(), t("engine.local"), t("engine.local.about"), pickEngine)}
        ${choice("engine", "cloud", cloud, t("engine.cloud"), t("engine.cloud.about"), pickEngine)}
      </div>
      ${shell("ROOKEY_BACKEND")}
      <ul class="rows" hidden="${() => !cloud()}">${() => (cloud() ? ProviderRow(provider("elevenlabs"), "engine") : "")}</ul>
      <div hidden="${() => !cloud()}">
        ${toggle("stream", t("engine.stream"), t("engine.stream.about"), () => engine() === "elevenlabs-realtime", (e) =>
          save({ ROOKEY_BACKEND: e.target.checked ? "elevenlabs-realtime" : "elevenlabs" }))}
      </div>
      <p class="hint" hidden="${() => cloud() || !ui.s.models.found || !using()}">${() =>
        using() ? t("engine.uses", { name: using().name || using().file, size: size(using().mb) }) : ""}</p>
      <ul class="rows" hidden="${() => !offer()}">${() => (offer() ? ModelRow(ui.s.models.catalog[0]) : "")}</ul>
    </section>`;
}

function Models() {
  const models = () => ui.s.models;
  const listed = () => models().installed.some((m) => m.path === models().in_use);
  const wanted = () => models().catalog.filter((m) => !m.installed);
  return html`
    <div class="sub" id="models">
      <h3 id="models-title">${t("models.title")}</h3>
      <p class="about">${t("models.about")}</p>
      <div class="choices" role="radiogroup" aria-labelledby="models-title">
        ${() => models().installed.map((m) =>
          choice("model", m.path, m.path === models().in_use, m.name || m.file,
            t("models.in", { size: size(m.mb), where: m.shown.slice(0, m.shown.lastIndexOf("/")) }),
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
  const start = async () => {
    if (await send("/api/model", { download: m.file }, "download.starting", { name: m.name })) say("download.running", { name: m.name });
  };
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
        <button type="button" class="button" hidden="${running}" disabled="${() => Boolean(download()?.running && !mine())}" @click="${start}">${() =>
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
    <section class="group" aria-labelledby="language-title">
      <h2 id="language-title">${t("lang.title")}</h2>
      <p class="about">${t("lang.about")}</p>
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
    </section>`;
}

// ---- cleanup and screen terms --------------------------------------------

function Cleanup() {
  const sanitize = () =>
    cloud()
      ? tx("sanitize.cloud", { um: code("um"), uh: code("uh"), yk: code("you know") })
      : tx("sanitize.local", { music: code("[music]") });
  const terms = () =>
    termsMode() === "command" ? t("terms.command") : t("terms.screen") + (reader() === "ocr" ? t("terms.local") : "");
  return html`
    <section class="group" aria-labelledby="cleanup-title">
      <h2 id="cleanup-title">${t("cleanup.title")}</h2>
      ${toggle("sanitize", t("sanitize"), sanitize, () => isOn(values().ROOKEY_SANITIZE), (e) =>
        save({ ROOKEY_SANITIZE: e.target.checked ? "1" : "" }))}
      ${shell("ROOKEY_SANITIZE")}
      ${toggle("terms", t("terms"), terms, () => termsMode() !== "off", (e) => save({ ROOKEY_CONTEXT: e.target.checked ? "1" : "" }))}
      ${shell("ROOKEY_CONTEXT")}
    </section>`;
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
    <section class="group" aria-labelledby="words-title">
      <h2 id="words-title">${t("words.title")}</h2>
      <p class="about">${t("words.about")}</p>
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
    </section>`;
}

/** "OpenAI needs a key" with the way to the place where keys go. */
function needsKey(id) {
  const open = () => {
    ui.advanced = true;
    ui.provider = { id, where: "keys", doing: "edit" };
    focus(`#key-${id}-keys`);
  };
  const link = html`<a href="#providers" @click="${open}">${t("reader.add-key")}</a>`;
  return html`<p class="problem">${tx("reader.needs-key", { name: provider(id).name, link })}</p>`;
}

function Advanced() {
  const toggled = (e) => {
    ui.advanced = e.target.open;
    // the page opening it as it draws is not a change to save
    if (ui.advanced !== isOn(values().ROOKEY_UI_ADVANCED)) save({ ROOKEY_UI_ADVANCED: ui.advanced ? "1" : "" });
  };
  const editText = () => ui.edit ?? values().ROOKEY_EDIT;
  return html`
    <details class="group advanced" id="advanced" open="${() => ui.advanced}" @toggle="${toggled}">
      <summary>
        <span class="summary-text">
          <span class="summary-title">${t("advanced")}</span>
          <span class="about">${t("advanced.about")}</span>
        </span>
      </summary>

      ${Models()}

      <div class="${() => (cloud() ? "sub" : "sub is-off")}" id="edit-field">
        <h3><label for="edit">${t("edit.title")}</label></h3>
        <textarea id="edit" rows="3" maxlength="2000" placeholder="${t("edit.placeholder")}" disabled="${() => !cloud()}"
          .value="${editText}" @input="${(e) => (ui.edit = e.target.value)}"
          @change="${async (e) => (await save({ ROOKEY_EDIT: e.target.value })) && (ui.edit = null)}"></textarea>
        <p class="hint">${() => t(cloud() ? "edit.cloud" : "edit.local")}</p>
        ${shell("ROOKEY_EDIT")}
      </div>

      <div class="sub" id="reader-field">
        <h3 id="reader-title">${t("reader.title")}</h3>
        <div class="chips" role="radiogroup" aria-labelledby="reader-title">
          ${Object.entries(READERS).map(([id, r]) =>
            chip("radio", "reader", id, () => reader() === id, r.name(), () => save({ ROOKEY_READER: id === "ocr" ? "" : id })))}
        </div>
        <p class="hint">${() => t(`reader.${reader()}.about`)}</p>
        ${() => {
          const needs = READERS[reader()].provider;
          return needs && termsMode() === "screen" && !hasKey(needs) ? needsKey(needs) : "";
        }}
        ${shell("ROOKEY_READER")}

        <div class="field">
          <label for="command">${t("command")}</label>
          <input class="typed-input" id="command" type="text" spellcheck="false" autocomplete="off" placeholder="cat ~/.config/rookey/glossary.txt"
            .value="${() => (termsMode() === "command" ? values().ROOKEY_CONTEXT : "")}"
            @change="${(e) => save({ ROOKEY_CONTEXT: e.target.value.trim() || "1" })}">
          <p class="hint">${t("command.hint")}</p>
        </div>
        <p class="hint" hidden="${() => !cloud() || termsMode() === "off"}">${t("terms.cost")}</p>
      </div>

      ${Sounds()}

      ${() => (ui.s.os === "macos" ? Clipboard() : "")}

      <div class="sub" id="providers">
        <h3 id="providers-title">${t("keys.title")}</h3>
        <p class="about">${t("keys.about")}</p>
        <ul class="rows" aria-labelledby="providers-title">${() => ui.s.providers.map((p) => ProviderRow(p, "keys"))}</ul>
      </div>

      <div class="sub">
        <h3>${t("files.title")}</h3>
        <p class="hint">${() => tx("files", { config: code(ui.s.path), keys: code(ui.s.keys_path) })}</p>
      </div>
    </details>`;
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
      if (await send("/api/hotkey", { unlisten: true }, "hotkey.s.stopping")) say("hotkey.s.unlistened", { chord: was });
      return redraw({ way: "listen", editing: false, chord: null });
    }
    const was = ui.s.hotkey.bound;
    if (await send("/api/hotkey", { unbind: true }, "hotkey.s.unbinding")) say("hotkey.s.unbound", was);
    redraw({ editing: false, chord: null });
  };

  const summary = () => {
    const chord = code(set().chord);
    if (!listening()) return tx("hotkey.bound", { chord, file: code(set().file) });
    if (!ui.s.listen.running) return tx("hotkey.not-running", { chord });
    const more = ui.s.listen.swallowed ? t("hotkey.swallowed", { desktop: desktopName() }) : t("hotkey.passes");
    return [...tx("hotkey.hold", { chord }), more];
  };

  return html`
    <section class="group" aria-labelledby="hotkey-title">
      <h2 id="hotkey-title">${t("hotkey.title")}</h2>
      <p class="about">${t("hotkey.about")}</p>

      <div class="choices" role="radiogroup" aria-labelledby="hotkey-title">
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

      ${toggle("sounds", t("sounds"), t("sounds.about"), () => !isOn(values().ROOKEY_QUIET), (e) =>
        save({ ROOKEY_QUIET: e.target.checked ? "" : "1" }))}
      ${toggle("overlay", t("overlay"), tx("overlay.about", { cmd: code("rookey status --follow") }), () => !isOn(values().ROOKEY_NO_OVERLAY), (e) =>
        save({ ROOKEY_NO_OVERLAY: e.target.checked ? "" : "1" }))}
      ${() => (isOn(values().ROOKEY_NO_OVERLAY) ? "" : PillStyle())}
      ${toggle("notifications", t("notifications"), t("notifications.about"), () => !isOn(values().ROOKEY_NO_NOTIFICATIONS), (e) =>
        save({ ROOKEY_NO_NOTIFICATIONS: e.target.checked ? "" : "1" }))}
    </section>`;
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
    const chord = $("#chord").value.trim();
    if (!chord) return $("#chord").focus();
    const asked = listening ? { chord, listen: true, replace } : { chord, file: mine.file, replace };
    const reply = await send("/api/hotkey", asked, listening ? "hotkey.s.listening" : "hotkey.s.binding", { chord });
    if (!reply) return redraw({ chord, problem: t(ui.status.key, ui.status.vars) });
    if (reply.taken) {
      say("hotkey.s.taken", { chord: reply.taken.chord });
      return redraw({ chord, taken: reply.taken });
    }
    if (listening) say("hotkey.s.listens", { chord: ui.s.listen.chord });
    else say("hotkey.s.bound", ui.s.hotkey.bound);
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
    say("hotkey.s.got", { chord: reply.captured });
    redraw({ pressing: false, chord: reply.captured });
    focus("#hotkey-bind");
  };

  const hint = () =>
    mine.pressing
      ? t(listening ? "hotkey.pressing.listen" : "hotkey.pressing.desktop")
      : listening
        ? hotkey.writable ? t("hotkey.hint.listen-bound", { desktop: name }) : t("hotkey.hint.listen")
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
    return tx("hotkey.taken", { chord: code(chord), file: code(file), line, outcome });
  };

  return html`
    <form @submit="${(e) => (e.preventDefault(), bind(false))}">
      <div class="field">
        <label for="chord">${t("hotkey.keys")}</label>
        <div class="with-button">
          <input class="${() => (mine.pressing ? "typed-input is-listening" : "typed-input")}" id="chord" type="text" spellcheck="false" autocomplete="off"
            placeholder="Super+Shift+D" .value="${() => mine.chord ?? set?.chord ?? (listening ? "Control_R" : "Super+Shift+D")}"
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

// ---- the example sentence and the voice test -----------------------------

function Specimen() {
  const clean = () => cloud() && isOn(values().ROOKEY_SANITIZE);
  const terms = () => termsMode() !== "off";
  const cut = (key) => html`<span class="cut">${t(key)}</span>`;
  const typed = () => {
    const name = terms() ? html`<mark>spawn_model_loader</mark>` : html`<span>spawn model loader</span>`;
    return tx(clean() ? "typed.clean" : "typed.raw", { name });
  };
  // a new caret, keyed by the line, each time the typed line changes: its blink starts afresh.
  // A redraw with the same line keeps the class it had, so an unrelated change doesn't blink it.
  let shown = null;
  let fresh = false;
  const caret = () => {
    const line = `${clean()}${terms()}`;
    if (shown !== null && line !== shown) fresh = true;
    shown = line;
    return [html`<span class="${fresh ? "caret is-fresh" : "caret"}" aria-hidden="true"></span>`.key(line)];
  };
  return html`
    <div class="${() => ["specimen-body", clean() && "is-clean", terms() && "is-terms"].filter(Boolean).join(" ")}">
      <h2 class="visually-hidden" id="specimen-title">${t("specimen.title")}</h2>

      <p class="label">${t("specimen.say")}</p>
      <p class="said">${cut("said.um")}${t("said.a")}<span class="term">spawn model loader</span>${t("said.b")}${cut("said.the")}${t("said.c")}${cut("said.yk")}${t("said.end")}</p>

      <p class="label">${t("specimen.types")}</p>
      <p class="typed"><span>${typed}</span>${caret}</p>
      <p class="then" hidden="${() => !(cloud() && (ui.edit ?? values().ROOKEY_EDIT).trim())}">${t("specimen.then")}</p>

      <p class="caption">${t("specimen.caption")}</p>
      ${Trial()}
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

// While a download or a test runs, ask how it is going.
let watching = false;
const moving = () => Boolean(ui.s.models.download?.running) || ["listening", "working"].includes(ui.s.trial.phase);

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
    redraw({ pressing: false, chord });
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
    ui.advanced = isOn(values().ROOKEY_UI_ADVANCED);
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

  // Held open so `rookey ui` can tell when this page is closed, and the page when rookey ui is.
  const line = new EventSource(`/api/alive?t=${encodeURIComponent(token)}`);
  // it reconnects by itself; give up only once rookey ui is really gone
  line.onerror = () => call("/api/state").catch(() => line.close());
}

start();
