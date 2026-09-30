//! `rookey-window <url>`: opens the page `rookey ui` serves in a window, and exits when the
//! window is closed. rookey starts it from beside its own binary and falls back to the browser
//! when it's missing or can't start (no WebKitGTK, no WebView2).
#![windows_subsystem = "windows"] // no console window next to it

use std::process::{Command, Stdio};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

fn main() {
    let url = std::env::args().nth(1).unwrap_or_default();
    let Some(origin) = origin(&url) else {
        eprintln!("usage: rookey-window http://127.0.0.1:<port>/...  (the page `rookey ui` serves)");
        std::process::exit(2);
    };
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

fn run(url: &str, origin: String) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new();
    // ponytail: no window icon, the page's is an SVG and tao takes pixels; rasterize it at
    // build time if a taskbar ever shows the blank one.
    let window = WindowBuilder::new()
        .with_title("rookey settings")
        .with_inner_size(LogicalSize::new(1100.0, 860.0))
        .with_min_inner_size(LogicalSize::new(380.0, 480.0))
        .build(&event_loop)?;

    // WebView2 would keep its data next to the binary, which may be read-only
    let data = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("rookey").join("webview"))
    } else {
        None
    };
    let mut context = WebContext::new(data);
    let stay = origin.clone();
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
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *flow = ControlFlow::Exit;
        }
    })
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
