// The page's words, by language: every locales/<id>.json, served together by rookey ui.
// A new language is one more file there and nothing else. `{name}` is filled in by t() or tx();
// a sentence that counts has one text per plural form, picked with Intl.PluralRules.

const WORDS = await (await fetch("/locales.json")).json();
const en = WORDS.en;

/** Every language, as { id: its own name }, for the picker. */
export const LOCALES = Object.fromEntries(Object.entries(WORDS).map(([id, w]) => [id, w._name]));

let lang = "en";

export const setLang = (code) => (lang = code in WORDS ? code : "en");

/** The language to start in: the one saved in the config, else the browser's first we have. */
export function pickLang(saved) {
  if (saved in WORDS) return saved;
  const wanted = (navigator.languages || [navigator.language]).map((l) => l.slice(0, 2).toLowerCase());
  return wanted.find((l) => l in WORDS) || "en";
}

/** The text for a key, in its plural form when it counts {n}. */
function sentence(key, vars) {
  const [found, value] = key in WORDS[lang] ? [lang, WORDS[lang][key]] : ["en", en[key] ?? key];
  if (typeof value === "string") return value;
  return value[new Intl.PluralRules(found).select(Number(vars.n))] ?? value.other;
}

/** A sentence, with {name} filled in from vars. */
export function t(key, vars = {}) {
  const text = sentence(key, vars);
  return text.replace(/\{(\w+)\}/g, (all, name) => (name in vars ? String(vars[name]) : all));
}

/** A sentence with parts that are templates (code, links): the pieces, for a template to render. */
export function tx(key, vars = {}) {
  const text = sentence(key, vars);
  return text.split(/(\{\w+\})/).filter(Boolean).map((part) => {
    const name = /^\{(\w+)\}$/.exec(part)?.[1];
    return name && name in vars ? vars[name] : part;
  });
}

/** Whether a key has a word of its own, so text the server sent can be swapped for it. */
export const has = (key) => key in en;
