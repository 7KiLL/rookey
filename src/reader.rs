//! Screen terms: what is on the screen when you start talking, so the recognizer can spell
//! it. A screenshot is read here by tesseract, or by a vision model at a provider.

use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine;
use serde_json::{Value, json};

use crate::{Res, setting};

pub const READERS: [&str; 3] = ["ocr", "openai", "anthropic"];
const OPENAI_MODEL: &str = "gpt-6-luna";
const ANTHROPIC_MODEL: &str = "claude-opus-5-5";

const PROMPT: &str = "\
This is a screenshot of someone's screen. They are about to dictate, and the speech \
recognizer needs to know how to spell what they are likely to say. List the terms on screen \
that a recognizer would get wrong: function, variable and file names, project, product and \
people names, acronyms, jargon. Skip ordinary words. Write each term exactly as it is on \
screen, one per line, the most useful first, 60 at most. Reply with the list only.";

#[derive(Default)]
pub struct Context {
    pub text: String,
    /// One term per line, already picked by a model. Otherwise it is raw text to pick from.
    pub listed: bool,
}

/// The command line as the system's shell runs it.
fn shell(cmd: &str) -> Command {
    let (sh, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    let mut shell = Command::new(sh);
    shell.args([flag, cmd]);
    shell
}

pub fn from_command(cmd: &str) -> Res<Context> {
    vlog!(2, "context: running {cmd:?}");
    let out = shell(cmd).output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(crate::t!("reader.command-failed", status = out.status, why = err.trim()).into());
    }
    Ok(Context { text: String::from_utf8_lossy(&out.stdout).into_owned(), listed: false })
}

pub fn from_screen() -> Res<Context> {
    let reader = setting("ROOKEY_READER").unwrap_or_else(|| "ocr".into());
    vlog!(2, "context: reading the screen with {reader}");
    match reader.as_str() {
        "ocr" => Ok(Context { text: ocr(&screenshot("ppm")?)?, listed: false }),
        "openai" => Ok(Context { text: openai(&screenshot("jpeg")?)?, listed: true }),
        "anthropic" => Ok(Context { text: anthropic(&screenshot("jpeg")?)?, listed: true }),
        other => Err(crate::t!("reader.unknown", name = format!("{other:?}"), readers = READERS.join(", ")).into()),
    }
}

/// The focused output where the desktop can name it, else all of them.
/// ROOKEY_SCREENSHOT replaces it with any command that prints an image.
fn screenshot(format: &str) -> Res<Vec<u8>> {
    let out = match setting("ROOKEY_SCREENSHOT") {
        Some(cmd) => shell(&cmd).output()?,
        #[cfg(target_os = "macos")]
        None => return screencapture(format),
        #[cfg(not(target_os = "macos"))]
        None => {
            let mut grim = Command::new("grim");
            grim.args(["-t", format]);
            if let Some(output) = crate::desktop::detect().focused_output() {
                grim.args(["-o", &output]);
            }
            // ponytail: grim here and screencapture on macOS; Windows and X11 set ROOKEY_SCREENSHOT to a command of their own
            grim.arg("-").output().map_err(|e| crate::t!("reader.no-grim", why = e))?
        }
    };
    if !out.status.success() || out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(crate::t!("reader.no-shot", status = out.status, why = err.trim()).into());
    }
    vlog!(2, "context: screenshot, {} KB", out.stdout.len() / 1024);
    Ok(out.stdout)
}

/// The main display through macOS's own screencapture, which writes only to a file.
#[cfg(target_os = "macos")]
fn screencapture(format: &str) -> Res<Vec<u8>> {
    if !crate::mac::screen() {
        if crate::mac::is_app() {
            // puts Rookey in the list, where its switch is; the first time, it asks as well
            crate::mac::ask_screen();
            return Err(crate::t!("reader.mac-rookey").into());
        }
        return Err(crate::t!("reader.mac-app").into());
    }
    // tesseract reads png as well as ppm, which screencapture doesn't write
    let kind = if format == "jpeg" { "jpg" } else { "png" };
    let file = std::env::temp_dir().join(format!("rookey-screen-{}.{kind}", std::process::id()));
    // -x without the shutter sound, -m the main display, -t the format
    let out = Command::new("screencapture").args(["-x", "-m", "-t", kind]).arg(&file).output()?;
    let image = std::fs::read(&file);
    let _ = std::fs::remove_file(&file);
    match image {
        Ok(image) if out.status.success() && !image.is_empty() => {
            vlog!(2, "context: screenshot, {} KB", image.len() / 1024);
            Ok(image)
        }
        _ => Err(crate::t!("reader.no-shot", status = out.status, why = String::from_utf8_lossy(&out.stderr).trim()).into()),
    }
}

/// Tesseract's pack for a language in ROOKEY_LANG (ISO 639-1). None for a code not mapped here.
// ponytail: the page's languages and the common ones next to them; another code is read as English.
pub fn tesseract_lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "eng",
        "uk" => "ukr",
        "ru" => "rus",
        "be" => "bel",
        "bg" => "bul",
        "sr" => "srp",
        "de" => "deu",
        "es" => "spa",
        "fr" => "fra",
        "pl" => "pol",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "cs" => "ces",
        "sk" => "slk",
        "ro" => "ron",
        "hu" => "hun",
        "sv" => "swe",
        "da" => "dan",
        "no" | "nb" => "nor",
        "fi" => "fin",
        "tr" => "tur",
        "el" => "ell",
        "he" => "heb",
        "ar" => "ara",
        "hi" => "hin",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" => "chi_sim",
        "vi" => "vie",
        "id" => "ind",
        _ => return None,
    })
}

/// The packs in `tesseract --list-langs`: one per line under a header, which is
/// `List of available languages in "<dir>" (n):` in 5.x and has no dir in 4.x.
fn listed_langs(out: &str) -> Vec<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("List of")).map(str::to_string).collect()
}

/// The packs tesseract has here; none where it doesn't run.
pub fn installed_langs() -> Vec<String> {
    let out = Command::new("tesseract").arg("--list-langs").stderr(Stdio::null()).output();
    out.map(|o| listed_langs(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// The picked languages' packs that are installed, and the picked languages whose pack isn't.
/// English is left out of both: it is always read.
pub fn ocr_langs<'a>(picked: &'a [String], installed: &[String]) -> (Vec<&'static str>, Vec<&'a str>) {
    let (mut have, mut missing) = (Vec::new(), Vec::new());
    for code in picked {
        match tesseract_lang(code) {
            None | Some("eng") => {}
            Some(pack) if installed.iter().any(|i| i == pack) => have.push(pack),
            Some(_) => missing.push(code.as_str()),
        }
    }
    (have, missing)
}

fn ocr(image: &[u8]) -> Res<String> {
    // asked once per process: a pack installed meanwhile counts from the next run
    static INSTALLED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let picked = crate::languages();
    let (packs, missing) = match picked.is_empty() {
        true => Default::default(),
        false => ocr_langs(&picked, INSTALLED.get_or_init(installed_langs)),
    };
    if !missing.is_empty() {
        vlog!(1, "context: tesseract has no pack for {}, so that text is read as English", missing.join(", "));
    }
    let langs = ["eng"].into_iter().chain(packs).collect::<Vec<_>>().join("+");
    vlog!(2, "context: tesseract -l {langs}");
    let mut tesseract = Command::new("tesseract")
        .args(["-", "-", "-l", &langs])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| crate::t!("reader.no-tesseract", why = e))?;
    // written from its own thread: tesseract may start printing before it has read it all
    let mut stdin = tesseract.stdin.take().unwrap();
    let image = image.to_vec();
    let feed = std::thread::spawn(move || stdin.write_all(&image));
    let out = tesseract.wait_with_output()?;
    let _ = feed.join();
    if !out.status.success() {
        return Err(crate::t!("reader.tesseract-failed", status = out.status).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn key(name: &str, provider: &str) -> Res<String> {
    Ok(setting(name).ok_or_else(|| crate::t!("reader.needs", provider = provider, name = name))?)
}

fn post(url: &str, headers: &[(&str, &str)], body: &Value) -> Res<Value> {
    let mut req = ureq::post(url).header("content-type", "application/json");
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let t = std::time::Instant::now();
    let mut res = req
        .config()
        .http_status_as_error(false) // keep the error body, it says what went wrong
        .build()
        .send(body.to_string())?;
    let text = res.body_mut().read_to_string()?;
    vlog!(2, "http: {} from {url} in {} ms", res.status(), t.elapsed().as_millis());
    if !res.status().is_success() {
        return Err(crate::t!("reader.http", url = url, status = res.status(), why = text).into());
    }
    Ok(serde_json::from_str(&text)?)
}

fn openai(jpeg: &[u8]) -> Res<String> {
    let key = key("OPENAI_API_KEY", "OpenAI")?;
    let image = base64::engine::general_purpose::STANDARD.encode(jpeg);
    let body = json!({
        "model": setting("ROOKEY_READER_MODEL").unwrap_or_else(|| OPENAI_MODEL.into()),
        "input": [{
            "role": "user",
            "content": [
                // small text is the whole point, so no downscaling
                { "type": "input_image", "image_url": format!("data:image/jpeg;base64,{image}"), "detail": "high" },
                { "type": "input_text", "text": PROMPT },
            ],
        }],
    });
    let auth = format!("Bearer {key}");
    let res = post("https://api.openai.com/v1/responses", &[("authorization", &auth)], &body)?;
    Ok(openai_text(&res))
}

/// The text of a Responses API reply: the output_text parts of its message items.
fn openai_text(res: &Value) -> String {
    let items = res["output"].as_array().into_iter().flatten();
    let parts = items
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "output_text")
        .filter_map(|part| part["text"].as_str());
    parts.collect::<Vec<_>>().join("\n")
}

fn anthropic(jpeg: &[u8]) -> Res<String> {
    let key = key("ANTHROPIC_API_KEY", "Claude")?;
    let model = setting("ROOKEY_READER_MODEL").unwrap_or_else(|| ANTHROPIC_MODEL.into());
    let image = base64::engine::general_purpose::STANDARD.encode(jpeg);
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": image } },
                { "type": "text", "text": PROMPT },
            ],
        }],
    });
    let mut headers = vec![("x-api-key", key.as_str()), ("anthropic-version", "2023-06-01")];
    // What follows is for the default model. One set in ROOKEY_READER_MODEL may not take
    // either, so it gets the plain request.
    if model == ANTHROPIC_MODEL {
        // picking words off a screenshot needs little thought
        body["output_config"] = json!({ "effort": "low" });
        // a screen can show anything; a declined request is retried on the model
        // Anthropic recommends for it rather than lost
        body["fallbacks"] = json!("default");
        headers.push(("anthropic-beta", "server-side-fallback-2026-07-01"));
    }
    let res = post("https://api.anthropic.com/v1/messages", &headers, &body)?;
    if res["stop_reason"] == "refusal" {
        return Err(crate::t!("reader.refused", why = res["stop_details"]).into());
    }
    Ok(anthropic_text(&res))
}

fn anthropic_text(res: &Value) -> String {
    let blocks = res["content"].as_array().into_iter().flatten();
    let texts = blocks.filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str());
    texts.collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_text() {
        let openai = json!({ "output": [
            { "type": "reasoning", "summary": [] },
            { "type": "message", "content": [{ "type": "output_text", "text": "ROOKEY_LANG\nniri" }] },
        ]});
        assert_eq!(openai_text(&openai), "ROOKEY_LANG\nniri");
        assert_eq!(openai_text(&json!({ "error": "x" })), "");

        let claude = json!({ "stop_reason": "end_turn", "content": [
            { "type": "thinking", "thinking": "" },
            { "type": "text", "text": "ROOKEY_LANG\nniri" },
        ]});
        assert_eq!(anthropic_text(&claude), "ROOKEY_LANG\nniri");
    }

    #[test]
    fn ocr_languages() {
        let v5 = "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nosd\nukr\n";
        let v4 = "List of available languages (2):\r\neng\r\nscript/Cyrillic\r\n";
        assert_eq!(listed_langs(v5), ["eng", "osd", "ukr"]);
        assert_eq!(listed_langs(v4), ["eng", "script/Cyrillic"]);
        assert!(listed_langs("").is_empty());

        assert_eq!(tesseract_lang("uk"), Some("ukr"));
        assert_eq!(tesseract_lang("de"), Some("deu"));
        assert_eq!(tesseract_lang("zh"), Some("chi_sim"));
        assert_eq!(tesseract_lang("xx"), None);
        // every language the page offers has a pack
        for code in ["en", "uk", "ru", "de", "es", "fr", "pl"] {
            assert!(tesseract_lang(code).is_some(), "{code}");
        }

        let installed = listed_langs(v5);
        let picked: Vec<String> = ["en", "uk", "ru", "xx"].map(String::from).into();
        // English is always read, an unknown code is skipped, ru has no pack here
        assert_eq!(ocr_langs(&picked, &installed), (vec!["ukr"], vec!["ru"]));
        assert_eq!(ocr_langs(&[], &installed), (vec![], vec![]));
    }

    /// Needs a screen, grim and a key:
    /// OPENAI_API_KEY=... ANTHROPIC_API_KEY=... cargo test -- --ignored screen
    #[test]
    #[ignore]
    fn screen_terms_live() {
        let jpeg = screenshot("jpeg").unwrap();
        if setting("OPENAI_API_KEY").is_some() {
            let terms = openai(&jpeg).unwrap();
            println!("openai:\n{terms}");
            assert!(!terms.trim().is_empty());
        }
        if setting("ANTHROPIC_API_KEY").is_some() {
            let terms = anthropic(&jpeg).unwrap();
            println!("claude:\n{terms}");
            assert!(!terms.trim().is_empty());
        }
    }
}
