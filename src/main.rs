//! Veyra command line entry point.

use std::process::ExitCode;

use veyra_common::Size;
use veyra_engine::{LoadOptions, Page};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return match veyra_shell::run(None) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("veyra: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match args[0].as_str() {
        "ui" => {
            let home = args.get(1).cloned();
            match veyra_shell::run(home) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("veyra: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "shot" | "screenshot" => shot(&args[1..]),
        "dump" => dump(&args[1..]),
        "eval" => eval(&args[1..]),
        "boxes" => boxes(&args[1..]),
        "fetch" => fetch(&args[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        "version" | "--version" | "-V" => {
            println!("veyra {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        url if url.starts_with("http://") || url.starts_with("https://") || url.contains('.') => {
            match veyra_shell::run(Some(url.to_string())) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("veyra: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("veyra: unknown command '{other}'");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "Veyra — an independent desktop web browser with its own engine\n\n\
         Usage:\n\
         veyra [url]                     Open the browser window (optionally at a URL)\n\
         veyra ui [url]                  Same as above\n\
         veyra shot <url> <out.png>      Render a page headlessly to a PNG\n\
             --width N --height N --wait MS --scroll Y\n\
                  veyra dump <url>                Print document/render/JS diagnostics\n\
         --width N --height N --wait MS\n\
         veyra eval <url> <js>           Evaluate JavaScript after the page loads\n\
         veyra boxes <url>               Print the layout box tree\n\
         veyra fetch <url>               Fetch a URL and print its body\n\
         veyra version | help\n"
    );
}

#[derive(Default)]
struct Flags {
    width: Option<f32>,
    height: Option<f32>,
    wait: u64,
    scroll: f32,
    positional: Vec<String>,
}

fn parse_flags(args: &[String]) -> Flags {
    let mut flags = Flags::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--width" => {
                flags.width = args.get(i + 1).and_then(|v| v.parse().ok());
                i += 2;
            }
            "--height" => {
                flags.height = args.get(i + 1).and_then(|v| v.parse().ok());
                i += 2;
            }
            "--wait" => {
                flags.wait = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 2;
            }
            "--scroll" => {
                flags.scroll = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            other => {
                flags.positional.push(other.to_string());
                i += 1;
            }
        }
    }
    flags
}

fn load_page(url: &str, flags: &Flags) -> Page {
    let trace = std::env::var_os("VEYRA_DEBUG_LOAD").is_some();
    let t_load = std::time::Instant::now();
    let mut page = Page::load(
        url,
        LoadOptions {
            viewport: Size::new(flags.width.unwrap_or(1280.0), flags.height.unwrap_or(900.0)),
            scripts: true,
            max_script_bytes: 64_000_000,
        },
    );
    if trace { eprintln!("CLI Page::load ms={}", t_load.elapsed().as_millis()); }
    let t_pump = std::time::Instant::now();
    page.pump();
    if trace { eprintln!("CLI pump ms={}", t_pump.elapsed().as_millis()); }
    if flags.wait > 0 {
        std::thread::sleep(std::time::Duration::from_millis(flags.wait));
        let t_wait_pump = std::time::Instant::now();
        page.pump();
        if trace { eprintln!("CLI wait pump ms={}", t_wait_pump.elapsed().as_millis()); }
        let t_timers = std::time::Instant::now();
        page.run_timers_once();
        if trace { eprintln!("CLI timers ms={}", t_timers.elapsed().as_millis()); }
    }
    if flags.scroll > 0.0 {
        page.set_scroll(veyra_common::Point::new(0.0, flags.scroll));
    }
    page
}

fn shot(args: &[String]) -> ExitCode {
    let flags = parse_flags(args);
    if flags.positional.len() < 2 {
        eprintln!("usage: veyra shot <url> <out.png> [--width N] [--height N] [--wait MS] [--scroll Y]");
        return ExitCode::FAILURE;
    }
    let url = &flags.positional[0];
    let out = &flags.positional[1];
    let mut page = load_page(url, &flags);
    let png = page.screenshot_png();
    if let Err(e) = std::fs::write(out, &png) {
        eprintln!("veyra: cannot write {out}: {e}");
        return ExitCode::FAILURE;
    }
    println!("{}", page.info());
    for e in &page.errors {
        eprintln!("error: {e}");
    }
    println!("saved {out} ({} bytes)", png.len());
    ExitCode::SUCCESS
}

fn dump(args: &[String]) -> ExitCode {
    let flags = parse_flags(args);
    let Some(url) = flags.positional.first() else {
        eprintln!("usage: veyra dump <url> [--width N] [--height N] [--wait MS]");
        return ExitCode::FAILURE;
    };
    let page = load_page(url, &flags);
    println!("{}", page.info());
    println!("--- console ---");
    for line in page.console.borrow().iter() {
        println!("{line}");
    }
    println!("--- errors ---");
    for e in &page.errors {
        println!("{e}");
    }
    println!("--- text ---");
    let text = {
        let s = page.state.borrow();
        s.doc.text_content(s.doc.root)
    };
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    println!("{}", truncate_chars(&compact, 4000));
    ExitCode::SUCCESS
}

fn eval(args: &[String]) -> ExitCode {
    let flags = parse_flags(args);
    if flags.positional.len() < 2 {
        eprintln!("usage: veyra eval <url> <javascript> [--width N] [--height N] [--wait MS]");
        return ExitCode::FAILURE;
    }
    let mut page = load_page(&flags.positional[0], &flags);
    match page.run_js(&flags.positional[1]) {
        Ok(v) => {
            println!("{v}");
            for e in &page.errors {
                eprintln!("error: {e}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("JavaScript error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn boxes(args: &[String]) -> ExitCode {
    let flags = parse_flags(args);
    let Some(url) = flags.positional.first() else {
        eprintln!("usage: veyra boxes <url>");
        return ExitCode::FAILURE;
    };
    let mut page = load_page(url, &flags);
    page.rebuild();
    println!("{}", page.info());
    if let Some(layout) = &page.layout {
        if let Some(root) = &layout.root {
            print_box(root, 0);
        }
        for t in &layout.texts {
            println!("TEXT {:?} rect={:?} baseline={}", t.text, t.rect, t.baseline);
        }
    }
    ExitCode::SUCCESS
}

fn print_box(b: &veyra_layout::LayoutBox, depth: usize) {
    println!(
        "{}{} node={:?} rect=({:.1},{:.1},{:.1},{:.1}) display={:?}",
        "  ".repeat(depth),
        b.tag,
        b.node,
        b.rect.x,
        b.rect.y,
        b.rect.w,
        b.rect.h,
        b.style.display
    );
    if b.style.background_color.a > 0 {
        println!(
            "{}  bg={:?} radius={}",
            "  ".repeat(depth),
            b.style.background_color,
            b.style.border_radius
        );
    }
    for c in &b.children {
        print_box(c, depth + 1);
    }
}

fn fetch(args: &[String]) -> ExitCode {
    let Some(url) = args.first() else {
        eprintln!("usage: veyra fetch <url>");
        return ExitCode::FAILURE;
    };
    let client = match veyra_net::Client::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("veyra: {e}");
            return ExitCode::FAILURE;
        }
    };
    match client.get_text(url) {
        Ok(resp) => {
            println!("HTTP {} {} ({})", resp.status, resp.reason, resp.url);
            for (k, v) in &resp.headers {
                println!("{k}: {v}");
            }
            println!();
            print!("{}", String::from_utf8_lossy(&resp.body));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("veyra: {e}");
            ExitCode::FAILURE
        }
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
