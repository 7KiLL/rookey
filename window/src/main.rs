//! `rookey-window <url>`: opens the page `rookey ui` serves in a window, and exits when the
//! window is closed. rookey starts it from beside its own binary and falls back to the browser
//! when it's missing or can't start (no WebKitGTK, no WebView2).
#![windows_subsystem = "windows"] // no console window next to it

use std::process::{Command, Stdio};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

fn main() {
    let url = std::env::args().nth(1).unwrap_or_default();
    let Some(origin) = origin(&url) else {
        eprintln!("usage: rookey-window http://127.0.0.1:<port>/...  (the page `rookey ui` serves)");
        std::process::exit(2);
    };
    #[cfg(target_os = "macos")]
    bundle::enter(&url);
    if let Err(e) = run(&url, origin) {
        eprintln!("rookey-window: {e}");
        std::process::exit(1);
    }
}

/// The origin of a page `rookey ui` serves, `http://127.0.0.1:<port>/`, and nothing else:
/// this window only ever shows rookey's own page.
fn origin(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://127.0.0.1:")?;
    let (port, _) = rest.split_once('/')?;
    if !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port.parse().ok().filter(|p| *p != 0)?;
    Some(format!("http://127.0.0.1:{port}/"))
}

/// What the page asks of the window on macOS, where it draws under the title bar.
enum Chrome {
    Drag,
    Zoom,
}

/// On macOS the page fills the window and the buttons float over it: a strip of paper keeps
/// the page from scrolling under them, and pressing it moves the window like a title bar
/// would. Only rookey's own page loads here (see the navigation handler).
const MAC_CHROME: &str = r#"(() => {
  document.documentElement.dataset.window = "mac";
  addEventListener("mousedown", (e) => {
    if (e.button !== 0 || e.clientY >= 28 || e.target.closest?.("a, button, input, select, textarea, label, summary")) return;
    window.ipc.postMessage(e.detail === 2 ? "zoom" : "drag");
  });
})();"#;

fn run(url: &str, origin: String) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoopBuilder::<Chrome>::with_user_event().build();
    // ponytail: no window icon on Windows and Linux, the page's is an SVG and tao takes
    // pixels (macOS gets its icon from the bundle); rasterize it at build time if a taskbar
    // ever shows the blank one.
    let builder = WindowBuilder::new()
        .with_title("rookey settings")
        .with_inner_size(LogicalSize::new(1100.0, 860.0))
        .with_min_inner_size(LogicalSize::new(380.0, 480.0));
    #[cfg(target_os = "macos")]
    let builder = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
    };
    let window = builder.build(&event_loop)?;

    // WebView2 would keep its data next to the binary, which may be read-only
    let data = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("rookey").join("webview"))
    } else {
        None
    };
    let mut context = WebContext::new(data);
    let stay = origin.clone();
    #[cfg(target_os = "macos")]
    let origin_again = origin.clone();
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        // links out of the page (an API key's account page) open in the browser instead
        .with_navigation_handler(move |to| to.starts_with(&stay) || !browse(&to))
        .with_new_window_req_handler(move |to, _| {
            if !to.starts_with(&origin) {
                browse(&to);
            }
            NewWindowResponse::Deny
        });
    #[cfg(target_os = "macos")]
    let builder = {
        let proxy = event_loop.create_proxy();
        let page = origin_again;
        builder.with_initialization_script(MAC_CHROME).with_ipc_handler(move |asked| {
            if !asked.uri().to_string().starts_with(&page) {
                return;
            }
            let _ = proxy.send_event(match asked.body().as_str() {
                "drag" => Chrome::Drag,
                "zoom" => Chrome::Zoom,
                _ => return,
            });
        })
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().ok_or("no GTK box in the window")?)?
    };
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let _webview = builder.build(&window)?;

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            Event::UserEvent(Chrome::Drag) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(Chrome::Zoom) => window.set_maximized(!window.is_maximized()),
            _ => {}
        }
    })
}

/// macOS names an app in the Dock and the menu bar after the bundle it runs from, and a bare
/// binary after its file: "rookey-window" with a blank icon. So the window puts a bundle of
/// its own together, a copy of itself with a name and an icon, and runs from there. Kept in
/// the caches: the release, the install script and `rookey update` only ever see one file.
#[cfg(target_os = "macos")]
mod bundle {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::{env, fs, io};

    // ponytail: the bundle shows up in Spotlight, and opened from there it has no page to show
    // and exits; the way up is to run `rookey ui` from beside the binary it was copied from.
    const ICON: &[u8] = include_bytes!("../macos/rookey.icns");

    /// Runs this window from its bundle, under the same pid, so `rookey ui` still sees its
    /// child. Returns only when it can't, and then the window opens as it is.
    pub fn enter(url: &str) {
        let Ok(me) = env::current_exe() else { return };
        if me.to_string_lossy().contains(".app/Contents/MacOS/") {
            return;
        }
        let Some(app) = env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/rookey/Rookey Settings.app")) else {
            return;
        };
        match place(&me, &app) {
            Ok(exe) => eprintln!("rookey-window: can't run from {} ({})", app.display(), std::process::Command::new(exe).arg(url).exec()),
            Err(e) => eprintln!("rookey-window: can't put {} together ({e})", app.display()),
        }
    }

    /// The bundle with this binary in it, written again whenever the binary changed.
    fn place(me: &Path, app: &Path) -> io::Result<PathBuf> {
        let contents = app.join("Contents");
        let exe = contents.join("MacOS/rookey-window");
        let bytes = fs::read(me)?;
        if fs::read(&exe).is_ok_and(|there| there == bytes) {
            return Ok(exe);
        }
        fs::create_dir_all(contents.join("MacOS"))?;
        fs::create_dir_all(contents.join("Resources"))?;
        fs::write(contents.join("Info.plist"), info())?;
        fs::write(contents.join("Resources/rookey.icns"), ICON)?;
        // a new file then a rename: another window may be running the old copy
        let new = exe.with_extension("new");
        fs::write(&new, &bytes)?;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
        fs::rename(&new, &exe)?;
        Ok(exe)
    }

    /// Changing the identifier loses what macOS keeps under it: the webview's storage now,
    /// and any permission this bundle is ever given.
    pub fn info() -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>io.github.7kill.rookey</string>
  <key>CFBundleName</key><string>Rookey Settings</string>
  <key>CFBundleDisplayName</key><string>Rookey Settings</string>
  <key>CFBundleExecutable</key><string>rookey-window</string>
  <key>CFBundleIconFile</key><string>rookey</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#,
            env!("CARGO_PKG_VERSION")
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn puts_itself_together_once() {
            let dir = env::temp_dir().join(format!("rookey-bundle-{}", std::process::id()));
            let (me, app) = (dir.join("rookey-window"), dir.join("Rookey Settings.app"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(&me, "one").unwrap();
            let exe = place(&me, &app).unwrap();
            assert_eq!(fs::read_to_string(&exe).unwrap(), "one");
            assert_eq!(fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
            assert!(app.join("Contents/Resources/rookey.icns").is_file());
            // CFBundleName shows only 15 characters
            assert!(info().contains("<string>Rookey Settings</string>") && "Rookey Settings".len() <= 15);
            fs::write(&me, "two").unwrap();
            assert_eq!(fs::read_to_string(place(&me, &app).unwrap()).unwrap(), "two");
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

/// Opens an https link in the default browser. False when it isn't one, or nothing opened it.
fn browse(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    // rundll32 rather than `cmd /c start`, which would split the link at its &
    let (opener, args): (&str, &[&str]) = if cfg!(windows) {
        ("rundll32", &["url.dll,FileProtocolHandler", url])
    } else if cfg!(target_os = "macos") {
        ("open", &[url])
    } else {
        ("xdg-open", &[url])
    };
    Command::new(opener).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::origin;

    #[test]
    fn only_the_local_page() {
        assert_eq!(origin("http://127.0.0.1:4242/?t=ab12").as_deref(), Some("http://127.0.0.1:4242/"));
        assert_eq!(origin("http://127.0.0.1:65535/").as_deref(), Some("http://127.0.0.1:65535/"));
        for bad in [
            "",
            "http://127.0.0.1:4242", // no path
            "http://127.0.0.1/",     // no port
            "http://127.0.0.1:0/",
            "http://127.0.0.1:65536/",
            "http://127.0.0.1:+80/",
            "http://127.0.0.1:80@evil.example/",
            "http://127.0.0.1.evil.example:80/",
            "https://127.0.0.1:4242/",
            "http://localhost:4242/",
            "http://[::1]:4242/",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert_eq!(origin(bad), None, "{bad}");
        }
    }
}
