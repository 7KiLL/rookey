// rookey settings. Every change is saved at once through the local server behind `rookey ui`.
// The page draws itself from the state the server sends back after each change.

const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];

/** An element with properties and children; strings become text. */
function el(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children.filter((child) => child !== null && child !== false));
  return node;
}

const code = (text) => el("code", {}, text);

const LANGUAGES = [
  ["en", "English"],
  ["uk", "Ukrainian"],
  ["ru", "Russian"],
  ["de", "German"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["pl", "Polish"],
];
const LANGUAGE_NAMES = Object.fromEntries(LANGUAGES);

const READERS = {
  ocr: {
    name: "This machine",
    provider: null,
    about:
      "tesseract reads the screenshot here, and only the words it picks go to the engine. It finds names written like_this, likeThis or LIKE_THIS, and misses jargon in plain lowercase.",
  },
  openai: {
    name: "OpenAI",
    provider: "openai",
    about:
      "The screenshot goes to OpenAI, where a vision model picks the terms, plain lowercase jargon included. Whatever is on your screen at that moment is in it.",
  },
  anthropic: {
    name: "Claude",
    provider: "anthropic",
    about:
      "The screenshot goes to Anthropic, where Claude picks the terms, plain lowercase jargon included. Whatever is on your screen at that moment is in it.",
  },
};

// For desktops whose config rookey doesn't write: the line to add by hand.
const MANUAL = {
  niri: {
    name: "niri",
    snippet: 'Mod+Shift+D repeat=false { spawn "rookey" "toggle"; }',
    hint: "Goes inside binds { } in ~/.config/niri/config.kdl.",
  },
  hyprland: {
    name: "Hyprland",
    snippet: 'hl.bind("SUPER + SHIFT + D", hl.dsp.exec_cmd("rookey toggle"))',
    hint: "Goes in ~/.config/hypr/hyprland.lua. Before Hyprland 0.55 it is bind = SUPER SHIFT, D, exec, rookey toggle in hyprland.conf.",
  },
  macos: {
    name: "macOS",
    snippet: "cmd + shift - d : rookey toggle",
    hint: "An skhd binding, for ~/.config/skhd/skhdrc. Raycast and Shortcuts can run rookey toggle too. Whatever runs it needs the Accessibility permission.",
  },
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

let state = null;
let stopped = false;
// choices that live in the page until there is something to save
const view = {
  otherLanguage: false,
  desktop: null,
  provider: null, // { id, where: "engine" | "keys", doing: "edit" | "remove" }
  hotkey: { way: null, editing: false, chord: null, file: null, files: false, taken: null, problem: "", pressing: false },
};

async function call(path, body) {
  const options = body ? { method: "POST", body: JSON.stringify(body) } : {};
  let response;
  try {
    response = await fetch(`${path}?t=${encodeURIComponent(token || "")}`, options);
  } catch {
    stop();
    throw new Error("rookey ui has stopped.");
  }
  const reply = await response.json();
  if (!response.ok) throw new Error(reply.error);
  return reply;
}

function stop() {
  if (stopped) return;
  stopped = true;
  $("#stopped").hidden = false;
  document.body.classList.add("is-stopped");
  $("#settings").inert = true;
  $("#specimen").inert = true;
}

function say(text, problem = false) {
  const status = $("#status");
  status.textContent = text;
  status.classList.toggle("is-problem", problem);
}

/** Sends a change. What comes back is the new state, or an answer to look at. */
async function send(path, body, saying = "Saving") {
  say(saying);
  try {
    const reply = await call(path, body);
    if (reply.values) state = reply;
    if (reply.trial) state.trial = reply.trial;
    return reply;
  } catch (e) {
    say(`That didn't work. ${e.message}`, true);
    return null;
  }
}

async function save(changes) {
  const saved = await send("/api/save", changes);
  if (saved) {
    say(`Saved to ${state.path}`);
    draw();
  }
  return Boolean(saved);
}

const isOn = (value) => value !== "" && value !== "0" && value !== "false";
const engine = () => state.values.ROOKEY_BACKEND || "local";
const cloud = () => engine() !== "local";
const reader = () => (state.values.ROOKEY_READER in READERS ? state.values.ROOKEY_READER : "ocr");
const provider = (id) => state.providers.find((p) => p.id === id);
const hasKey = (id) => provider(id).saved || provider(id).env;
const size = (mb) => (mb >= 1000 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`);

function termsMode() {
  const value = state.values.ROOKEY_CONTEXT;
  if (!isOn(value)) return "off";
  return value === "1" || value === "true" ? "screen" : "command";
}

/** The picked languages, "en,uk" as ["en", "uk"]; none picked means any. */
const languages = () => (state.values.ROOKEY_LANG || "").split(",").filter((l) => l && l !== "auto");

/** Sets a field from the saved state, unless it is being typed in right now. */
function fill(input, value) {
  if (document.activeElement !== input) input.value = value;
}

function choose(name, value) {
  for (const input of $$(`input[name="${name}"]`)) input.checked = input.value === value;
}

function chip(name, value, text) {
  return el("label", { className: "chip" }, el("input", { type: "radio", name, value }), el("span", {}, text));
}

/** "OpenAI needs a key" with the way to the place where keys go. */
function needsKey(node, id) {
  const missing = !hasKey(id);
  node.hidden = !missing;
  node.className = "problem";
  if (missing) {
    const link = el("a", { href: "#providers" }, "Add one under Advanced, Keys");
    link.addEventListener("click", () => {
      $("#advanced").open = true;
      view.provider = { id, where: "keys", doing: "edit" };
      drawProviders();
    });
    node.replaceChildren(`${provider(id).name} needs a key, and there is none yet. `, link, ".");
  }
}

function draw() {
  drawChecks();
  drawEngine();
  drawModels();
  drawLanguage();
  drawCleanup();
  drawTerms();
  drawHotkey();
  drawProviders();
  drawShell();
  drawSpecimen();
  drawTrial();
  watch();
}

function drawChecks() {
  const failing = state.checks.filter((c) => !c.ok);
  $("#setup").hidden = failing.length === 0;
  $("#ready").hidden = failing.length !== 0;
  $("#checks").replaceChildren(
    ...state.checks.map((c) => {
      const fix = c.fix
        ? el("span", { className: "with-button" }, el("code", { className: "fix" }, c.fix), copyButton(() => c.fix))
        : null;
      return el(
        "li",
        { className: `check ${c.ok ? "is-ok" : "is-missing"}` },
        el("span", { className: "check-mark", ariaLabel: c.ok ? "Ready" : "Missing", role: "img" }),
        el(
          "span",
          { className: "row-text" },
          el("span", { className: "choice-name" }, c.title),
          c.ok ? null : el("span", { className: "choice-about" }, ...linkEngine(c.missing)),
          fix,
        ),
      );
    }),
  );
}

/** "…under Engine." with Engine a link to that section. */
function linkEngine(text) {
  const [before, after] = text.split("under Engine");
  return after === undefined ? [text] : [before, "under ", el("a", { href: "#engine-title" }, "Engine"), after];
}

function copyButton(text) {
  const button = el("button", { type: "button", className: "button" }, "Copy");
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(text());
      button.textContent = "Copied";
    } catch {
      button.textContent = "Select it and copy";
    }
    setTimeout(() => (button.textContent = "Copy"), 2000);
  });
  return button;
}

function drawEngine() {
  choose("engine", cloud() ? "cloud" : "local");
  $("#stream").checked = engine() === "elevenlabs-realtime";
  $("#engine-key").hidden = !cloud();
  $("#stream-field").hidden = !cloud();
  $("#engine-key").replaceChildren(...(cloud() ? [providerRow(provider("elevenlabs"), "engine")] : []));

  // on this machine: which model, or the download of the usual one
  const models = state.models;
  const using = models.installed.find((m) => m.path === models.in_use);
  const usual = models.catalog[0];
  $("#engine-model").hidden = cloud() || !models.found;
  if (using) $("#engine-model").textContent = `Uses ${using.name || using.file}, ${size(using.mb)}. Other models are under Advanced.`;
  const offer = !cloud() && !models.found && !usual.installed;
  $("#engine-download").hidden = !offer;
  $("#engine-download").replaceChildren(...(offer ? [modelRow(usual)] : []));
}

function drawModels() {
  const models = state.models;
  const listed = models.installed.some((m) => m.path === models.in_use);

  $("#models-installed").replaceChildren(
    ...models.installed.map((m) => {
      const input = el("input", { type: "radio", name: "model", value: m.path, checked: m.path === models.in_use });
      input.addEventListener("change", () => save({ ROOKEY_MODEL: m.default ? "" : m.path }));
      const where = m.shown.slice(0, m.shown.lastIndexOf("/"));
      return el(
        "label",
        { className: "choice" },
        input,
        el(
          "span",
          { className: "choice-text" },
          el("span", { className: "choice-name" }, m.name || m.file),
          el("span", { className: "choice-about" }, `${size(m.mb)}, in ${where}`),
        ),
      );
    }),
  );

  const missing = $("#models-missing");
  missing.hidden = models.found;
  if (!models.found) {
    missing.replaceChildren("There is no model at ", code(models.in_use), ". Download one below, or point to a file.");
  }

  const wanted = models.catalog.filter((m) => !m.installed);
  $("#models-more-title").hidden = wanted.length === 0;
  $("#models-where").hidden = wanted.length === 0;
  $("#models-where").textContent = `Downloads go to ${models.dir}.`;
  $("#models-catalog").replaceChildren(...wanted.map(modelRow));
  drawDownload();

  fill($("#model"), listed ? "" : state.values.ROOKEY_MODEL);
}

function modelRow(m) {
  const download = el("button", { type: "button", className: "button" }, "Download");
  download.addEventListener("click", async () => {
    if (await send("/api/model", { download: m.file }, `Starting on ${m.name}`)) {
      say(`Downloading ${m.name}`);
      draw();
    }
  });
  const halt = el("button", { type: "button", className: "button", hidden: true }, "Stop");
  halt.addEventListener("click", async () => {
    await send("/api/model", { cancel: true }, "Stopping the download");
    say(`Stopped. Nothing of ${m.name} is kept.`);
    watch();
  });
  const row = el(
    "li",
    { className: "row model-row" },
    el(
      "span",
      { className: "row-text" },
      el("span", { className: "choice-name" }, m.name, el("span", { className: "choice-meta" }, `, ${size(m.mb)}`)),
      el("span", { className: "choice-about" }, m.about),
    ),
    el("span", { className: "actions" }, download, halt),
    el(
      "div",
      { className: "row-wide", hidden: true },
      el(
        "div",
        { className: "progress", role: "progressbar", ariaLabel: `Downloading ${m.name}`, ariaValueMin: "0", ariaValueMax: "100" },
        el("div", { className: "progress-done" }),
      ),
      el("p", { className: "progress-text" }),
    ),
    el("p", { className: "problem row-wide", hidden: true }),
  );
  row.dataset.file = m.file;
  return row;
}

/** The part of the models that moves by itself. */
function drawDownload() {
  const download = state.models.download;
  for (const row of $$(".model-row")) {
    const mine = download && download.file === row.dataset.file;
    const running = Boolean(mine && download.running);
    const [start, halt] = row.querySelectorAll("button");
    const [bar, problem] = row.querySelectorAll(".row-wide");
    start.hidden = running;
    start.disabled = Boolean(download && download.running && !mine);
    start.textContent = mine && download.error ? "Try again" : "Download";
    halt.hidden = !running;
    bar.hidden = !running;
    problem.hidden = !(mine && download.error);
    if (mine && download.error) problem.textContent = `The download didn't finish. ${download.error}`;
    if (running) {
      const part = download.total ? download.done / download.total : 0;
      row.querySelector(".progress-done").style.width = `${(part * 100).toFixed(1)}%`;
      row.querySelector(".progress").ariaValueNow = String(Math.round(part * 100));
      const mb = (bytes) => Math.round(bytes / 1048576);
      row.querySelector(".progress-text").textContent = download.total
        ? `${mb(download.done)} of ${mb(download.total)} MB`
        : "Connecting";
    }
  }
}

function drawLanguage() {
  const picked = languages();
  const known = LANGUAGES.map(([id]) => id);
  const extra = picked.filter((l) => !known.includes(l));
  $("#languages").replaceChildren(
    ...[...LANGUAGES, ...extra.map((l) => [l, l])].map(([id, name]) => {
      const input = el("input", { type: "checkbox", value: id, checked: picked.includes(id) });
      input.addEventListener("change", () => {
        const now = input.checked ? [...picked, id] : picked.filter((l) => l !== id);
        save({ ROOKEY_LANG: now.join(",") });
      });
      return el("label", { className: "chip" }, input, el("span", {}, name));
    }),
    (() => {
      const input = el("input", { type: "checkbox", checked: view.otherLanguage });
      input.addEventListener("change", () => {
        view.otherLanguage = input.checked;
        drawLanguage();
        if (input.checked) $("#language-code").focus();
      });
      return el("label", { className: "chip" }, input, el("span", {}, "Another…"));
    })(),
  );
  $("#language-other").hidden = !view.otherLanguage;
  const names = picked.map((l) => LANGUAGE_NAMES[l] || l);
  $("#language-about").textContent =
    picked.length === 0
      ? "Listening for any language."
      : picked.length === 1
        ? `Always ${names[0]}.`
        : `Listening for ${names.slice(0, -1).join(", ")} and ${names.at(-1)}.` +
          (cloud() ? " ElevenLabs tells languages apart on its own, among all of them." : "");
}

function drawCleanup() {
  $("#sanitize").checked = isOn(state.values.ROOKEY_SANITIZE);
  $("#sanitize-about").replaceChildren(
    ...(cloud()
      ? ["Takes out ", code("um"), ", ", code("uh"), ", ", code("you know"), ", false starts and noises, and fixes the punctuation."]
      : ["Whisper already skips most of those. On this machine the switch only mutes noises like ", code("[music]"), "."]),
  );
  fill($("#edit"), state.values.ROOKEY_EDIT);
  $("#edit").disabled = !cloud();
  $("#edit-field").classList.toggle("is-off", !cloud());
  $("#edit-hint").textContent = cloud()
    ? "ElevenLabs rewrites the finished text by this, and bills it extra. Leave it empty for no rewrite."
    : "Only ElevenLabs can rewrite. Whisper on this machine types what it hears.";
}

function drawTerms() {
  const terms = termsMode();
  $("#terms").checked = terms !== "off";
  $("#terms-about").textContent =
    terms === "command"
      ? "The terms come from your command, set under Advanced."
      : "Reads your screen as you start, so names like spawn_model_loader come out the way your code spells them." +
        (reader() === "ocr" ? " Nothing leaves your computer." : "");

  const chosen = READERS[reader()];
  choose("reader", reader());
  const about = $("#reader-about");
  about.replaceChildren(chosen.about);
  const key = $("#reader-key");
  key.hidden = true;
  if (chosen.provider && terms === "screen") needsKey(key, chosen.provider);

  const command = state.values.ROOKEY_CONTEXT;
  fill($("#command"), terms === "command" ? command : "");
  $("#command-hint").textContent =
    "One term per line. Runs through sh each time you start talking, instead of reading the screen. Empty reads the screen.";
  $("#terms-cost").hidden = !cloud() || terms === "off";
}

/** "listen" when rookey reads the keys itself, "desktop" when a compositor bind runs it. */
function hotkeyWay() {
  if (view.hotkey.way) return view.hotkey.way;
  if (state.listen.chord) return "listen";
  return state.hotkey.bound || state.listen.blocked ? "desktop" : "listen";
}

function drawHotkey() {
  const hotkey = state.hotkey;
  const listen = state.listen;
  const mine = view.hotkey;
  const listening = hotkeyWay() === "listen";

  choose("hotkey-way", listening ? "listen" : "desktop");
  $('input[name="hotkey-way"][value="listen"]').disabled = Boolean(listen.blocked) && !listen.chord;
  $("#listen-blocked").hidden = !(listening && listen.blocked);
  $("#listen-blocked").textContent = listen.blocked || "";
  $("#sounds").checked = !isOn(state.values.ROOKEY_QUIET);
  $("#notifications").checked = !isOn(state.values.ROOKEY_NO_NOTIFICATIONS);

  const writable = listening ? !listen.blocked : hotkey.writable;
  const set = listening ? (listen.chord ? { chord: listen.chord } : null) : hotkey.bound;
  const editing = writable && (mine.editing || !set);

  $("#hotkey-manual").hidden = listening || hotkey.writable;
  $("#hotkey-bound").hidden = !writable || editing;
  $("#hotkey-form").hidden = !editing;
  $("#hotkey-problem").hidden = !mine.problem;
  $("#hotkey-problem").textContent = mine.problem;

  if (!listening && !hotkey.writable) {
    $("#hotkey-blocked").textContent =
      hotkey.blocked ||
      (hotkey.desktop === "macos"
        ? "macOS keeps hotkeys in whichever app you bind them with, so this one is yours to add."
        : "rookey can set the hotkey itself on niri and Hyprland. On this desktop, bind the command in its own settings.");
    const desktop = MANUAL[view.desktop] || MANUAL[hotkey.desktop] || MANUAL.niri;
    choose("desktop", Object.keys(MANUAL).find((id) => MANUAL[id] === desktop));
    $("#snippet").textContent = desktop.snippet;
    $("#snippet-hint").textContent = `${desktop.hint} Typing the text needs wtype on Linux.`;
    return;
  }
  if (!writable) return;

  if (set) {
    $("#hotkey-summary").replaceChildren(
      ...(listening
        ? listen.running
          ? [
              "Hold ",
              code(set.chord),
              " to talk.",
              listen.swallowed
                ? ` ${hotkey.desktop === "niri" ? "niri" : "Hyprland"} keeps these keys from your windows.`
                : " These keys reach the window you are in as well.",
            ]
          : [code(set.chord), " is set, but rookey isn't listening. Set the keys again to start it."]
        : [code(set.chord), " starts and stops a recording. Set in ", code(set.file), "."]),
    );
    $("#hotkey-unbind").textContent = listening ? "Stop listening" : "Unbind";
  }
  if (!editing) return;

  const name = hotkey.desktop === "niri" ? "niri" : "Hyprland";
  fill($("#chord"), mine.chord ?? set?.chord ?? (listening ? "Control_R" : "Super+Shift+D"));
  $("#chord").classList.toggle("is-listening", mine.pressing);
  $("#chord-press").textContent = mine.pressing ? "Stop listening" : "Press keys";
  $("#chord-hint").textContent = mine.pressing
    ? listening
      ? "Press the keys now, rookey reads them from the keyboard. A key on its own, like the right Ctrl, counts as you let it go. Esc leaves it as it is."
      : "Press the combination now. Esc leaves it as it is."
    : listening
      ? hotkey.writable
        ? `A spare key like the right Ctrl, or a combination with Super. ${hotkey.desktop === "niri" ? "niri" : "Hyprland"} gets a bind that does nothing, so the keys don't type into your windows.`
        : "The keys reach the window you are in as well, so pick ones it has no use for: a spare key like the right Ctrl, or a combination with Super."
      : `Keys ${name} already uses never reach this page. Those you can type in, the way ${name} writes them.`;
  $("#hotkey-cancel").hidden = !set;
  $("#hotkey-actions").hidden = Boolean(mine.taken);
  $("#hotkey-other-file").hidden = listening;
  $("#hotkey-bind").textContent = listening ? "Use these keys" : "Bind";
  $("#hotkey-where").replaceChildren(
    ...(listening
      ? ["Runs ", code("rookey listen"), " as a service of your user, started with your desktop."]
      : [
          "Goes into ",
          code(mine.file || hotkey.file),
          hotkey.runs ? ", and runs " : "",
          hotkey.runs ? code(hotkey.runs) : "",
          `. ${name} checks the change first, and a change it won't take is undone.`,
        ]),
  );

  $("#hotkey-files").hidden = listening || !mine.files;
  $("#hotkey-file-list").replaceChildren(
    ...(hotkey.files || []).map((file) => {
      const input = el("input", { type: "radio", name: "hotkey-file", value: file, checked: file === (mine.file || hotkey.file) });
      input.addEventListener("change", () => {
        mine.file = file;
        drawHotkey();
      });
      return el("label", { className: "choice" }, input, el("span", { className: "choice-name" }, file));
    }),
  );

  $("#hotkey-taken").hidden = !mine.taken;
  if (mine.taken) {
    const { chord, file, line, text } = mine.taken;
    const outcome = listening
      ? "rookey would start as well, both on one press."
      : hotkey.desktop === "niri"
        ? "Bound here, it stops doing that."
        : "Bound here as well, both would run on one press.";
    $("#hotkey-taken-text").replaceChildren(code(chord), " already does something, in ", code(file), ` on line ${line}. ${outcome}`);
    $("#hotkey-taken-line").textContent = text;
  }
}

function drawProviders() {
  $("#provider-list").replaceChildren(...state.providers.map((p) => providerRow(p, "keys")));
  $("#files").replaceChildren(
    "Settings are in ",
    code(state.path),
    ". Keys are in ",
    code(state.keys_path),
    ", which only your user can read, and apart from the settings on purpose: what is in ~/.config tends to get synced and published with dotfiles. Environment variables win over both.",
  );
}

/** A provider's key, with what can be done with it. The same row shows under Engine and Keys. */
function providerRow(p, where) {
  const here = view.provider?.id === p.id && view.provider.where === where;
  const doing = here ? view.provider.doing : null;
  const status = p.saved
    ? el("span", { className: "choice-about" }, "Saved: ", code(p.hint || "a short key"))
    : el("span", { className: "choice-about" }, where === "engine" ? "No key yet." : `No key. ${p.does}`);
  const shell = p.env
    ? el("span", { className: "choice-about" }, `Your shell sets ${p.var} too, and it wins while it is set. A hotkey doesn't see your shell.`)
    : null;

  const redraw = () => (where === "engine" ? drawEngine() : drawProviders());
  const button = (text, act, className = "button") => {
    const b = el("button", { type: "button", className }, text);
    b.addEventListener("click", act);
    return b;
  };
  const show = (what) => () => {
    view.provider = what && { id: p.id, where, doing: what };
    redraw();
  };

  const actions = el("span", { className: "actions" });
  if (!doing && p.saved) actions.append(button("Replace", show("edit")), button("Remove", show("remove")));
  if (!doing && !p.saved) actions.append(button("Add key", show("edit"), where === "engine" ? "button is-primary" : "button"));

  const row = el(
    "li",
    { className: "row" },
    el("span", { className: "row-text" }, el("span", { className: "choice-name" }, where === "engine" ? `Your ${p.name} key` : p.name), status, shell),
    actions,
  );

  if (doing === "edit") {
    const input = el("input", {
      className: "typed-input", type: "password", spellcheck: false, autocomplete: "off",
      ariaLabel: `${p.name} key`, placeholder: "Paste the key",
    });
    const form = el(
      "form",
      { className: "row-wide" },
      el("div", { className: "with-button" }, input, el("button", { type: "submit", className: "button is-primary" }, "Save key"), button("Cancel", show(null))),
      el(
        "p",
        { className: "hint" },
        "Keys are made in your ",
        el("a", { href: p.site, target: "_blank", rel: "noreferrer noopener" }, `${p.name} account`),
        ". Once saved, this page never shows it again.",
      ),
    );
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      if (!input.value.trim()) return input.focus();
      if (await send("/api/key", { provider: p.id, key: input.value.trim() })) {
        view.provider = null;
        input.value = "";
        say(`${p.name} key saved to ${state.keys_path}`);
        draw();
      }
    });
    row.append(form);
    queueMicrotask(() => input.focus());
  }

  if (doing === "remove") {
    const remove = button(
      "Remove key",
      async () => {
        view.provider = null;
        if (await send("/api/key", { provider: p.id, key: "" })) say(`${p.name} key removed from ${state.keys_path}`);
        draw();
      },
      "button is-danger",
    );
    const keep = button("Keep it", show(null));
    row.append(
      el(
        "div",
        { className: "row-wide" },
        el("p", { className: "confirm-text" }, "Remove this key? It can't be read back from here."),
        el("div", { className: "actions" }, remove, keep),
      ),
    );
    queueMicrotask(() => keep.focus());
  }
  return row;
}

/** A setting the shell overrides, noted next to it. */
function drawShell() {
  for (const note of $$("[data-env]")) {
    const name = note.dataset.env;
    const set = name in state.env;
    note.hidden = !set;
    if (set) note.replaceChildren("Your shell sets ", code(`${name}=${state.env[name]}`), ". It wins over this page while it is set.");
  }
}

// The example sentence, cut up by what each setting does to it.
function drawSpecimen() {
  const clean = cloud() && $("#sanitize").checked;
  const terms = $("#terms").checked;

  const specimen = $("#specimen");
  specimen.classList.toggle("is-clean", clean);
  specimen.classList.toggle("is-terms", terms);

  const start = clean ? "We" : "Um, so we";
  const name = terms ? "spawn_model_loader" : "spawn model loader";
  const middle = clean ? "before the recording starts" : "before the, the recording starts, you know";
  const typed = $("#typed");
  const before = typed.textContent;
  typed.replaceChildren(`${start} need to call `, el(terms ? "mark" : "span", {}, name), ` ${middle}.`);
  $("#then").hidden = !(cloud() && $("#edit").value.trim());

  if (before && before !== typed.textContent) {
    const caret = $("#caret");
    caret.classList.remove("is-fresh");
    void caret.offsetWidth; // restart the blink
    caret.classList.add("is-fresh");
  }
}

function drawTrial() {
  const { phase, text, error, waited_ms } = state.trial;
  const says = $("#trial-state");
  const button = $("#trial-button");
  const heard = phase === "done" && text.trim() !== "";

  says.className = "trial-state";
  says.classList.toggle("is-listening", phase === "listening");
  says.classList.toggle("is-problem", phase === "failed");
  says.textContent = {
    idle: "A short recording, taken through the settings as they are now.",
    listening: "Listening. Say something, then stop.",
    working: "Transcribing",
    done: heard ? "rookey heard" : "Nothing was heard. Is the microphone you talk into the default one?",
    // the first line is the fault, the rest is advice for a terminal
    failed: `The test didn't work. ${error.split("\n")[0]}`,
  }[phase];

  $("#trial-text").hidden = !heard;
  $("#trial-text").textContent = text;
  $("#trial-note").hidden = !heard;
  $("#trial-note").textContent = `Ready ${(waited_ms / 1000).toFixed(1)} s after you stopped.`;

  button.textContent = { listening: "Stop", working: "Stop", done: "Record another" }[phase] || "Record a test";
  button.disabled = phase === "working";
  button.classList.toggle("is-danger", phase === "listening");
  button.classList.toggle("is-primary", phase !== "listening");
}

// While a download or a test runs, ask how it is going.
let watching = false;
const moving = () => Boolean(state.models.download?.running) || ["listening", "working"].includes(state.trial.phase);

async function watch() {
  if (watching || stopped || !moving()) return;
  watching = true;
  while (moving() && !stopped) {
    await new Promise((done) => setTimeout(done, 400));
    let progress;
    try {
      progress = await call("/api/progress");
    } catch {
      break;
    }
    const wasDownloading = state.models.download?.running;
    const wasTrying = state.trial.phase;
    state.models.download = progress.download;
    state.trial = progress.trial;
    drawDownload();
    drawTrial();
    if (wasTrying !== progress.trial.phase && progress.trial.phase === "done") say("The test is done. Nothing of it was typed or kept.");
    if (wasTrying !== progress.trial.phase && progress.trial.phase === "failed") say("The test didn't work.", true);
    if (wasDownloading && !progress.download) {
      // it landed: the list of models on disk has changed
      state = await call("/api/state").catch(() => state);
      say(state.models.found ? "The model is downloaded and in use." : "The model is downloaded. Pick it under Advanced, Local model.");
      draw();
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

function listen() {
  $("#readers").append(...Object.entries(READERS).map(([id, r]) => chip("reader", id, r.name)));
  $("#desktops").append(...Object.entries(MANUAL).map(([id, d]) => chip("desktop", id, d.name)));

  // engine
  for (const input of $$('input[name="engine"]')) {
    input.addEventListener("change", () => {
      const streaming = $("#stream").checked || !state.values.ROOKEY_BACKEND;
      save({ ROOKEY_BACKEND: input.value === "local" ? "" : streaming ? "elevenlabs-realtime" : "elevenlabs" });
    });
  }
  $("#stream").addEventListener("change", (e) => {
    save({ ROOKEY_BACKEND: e.target.checked ? "elevenlabs-realtime" : "elevenlabs" });
  });
  $("#model").addEventListener("change", (e) => save({ ROOKEY_MODEL: e.target.value }));
  $("#checks-again").addEventListener("click", async () => {
    say("Checking");
    try {
      state = await call("/api/state");
      draw();
      say(state.checks.every((c) => c.ok) ? "Everything it needs is here." : "Some things are still missing.");
    } catch (e) {
      say(e.message, true);
    }
  });

  // languages
  $("#language-code").addEventListener("keydown", (e) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    const code = e.target.value.trim().toLowerCase();
    if (!code) return;
    e.target.value = "";
    view.otherLanguage = false;
    const picked = languages();
    if (!picked.includes(code)) save({ ROOKEY_LANG: [...picked, code].join(",") });
    else drawLanguage();
  });

  // cleanup
  $("#sanitize").addEventListener("change", (e) => {
    drawSpecimen();
    save({ ROOKEY_SANITIZE: e.target.checked ? "1" : "" });
  });
  $("#edit").addEventListener("input", drawSpecimen);
  $("#edit").addEventListener("change", (e) => save({ ROOKEY_EDIT: e.target.value }));

  // screen terms
  $("#terms").addEventListener("change", (e) => {
    drawSpecimen();
    save({ ROOKEY_CONTEXT: e.target.checked ? "1" : "" });
  });
  $("#readers").addEventListener("change", (e) => save({ ROOKEY_READER: e.target.value === "ocr" ? "" : e.target.value }));
  $("#command").addEventListener("change", (e) => {
    const command = e.target.value.trim();
    // an emptied command goes back to reading the screen
    save({ ROOKEY_CONTEXT: command || "1" });
  });

  // Advanced stays open or closed the way it was left, in this browser
  try {
    $("#advanced").open = localStorage.getItem("rookey-advanced") === "open";
  } catch {
    // storage is off: it starts closed
  }
  $("#advanced").addEventListener("toggle", () => {
    try {
      localStorage.setItem("rookey-advanced", $("#advanced").open ? "open" : "");
    } catch {
      // nothing to remember it in
    }
  });

  listenHotkey();

  // the voice test
  $("#trial-button").addEventListener("click", async () => {
    const running = state.trial.phase === "listening";
    const reply = await send("/api/try", running ? { stop: true } : { start: true }, running ? "Transcribing" : "Listening");
    if (reply) say(running ? "Transcribing the test" : "Recording a test");
    drawTrial();
    watch();
  });
}

function listenHotkey() {
  const mine = view.hotkey;
  const redraw = (changes) => {
    Object.assign(mine, { taken: null, problem: "" }, changes);
    drawHotkey();
  };

  const bind = async (replace) => {
    const chord = $("#chord").value.trim();
    if (!chord) return $("#chord").focus();
    const listening = hotkeyWay() === "listen";
    const asked = listening ? { chord, listen: true, replace } : { chord, file: mine.file, replace };
    const reply = await send("/api/hotkey", asked, listening ? `Listening for ${chord}` : `Binding ${chord}`);
    if (!reply) return redraw({ chord, problem: $("#status").textContent });
    if (reply.taken) {
      say(`${reply.taken.chord} is taken. Nothing was changed.`);
      return redraw({ chord, taken: reply.taken });
    }
    say(listening ? `rookey listens for ${state.listen.chord}` : `${state.hotkey.bound.chord} is bound, in ${state.hotkey.bound.file}`);
    Object.assign(mine, { way: null, editing: false, chord: null, files: false, pressing: false });
    redraw({});
  };

  for (const input of $$('input[name="hotkey-way"]')) {
    input.addEventListener("change", async () => {
      Object.assign(mine, { way: input.value, editing: false, chord: null, files: false, pressing: false });
      // both at once would start and stop a recording on one press
      if (input.value === "desktop" && state.listen.chord) {
        if (await send("/api/hotkey", { unlisten: true }, "Stopping")) say("rookey stopped listening. Bind rookey toggle instead.");
      }
      redraw({});
    });
  }
  $("#sounds").addEventListener("change", (e) => save({ ROOKEY_QUIET: e.target.checked ? "" : "1" }));
  $("#notifications").addEventListener("change", (e) => save({ ROOKEY_NO_NOTIFICATIONS: e.target.checked ? "" : "1" }));

  $("#hotkey-form").addEventListener("submit", (e) => {
    e.preventDefault();
    bind(false);
  });
  $("#hotkey-replace").addEventListener("click", () => bind(true));
  $("#hotkey-keep").addEventListener("click", () => {
    redraw({});
    $("#chord").focus();
  });
  $("#hotkey-change").addEventListener("click", () => {
    redraw({ editing: true });
    $("#chord").focus();
  });
  $("#hotkey-cancel").addEventListener("click", () => redraw({ editing: false, chord: null, files: false, pressing: false }));
  $("#hotkey-other-file").addEventListener("click", () => redraw({ files: !mine.files }));
  $("#hotkey-unbind").addEventListener("click", async () => {
    if (hotkeyWay() === "listen") {
      const was = state.listen.chord;
      if (await send("/api/hotkey", { unlisten: true }, "Stopping")) say(`rookey stopped listening for ${was}`);
      return redraw({ way: "listen", editing: false, chord: null });
    }
    const was = state.hotkey.bound;
    if (await send("/api/hotkey", { unbind: true }, "Unbinding")) say(`${was.chord} is unbound, ${was.file} is as it was before`);
    redraw({ editing: false, chord: null });
  });
  $("#chord").addEventListener("input", (e) => {
    mine.chord = e.target.value;
  });

  $("#chord-press").addEventListener("click", async () => {
    if (hotkeyWay() !== "listen" || mine.pressing) return redraw({ pressing: !mine.pressing });
    // from the keyboard itself: keys the compositor keeps from the browser come through too
    redraw({ pressing: true });
    const reply = await send("/api/hotkey", { capture: true }, "Press the keys");
    if (!mine.pressing) return;
    if (!reply) return redraw({ pressing: false, problem: $("#status").textContent });
    if (!reply.captured) {
      say("No keys were pressed. Nothing was changed.");
      return redraw({ pressing: false });
    }
    say(`Got ${reply.captured}`);
    $("#chord").value = reply.captured;
    redraw({ pressing: false, chord: reply.captured });
    $("#hotkey-bind").focus();
  });
  // on the way down, before the page or the browser acts on the keys
  window.addEventListener(
    "keydown",
    (e) => {
      if (!mine.pressing) return;
      // rookey reads them from the keyboard, the page only keeps them from acting here
      if (hotkeyWay() === "listen") return e.preventDefault();
      if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return redraw({ pressing: false });
      const chord = chordOf(e);
      if (!chord) return redraw({ pressing: false, problem: "That key goes by a name this page doesn't know. Type it in instead." });
      $("#chord").value = chord;
      redraw({ pressing: false, chord });
      $("#hotkey-bind").focus();
    },
    true,
  );
  // the line to add by hand
  $("#desktops").addEventListener("change", (e) => {
    view.desktop = e.target.value;
    drawHotkey();
  });
  $("#copy").addEventListener("click", async () => {
    const button = $("#copy");
    try {
      await navigator.clipboard.writeText($("#snippet").textContent);
      button.textContent = "Copied";
    } catch {
      button.textContent = "Select it and copy";
    }
    setTimeout(() => (button.textContent = "Copy"), 2000);
  });
}

async function start() {
  listen();
  try {
    state = await call("/api/state");
  } catch (e) {
    say(e.message, true);
    stop();
    return;
  }
  draw();
  say(`Changes are saved as you make them, to ${state.path}`);

  // Held open so `rookey ui` can tell when this page is closed, and the page when rookey ui is.
  const line = new EventSource(`/api/alive?t=${encodeURIComponent(token)}`);
  // it reconnects by itself; give up only once rookey ui is really gone
  line.onerror = () => call("/api/state").catch(() => line.close());
}

start();
