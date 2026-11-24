//! A loaded page: networking, HTML/CSS, layout, painting and JavaScript.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use veyra_common::util::escape_html;
use veyra_common::{Point, Size, Url};
use veyra_html::parse_document;
use veyra_js::value::define;
use veyra_js::{FetchResponse, Host, Interp, Value};
use veyra_js::value::{Func, ObjKind};
use veyra_layout::font::ImageInfo;
use veyra_layout::{layout_document, LayoutTree};
use veyra_net::{Client, Request, ResourceType, Response};
use veyra_paint::{paint_page, Canvas, FontSystem, ImageCache, PaintConfig};

use crate::bindings;
use crate::event_api::dispatch_event;
use crate::state::{DomState, Mutation};

#[derive(Clone, Copy)]
pub struct LoadOptions {
    pub viewport: Size,
    pub scripts: bool,
    pub max_script_bytes: usize,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions {
            viewport: Size::new(1280.0, 800.0),
            scripts: true,
            max_script_bytes: 64_000_000,
        }
    }
}

struct PageHost {
    client: Rc<Client>,
    logs: Rc<RefCell<Vec<String>>>,
    start: Instant,
    seed: u64,
}

impl Host for PageHost {
    fn log(&mut self, level: &str, message: &str) {
        let mut logs = self.logs.borrow_mut();
        if logs.len() < 2000 {
            if level == "log" || level.is_empty() {
                logs.push(message.to_string());
            } else {
                logs.push(format!("{level}: {message}"));
            }
        }
    }

    fn fetch(
        &mut self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Result<FetchResponse, String> {
        let mut req = if method.eq_ignore_ascii_case("POST") {
            Request::post(url, body.unwrap_or("").as_bytes().to_vec())
        } else {
            Request::get(url)
        };
        req = req.kind(ResourceType::Fetch);
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("host")
                || k.eq_ignore_ascii_case("content-length")
                || k.eq_ignore_ascii_case("connection")
            {
                continue;
            }
            req = req.header(k, v);
        }
        let resp = self.client.fetch(req).map_err(|e| e.to_string())?;
        Ok(FetchResponse {
            status: resp.status,
            url: resp.url.to_string(),
            headers: resp.headers.clone(),
            body: String::from_utf8_lossy(&resp.body).to_string(),
        })
    }

    fn now_ms(&mut self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn random(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.seed >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

struct EngineImages<'a>(&'a ImageCache);

impl ImageInfo for EngineImages<'_> {
    fn image_size(&self, url: &Url) -> Option<Size> {
        self.0.get(url).map(|i| Size::new(i.width as f32, i.height as f32))
    }
}

pub struct Page {
    pub url: Url,
    pub status: u16,
    pub viewport: Size,
    pub state: Rc<RefCell<DomState>>,
    pub js: Interp,
    pub fonts: FontSystem,
    pub layout: Option<LayoutTree>,
    pub client: Rc<Client>,
    pub errors: Vec<String>,
    pub console: Rc<RefCell<Vec<String>>>,
    pub scripts_run: usize,
    pub modules_skipped: usize,
    pub load_ms: u64,
    pub started: Instant,
    css_seen: HashSet<String>,
    script_texts: Vec<String>,
    scripts_enabled: bool,
    max_script_bytes: usize,
    /// Already-evaluated ES modules, keyed by absolute URL.
    module_cache: HashMap<String, HashMap<String, veyra_js::Value>>,
    import_map: HashMap<String, String>,
    /// The last painted page bitmap. Repainting a full page at 60 fps is the
    /// single biggest source of UI lag on complex sites, so the shell reuses
    /// this until the document, scroll position or viewport changes.
    paint_cache: Option<Canvas>,
    paint_scroll: Point,
    paint_viewport: Size,
    /// True while the initial document load is still running. Style/layout is
    /// deferred to the single rebuild at the end instead of being recomputed
    /// after every timer and observer turn.
    loading: bool,
}

impl Page {
    pub fn load(url: &str, opts: LoadOptions) -> Page {
        let client = Rc::new(Client::new().unwrap_or_else(|_| Client::default()));
        Page::load_with_client(client, url, opts)
    }

    pub fn load_with_client(client: Rc<Client>, url: &str, opts: LoadOptions) -> Page {
        let started = Instant::now();
        let target = Url::parse(url).unwrap_or_else(|| Url::parse("about:blank").unwrap());
        let response = if target.scheme == "file" {
            let path = file_path(&target);
            match std::fs::read(&path) {
                Ok(body) => Ok(Response {
                    url: target.clone(),
                    status: 200,
                    reason: "OK".into(),
                    headers: vec![("content-type".into(), guess_mime(&path, &body))],
                    body,
                    from_cache: false,
                    redirects: vec![],
                    elapsed_ms: 0,
                }),
                Err(e) => Err(veyra_net::NetError::Io(std::io::Error::new(e.kind(), e.to_string()))),
            }
        } else if target.scheme == "veyra" || target.scheme == "about" {
            Ok(Response {
                url: target.clone(),
                status: 200,
                reason: "OK".into(),
                headers: vec![("content-type".into(), "text/html; charset=utf-8".into())],
                body: newtab_html().into_bytes(),
                from_cache: false,
                redirects: vec![],
                elapsed_ms: 0,
            })
        } else {
            client.fetch(Request::get(target.to_string()).kind(ResourceType::Document))
        };
        let mut load_error = None;
        let (response, url, status) = match response {
            Ok(r) => {
                let u = r.url.clone();
                let s = r.status;
                (r, u, s)
            }
            Err(err) => {
                load_error = Some(err.to_string());
                let html = error_page(&target, &err.to_string());
                let body = html.into_bytes();
                let headers = vec![("content-type".to_string(), "text/html; charset=utf-8".to_string())];
                (
                    Response {
                        url: target.clone(),
                        status: 0,
                        reason: "Network error".into(),
                        headers,
                        body,
                        from_cache: false,
                        redirects: vec![],
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    },
                    target.clone(),
                    0u16,
                )
            }
        };
        let html = String::from_utf8_lossy(&response.body).to_string();
        let (doc, _stats) = parse_document(&html, url.clone());
        let state = Rc::new(RefCell::new(DomState::new(doc, opts.viewport)));
        if response.status >= 400 {
            state.borrow_mut().dirty = true;
        }
        let logs = Rc::new(RefCell::new(Vec::new()));
        let host = PageHost { client: client.clone(), logs: logs.clone(), start: started, seed: 0x9e3779b97f4a7c15 };
        let mut js = Interp::new(Box::new(host));
        js.set_now(0.0);
        bindings::install(&mut js, state.clone());

        let mut page = Page {
            url: url.clone(),
            status,
            viewport: opts.viewport,
            state,
            js,
            fonts: FontSystem::new(),
            layout: None,
            client,
            errors: Vec::new(),
            console: logs,
            scripts_run: 0,
            modules_skipped: 0,
            load_ms: 0,
            started,
            css_seen: HashSet::new(),
            script_texts: Vec::new(),
            scripts_enabled: opts.scripts,
            max_script_bytes: opts.max_script_bytes,
            module_cache: HashMap::new(),
            import_map: HashMap::new(),
            paint_cache: None,
            paint_scroll: Point::new(-1.0, -1.0),
            paint_viewport: Size::new(0.0, 0.0),
            loading: true,
        };
        if let Some(e) = load_error {
            page.errors.push(format!("navigation: {e}"));
        }
        let trace = std::env::var_os("VEYRA_DEBUG_LOAD").is_some();
        let trace_start = std::time::Instant::now();
        if trace { eprintln!("LOAD stage=stylesheets ms={}", trace_start.elapsed().as_millis()); }
        page.collect_stylesheets();
        if trace { eprintln!("LOAD stage=images ms={}", trace_start.elapsed().as_millis()); }
        page.load_images();
        if opts.scripts {
            if trace { eprintln!("LOAD stage=scripts ms={}", trace_start.elapsed().as_millis()); }
            page.run_scripts();
        }
        if trace { eprintln!("LOAD stage=finish ms={}", trace_start.elapsed().as_millis()); }
        page.finish_load();
        page.loading = false;
        if trace { eprintln!("LOAD stage=done ms={}", trace_start.elapsed().as_millis()); }
        page.load_ms = started.elapsed().as_millis() as u64;
        let _ = status;
        page
    }

    pub fn title(&self) -> String {
        let title = self.state.borrow().title.clone();
        if title.is_empty() {
            self.url.to_string()
        } else {
            title
        }
    }

    pub fn scroll(&self) -> Point {
        self.state.borrow().scroll
    }

    pub fn content_size(&self) -> Size {
        self.layout.as_ref().map(|l| l.content_size).unwrap_or(self.viewport)
    }

    pub fn max_scroll(&self) -> Point {
        let content = self.content_size();
        Point::new(
            (content.w - self.viewport.w).max(0.0),
            (content.h - self.viewport.h).max(0.0),
        )
    }

    pub fn set_scroll(&mut self, p: Point) {
        let max = self.max_scroll();
        let p = Point::new(p.x.clamp(0.0, max.x), p.y.clamp(0.0, max.y));
        self.state.borrow_mut().scroll = p;
    }

    pub fn scroll_by(&mut self, dx: f32, dy: f32) {
        let p = self.scroll();
        self.set_scroll(Point::new(p.x + dx, p.y + dy));
    }

    pub fn resize(&mut self, viewport: Size) {
        self.viewport = viewport;
        self.state.borrow_mut().viewport = viewport;
        self.rebuild();
        self.set_scroll(self.scroll());
    }

    pub fn rebuild(&mut self) {
        let trace = std::env::var_os("VEYRA_DEBUG_LOAD").is_some();
        let t0 = std::time::Instant::now();
        let mut sources = Vec::new();
        let mut shadow_sources: Vec<(String, veyra_dom::NodeId)> = Vec::new();
        {
            let s = self.state.borrow();
            sources.extend(s.net_css.iter().cloned());
            for n in s.doc.elements_in_document_order() {
                if s.doc.tag_name(n) == "style" {
                    sources.push(s.doc.text_content(n));
                }
            }
            // Styles inside a shadow tree stay scoped to that tree.
            for (_host, root) in s.doc.shadow_roots.clone() {
                let mut stack = vec![root];
                while let Some(n) = stack.pop() {
                    if s.doc.tag_name(n) == "style" {
                        shadow_sources.push((s.doc.text_content(n), root));
                    }
                    stack.extend(s.doc.children(n).iter().copied());
                }
            }
            sources.extend(s.extra_css.iter().cloned());
        }
        let t_parse = std::time::Instant::now();
        {
            let mut s = self.state.borrow_mut();
            let base = s.base.clone();
            s.styles.clear_author_sheets();
            for css in &sources {
                if !css.trim().is_empty() {
                    s.styles.add_stylesheet(css, &base);
                }
            }
            for (css, root) in &shadow_sources {
                if !css.trim().is_empty() {
                    s.styles.add_stylesheet_scoped(css, &base, *root);
                }
            }
            s.doc.distribute_slots();
            let doc = s.doc.clone();
            s.styles.set_viewport(self.viewport.w, self.viewport.h);
            if trace { eprintln!("REBUILD parse ms={} sheets={} bytes={}", t_parse.elapsed().as_millis(), sources.len(), sources.iter().map(|s| s.len()).sum::<usize>()); }
            let t_compute = std::time::Instant::now();
            s.styles.compute(&doc);
            if trace { eprintln!("REBUILD compute ms={} elements={} matched={} candidates={}", t_compute.elapsed().as_millis(), s.styles.stats.elements, s.styles.stats.matched_rules, s.styles.stats.candidates); }
            s.dirty = false;
        }
        if trace { eprintln!("REBUILD styles ms={} sheets={}", t0.elapsed().as_millis(), sources.len()); }
        let t1 = std::time::Instant::now();
        let tree = {
            let s = self.state.borrow();
            layout_document(
                &s.doc,
                &s.styles,
                &self.fonts,
                &EngineImages(&s.images),
                self.viewport,
            )
        };
        if trace { eprintln!("REBUILD layout ms={}", t1.elapsed().as_millis()); }
        {
            let mut s = self.state.borrow_mut();
            s.boxes = tree.boxes.clone();
            let max = Point::new(
                (tree.content_size.w - self.viewport.w).max(0.0),
                (tree.content_size.h - self.viewport.h).max(0.0),
            );
            s.scroll = Point::new(s.scroll.x.clamp(0.0, max.x), s.scroll.y.clamp(0.0, max.y));
        }
        self.layout = Some(tree);
    }

    pub fn ensure_layout(&mut self) {
        if self.layout.is_none() || self.state.borrow().dirty {
            self.rebuild();
        }
    }

    pub fn paint(&mut self) -> &Canvas {
        let scroll = self.state.borrow().scroll;
        let needs_paint = self.paint_cache.is_none()
            || self.state.borrow().dirty
            || self.paint_scroll != scroll
            || self.paint_viewport != self.viewport;
        if needs_paint {
            self.ensure_layout();
            let layout = self.layout.as_ref().expect("layout");
            let canvas = {
                let state = self.state.borrow();
                let config = PaintConfig {
                    tree: layout,
                    fonts: &self.fonts,
                    images: &state.images,
                    viewport: self.viewport,
                    scroll,
                    scale: 1.0,
                    background: veyra_common::Color::WHITE,
                };
                paint_page(&config)
            };
            self.paint_cache = Some(canvas);
            self.paint_scroll = scroll;
            self.paint_viewport = self.viewport;
            self.state.borrow_mut().dirty = false;
        }
        self.paint_cache.as_ref().expect("paint cache")
    }

    pub fn screenshot_png(&mut self) -> Vec<u8> {
        self.paint().to_png()
    }

    /// Dispatch a click at content coordinates and return a navigation target
    /// when a link/JS navigation was requested.
    pub fn handle_click(&mut self, x: f32, y: f32) -> Option<Url> {
        self.ensure_layout();
        let scroll = self.state.borrow().scroll;
        let point = Point::new(x + scroll.x, y + scroll.y);
        let hit = self
            .layout
            .as_ref()
            .and_then(|l| l.hit_test(point))
            .unwrap_or_else(|| self.state.borrow().doc.root);
        let target = {
            let s = self.state.borrow();
            let mut n = hit;
            while !s.doc.is_element(n) && n != s.doc.root {
                match s.doc.parent(n) {
                    Some(p) => n = p,
                    None => break,
                }
            }
            n
        };
        for ty in ["mousedown", "mouseup", "click"] {
            if let Err(e) = dispatch_event(&mut self.js, &self.state, target, ty) {
                self.errors.push(format!("{ty}: {e}"));
            }
        }
        self.pump();
        self.take_navigation()
    }

    pub fn handle_mouse(&mut self, event: &str, x: f32, y: f32) {
        self.ensure_layout();
        let scroll = self.state.borrow().scroll;
        let point = Point::new(x + scroll.x, y + scroll.y);
        let hit = self.layout.as_ref().and_then(|l| l.hit_test(point)).unwrap_or_else(|| self.state.borrow().doc.root);
        if let Err(e) = dispatch_event(&mut self.js, &self.state, hit, event) {
            self.errors.push(format!("{event}: {e}"));
        }
        self.pump();
    }

    pub fn handle_key(&mut self, key: &str, ctrl: bool) {
        let focused = self.state.borrow().focused;
        let target = focused.unwrap_or_else(|| self.state.borrow().doc.body().unwrap_or(self.state.borrow().doc.root));
        if let Err(e) = dispatch_event(&mut self.js, &self.state, target, "keydown") {
            self.errors.push(format!("keydown: {e}"));
        }
        if let Some(node) = focused {
            let is_field = {
                let s = self.state.borrow();
                matches!(s.doc.tag_name(node), "input" | "textarea")
            };
            if is_field {
                if key == "Backspace" {
                    let mut s = self.state.borrow_mut();
                    let mut cur = s
                        .form_values
                        .get(&node)
                        .cloned()
                        .or_else(|| s.doc.attr(node, "value").map(|v| v.to_string()))
                        .unwrap_or_default();
                    cur.pop();
                    s.form_values.insert(node, cur.clone());
                    s.doc.set_attr(node, "value", &cur);
                    s.mark_dirty();
                } else if key == "Enter" {
                    let mut s = self.state.borrow_mut();
                    s.mark_dirty();
                } else if key.chars().count() == 1 && !ctrl {
                    let mut s = self.state.borrow_mut();
                    let mut cur = s
                        .form_values
                        .get(&node)
                        .cloned()
                        .or_else(|| s.doc.attr(node, "value").map(|v| v.to_string()))
                        .unwrap_or_default();
                    cur.push_str(key);
                    s.form_values.insert(node, cur.clone());
                    s.doc.set_attr(node, "value", &cur);
                    s.mark_dirty();
                }
            }
        }
        self.pump();
    }

    pub fn take_navigation(&mut self) -> Option<Url> {
        self.state.borrow_mut().navigation.take()
    }

    pub fn reload_requested(&mut self) -> bool {
        let r = self.state.borrow().reload;
        if r {
            self.state.borrow_mut().reload = false;
        }
        r
    }

    pub fn run_js(&mut self, source: &str) -> Result<String, String> {
        match self.js.eval_script(source) {
            Ok(v) => {
                let text = self.js.to_string(&v).unwrap_or_default();
                self.pump();
                Ok(text)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn pump(&mut self) {
        let _ = self.js.run_microtasks();
        let now = self.started.elapsed().as_secs_f64() * 1000.0;
        for _ in 0..4 {
            let scripts_ran = self.run_pending_scripts();
            let ran = self.js.run_due_timers(now).unwrap_or(0);
            let _ = self.js.run_microtasks();
            // Template stamping can attach a shadow tree after its host was
            // already connected; settle any custom elements that appeared in
            // a tree without a connected insertion path.
            let root = self.state.borrow().doc.root;
            crate::bindings::connect_subtree(&mut self.js, &self.state, root);
            crate::bindings::run_custom_element_lifecycle(&mut self.js, &self.state);
            let mutated = self.flush_observers();
            if ran == 0 && !mutated && !scripts_ran {
                break;
            }
        }
        // Images created or src-swapped by script (lazy loaders use
        // IntersectionObserver) are fetched after the JS turn settles.
        if self.state.borrow().images_dirty {
            self.state.borrow_mut().images_dirty = false;
            self.load_images();
        }
        if self.state.borrow().dirty && !self.loading {
            self.rebuild();
        }
    }

    fn flush_observers(&mut self) -> bool {
        let (observers, mutations) = {
            let mut s = self.state.borrow_mut();
            if s.mutations.is_empty() || s.observers.is_empty() {
                return false;
            }
            (s.observers.clone(), std::mem::take(&mut s.mutations))
        };
        let mut any = false;
        for obs in observers {
            let relevant: Vec<Mutation> = mutations
                .iter()
                .filter(|m| {
                    if m.target == obs.target {
                        return true;
                    }
                    if obs.subtree {
                        let s = self.state.borrow();
                        let mut cur = Some(m.target);
                        while let Some(c) = cur {
                            if c == obs.target {
                                return true;
                            }
                            cur = s.doc.parent(c);
                        }
                    }
                    false
                })
                .cloned()
                .collect();
            if relevant.is_empty() {
                continue;
            }
            let arr = crate::event_api::records_to_array(&mut self.js, &self.state, &relevant);
            let this = obs.owner.clone().unwrap_or(veyra_js::Value::Undefined);
            let result = self.js.call_value(&obs.callback, this.clone(), vec![arr, this]);
            if let Err(e) = result {
                self.errors.push(format!("MutationObserver: {e} [{}]", e.stack.join("; ")));
            }
            any = true;
        }
        any
    }

    fn collect_stylesheets(&mut self) {
        let links: Vec<(Url, String)> = {
            let s = self.state.borrow();
            s.doc
                .elements_in_document_order()
                .into_iter()
                .filter(|n| s.doc.tag_name(*n) == "link")
                .filter_map(|n| {
                    let rel = s.doc.attr(n, "rel")?.to_ascii_lowercase();
                    if !rel.split_whitespace().any(|r| r == "stylesheet") {
                        return None;
                    }
                    let href = s.doc.attr(n, "href")?;
                    Some((s.base.resolve(href), s.doc.attr(n, "media").unwrap_or("").to_string()))
                })
                .take(24)
                .collect()
        };
        for (url, media) in links {
            if !media.is_empty() && !crate::style_api::media_matches(&media, self.viewport, false) {
                continue;
            }
            self.fetch_css(url);
        }
    }

    fn load_images(&mut self) {
        let budget_ms = std::env::var("VEYRA_IMAGE_BUDGET_MS")
            .ok()
            .and_then(|v| v.parse::<u128>().ok())
            .unwrap_or(1200);
        let started = Instant::now();
        let urls: Vec<Url> = {
            let s = self.state.borrow();
            s.doc
                .elements_in_document_order()
                .into_iter()
                .filter(|n| matches!(s.doc.tag_name(*n), "img" | "source"))
                .filter_map(|n| {
                    if let Some(src) = s.doc.attr(n, "src") {
                        return Some(s.base.resolve(src));
                    }
                    let srcset = s.doc.attr(n, "srcset")?;
                    let first = srcset.split(',').next()?.split_whitespace().next()?;
                    Some(s.base.resolve(first))
                })
                .take(80)
                .collect()
        };
        for url in urls {
            if started.elapsed().as_millis() >= budget_ms {
                break;
            }
            let key = url.to_string();
            if self.state.borrow().images.get_str(&key).is_some() {
                continue;
            }
            if self.state.borrow().images.failed(&key) {
                continue;
            }
            match self.client.fetch(Request::get(key.clone()).kind(ResourceType::Image)) {
                Ok(resp) if resp.status < 400 => {
                    if !self.state.borrow_mut().images.insert_bytes(&key, &resp.body) {
                        self.state.borrow_mut().images.mark_failed(&key);
                    }
                }
                Ok(_) => self.state.borrow_mut().images.mark_failed(&key),
                Err(e) => {
                    self.errors.push(format!("image {url}: {e}"));
                    self.state.borrow_mut().images.mark_failed(&key);
                }
            }
        }
    }

    fn fetch_css(&mut self, url: Url) {
        let key = url.to_string();
        if !self.css_seen.insert(key.clone()) {
            return;
        }
        match self.client.fetch(Request::get(key.clone()).kind(ResourceType::StyleSheet)) {
            Ok(resp) => {
                if resp.body.len() <= 5_000_000 {
                    let css = String::from_utf8_lossy(&resp.body).to_string();
                    self.state.borrow_mut().net_css.push(css);
                }
            }
            Err(e) => self.errors.push(format!("stylesheet {url}: {e}")),
        }
    }

    /// Modern bundlers (Next.js, Vite, Nuxt) often inject stylesheet links from
    /// JavaScript. Until script execution can hydrate CSS modules, scan script
    /// and RSC payload text for same-origin `.css` chunks and fetch them.
    fn harvest_css(&mut self) {
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for text in &self.script_texts {
            for href in extract_css_urls(text) {
                if seen.insert(href.clone()) {
                    candidates.push(self.state.borrow().base.resolve(&href));
                }
            }
        }
        for url in candidates.into_iter().take(20) {
            self.fetch_css(url);
        }
    }

    /// Reads `<script type="importmap">` into a flat specifier map.
    fn collect_import_map(&mut self) {
        let text = {
            let s = self.state.borrow();
            s.doc
                .elements_in_document_order()
                .into_iter()
                .find(|n| {
                    s.doc.tag_name(*n) == "script"
                        && s.doc.attr(*n, "type").map(|t| t.trim().eq_ignore_ascii_case("importmap")).unwrap_or(false)
                })
                .map(|n| s.doc.text_content(n))
        };
        let Some(text) = text else { return };
        let expr = format!(
            "(function(){{try{{var m=JSON.parse({:?});return Object.entries((m&&m.imports)||{{}});}}catch(e){{return [];}}}})()",
            text
        );
        let Ok(value) = self.js.eval_expression(&expr) else { return };
        let Ok(entries) = self.js.iterate(&value) else { return };
        for entry in entries {
            let Ok(pair) = self.js.iterate(&entry) else { continue };
            if pair.len() < 2 {
                continue;
            }
            let key = self.js.to_string(&pair[0]).unwrap_or_default();
            let value = self.js.to_string(&pair[1]).unwrap_or_default();
            if !key.is_empty() && !value.is_empty() {
                self.import_map.insert(key, value);
            }
        }
    }

    fn resolve_specifier(&self, specifier: &str, module_url: &str) -> Result<String, String> {
        let base = if module_url.starts_with("inline:") {
            self.url.clone()
        } else {
            Url::parse(module_url).unwrap_or_else(|| self.url.clone())
        };
        let relative = specifier.starts_with('/')
            || specifier.starts_with("./")
            || specifier.starts_with("../")
            || specifier.contains(':');
        if relative {
            return Ok(base.resolve(specifier).to_string());
        }
        if let Some(mapped) = self.import_map.get(specifier) {
            return Ok(base.resolve(mapped).to_string());
        }
        // Prefix import-map entries end in `/` (scoped package maps).
        let mut best: Option<(&String, &String)> = None;
        for (key, value) in &self.import_map {
            if key.ends_with('/') && specifier.starts_with(key.as_str()) {
                if best.map(|(k, _)| key.len() > k.len()).unwrap_or(true) {
                    best = Some((key, value));
                }
            }
        }
        if let Some((key, value)) = best {
            let suffix = &specifier[key.len()..];
            let joined = format!("{}{}", value.trim_end_matches('/'), format!("/{suffix}"));
            return Ok(base.resolve(&joined).to_string());
        }
        Err(format!("unresolved bare module specifier `{specifier}`"))
    }

    fn namespace_object(&mut self, ns: &HashMap<String, Value>) -> Value {
        let obj = self.js.object();
        if let Some(o) = obj.as_object() {
            for (key, value) in ns {
                define(o, key, value.clone());
            }
            define(o, "__esModule", Value::Bool(true));
        }
        obj
    }

    fn load_module(
        &mut self,
        key: &str,
        inline: Option<&str>,
        stack: &mut Vec<String>,
        count: &mut usize,
    ) -> Result<HashMap<String, Value>, String> {
        if let Some(ns) = self.module_cache.get(key) {
            return Ok(ns.clone());
        }
        if stack.iter().any(|s| s == key) {
            // A cycle: expose the partial namespace (empty for the first
            // evaluation). Live bindings are approximated by initial values.
            return Ok(HashMap::new());
        }
        if *count > 250 {
            return Err("module graph exceeded 250 modules".into());
        }
        *count += 1;
        stack.push(key.to_string());
        let module_start = Instant::now();
        let debug_modules = std::env::var_os("VEYRA_DEBUG_MODULES").is_some();
        let source = match inline {
            Some(source) => source.to_string(),
            None => {
                if let Some(path) = key.strip_prefix("file:") {
                    let path = path
                        .trim_start_matches("//")
                        .split(['?', '#'])
                        .next()
                        .unwrap_or(path);
                    std::fs::read_to_string(path)
                        .map_err(|e| format!("module {key}: {e}"))?
                } else {
                    let request = Request::get(key.to_string()).kind(ResourceType::Script);
                    let resp = self.client.fetch(request).map_err(|e| format!("module {key}: {e}"))?;
                    if resp.body.len() > self.max_script_bytes {
                        stack.pop();
                        return Err(format!("module too large: {key}"));
                    }
                    String::from_utf8_lossy(&resp.body).to_string()
                }
            }
        };
        let module = veyra_js::parser::parse_module(&source)
            .map_err(|e| format!("module {key}: line {}: {}", e.line, e.message))?;
        if debug_modules {
            eprintln!("MODULE {} bytes={} parse_ms={}", key, source.len(), module_start.elapsed().as_millis());
        }

        let mut specs: Vec<String> = Vec::new();
        for import in &module.imports {
            specs.push(import.source.clone());
        }
        for (_, _, _) in &module.reexports {
            // reexports carry their own source below
        }
        for (source, _, _) in &module.reexports {
            specs.push(source.clone());
        }
        for source in &module.export_all {
            specs.push(source.clone());
        }
        specs.sort();
        specs.dedup();
        let mut deps: HashMap<String, HashMap<String, Value>> = HashMap::new();
        for spec in specs {
            let resolved = self.resolve_specifier(&spec, key)?;
            let child = self.load_module(&resolved, None, stack, count)?;
            deps.insert(spec, child);
        }

        let mut imports: HashMap<String, Value> = HashMap::new();
        for binding in &module.imports {
            let ns = deps.get(&binding.source).cloned().unwrap_or_default();
            if let Some(name) = &binding.default {
                imports.insert(name.clone(), ns.get("default").cloned().unwrap_or(Value::Undefined));
            }
            if let Some(name) = &binding.namespace {
                imports.insert(name.clone(), self.namespace_object(&ns));
            }
            for (imported, local) in &binding.named {
                imports.insert(local.clone(), ns.get(imported).cloned().unwrap_or(Value::Undefined));
            }
        }

        let exports = self
            .js
            .eval_module(&module, &imports, key)
            .map_err(|e| format!("module {key}: {e} [{}]", e.stack.join("; ")))?;
        let mut namespace = exports;
        for (source, imported, exported) in &module.reexports {
            let ns = deps.get(source).cloned().unwrap_or_default();
            let value = if imported == "*" {
                self.namespace_object(&ns)
            } else {
                ns.get(imported).cloned().unwrap_or(Value::Undefined)
            };
            namespace.insert(exported.clone(), value);
        }
        for source in &module.export_all {
            if let Some(ns) = deps.get(source) {
                for (name, value) in ns {
                    if name != "default" {
                        namespace.entry(name.clone()).or_insert_with(|| value.clone());
                    }
                }
            }
        }
        self.module_cache.insert(key.to_string(), namespace.clone());
        if debug_modules {
            eprintln!("MODULE {} total_ms={} exports={}", key, module_start.elapsed().as_millis(), namespace.len());
        }
        stack.pop();

        // Dynamic imports requested while the module ran.
        let pending = self.js.take_pending_imports();
        for (specifier, resolve, reject) in pending {
            match self.resolve_specifier(&specifier, key) {
                Ok(resolved) => match self.load_module(&resolved, None, stack, count) {
                    Ok(ns) => {
                        let obj = self.namespace_object(&ns);
                        let _ = self.js.call_value(&resolve, Value::Undefined, vec![obj]);
                    }
                    Err(e) => {
                        let err = self.js.make_error("TypeError", &e);
                        let _ = self.js.call_value(&reject, Value::Undefined, vec![err]);
                    }
                },
                Err(e) => {
                    let err = self.js.make_error("TypeError", &e);
                    let _ = self.js.call_value(&reject, Value::Undefined, vec![err]);
                }
            }
        }
        let _ = self.js.run_microtasks();
        Ok(namespace)
    }

    fn run_module_scripts(&mut self) {
        let modules: Vec<(veyra_dom::NodeId, Option<Url>, String)> = {
            let s = self.state.borrow();
            s.doc
                .elements_in_document_order()
                .into_iter()
                .filter(|n| s.doc.tag_name(*n) == "script")
                .filter(|n| {
                    s.doc
                        .attr(*n, "type")
                        .map(|t| t.trim().eq_ignore_ascii_case("module"))
                        .unwrap_or(false)
                })
                .take(40)
                .map(|n| {
                    let src = s.doc.attr(n, "src").map(|v| s.base.resolve(v));
                    (n, src, s.doc.text_content(n))
                })
                .collect()
        };
        if modules.is_empty() {
            return;
        }
        self.collect_import_map();
        let mut count = 0usize;
        for (node, src, inline) in modules {
            let key = src
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| format!("inline:module:{}", node.0));
            self.state.borrow_mut().current_script = Some(node);
            self.scripts_run += 1;
            let mut stack = Vec::new();
            let inline_source = if src.is_some() { None } else { Some(inline.as_str()) };
            let result = self.load_module(&key, inline_source, &mut stack, &mut count);
            self.state.borrow_mut().current_script = None;
            match result {
                Ok(_) => {
                    let _ = self.js.run_microtasks();
                }
                Err(e) => self.errors.push(format!("module error [{key}]: {e}")),
            }
        }
        let _ = self.pump();
    }

    /// Executes script elements inserted by page code after parsing. A script
    /// loader appends a script and waits for its `load` event before starting
    /// the next dependency, so fetching and evaluating here (rather than at
    /// the next navigation) is required for module/bootstrap graphs.
    fn run_pending_scripts(&mut self) -> bool {
        let mut any = false;
        loop {
            let pending: Vec<veyra_dom::NodeId> = {
                let mut s = self.state.borrow_mut();
                std::mem::take(&mut s.pending_scripts)
            };
            if pending.is_empty() {
                break;
            }
            for node in pending {
                let already = {
                    let mut s = self.state.borrow_mut();
                    !s.executed_scripts.insert(node)
                };
                if already {
                    continue;
                }
                let (src, ty, inline) = {
                    let s = self.state.borrow();
                    let ty = s.doc.attr(node, "type").unwrap_or("").to_ascii_lowercase();
                    let src = s.doc.attr(node, "src").map(|v| s.base.resolve(v));
                    (src, ty, s.doc.text_content(node))
                };
                if ty.contains("module") {
                    // Module scripts have their own registry and are handled by
                    // the module loader; evaluating them as classic scripts
                    // would give them the wrong `this` and no imports.
                    continue;
                }
                if !ty.is_empty()
                    && ty != "text/javascript"
                    && ty != "application/javascript"
                    && ty != "text/ecmascript"
                {
                    continue;
                }
                let source = match &src {
                    Some(url) => match self.client.fetch(Request::get(url.to_string()).kind(ResourceType::Script)) {
                        Ok(resp) => {
                            if resp.body.len() > self.max_script_bytes {
                                self.errors.push(format!("script too large: {url}"));
                                let _ = dispatch_event(&mut self.js, &self.state, node, "error");
                                continue;
                            }
                            String::from_utf8_lossy(&resp.body).to_string()
                        }
                        Err(e) => {
                            self.errors.push(format!("script {url}: {e}"));
                            let _ = dispatch_event(&mut self.js, &self.state, node, "error");
                            continue;
                        }
                    },
                    None => inline,
                };
                if source.trim().is_empty() {
                    let _ = dispatch_event(&mut self.js, &self.state, node, "load");
                    continue;
                }
                self.state.borrow_mut().current_script = Some(node);
                let result = self.js.eval_script(&source);
                self.state.borrow_mut().current_script = None;
                match result {
                    Ok(_) => {
                        let _ = self.js.run_microtasks();
                        let _ = dispatch_event(&mut self.js, &self.state, node, "load");
                    }
                    Err(e) => {
                        self.errors.push(format!(
                            "script error [{}] {}: {} [stack: {}]",
                            src.as_ref().map(|u| u.to_string()).unwrap_or_else(|| "inline".into()),
                            e.kind,
                            e.message,
                            e.stack.join("; ")
                        ));
                        let _ = dispatch_event(&mut self.js, &self.state, node, "error");
                    }
                }
                any = true;
            }
        }
        any
    }

    fn run_scripts(&mut self) {
        let scripts: Vec<(veyra_dom::NodeId, Option<Url>, String, String)> = {
            let s = self.state.borrow();
            s.doc
                .elements_in_document_order()
                .into_iter()
                .filter(|n| s.doc.tag_name(*n) == "script")
                .take(60)
                .map(|n| {
                    let ty = s.doc.attr(n, "type").unwrap_or("").to_ascii_lowercase();
                    let src = s.doc.attr(n, "src").map(|v| s.base.resolve(v));
                    (n, src, ty, s.doc.text_content(n))
                })
                .collect()
        };
        for (script_node, src, ty, inline) in scripts {
            if ty.contains("module") {
                continue;
            }
            if !ty.is_empty() && ty != "text/javascript" && ty != "application/javascript" && ty != "text/ecmascript" {
                continue;
            }
            let source = match &src {
                Some(url) => match self.client.fetch(Request::get(url.to_string()).kind(ResourceType::Script)) {
                    Ok(resp) => {
                        if resp.body.len() > self.max_script_bytes {
                            self.errors.push(format!("script too large: {url}"));
                            continue;
                        }
                        String::from_utf8_lossy(&resp.body).to_string()
                    }
                    Err(e) => {
                        self.errors.push(format!("script {url}: {e}"));
                        continue;
                    }
                },
                None => inline,
            };
            if source.trim().is_empty() {
                continue;
            }
            if self.script_texts.len() < 80 {
                self.script_texts.push(source.clone());
            }
            self.state.borrow_mut().current_script = Some(script_node);
            self.scripts_run += 1;
            let source_name = src
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| format!("inline-script-{}", self.scripts_run));
            self.js.current_source = Some(source_name);
            let debug_scripts = std::env::var("VEYRA_DEBUG_SCRIPTS").is_ok();
            if debug_scripts {
                eprintln!(
                    "SCRIPT {} len={} start={:?}",
                    src.as_ref().map(|u| u.to_string()).unwrap_or_else(|| "inline".into()),
                    source.len(),
                    &source[..source.len().min(60)]
                );
            }
            let script_start = std::time::Instant::now();
            let steps_before = self.js.steps;
            let eval_result = self.js.eval_script(&source);
            self.js.current_source = None;
            crate::bindings::sync_shady_wrappers(&mut self.js, &self.state);
            let script_ms = script_start.elapsed().as_millis();
            let script_steps = self.js.steps.saturating_sub(steps_before);
            self.state.borrow_mut().current_script = None;
            if debug_scripts {
                let html_class = {
                    let s = self.state.borrow();
                    s.doc
                        .document_element()
                        .and_then(|n| s.doc.attr(n, "class"))
                        .unwrap_or("")
                        .chars()
                        .take(24)
                        .collect::<String>()
                };
                eprintln!("SCRIPT OK ms={script_ms} steps={script_steps} class={html_class}");
            }
            match eval_result {
                Ok(_) => {
                    let _ = self.js.run_microtasks();
                }
                Err(e) => {
                    let line = e
                        .stack
                        .iter()
                        .find_map(|s| s.strip_prefix("at line "))
                        .and_then(|n| n.parse::<usize>().ok());
                    let label = src.as_ref().map(|u| u.to_string()).unwrap_or_else(|| "inline".into());
                    let offset = e
                        .stack
                        .iter()
                        .find_map(|s| s.strip_prefix("at offset "))
                        .and_then(|n| n.parse::<usize>().ok());
                    let snippet = offset
                        .map(|off| source_excerpt(&source, off))
                        .unwrap_or_default();
                    self.errors.push(format!(
                        "script error [{}:{}] {}: {}{} [stack: {}] [source: {}]",
                        label,
                        line.map(|n| n.to_string()).unwrap_or_else(|| "?".into()),
                        e.kind,
                        e.message,
                        if snippet.is_empty() { String::new() } else { format!(" | {snippet}") },
                        e.stack.join("; "),
                        source.chars().take(70).collect::<String>().replace('\n', " ")
                    ));
                }
            }
            if self.js.output.len() > 0 {
                let mut logs = self.console.borrow_mut();
                for line in self.js.output.drain(..) {
                    if logs.len() < 2000 {
                        logs.push(line);
                    }
                }
            }
        }
        self.run_module_scripts();
    }

    fn finish_load(&mut self) {
        let trace = std::env::var_os("VEYRA_DEBUG_LOAD").is_some();
        let mut stage = std::time::Instant::now();
        {
            let mut s = self.state.borrow_mut();
            s.doc.ready_state = "interactive".into();
        }
        let root = self.state.borrow().doc.root;
        if let Err(e) = dispatch_event(&mut self.js, &self.state, root, "DOMContentLoaded") {
            self.errors.push(format!("DOMContentLoaded: {e}"));
        }
        if trace { eprintln!("FINISH dcl ms={} steps={}", stage.elapsed().as_millis(), self.js.steps); stage = std::time::Instant::now(); }
        self.pump();
        if trace { eprintln!("FINISH pump1 ms={} steps={}", stage.elapsed().as_millis(), self.js.steps); stage = std::time::Instant::now(); }
        {
            let mut s = self.state.borrow_mut();
            s.doc.ready_state = "complete".into();
        }
        if let Err(e) = dispatch_event(&mut self.js, &self.state, root, "load") {
            self.errors.push(format!("load: {e}"));
        }
        if trace { eprintln!("FINISH load ms={} steps={}", stage.elapsed().as_millis(), self.js.steps); stage = std::time::Instant::now(); }
        self.collect_stylesheets();
        self.harvest_css();
        if trace { eprintln!("FINISH css ms={} styles={}", stage.elapsed().as_millis(), self.state.borrow().net_css.len()); stage = std::time::Instant::now(); }
        self.pump();
        if trace { eprintln!("FINISH pump2 ms={} steps={}", stage.elapsed().as_millis(), self.js.steps); stage = std::time::Instant::now(); }
        self.rebuild();
        if trace { eprintln!("FINISH rebuild ms={} steps={}", stage.elapsed().as_millis(), self.js.steps); }
    }

    pub fn info(&self) -> String {
        let s = self.state.borrow();
        let boxes = self.layout.as_ref().map(|l| count_boxes(&l.root)).unwrap_or(0);
        format!(
            "url: {}\nstatus: {}\ntitle: {}\nviewport: {}x{}\nelements: {}\nlayout boxes: {}\nscripts: {} run, {} modules skipped\nstylesheets: {} network, {} inserted\nimages: {} cached\nerrors: {}\n",
            self.url,
            self.status,
            self.title(),
            self.viewport.w,
            self.viewport.h,
            s.doc.elements_in_document_order().len(),
            boxes,
            self.scripts_run,
            self.modules_skipped,
            s.net_css.len(),
            s.extra_css.len(),
            s.images.len(),
            self.errors.len(),
        )
    }

    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    pub fn run_timers_once(&mut self) {
        let now = self.started.elapsed().as_secs_f64() * 1000.0;
        let _ = self.js.run_due_timers(now);
        self.pump();
    }

    pub fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

fn count_boxes(b: &Option<veyra_layout::LayoutBox>) -> usize {
    fn rec(b: &veyra_layout::LayoutBox) -> usize {
        1 + b.children.iter().map(rec).sum::<usize>()
    }
    b.as_ref().map(rec).unwrap_or(0)
}

fn newtab_html() -> String {
    r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>New Tab</title>
<style>
*{box-sizing:border-box}
body{margin:0;font-family:Arial,'DejaVu Sans',sans-serif;background:#fff;color:#202124;min-height:100vh;}
.top{position:fixed;top:0;right:18px;width:190px;display:flex;align-items:center;justify-content:flex-end;gap:16px;padding:16px 0;font-size:13px;}
.top a{color:#202124;text-decoration:none;}
.top a:hover{text-decoration:underline;}
.avatar{width:32px;height:32px;border-radius:50%;background:#1a73e8;color:#fff;display:flex;align-items:center;justify-content:center;font-size:14px;font-weight:bold;}
.main{display:flex;flex-direction:column;align-items:center;padding-top:158px;}
.logo{font-size:68px;font-weight:bold;letter-spacing:-3px;line-height:1;margin:0 0 30px;}
.logo span{color:#1a73e8;}
.search{width:584px;max-width:calc(100vw - 48px);height:48px;border:1px solid #dfe1e5;border-radius:24px;background:#fff;box-shadow:0 1px 6px rgba(32,33,36,.18);display:flex;align-items:center;padding:0 16px;}
.search:hover{box-shadow:0 1px 8px rgba(32,33,36,.3);}
.mag{position:relative;width:12px;height:12px;border:2px solid #5f6368;border-radius:50%;margin-right:14px;}
.mag::after{content:'';position:absolute;left:10px;top:10px;width:7px;height:2px;background:#5f6368;transform:rotate(45deg);}
.search input{flex:1;border:0;outline:0;background:transparent;font-family:inherit;font-size:16px;color:#202124;height:100%;}
.search input::placeholder{color:#5f6368;}
.sc{display:flex;align-items:center;justify-content:flex-end;gap:16px;width:56px;}
.mic{position:relative;width:10px;height:14px;border:2px solid #5f6368;border-radius:7px;}
.mic::after{content:'';position:absolute;left:2px;top:15px;width:2px;height:4px;background:#5f6368;}
.lens{position:relative;width:15px;height:15px;border:2px solid #5f6368;border-radius:50%;}
.lens::after{content:'';position:absolute;left:13px;top:13px;width:6px;height:2px;background:#5f6368;transform:rotate(45deg);}
.shortcuts{display:grid;grid-template-columns:repeat(6,112px);gap:8px;margin-top:38px;width:712px;}
.tile{display:flex;flex-direction:column;align-items:center;gap:9px;text-decoration:none;color:#202124;padding:10px 4px 8px;border-radius:10px;}
.tile:hover{background:#f1f3f4;}
.ico{width:48px;height:48px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:20px;font-weight:bold;color:#fff;background:#5f6368;}
.label{font-size:12px;line-height:1.25;text-align:center;max-width:104px;}
.yt{background:#ff0000;}
.wiki{background:#000;font-family:Georgia,serif;}
.tw{background:#0ea5e9;}
.rust{background:#b7410e;}
.hn{background:#ff6600;}
.example{background:#5f6368;}
.plus{background:#f1f3f4;color:#5f6368;font-size:28px;font-weight:normal;}
.customize{position:fixed;right:20px;bottom:18px;display:flex;align-items:center;gap:9px;background:#fff;border:1px solid #dadce0;border-radius:20px;padding:8px 14px;font-size:13px;color:#3c4043;box-shadow:0 1px 3px rgba(60,64,67,.22);text-decoration:none;}
.pencil{position:relative;width:14px;height:3px;background:#3c4043;border-radius:2px;transform:rotate(-45deg);}
.pencil::after{content:'';position:absolute;right:-5px;top:0;width:0;height:0;border-left:5px solid #3c4043;border-top:3px solid transparent;border-bottom:3px solid transparent;}
</style></head>
<body>
<div class="top"><a href="https://mail.google.com">Gmail</a><a href="https://images.google.com">Images</a><div class="avatar">V</div></div>
<div class="main">
<div class="logo">Veyra<span>.</span></div>
<form class="search" action="https://www.google.com/search" method="get">
<div class="mag"></div>
<input name="q" type="text" placeholder="Search Veyra or type a URL" autocomplete="off">
<div class="sc"><div class="mic"></div><div class="lens"></div></div>
</form>
<div class="shortcuts">
<a class="tile" href="https://www.youtube.com"><div class="ico yt">▶</div><span class="label">YouTube</span></a>
<a class="tile" href="https://en.wikipedia.org/wiki/Web_browser"><div class="ico wiki">W</div><span class="label">Wikipedia</span></a>
<a class="tile" href="https://tailwindcss.com"><div class="ico tw">T</div><span class="label">Tailwind CSS</span></a>
<a class="tile" href="https://www.rust-lang.org"><div class="ico rust">R</div><span class="label">Rust</span></a>
<a class="tile" href="https://news.ycombinator.com"><div class="ico hn">Y</div><span class="label">Hacker News</span></a>
<a class="tile" href="https://example.com"><div class="ico example">E</div><span class="label">Example.com</span></a>
</div>
</div>
<a class="customize" href="veyra://newtab"><div class="pencil"></div>Customize Veyra</a>
<script>
(function(){
  var form=document.querySelector('.search');
  var input=form.querySelector('input');
  function go(){var q=input.value.trim(); if(!q) return; if(q.indexOf('://')>0 || q.indexOf('.')>0 && q.indexOf(' ')<0){location.assign(q.indexOf('://')>0?q:'https://'+q);}else{location.assign('https://www.google.com/search?q='+encodeURIComponent(q));}}
  form.addEventListener('submit',function(e){e.preventDefault();go();});
  input.addEventListener('keydown',function(e){if(e.key==='Enter'){e.preventDefault();go();}});
})();
</script>
</body></html>"#.to_string()
}

fn source_excerpt(source: &str, offset: usize) -> String {
    let offset = offset.min(source.len());
    let mut start = offset.saturating_sub(140);
    while !source.is_char_boundary(start) && start > 0 {
        start -= 1;
    }
    let mut end = (offset + 200).min(source.len());
    while !source.is_char_boundary(end) && end > start {
        end -= 1;
    }
    let before = source[start..offset].replace('\n', " ");
    let after = source[offset..end].replace('\n', " ");
    format!("{}«{}»{}", before.trim(), after.chars().take(1).collect::<String>(), after.chars().skip(1).take(330).collect::<String>())
}

fn file_path(url: &Url) -> PathBuf {
    let decoded = percent_decode(&url.path);
    #[cfg(windows)]
    {
        let trimmed = decoded.trim_start_matches('/');
        return PathBuf::from(trimmed);
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(decoded)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn guess_mime(path: &std::path::Path, body: &[u8]) -> String {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8".into(),
        "css" => "text/css; charset=utf-8".into(),
        "js" | "mjs" => "text/javascript; charset=utf-8".into(),
        "png" => "image/png".into(),
        "jpg" | "jpeg" => "image/jpeg".into(),
        "gif" => "image/gif".into(),
        "webp" => "image/webp".into(),
        "svg" => "image/svg+xml".into(),
        other => match veyra_net::guess_mime(&format!("x.{other}")) {
            Some(m) => format!("{m}; charset=utf-8"),
            None => "application/octet-stream".into(),
        },
    }
}

fn extract_css_urls(text: &str) -> Vec<String> {
    let normalized = text.replace("\\u002F", "/").replace("\\u002f", "/");
    let bytes = normalized.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;
    while out.len() < 12 {
        let Some(rel) = normalized[search..].find(".css") else { break };
        let end = search + rel + 4;
        // Walk backwards to the nearest quote or whitespace.
        let mut start = search + rel;
        while start > 0 {
            let b = bytes[start - 1];
            if b == b'\'' || b == b'"' || b == b'`' || b == b' ' || b == b'(' || b == b',' {
                break;
            }
            start -= 1;
        }
        if start < end {
            let candidate = &normalized[start..end];
            if (candidate.starts_with('/') || candidate.starts_with("http") || candidate.starts_with("./"))
                && !candidate.contains(' ')
                && !candidate.contains('\\')
            {
                out.push(candidate.to_string());
            }
        }
        search = end;
        if search >= normalized.len() {
            break;
        }
    }
    out
}

fn error_page(url: &Url, message: &str) -> String {
    format!(
        "<!doctype html><html><head><title>Cannot reach {}</title><style>body{{font-family:sans-serif;background:#14161a;color:#e6e8eb;padding:48px;}}h1{{font-size:28px;}}code{{background:#23262d;padding:4px 8px;border-radius:6px;}}pre{{background:#23262d;padding:20px;border-radius:10px;overflow:auto;}}</style></head><body><h1>This page could not be loaded</h1><p><code>{}</code></p><pre>{}</pre><p>Check the address, your network connection, or the server's TLS certificate.</p></body></html>",
        escape_html(&url.to_string()),
        escape_html(&url.to_string()),
        escape_html(message)
    )
}

