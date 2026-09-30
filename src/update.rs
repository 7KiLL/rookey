//! Updates: rookey asks GitHub for the latest release, downloads this system's archive,
//! checks it against the release's SHA256SUMS, and puts the new binary in place of this one.
//! A running rookey keeps the file it started from, so the new one is used from the next start.
//!
//!   rookey update           checks now, and installs a newer release
//!   rookey update --check   only says whether there is one
//!
//! `rookey ui` and `rookey listen` also check by themselves, at most once a day, in the
//! background; ROOKEY_AUTOUPDATE=0 keeps that to checking and telling. That check is the only
//! network call rookey makes without being asked.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, thread};

use serde_json::{Value, json};

use crate::Res;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const API: &str = "https://api.github.com/repos/7KiLL/rookey/releases/latest";
/// Published by .github/workflows/release.yml next to the archives.
const SUMS: &str = "SHA256SUMS";
const DAY: u64 = 24 * 60 * 60;

/// This build's archive in a release, named as release.yml names them. None for a build that
/// has no release counterpart, like a source build for another system.
pub const ASSET: Option<&str> = asset(env::consts::OS, env::consts::ARCH, cfg!(feature = "cuda"));

const fn asset(os: &str, arch: &str, cuda: bool) -> Option<&'static str> {
    // const fns can't compare strings with ==
    const fn is(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut i = 0;
        while i < a.len() {
            if a[i] != b[i] {
                return false;
            }
            i += 1;
        }
        true
    }
    if !is(arch, "x86_64") {
        // the release builds only Apple Silicon for macOS; its Metal build runs on every one
        return if is(os, "macos") && is(arch, "aarch64") { Some("rookey-aarch64-macos.tar.gz") } else { None };
    }
    match (is(os, "linux"), is(os, "windows"), cuda) {
        (true, _, false) => Some("rookey-x86_64-linux.tar.gz"),
        (true, _, true) => Some("rookey-x86_64-linux-cuda.tar.gz"),
        (_, true, false) => Some("rookey-x86_64-windows.zip"),
        (_, true, true) => Some("rookey-x86_64-windows-cuda.zip"),
        _ => None,
    }
}

/// What the updater is doing in this process, for the page.
struct Status {
    phase: &'static str, // idle, checking, current, available, downloading, installed, failed
    latest: String,
    error: String,
}

static STATUS: Mutex<Status> = Mutex::new(Status { phase: "idle", latest: String::new(), error: String::new() });

fn set(phase: &'static str, latest: &str, error: &str) {
    *STATUS.lock().unwrap() = Status { phase, latest: latest.into(), error: error.into() };
}

/// The version this rookey goes by when it compares.
// ponytail: ROOKEY_UPDATE_FROM pretends to be an older version, so an update can be tried
// out against the one real release; it changes only what gets compared. A test release
// channel would be the way up.
fn current() -> String {
    env::var("ROOKEY_UPDATE_FROM").ok().filter(|v| version(v).is_some()).unwrap_or_else(|| VERSION.into())
}

/// Where to ask. ROOKEY_UPDATE_API points a try-out at a fake release server on this machine,
/// nowhere else: a release from anywhere else would be trusted as far as its own checksums.
fn api() -> String {
    env::var("ROOKEY_UPDATE_API").ok().filter(|u| u.starts_with("http://127.0.0.1:")).unwrap_or_else(|| API.into())
}

/// "v1.2.3" or "1.2.3" as numbers. Anything else (a pre-release, a typo) is None.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

fn newer(latest: &str, than: &str) -> bool {
    matches!((version(latest), version(than)), (Some(a), Some(b)) if a > b)
}

/// ROOKEY_AUTOUPDATE: on unless it is 0 or false. Read from the file each time: the page
/// changes it while this runs.
pub fn auto() -> bool {
    let key = "ROOKEY_AUTOUPDATE";
    let raw = env::var(key).ok().or_else(|| {
        let text = fs::read_to_string(crate::config_path()?).ok()?;
        crate::parse_config(&text).remove(key)
    });
    !matches!(raw.as_deref(), Some("0" | "false"))
}

/// A release, as far as the updater cares.
#[derive(Debug, PartialEq)]
struct Release {
    version: String,
    archive: String,
    sums: Option<String>,
}

/// Picks this build's archive and the checksums out of GitHub's answer.
fn release(answer: &Value, asset: &str) -> Res<Release> {
    if answer["draft"] == true || answer["prerelease"] == true {
        return Err("The latest release is not a finished one.".into());
    }
    let tag = answer["tag_name"].as_str().unwrap_or_default();
    let version = version(tag).ok_or_else(|| format!("The latest release is tagged {tag:?}, not a version."))?;
    let url = |name: &str| {
        answer["assets"].as_array()?.iter().find(|a| a["name"] == name)?["browser_download_url"].as_str().map(str::to_string)
    };
    Ok(Release {
        version: format!("{}.{}.{}", version.0, version.1, version.2),
        archive: url(asset).ok_or_else(|| format!("Release {tag} has no {asset}."))?,
        sums: url(SUMS),
    })
}

/// The expected sha256 of `name` from a SHA256SUMS file ("<hex>  <name>", or "*<name>").
fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start();
        let ok = (file == name || file.strip_prefix('*') == Some(name)) && hash.len() == 64;
        ok.then(|| hash.to_ascii_lowercase()).filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()))
    })
}

/// Whether what came down is what the release says it published.
fn verify(sums: &str, asset: &str, got: &str) -> Res<()> {
    let want = sum_for(sums, asset).ok_or(format!("{SUMS} has no line for {asset}. Not installing it."))?;
    if got != want {
        return Err(format!("{asset} doesn't match its checksum ({got}, expected {want}). Not installing it.").into());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Who looks after rookey at this path, if not rookey itself: a system location, a package
/// manager, or a build tree. `path` is the resolved one, with symlinks followed.
fn managed(path: &Path) -> Option<&'static str> {
    let text = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let under = |prefix: &str| text.starts_with(prefix);
    let has = |part: &str| text.contains(part);
    if has("/cellar/") || under("/opt/homebrew/") || under("/home/linuxbrew/") {
        return Some("homebrew");
    }
    if under("/nix/") || has("/.nix-profile/") || under("/gnu/") {
        return Some("nix");
    }
    // `cargo install`, and a build of your own
    let build = path.parent().and_then(Path::parent).and_then(Path::file_name).is_some_and(|n| n == "target");
    if has("/.cargo/bin/") || build {
        return Some("cargo");
    }
    let system = ["/usr/", "/bin/", "/sbin/", "/lib", "/opt/", "/snap/", "/var/lib/flatpak/", "/applications/"];
    let windows = ["/program files", "/windowsapps/", "/scoop/", "/chocolatey/", "/winget/"];
    if system.iter().any(|p| under(p)) || windows.iter().any(|p| has(p)) {
        return Some("package");
    }
    None
}

/// Words for `managed`, for the terminal.
fn managed_words(who: &str) -> &'static str {
    match who {
        "homebrew" => "Homebrew",
        "nix" => "Nix",
        "cargo" => "cargo (installed with cargo, or a build of your own)",
        _ => "your package manager or installer",
    }
}

/// Where this rookey is, resolved, and who manages it if that is not rookey.
fn here() -> Res<(PathBuf, Option<&'static str>)> {
    let exe = crate::exe()?;
    let exe = fs::canonicalize(&exe).unwrap_or(exe);
    let who = managed(&exe);
    Ok((exe, who))
}

/// Whether an automatic check is due: never checked, a day since, or a clock that went back.
fn due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| now >= last + DAY || last > now)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `<data_dir>/rookey/update`: when rookey last checked, and the latest version it saw then.
fn last_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("rookey").join("update"))
}

fn read_last(text: &str) -> (Option<u64>, Option<String>) {
    let mut words = text.split_whitespace();
    let when = words.next().and_then(|w| w.parse().ok());
    (when, words.next().filter(|v| version(v).is_some()).map(str::to_string))
}

fn last() -> (Option<u64>, Option<String>) {
    last_path().and_then(|p| fs::read_to_string(p).ok()).map_or((None, None), |t| read_last(&t))
}

fn remember(latest: Option<&str>) {
    let Some(path) = last_path() else { return };
    let _ = fs::create_dir_all(path.parent().unwrap());
    if let Err(e) = fs::write(&path, format!("{} {}\n", now(), latest.unwrap_or(""))) {
        vlog!(2, "update: can't write {}: {e}", path.display());
    }
}

/// What the page shows.
pub fn state() -> Value {
    let status = STATUS.lock().unwrap();
    let (mut phase, mut latest) = (status.phase, status.latest.clone());
    // not checked in this process: what the last check anywhere saw
    if phase == "idle" {
        if let (_, Some(seen)) = last() {
            if newer(&seen, &current()) {
                (phase, latest) = ("available", seen);
            }
        }
    }
    let (path, who) = here().map_or((None, None), |(p, w)| (Some(crate::ui::tilde(&p)), w));
    json!({
        "version": VERSION, "phase": phase, "latest": latest, "error": status.error,
        "supported": ASSET.is_some(), "managed": who, "path": path,
    })
}

/// Asks GitHub for the latest release, and remembers what it said.
fn check() -> Res<Release> {
    let asset = ASSET.ok_or("There is no release build for this system. Build a new one from source.")?;
    let t = Instant::now();
    let mut res = ureq::get(api())
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", format!("rookey/{VERSION}"))
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .call()
        .map_err(|e| format!("Can't reach GitHub to check for updates: {e}. Check the connection and try again."))?;
    let answer: Value = serde_json::from_str(&res.body_mut().read_to_string()?)?;
    vlog!(2, "update: asked {} in {} ms", api(), t.elapsed().as_millis());
    let found = release(&answer, asset)?;
    remember(Some(&found.version));
    Ok(found)
}

/// Checks, and installs a newer release if `install` and this copy is rookey's to replace.
/// Every step shows in `state()`. Ok(None) when there is nothing newer.
fn run_once(install: bool) -> Res<Option<(Release, bool)>> {
    set("checking", "", "");
    let found = check()?;
    if !newer(&found.version, &current()) {
        set("current", &found.version, "");
        return Ok(None);
    }
    if !install || here()?.1.is_some() {
        set("available", &found.version, "");
        return Ok(Some((found, false)));
    }
    set("downloading", &found.version, "");
    install_release(&found)?;
    set("installed", &found.version, "");
    Ok(Some((found, true)))
}

/// run_once, with a failure shown on the page.
fn attempt(install: bool) -> Res<Option<(Release, bool)>> {
    run_once(install).inspect_err(|e| {
        let latest = STATUS.lock().unwrap().latest.clone();
        set("failed", &latest, &e.to_string());
    })
}

/// For the page's buttons: a check, or a check and an install, in the background.
pub fn start(install: bool) -> Res<()> {
    if matches!(STATUS.lock().unwrap().phase, "checking" | "downloading") {
        return Ok(()); // one at a time: the page just asks again how it goes
    }
    set("checking", "", "");
    thread::spawn(move || drop(attempt(install)));
    Ok(())
}

/// The automatic check, off the main thread: once when `rookey ui` starts, and for `rookey
/// listen` once a day for as long as it runs. Failures are only logged: nobody asked.
pub fn in_background(keep_going: bool) {
    thread::spawn(move || {
        loop {
            if due(last().0, now()) {
                remember(last().1.as_deref()); // taken, before the network: two starts check once
                let auto = auto();
                match run_once(auto) {
                    Ok(Some((found, true))) => eprintln!("rookey: installed {}, used from the next start", found.version),
                    Ok(_) => {}
                    Err(e) => {
                        vlog!(2, "update: {e}");
                        set("idle", "", "");
                    }
                }
            }
            if !keep_going {
                return;
            }
            thread::sleep(Duration::from_secs(60 * 60));
        }
    });
}

/// `rookey update [--check]`.
pub fn run(args: &[String]) -> Res<()> {
    let install = match args {
        [] => true,
        [flag] if flag == "--check" => false,
        _ => return Err("usage: rookey update [--check]".into()),
    };
    let (exe, who) = here()?;
    match attempt(install)? {
        None => println!("rookey {} is the latest.", current()),
        Some((found, true)) => {
            println!("installed rookey {} at {}; it is used from the next start.", found.version, crate::ui::tilde(&exe));
            #[cfg(any(target_os = "linux", windows))]
            match crate::listen::restart(&crate::exe()?) {
                Ok(true) => println!("restarted the hotkey listener on it."),
                Ok(false) => {}
                Err(e) => eprintln!("rookey: the hotkey listener picks it up on its next start ({e})"),
            }
        }
        Some((found, false)) => {
            println!("rookey {} is out, this is {}.", found.version, current());
            match who {
                Some(who) => println!("{} came from {}: update it there.", crate::ui::tilde(&exe), managed_words(who)),
                None => println!("run `rookey update` to install it."),
            }
        }
    }
    Ok(())
}

/// Downloads, checks and puts the new files in place, next to this rookey.
fn install_release(found: &Release) -> Res<()> {
    let (exe, who) = here()?;
    if let Some(who) = who {
        return Err(format!("{} came from {}: update it there.", crate::ui::tilde(&exe), managed_words(who)).into());
    }
    let dir = exe.parent().ok_or("rookey has no folder")?;
    // next to the binary, so the files are renamed into place, never copied
    let work = dir.join(format!(".rookey-update-{}", std::process::id()));
    fs::create_dir_all(&work).map_err(|e| {
        format!("Can't write to {} ({e}), so rookey can't update itself there. Install it again with the install script.", crate::ui::tilde(dir))
    })?;
    let done = (|| -> Res<()> {
        let asset = ASSET.ok_or("There is no release build for this system.")?;
        let sums_url = found.sums.as_deref().ok_or(format!(
            "Release {} has no {SUMS}, so its download can't be checked. Not installing it.",
            found.version
        ))?;
        let sums = get(sums_url)?.body_mut().read_to_string()?;
        sum_for(&sums, asset).ok_or(format!("{SUMS} has no line for {asset}. Not installing it."))?; // before the download
        let archive = work.join(asset);
        verify(&sums, asset, &download(&found.archive, &archive)?)?;
        let out = work.join("out");
        fs::create_dir_all(&out)?;
        unpack(&archive, &out)?;
        let new = out.join(if cfg!(windows) { "rookey.exe" } else { "rookey" });
        says_version(&new, &found.version)?;
        put_in_place(&out, dir, exe.file_name().unwrap_or_default().as_ref(), cfg!(windows))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&work);
    done
}

fn get(url: &str) -> Res<ureq::http::Response<ureq::Body>> {
    Ok(ureq::get(url)
        .header("User-Agent", format!("rookey/{VERSION}"))
        .config()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .build()
        .call()?)
}

/// Streams `url` into `to`, and returns the sha256 of what came down.
fn download(url: &str, to: &Path) -> Res<String> {
    let t = Instant::now();
    let mut res = get(url)?;
    let mut body = res.body_mut().as_reader();
    let mut out = fs::File::create(to)?;
    let mut sha = ring::digest::Context::new(&ring::digest::SHA256);
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = body.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        sha.update(&chunk[..n]);
        out.write_all(&chunk[..n])?;
    }
    out.sync_all()?;
    vlog!(2, "update: downloaded {url} in {} ms", t.elapsed().as_millis());
    Ok(hex(sha.finish().as_ref()))
}

/// Unpacks a .tar.gz or .zip with the system's tar: bsdtar reads zips too, and Windows 10 and
/// later ship it.
// ponytail: no archive crates for one call a release; the tar and zip crates if a system
// without tar ever turns up.
fn unpack(archive: &Path, into: &Path) -> Res<()> {
    let tar = if cfg!(windows) {
        // Git's GNU tar may come first on PATH, and it can't read a zip
        PathBuf::from(env::var_os("SystemRoot").unwrap_or("C:\\Windows".into())).join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    };
    let mut cmd = Command::new(tar);
    cmd.arg("-xf").arg(archive).arg("-C").arg(into).stdin(Stdio::null());
    crate::no_window(&mut cmd);
    let out = cmd.output().map_err(|e| format!("Can't run tar to unpack the update: {e}"))?;
    if !out.status.success() {
        return Err(format!("tar couldn't unpack the update: {}", String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    Ok(())
}

/// The new binary has to run and say it is the version it should be, before it replaces this one.
fn says_version(exe: &Path, version: &str) -> Res<()> {
    #[cfg(unix)]
    fs::set_permissions(exe, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    let mut cmd = Command::new(exe);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("The downloaded rookey doesn't start: {e}. Not installing it."))?;
    let until = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > until {
            let _ = child.kill();
            return Err("The downloaded rookey didn't answer --version. Not installing it.".into());
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut said = String::new();
    child.stdout.take().map(|mut o| o.read_to_string(&mut said));
    let want = format!("rookey {version}");
    if !status.success() || said.trim() != want {
        return Err(format!("The downloaded rookey says {:?}, not {want:?}. Not installing it.", said.trim()).into());
    }
    Ok(())
}

/// The name a replaced file is moved to while it is still in use: rookey.exe to rookey.old.exe.
fn old_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.old.{ext}"),
        None => format!("{name}.old"),
    }
}

/// Moves rookey and the libraries that came with it from `from` into `to`, the binary under
/// `exe`'s name. On Unix a rename replaces a running binary and the process keeps the old one.
/// Windows won't replace a file in use but lets it be renamed (`aside`), so the old ones move
/// to *.old.* first and are cleared at the next start. Whatever fails halfway is put back.
fn put_in_place(from: &Path, to: &Path, exe: &Path, aside: bool) -> Res<()> {
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let binary = name == "rookey" || name == "rookey.exe";
        // only what a release carries, and never a folder or a link
        if !entry.file_type()?.is_file() || !(binary || name.to_lowercase().ends_with(".dll")) {
            continue;
        }
        let target = to.join(if binary { exe } else { Path::new(&name) });
        files.push((entry.path(), target));
    }
    if !files.iter().any(|(new, _)| new.file_name().is_some_and(|n| n == "rookey" || n == "rookey.exe")) {
        return Err("The update has no rookey in it. Not installing it.".into());
    }
    let mut moved: Vec<(&PathBuf, &PathBuf, Option<PathBuf>)> = Vec::new();
    let done = (|| -> std::io::Result<()> {
        for (new, target) in &files {
            let mut old = None;
            if aside && target.exists() {
                let name = target.file_name().unwrap_or_default().to_string_lossy();
                let away = target.with_file_name(old_name(&name));
                let _ = fs::remove_file(&away); // one from the last update, if it is free now
                fs::rename(target, &away)?;
                old = Some(away);
            }
            if let Err(e) = fs::rename(new, target) {
                if let Some(old) = old {
                    let _ = fs::rename(old, target);
                }
                return Err(e);
            }
            moved.push((new, target, old));
        }
        Ok(())
    })();
    if let Err(e) = done {
        for (new, target, old) in moved.into_iter().rev() {
            let _ = fs::rename(target, new);
            if let Some(old) = old {
                let _ = fs::rename(old, target);
            }
        }
        return Err(format!("Couldn't put the new files in {}: {e}", crate::ui::tilde(to)).into());
    }
    Ok(())
}

/// Clears what the last update on Windows moved aside, now that nothing runs from it.
#[cfg(windows)]
pub fn clear_old() {
    let Some(dir) = crate::exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return };
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if is_old(&entry.file_name().to_string_lossy()) {
            let _ = fs::remove_file(entry.path()); // still in use by an old listener: next time
        }
    }
}

#[cfg(any(windows, test))]
fn is_old(name: &str) -> bool {
    let name = name.to_lowercase();
    name.ends_with(".old.exe") || name.ends_with(".old.dll")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("rookey-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_compare() {
        assert!(newer("v0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.9")); // numbers, not text
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
        assert!(!newer("v0.2.0-rc1", "0.1.0")); // a pre-release is never picked
        assert!(!newer("0.2", "0.1.0"));
        assert!(!newer("garbage", "0.1.0"));
        assert_eq!(version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(version("1.2.3.4"), None);
    }

    #[test]
    fn asset_names_match_the_release() {
        assert_eq!(asset("linux", "x86_64", false), Some("rookey-x86_64-linux.tar.gz"));
        assert_eq!(asset("linux", "x86_64", true), Some("rookey-x86_64-linux-cuda.tar.gz"));
        assert_eq!(asset("macos", "aarch64", false), Some("rookey-aarch64-macos.tar.gz"));
        assert_eq!(asset("windows", "x86_64", false), Some("rookey-x86_64-windows.zip"));
        assert_eq!(asset("windows", "x86_64", true), Some("rookey-x86_64-windows-cuda.zip"));
        assert_eq!(asset("macos", "x86_64", false), None);
        assert_eq!(asset("linux", "aarch64", false), None);
        assert_eq!(asset("freebsd", "x86_64", false), None);
        // every name here is one release.yml builds
        let workflow = include_str!("../.github/workflows/release.yml");
        for (os, arch, cuda) in [("linux", "x86_64", false), ("linux", "x86_64", true), ("macos", "aarch64", false), ("windows", "x86_64", false), ("windows", "x86_64", true)] {
            let name = asset(os, arch, cuda).unwrap();
            let build = name.trim_start_matches("rookey-").trim_end_matches(".tar.gz").trim_end_matches(".zip");
            assert!(workflow.contains(&format!("name: {build},")), "{build} is not in release.yml");
        }
    }

    #[test]
    fn release_answers() {
        let answer = json!({
            "tag_name": "v0.2.0", "draft": false, "prerelease": false,
            "assets": [
                { "name": "rookey-x86_64-linux.tar.gz", "browser_download_url": "https://x/linux.tar.gz" },
                { "name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS" },
            ],
        });
        let found = release(&answer, "rookey-x86_64-linux.tar.gz").unwrap();
        assert_eq!(found, Release { version: "0.2.0".into(), archive: "https://x/linux.tar.gz".into(), sums: Some("https://x/SHA256SUMS".into()) });
        assert!(release(&answer, "rookey-x86_64-windows.zip").is_err());
        let mut draft = answer.clone();
        draft["prerelease"] = json!(true);
        assert!(release(&draft, "rookey-x86_64-linux.tar.gz").is_err());
        let mut old = answer.clone();
        old["assets"].as_array_mut().unwrap().pop();
        assert_eq!(release(&old, "rookey-x86_64-linux.tar.gz").unwrap().sums, None);
    }

    #[test]
    fn checksums_are_read_and_matched() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  rookey-x86_64-linux.tar.gz\n{b} *rookey-x86_64-windows.zip\nshort  rookey-aarch64-macos.tar.gz\n");
        assert_eq!(sum_for(&sums, "rookey-x86_64-linux.tar.gz"), Some(a.clone()));
        assert_eq!(sum_for(&sums, "rookey-x86_64-windows.zip"), Some("b".repeat(64)));
        assert_eq!(sum_for(&sums, "rookey-aarch64-macos.tar.gz"), None);
        assert_eq!(sum_for(&sums, "rookey-x86_64-linux"), None); // whole names only
        assert_eq!(sum_for(&format!("{}  x", "g".repeat(64)), "x"), None);
        // the hash download() computes is the one sha256sum writes
        let digest = ring::digest::digest(&ring::digest::SHA256, b"abc");
        assert_eq!(hex(digest.as_ref()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_download_is_checked_against_its_sum() {
        use std::io::Read as _;
        use std::net::TcpListener;
        let dir = scratch("refuse");
        // a local server that hands out one archive, whose sum is not the one published
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        thread::spawn(move || {
            for mut stream in server.incoming().flatten() {
                let _ = stream.read(&mut [0; 4096]);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
            }
        });
        let got = download(&format!("http://{addr}/a.tar.gz"), &dir.join("a.tar.gz")).unwrap();
        assert_eq!(got, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        let name = "rookey-x86_64-linux.tar.gz";
        assert!(verify(&format!("{}  {name}\n", "0".repeat(64)), name, &got).is_err());
        assert!(verify(&format!("{got}  rookey-x86_64-windows.zip\n"), name, &got).is_err());
        assert!(verify("", name, &got).is_err());
        verify(&format!("{got}  {name}\n"), name, &got).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn who_manages_a_path() {
        let m = |p: &str| managed(Path::new(p));
        assert_eq!(m("/usr/bin/rookey"), Some("package"));
        assert_eq!(m("/usr/local/bin/rookey"), Some("package"));
        assert_eq!(m("/opt/rookey/rookey"), Some("package"));
        assert_eq!(m("/nix/store/abc-rookey/bin/rookey"), Some("nix"));
        assert_eq!(m("/home/me/.nix-profile/bin/rookey"), Some("nix"));
        assert_eq!(m("/opt/homebrew/bin/rookey"), Some("homebrew"));
        assert_eq!(m("/usr/local/Cellar/rookey/0.1.0/bin/rookey"), Some("homebrew"));
        assert_eq!(m("/home/me/.cargo/bin/rookey"), Some("cargo"));
        assert_eq!(m("/home/me/code/rookey/target/release/rookey"), Some("cargo"));
        assert_eq!(m("C:\\Program Files\\rookey\\rookey.exe"), Some("package"));
        assert_eq!(m("C:\\Users\\me\\scoop\\apps\\rookey\\current\\rookey.exe"), Some("package"));
        // where the install scripts put it
        assert_eq!(m("/home/me/.local/bin/rookey"), None);
        assert_eq!(m("/Users/me/.local/bin/rookey"), None);
        assert_eq!(m("C:\\Users\\me\\AppData\\Local\\rookey\\rookey.exe"), None);
    }

    #[test]
    fn checks_once_a_day() {
        let now = 1_800_000_000;
        assert!(due(None, now));
        assert!(!due(Some(now - 60), now));
        assert!(!due(Some(now - DAY + 1), now));
        assert!(due(Some(now - DAY), now));
        assert!(due(Some(now + 3600), now)); // the clock went back
        assert_eq!(read_last("1800000000 0.2.0\n"), (Some(now), Some("0.2.0".into())));
        assert_eq!(read_last("1800000000 \n"), (Some(now), None));
        assert_eq!(read_last("junk"), (None, None));
    }

    #[test]
    fn swaps_on_unix() {
        let (new, bin) = (scratch("unix-new"), scratch("unix-bin"));
        fs::write(new.join("rookey"), "new").unwrap();
        fs::write(new.join("README"), "not ours").unwrap();
        fs::write(bin.join("rk"), "old").unwrap();
        put_in_place(&new, &bin, Path::new("rk"), false).unwrap();
        // under the name it has here, and nothing else came along
        assert_eq!(fs::read_to_string(bin.join("rk")).unwrap(), "new");
        assert_eq!(fs::read_dir(&bin).unwrap().count(), 1);
        assert!(put_in_place(&new, &bin, Path::new("rk"), false).is_err()); // no rookey left in it
        for dir in [new, bin] {
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn swaps_aside_on_windows() {
        let (new, bin) = (scratch("win-new"), scratch("win-bin"));
        fs::write(new.join("rookey.exe"), "new").unwrap();
        fs::write(new.join("cublas64_13.dll"), "new dll").unwrap();
        fs::write(bin.join("rookey.exe"), "running").unwrap();
        fs::write(bin.join("rookey.old.exe"), "from the last update").unwrap();
        put_in_place(&new, &bin, Path::new("rookey.exe"), true).unwrap();
        assert_eq!(fs::read_to_string(bin.join("rookey.exe")).unwrap(), "new");
        assert_eq!(fs::read_to_string(bin.join("rookey.old.exe")).unwrap(), "running");
        assert_eq!(fs::read_to_string(bin.join("cublas64_13.dll")).unwrap(), "new dll");
        assert!(is_old("rookey.old.exe") && is_old("cudart64_13.OLD.dll"));
        assert!(!is_old("rookey.exe") && !is_old("old.exe") && !is_old("rookey.old"));
        assert_eq!(old_name("rookey.exe"), "rookey.old.exe");
        for dir in [new, bin] {
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_swap_is_put_back() {
        use std::os::unix::fs::PermissionsExt;
        let (new, bin) = (scratch("back-new"), scratch("back-bin"));
        fs::write(new.join("rookey"), "new").unwrap();
        fs::write(bin.join("rookey"), "old").unwrap();
        // a folder that can't be written to: the rename fails
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o555)).unwrap();
        let failed = put_in_place(&new, &bin, Path::new("rookey"), false);
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(failed.is_err());
        assert_eq!(fs::read_to_string(bin.join("rookey")).unwrap(), "old");
        assert!(new.join("rookey").exists());
        for dir in [new, bin] {
            fs::remove_dir_all(dir).unwrap();
        }
    }
}
