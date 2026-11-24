//! Veyra's desktop shell: a framebuffer window with tabs, an address bar,
//! bookmarks, history and a small developer panel. Rendering is entirely
//! software; the window backend only supplies pixels and input events.

use std::time::{Duration, Instant};

use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use veyra_common::{Color, Rect, Size};
use veyra_engine::{Browser, Page};
use veyra_paint::{Canvas, FontSystem};
use veyra_layout::FontSpec;

const TAB_H: f32 = 34.0;
const TOOL_H: f32 = 46.0;
const BOOK_H: f32 = 26.0;
const CHROME_H: f32 = TAB_H + TOOL_H + BOOK_H;
const TAB_W: f32 = 188.0;
const OVERLAY_W: f32 = 430.0;

const BG: Color = Color::rgb(22, 24, 29);
const BG2: Color = Color::rgb(30, 33, 40);
const BG3: Color = Color::rgb(42, 46, 56);
const FG: Color = Color::rgb(232, 236, 244);
const DIM: Color = Color::rgb(148, 158, 176);
const ACCENT: Color = Color::rgb(110, 231, 183);
const ACCENT_DARK: Color = Color::rgb(24, 62, 52);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    DevTools,
    History,
    Bookmarks,
}

struct Shell {
    browser: Browser,
    fonts: FontSystem,
    editing: bool,
    address: String,
    overlay: Overlay,
    status: String,
    mouse_down: bool,
    last_size: (usize, usize),
    last_tick: Instant,
}

pub fn run(home: Option<String>) -> Result<(), String> {
    let mut browser = Browser::new(Size::new(1280.0, 700.0));
    if let Some(home) = home {
        if !home.trim().is_empty() {
            browser.navigate(&normalize_url(&home));
        }
    }
    let mut window = Window::new(
        "Veyra — independent browser",
        1280,
        820,
        WindowOptions {
            resize: true,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| format!("cannot open a window: {e}"))?;
    window.set_target_fps(60);
    let mut shell = Shell {
        browser,
        fonts: FontSystem::new(),
        editing: false,
        address: String::new(),
        overlay: Overlay::None,
        status: String::new(),
        mouse_down: false,
        last_size: (0, 0),
        last_tick: Instant::now(),
    };
    shell.resize(1280, 820);
    while window.is_open() {
        let (w, h) = window.get_size();
        if w < 360 || h < 240 {
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }
        if (w, h) != shell.last_size {
            shell.resize(w, h);
        }
        shell.handle_input(&mut window, w, h);
        if !window.is_open() {
            break;
        }
        if shell.editing {
            window.set_title("Veyra — type an address");
        } else if let Some(tab) = shell.browser.active_tab() {
            window.set_title(&format!("{} — Veyra", tab.title()));
        }
        let buffer = shell.render(w, h);
        if let Err(e) = window.update_with_buffer(&buffer, w, h) {
            return Err(format!("framebuffer update failed: {e}"));
        }
    }
    shell.browser.save_profile();
    Ok(())
}

impl Shell {
    fn resize(&mut self, w: usize, h: usize) {
        self.last_size = (w, h);
        let content_h = (h as f32 - CHROME_H).max(100.0);
        self.browser.set_viewport(Size::new(w as f32, content_h));
    }

    fn active_page_mut(&mut self) -> Option<&mut Page> {
        self.browser.active_page_mut()
    }

    fn address_text(&self) -> String {
        self.browser
            .active_tab()
            .map(|t| t.url())
            .unwrap_or_else(|| "veyra://newtab".into())
    }

    fn navigate(&mut self, input: &str) {
        let url = normalize_url(input);
        if url.is_empty() {
            return;
        }
        self.browser.navigate(&url);
        self.editing = false;
        self.status.clear();
        let (w, h) = self.last_size;
        self.browser.set_viewport(Size::new(w as f32, (h as f32 - CHROME_H).max(100.0)));
    }

    fn handle_input(&mut self, window: &mut Window, w: usize, h: usize) {
        let ctrl = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        let alt = window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt);
        let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);

        for key in window.get_keys_pressed(KeyRepeat::No) {
            match key {
                Key::F5 => self.browser.reload(),
                Key::F12 => self.toggle_overlay(Overlay::DevTools),
                Key::Escape => {
                    if self.editing {
                        self.editing = false;
                        self.address.clear();
                        self.overlay = Overlay::None;
                    } else if self.overlay != Overlay::None {
                        self.overlay = Overlay::None;
                    } else {
                        window.set_title("Veyra");
                    }
                }
                Key::L if ctrl => {
                    self.editing = true;
                    self.address = self.address_text();
                }
                Key::T if ctrl => {
                    self.browser.new_tab("veyra://newtab");
                    self.editing = false;
                }
                Key::W if ctrl => {
                    self.browser.close_tab(self.browser.active);
                    self.editing = false;
                }
                Key::R if ctrl => self.browser.reload(),
                Key::H if ctrl => self.toggle_overlay(Overlay::History),
                Key::B if ctrl => self.toggle_overlay(Overlay::Bookmarks),
                Key::D if ctrl => {
                    self.browser.bookmark_current();
                    self.status = if self.browser.is_bookmarked() {
                        "Bookmark saved".into()
                    } else {
                        "Bookmark removed".into()
                    };
                }
                Key::Left if alt => self.browser.back(),
                Key::Right if alt => self.browser.forward(),
                Key::Left => self.forward_page_key("ArrowLeft", ctrl),
                Key::Right => self.forward_page_key("ArrowRight", ctrl),
                Key::Up => self.forward_page_key("ArrowUp", ctrl),
                Key::Down => self.forward_page_key("ArrowDown", ctrl),
                Key::Backspace => {
                    if self.editing {
                        self.address.pop();
                    } else {
                        self.forward_page_key("Backspace", ctrl);
                    }
                }
                Key::Enter => {
                    if self.editing {
                        let value = self.address.clone();
                        self.navigate(&value);
                    } else {
                        self.forward_page_key("Enter", ctrl);
                    }
                }
                _ => {
                    if self.editing {
                        if let Some(ch) = key_char(key, shift) {
                            self.address.push(ch);
                        }
                    } else if let Some(ch) = key_char(key, shift) {
                        self.forward_page_key(&ch.to_string(), ctrl);
                    }
                }
            }
        }

        // Content area clicking and wheel scrolling.
        let (mx, my) = window.get_mouse_pos(MouseMode::Discard).unwrap_or((-1.0, -1.0));
        let down = window.get_mouse_down(MouseButton::Left);
        let pressed = down && !self.mouse_down;
        self.mouse_down = down;
        if pressed && mx >= 0.0 && my >= 0.0 {
            self.click(mx as f32, my as f32, w, h);
        }
        let wheel = window.get_scroll_wheel().map(|(_, y)| y).unwrap_or(0.0);
        if wheel.abs() > 0.01 && my as f32 >= CHROME_H && self.overlay == Overlay::None {
            if let Some(page) = self.active_page_mut() {
                page.scroll_by(-wheel * 90.0, -wheel * 120.0);
            }
        }

        // Timers and animation frames keep running even without input.
        if self.last_tick.elapsed() >= Duration::from_millis(16) {
            self.last_tick = Instant::now();
            if let Some(page) = self.active_page_mut() {
                page.run_timers_once();
            }
        }
    }

    fn forward_page_key(&mut self, key: &str, ctrl: bool) {
        if self.editing {
            return;
        }
        if let Some(page) = self.active_page_mut() {
            page.handle_key(key, ctrl);
        }
    }

    fn toggle_overlay(&mut self, overlay: Overlay) {
        self.overlay = if self.overlay == overlay { Overlay::None } else { overlay };
    }

    fn click(&mut self, x: f32, y: f32, w: usize, h: usize) {
        if self.overlay != Overlay::None {
            if x > w as f32 - OVERLAY_W {
                if y < TAB_H && x > w as f32 - 32.0 {
                    self.overlay = Overlay::None;
                    return;
                }
                if self.overlay == Overlay::History {
                    let idx = ((y - 60.0) / 38.0).floor() as isize;
                    if idx >= 0 {
                        let len = self.browser.history.len();
                        let start = len.saturating_sub(24);
                        let entries: Vec<String> = self.browser.history[start..]
                            .iter()
                            .rev()
                            .map(|e| e.url.to_string())
                            .collect();
                        if let Some(url) = entries.get(idx as usize).cloned() {
                            self.navigate(&url);
                        }
                    }
                } else if self.overlay == Overlay::Bookmarks {
                    let idx = ((y - 60.0) / 38.0).floor() as isize;
                    if idx >= 0 {
                        if let Some(url) = self.browser.bookmarks.get(idx as usize).map(|b| b.url.to_string()) {
                            self.navigate(&url);
                        }
                    }
                }
                return;
            }
            self.overlay = Overlay::None;
        }
        if y < TAB_H {
            let idx = ((x - 8.0) / (TAB_W + 4.0)).floor() as isize;
            if idx >= 0 && (idx as usize) < self.browser.tabs.len() {
                let left = 8.0 + idx as f32 * (TAB_W + 4.0);
                if x > left + TAB_W - 26.0 {
                    self.browser.close_tab(idx as usize);
                } else {
                    self.browser.activate(idx as usize);
                    self.editing = false;
                }
            } else if x >= 8.0 + self.browser.tabs.len() as f32 * (TAB_W + 4.0) {
                self.browser.new_tab("veyra://newtab");
                self.editing = false;
            }
            return;
        }
        if y < TAB_H + TOOL_H {
            if x < 44.0 {
                self.browser.back();
            } else if x < 78.0 {
                self.browser.forward();
            } else if x < 112.0 {
                self.browser.reload();
            } else if x > w as f32 - 44.0 {
                self.browser.bookmark_current();
                self.status = if self.browser.is_bookmarked() {
                    "Bookmark saved".into()
                } else {
                    "Bookmark removed".into()
                };
            } else if x > w as f32 - 86.0 {
                self.toggle_overlay(Overlay::History);
            } else {
                self.editing = true;
                self.address = self.address_text();
            }
            return;
        }
        if y < CHROME_H {
            let mut bx = 10.0;
            for i in 0..self.browser.bookmarks.len() {
                let title = self.browser.bookmarks[i].title.chars().take(18).collect::<String>();
                let bw = (title.chars().count() as f32 * 7.2 + 26.0).min(200.0);
                if x >= bx && x <= bx + bw {
                    let url = self.browser.bookmarks[i].url.to_string();
                    self.navigate(&url);
                    return;
                }
                bx += bw + 6.0;
            }
            return;
        }
        let cx = x;
        let cy = y - CHROME_H;
        let mut nav = None;
        if let Some(page) = self.active_page_mut() {
            nav = page.handle_click(cx, cy);
            if page.reload_requested() {
                self.browser.reload();
            }
        }
        if let Some(url) = nav {
            self.navigate(&url.to_string());
        }
        let (_, ch) = (w, h);
        let _ = ch;
    }

    fn render(&mut self, w: usize, h: usize) -> Vec<u32> {
        let mut canvas = Canvas::new(w, h);
        canvas.fill(BG);
        canvas.fill_rect(Rect::new(0.0, 0.0, w as f32, TAB_H), BG, Rect::new(0.0, 0.0, w as f32, h as f32));
        canvas.fill_rect(Rect::new(0.0, TAB_H, w as f32, TOOL_H), BG2, Rect::new(0.0, 0.0, w as f32, h as f32));
        canvas.fill_rect(Rect::new(0.0, TAB_H + TOOL_H, w as f32, BOOK_H), BG, Rect::new(0.0, 0.0, w as f32, h as f32));

        self.draw_tabs(&mut canvas, w);
        self.draw_toolbar(&mut canvas, w);
        self.draw_bookmarks(&mut canvas, w, h);

        let mut scroll_state = None;
        if let Some(page) = self.active_page_mut() {
            // `paint` returns a cached bitmap until the page, scroll offset or
            // viewport changes, so idle frames only composite instead of
            // laying out and rasterizing the whole document again.
            scroll_state = Some((page.content_size(), page.scroll()));
            let content = page.paint();
            let dst = Rect::new(0.0, CHROME_H, (w as f32).max(1.0), (h as f32 - CHROME_H).max(1.0));
            canvas.composite(content, dst, 1.0, Rect::new(0.0, CHROME_H, w as f32, h as f32 - CHROME_H));
        }
        if let Some((content_size, scroll)) = scroll_state {
            self.draw_scrollbar(&mut canvas, content_size, scroll, w, h);
        }

        if self.overlay != Overlay::None {
            self.draw_overlay(&mut canvas, w, h);
        }

        if !self.status.is_empty() {
            let status = self.status.clone();
            let bg = Rect::new(12.0, h as f32 - 34.0, (status.chars().count() as f32 * 7.4 + 24.0).min(w as f32 - 24.0), 24.0);
            canvas.fill_round_rect(bg, 8.0, BG3, Rect::new(0.0, 0.0, w as f32, h as f32));
            draw_text(&mut self.fonts, &mut canvas, &status, bg.x + 12.0, bg.y, bg.h, 12.0, false, FG, Rect::new(0.0, 0.0, w as f32, h as f32));
        }

        let mut buffer = Vec::with_capacity(w * h);
        for px in canvas.to_rgba_bytes().chunks_exact(4) {
            buffer.push(((px[0] as u32) << 16) | ((px[1] as u32) << 8) | px[2] as u32);
        }
        buffer
    }

    fn draw_tabs(&mut self, canvas: &mut Canvas, w: usize) {
        let clip = Rect::new(0.0, 0.0, w as f32, TAB_H);
        let count = self.browser.tabs.len();
        for i in 0..count {
            let x = 8.0 + i as f32 * (TAB_W + 4.0);
            if x > w as f32 - 40.0 {
                break;
            }
            let active = i == self.browser.active;
            let r = Rect::new(x, 4.0, TAB_W, TAB_H - 4.0);
            canvas.fill_round_rect(r, 8.0, if active { BG2 } else { BG }, clip);
            if active {
                canvas.fill_rect(Rect::new(x + 8.0, TAB_H - 2.0, TAB_W - 16.0, 2.0), ACCENT, clip);
            }
            let title = self.browser.tabs[i].title();
            let title = truncate(&title, 22);
            draw_text(&mut self.fonts, canvas, &title, x + 12.0, r.y, r.h, 12.0, active, if active { FG } else { DIM }, clip);
            draw_text(&mut self.fonts, canvas, "x", x + TAB_W - 20.0, r.y, r.h, 12.0, false, DIM, clip);
        }
        let plus_x = 8.0 + count as f32 * (TAB_W + 4.0);
        if plus_x < w as f32 - 30.0 {
            draw_text(&mut self.fonts, canvas, "+", plus_x + 8.0, 0.0, TAB_H, 18.0, false, DIM, clip);
        }
    }

    fn draw_toolbar(&mut self, canvas: &mut Canvas, w: usize) {
        let y = TAB_H;
        let clip = Rect::new(0.0, y, w as f32, TOOL_H);
        for (label, x, enabled) in [
            ("<", 16.0, self.browser.active_tab().map(|t| t.can_go_back()).unwrap_or(false)),
            (">", 50.0, self.browser.active_tab().map(|t| t.can_go_forward()).unwrap_or(false)),
            ("R", 84.0, true),
        ] {
            draw_text(
                &mut self.fonts,
                canvas,
                label,
                x,
                y,
                TOOL_H,
                17.0,
                true,
                if enabled { FG } else { BG3 },
                clip,
            );
        }
        let url_x = 116.0;
        let url_w = (w as f32 - url_x - 100.0).max(80.0);
        let bar = Rect::new(url_x, y + 8.0, url_w, 30.0);
        canvas.fill_round_rect(bar, 9.0, BG, clip);
        canvas.stroke_round_rect(bar, 9.0, 1.0, if self.editing { ACCENT } else { BG3 }, clip);
        let shown = if self.editing {
            self.address.clone()
        } else {
            self.address_text()
        };
        let chars = (url_w / 6.5) as usize;
        let shown = tail(&shown, chars.saturating_sub(4));
        draw_text(&mut self.fonts, canvas, &shown, url_x + 12.0, bar.y, bar.h, 12.5, false, if self.editing { FG } else { DIM }, clip);
        if self.editing {
            let caret_x = url_x + 12.0 + shown.chars().count() as f32 * 6.5;
            canvas.fill_rect(Rect::new(caret_x, bar.y + 6.0, 1.5, 18.0), ACCENT, clip);
        }
        let star = if self.browser.is_bookmarked() { "*" } else { "+" };
        draw_text(&mut self.fonts, canvas, star, w as f32 - 38.0, y, TOOL_H, 18.0, false, if self.browser.is_bookmarked() { ACCENT } else { DIM }, clip);
        draw_text(&mut self.fonts, canvas, "H", w as f32 - 76.0, y, TOOL_H, 13.0, true, DIM, clip);
        if !self.status.is_empty() {
            let status = self.status.clone();
            draw_text(&mut self.fonts, canvas, &status, w as f32 - 330.0, y, TOOL_H, 11.0, false, DIM, clip);
        }
    }

    fn draw_bookmarks(&mut self, canvas: &mut Canvas, w: usize, h: usize) {
        let y = TAB_H + TOOL_H;
        let clip = Rect::new(0.0, y, w as f32, BOOK_H);
        canvas.fill_rect(Rect::new(0.0, y, w as f32, BOOK_H), BG2, clip);
        let mut x = 10.0;
        let bookmarks: Vec<(String, String)> = self
            .browser
            .bookmarks
            .iter()
            .map(|b| (b.title.clone(), b.url.to_string()))
            .collect();
        if bookmarks.is_empty() {
            draw_text(&mut self.fonts, canvas, "No bookmarks yet — press Ctrl+D to save this page", 12.0, y, BOOK_H, 11.0, false, DIM, clip);
            return;
        }
        for (title, _url) in &bookmarks {
            let label = truncate(title, 20);
            let bw = (label.chars().count() as f32 * 7.1 + 22.0).min(200.0);
            if x + bw > w as f32 - 8.0 {
                break;
            }
            canvas.fill_round_rect(Rect::new(x, y + 4.0, bw, BOOK_H - 8.0), 6.0, BG3, clip);
            draw_text(&mut self.fonts, canvas, &label, x + 10.0, y + 4.0, BOOK_H - 8.0, 11.0, false, FG, clip);
            x += bw + 6.0;
        }
        let _ = h;
    }

    fn draw_scrollbar(
        &mut self,
        canvas: &mut Canvas,
        content_size: Size,
        scroll: veyra_common::Point,
        w: usize,
        h: usize,
    ) {
        let content_h = (h as f32 - CHROME_H).max(1.0);
        let total = content_size.h.max(content_h);
        if total <= content_h + 1.0 {
            return;
        }
        let track = Rect::new(w as f32 - 7.0, CHROME_H + 2.0, 6.0, content_h - 4.0);
        canvas.fill_round_rect(track, 3.0, Color::rgba(255, 255, 255, 14), Rect::new(0.0, CHROME_H, w as f32, content_h));
        let frac = content_h / total;
        let thumb_h = (track.h * frac).max(28.0);
        let max_scroll = (total - content_h).max(1.0);
        let pos = (scroll.y / max_scroll).clamp(0.0, 1.0);
        let thumb = Rect::new(track.x, track.y + (track.h - thumb_h) * pos, track.w, thumb_h);
        canvas.fill_round_rect(thumb, 3.0, Color::rgba(255, 255, 255, 70), Rect::new(0.0, CHROME_H, w as f32, content_h));
    }

    fn draw_overlay(&mut self, canvas: &mut Canvas, w: usize, h: usize) {
        let x = w as f32 - OVERLAY_W;
        let clip = Rect::new(x, 0.0, OVERLAY_W, h as f32);
        canvas.fill_rect(Rect::new(x, 0.0, OVERLAY_W, h as f32), Color::rgb(18, 20, 25), clip);
        canvas.fill_rect(Rect::new(x, 0.0, 1.0, h as f32), BG3, clip);
        let title = match self.overlay {
            Overlay::DevTools => "Developer tools",
            Overlay::History => "History",
            Overlay::Bookmarks => "Bookmarks",
            Overlay::None => "",
        };
        draw_text(&mut self.fonts, canvas, title, x + 20.0, 0.0, 44.0, 14.0, true, FG, clip);
        draw_text(&mut self.fonts, canvas, "x", w as f32 - 28.0, 0.0, 44.0, 14.0, false, DIM, clip);
        match self.overlay {
            Overlay::DevTools => self.draw_devtools(canvas, x, h, clip),
            Overlay::History => {
                let mut y = 60.0;
                let entries: Vec<(String, String)> = self
                    .browser
                    .history
                    .iter()
                    .rev()
                    .take(24)
                    .map(|e| (e.title.clone(), e.url.to_string()))
                    .collect();
                if entries.is_empty() {
                    draw_text(&mut self.fonts, canvas, "No history yet.", x + 20.0, y, 18.0, 12.0, false, DIM, clip);
                }
                for (title, url) in entries {
                    draw_text(&mut self.fonts, canvas, &truncate(&title, 46), x + 20.0, y, 18.0, 12.0, false, FG, clip);
                    draw_text(&mut self.fonts, canvas, &truncate(&url, 58), x + 20.0, y + 18.0, 16.0, 10.5, false, DIM, clip);
                    y += 38.0;
                    if y > h as f32 - 30.0 {
                        break;
                    }
                }
            }
            Overlay::Bookmarks => {
                let mut y = 60.0;
                let entries: Vec<(String, String)> = self
                    .browser
                    .bookmarks
                    .iter()
                    .map(|b| (b.title.clone(), b.url.to_string()))
                    .collect();
                if entries.is_empty() {
                    draw_text(&mut self.fonts, canvas, "No bookmarks. Press Ctrl+D on a page.", x + 20.0, y, 18.0, 12.0, false, DIM, clip);
                }
                for (title, url) in entries {
                    draw_text(&mut self.fonts, canvas, &truncate(&title, 46), x + 20.0, y, 18.0, 12.0, false, FG, clip);
                    draw_text(&mut self.fonts, canvas, &truncate(&url, 58), x + 20.0, y + 18.0, 16.0, 10.5, false, DIM, clip);
                    y += 38.0;
                    if y > h as f32 - 30.0 {
                        break;
                    }
                }
            }
            Overlay::None => {}
        }
    }

    fn draw_devtools(&mut self, canvas: &mut Canvas, x: f32, h: usize, clip: Rect) {
        let mut y = 56.0;
        let report = self
            .browser
            .active_tab()
            .map(|t| t.page.info())
            .unwrap_or_else(|| "No page loaded".into());
        for line in report.lines().take(4) {
            draw_text(&mut self.fonts, canvas, line, x + 20.0, y, 17.0, 11.0, false, DIM, clip);
            y += 17.0;
        }
        y += 10.0;
        draw_text(&mut self.fonts, canvas, "Console", x + 20.0, y, 20.0, 13.0, true, FG, clip);
        y += 24.0;
        let (logs, errors, failed) = {
            let tab = self.browser.active_tab();
            match tab {
                Some(t) => (
                    t.page.console.borrow().iter().rev().take(40).cloned().collect::<Vec<_>>(),
                    t.page.errors.iter().rev().take(40).cloned().collect::<Vec<_>>(),
                    t.page.url.to_string(),
                ),
                None => (vec![], vec![], String::new()),
            }
        };
        let _ = failed;
        for log in errors.into_iter().rev() {
            draw_text(&mut self.fonts, canvas, &truncate(&log, 70), x + 20.0, y, 15.0, 10.5, false, Color::rgb(248, 113, 113), clip);
            y += 15.0;
            if y > h as f32 - 24.0 {
                return;
            }
        }
        for log in logs {
            draw_text(&mut self.fonts, canvas, &truncate(&log, 70), x + 20.0, y, 15.0, 10.5, false, FG, clip);
            y += 15.0;
            if y > h as f32 - 24.0 {
                return;
            }
        }
        draw_text(&mut self.fonts, canvas, "Ctrl+H history · Ctrl+B bookmarks · F12 close", x + 20.0, h as f32 - 26.0, 20.0, 10.0, false, DIM, clip);
    }
}

fn draw_text(
    fonts: &mut FontSystem,
    canvas: &mut Canvas,
    text: &str,
    x: f32,
    top: f32,
    height: f32,
    size: f32,
    bold: bool,
    color: Color,
    clip: Rect,
) {
    let spec = FontSpec {
        family: "DejaVu Sans".into(),
        size,
        weight: if bold { 700 } else { 400 },
        italic: false,
        letter_spacing: 0.0,
    };
    // Vertically center the font's line box inside the control, rather than
    // treating the caller's offset as a baseline (which left glyphs clipped by
    // the top edge of toolbars and rows).
    let (ascent, descent) = fonts.ascent_descent(&spec);
    let baseline = top + ((height - ascent - descent).max(0.0) / 2.0) + ascent;
    let mut pen = x;
    for ch in text.chars() {
        if ch == '\n' || ch == '\r' {
            continue;
        }
        let g = fonts.glyph(&spec, ch);
        let gx = (pen + g.xmin as f32).round() as i32;
        let gy = (baseline - g.ymin as f32 - g.height as f32).round() as i32;
        canvas.blit_glyph(&g.bitmap, g.width, g.height, gx, gy, color, clip, 1.0);
        pen += g.advance;
    }
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn tail(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    s.chars().skip(count - max).collect()
}

fn key_char(key: Key, shift: bool) -> Option<char> {
    let c = match key {
        Key::A => 'a', Key::B => 'b', Key::C => 'c', Key::D => 'd', Key::E => 'e',
        Key::F => 'f', Key::G => 'g', Key::H => 'h', Key::I => 'i', Key::J => 'j',
        Key::K => 'k', Key::L => 'l', Key::M => 'm', Key::N => 'n', Key::O => 'o',
        Key::P => 'p', Key::Q => 'q', Key::R => 'r', Key::S => 's', Key::T => 't',
        Key::U => 'u', Key::V => 'v', Key::W => 'w', Key::X => 'x', Key::Y => 'y',
        Key::Z => 'z',
        Key::Key0 => '0', Key::Key1 => '1', Key::Key2 => '2', Key::Key3 => '3',
        Key::Key4 => '4', Key::Key5 => '5', Key::Key6 => '6', Key::Key7 => '7',
        Key::Key8 => '8', Key::Key9 => '9',
        Key::Space => ' ',
        Key::Period => '.',
        Key::Comma => ',',
        Key::Slash => '/',
        Key::Minus => '-',
        Key::Equal => '=',
        Key::Semicolon => ';',
        Key::Apostrophe => '\'',
        Key::Backslash => '\\',
        Key::LeftBracket => '[',
        Key::RightBracket => ']',
        Key::Backquote => '`',
        _ => return None,
    };
    if shift {
        Some(match c {
            '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%',
            '6' => '^', '7' => '&', '8' => '*', '9' => '(', '0' => ')',
            '/' => '?', '.' => '>', ',' => '<', '-' => '_', '=' => '+',
            ';' => ':', '\'' => '"', '[' => '{', ']' => '}', '\\' => '|',
            c => c.to_ascii_uppercase(),
        })
    } else {
        Some(c)
    }
}

fn normalize_url(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() {
        return "veyra://newtab".into();
    }
    if input.starts_with("http://")
        || input.starts_with("https://")
        || input.starts_with("veyra://")
        || input.starts_with("about:")
    {
        return input.to_string();
    }
    if input.contains('.') && !input.contains(' ') {
        return format!("https://{input}");
    }
    format!("https://html.duckduckgo.com/html/?q={}", percent_encode(input))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[allow(dead_code)]
fn unused(_: &Shell) {
    let _ = ACCENT_DARK;
}
