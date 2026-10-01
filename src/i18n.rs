//! rookey's words, by language: every locales/<id>.json, baked in by build.rs. The page fetches
//! the same files, so one translation covers the page, the CLI, the pill and the notifications.
//!
//! A value is a sentence with `{name}` slots, or, for a sentence that counts, one per plural
//! form: `{"one": "{n} word", "other": "{n} words"}` (the forms of Intl.PluralRules).

use serde_json::{Map, Value};
use std::{env, fmt::Display, sync::OnceLock};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

fn parsed() -> &'static [(&'static str, Map<String, Value>)] {
    static WORDS: OnceLock<Vec<(&str, Map<String, Value>)>> = OnceLock::new();
    WORDS.get_or_init(|| {
        LOCALES
            .iter()
            .map(|(id, text)| (*id, serde_json::from_str(text).unwrap_or_else(|e| panic!("locales/{id}.json: {e}"))))
            .collect()
    })
}

fn words(id: &str) -> Option<&'static Map<String, Value>> {
    parsed().iter().find(|(l, _)| *l == id).map(|(_, w)| w)
}

/// Whether there is a locales/<id>.json.
pub fn known(id: &str) -> bool {
    LOCALES.iter().any(|(l, _)| *l == id)
}

/// Every language as `{"en": {...}, "uk": {...}}`, for the page.
pub fn all_json() -> String {
    let rows: Vec<String> = LOCALES.iter().map(|(id, text)| format!("{id:?}: {text}")).collect();
    format!("{{{}}}", rows.join(","))
}

/// A sentence in rookey's language: `t!("cli.no-words")`, `t!("pill.typed", n = 3)`.
#[macro_export]
macro_rules! t {
    ($key:expr $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::t($key, &[$((stringify!($name), &$value as &dyn std::fmt::Display)),*])
    };
}

/// The language rookey speaks: ROOKEY_UI_LANG, else the system's, else English.
// ponytail: the system's language is read from LANG and friends only; Windows and macOS don't
// set them for apps, so there it's English until ROOKEY_UI_LANG names one. GetUserDefaultLocaleName
// and NSLocale are the way up.
pub fn lang() -> String {
    // tests read English whatever the shell's language is
    if cfg!(test) {
        return "en".into();
    }
    let wanted = crate::setting("ROOKEY_UI_LANG")
        .filter(|l| !l.is_empty())
        .or_else(|| ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| env::var(v).ok().filter(|l| !l.is_empty())))
        .unwrap_or_default();
    pick(&wanted).to_string()
}

/// "uk_UA.UTF-8" or "uk" as a language there are words for, else "en".
fn pick(wanted: &str) -> &'static str {
    let short = wanted.split(['_', '-', '.']).next().unwrap_or("").to_lowercase();
    LOCALES.iter().map(|(id, _)| *id).find(|id| *id == short).unwrap_or("en")
}

/// A sentence in rookey's language, with `{name}` filled in from vars.
pub fn t(key: &str, vars: &[(&str, &dyn Display)]) -> String {
    say(&lang(), key, vars)
}

fn say(lang: &str, key: &str, vars: &[(&str, &dyn Display)]) -> String {
    let found = words(lang).and_then(|w| w.get(key)).map(|v| (lang, v));
    let Some((lang, value)) = found.or_else(|| words("en")?.get(key).map(|v| ("en", v))) else {
        return key.to_string();
    };
    let text = match value {
        Value::Object(forms) => {
            let n = vars.iter().find(|(name, _)| *name == "n").and_then(|(_, v)| v.to_string().parse().ok()).unwrap_or(0);
            forms.get(plural(lang, n)).or_else(|| forms.get("other")).and_then(Value::as_str).unwrap_or(key)
        }
        v => v.as_str().unwrap_or(key),
    };
    let mut out = text.to_string();
    for (name, v) in vars {
        out = out.replace(&format!("{{{name}}}"), &v.to_string());
    }
    out
}

/// The plural form of n, as Intl.PluralRules names it.
// ponytail: the rules for the languages people have asked for; any other gets one/other, which
// is right for most of Western Europe. CLDR's plurals.json is the way up.
fn plural(lang: &str, n: u64) -> &'static str {
    let (ten, hundred) = (n % 10, n % 100);
    match lang {
        "uk" | "ru" | "be" => match ten {
            1 if hundred != 11 => "one",
            2..=4 if !(12..=14).contains(&hundred) => "few",
            _ => "many",
        },
        "pl" => match ten {
            _ if n == 1 => "one",
            2..=4 if !(12..=14).contains(&hundred) => "few",
            _ => "many",
        },
        _ if n == 1 => "one",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(text: &str) -> Vec<&str> {
        let mut found: Vec<&str> = text.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(s, _)| s)).collect();
        found.sort();
        found.dedup();
        found
    }

    fn texts(v: &Value) -> Vec<&str> {
        match v {
            Value::Object(forms) => forms.values().filter_map(Value::as_str).collect(),
            v => v.as_str().into_iter().collect(),
        }
    }

    #[test]
    fn every_translation_fits_english() {
        let en = words("en").unwrap();
        assert!(en.len() > 100);
        for (id, w) in parsed() {
            assert!(w.get("_name").and_then(Value::as_str).is_some(), "locales/{id}.json has no _name");
            for (key, value) in w {
                // a missing sentence falls back to English; an unknown one is a typo or a leftover
                let theirs = en.get(key).unwrap_or_else(|| panic!("locales/{id}.json: {key} isn't in en.json"));
                if key == "_name" {
                    continue;
                }
                let allowed: Vec<&str> = texts(theirs).into_iter().flat_map(slots).collect();
                for text in texts(value) {
                    for slot in slots(text) {
                        assert!(allowed.contains(&slot), "locales/{id}.json: {key} has {{{slot}}}, which English doesn't fill in");
                    }
                }
            }
        }
    }

    #[test]
    fn every_key_the_code_asks_for_is_there() {
        let en = words("en").unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut asked = 0;
        for file in std::fs::read_dir(src).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rs")) {
            let text = std::fs::read_to_string(&file).unwrap();
            // literal keys only: t!(&format!(...)) ones are checked where they're built
            let calls = text.match_indices("t!(\"").filter(|(at, _)| !text[..*at].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
            let starts = calls.map(|(at, m)| at + m.len()).chain(text.match_indices("i18n::t(\"").map(|(at, m)| at + m.len()));
            for start in starts.collect::<Vec<_>>() {
                let key = text[start..].split('"').next().unwrap();
                if key == "key" || key == "no.such.key" {
                    continue; // this file's own examples
                }
                asked += 1;
                assert!(en.contains_key(key), "{} asks for {key}, which en.json doesn't have", file.display());
            }
        }
        assert!(asked > 100);
    }

    #[test]
    fn the_shipped_languages_are_whole() {
        let en: Vec<&String> = words("en").unwrap().keys().collect();
        let uk: Vec<&String> = words("uk").unwrap().keys().collect();
        assert_eq!(en, uk);
    }

    #[test]
    fn falls_back_and_counts() {
        assert_eq!(pick("uk_UA.UTF-8"), "uk");
        assert_eq!(pick("C"), "en");
        assert_eq!(pick(""), "en");
        assert_eq!(say("xx", "pill.typed", &[("n", &1)]), "1 word");
        assert_eq!(say("en", "pill.typed", &[("n", &5)]), "5 words");
        assert_eq!(say("uk", "pill.typed", &[("n", &21)]), "21 слово");
        assert_eq!(say("uk", "pill.typed", &[("n", &3)]), "3 слова");
        assert_eq!(say("uk", "pill.typed", &[("n", &11)]), "11 слів");
        assert_eq!(say("en", "no.such.key", &[]), "no.such.key");
        assert_eq!(plural("pl", 22), "few");
        assert_eq!(plural("pl", 5), "many");
    }
}
