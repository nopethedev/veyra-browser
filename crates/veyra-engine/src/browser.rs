//! Tabs, session history, bookmarks and the persistent browser profile.

use std::path::PathBuf;
use std::rc::Rc;

use veyra_common::{Point, Size, Url};
use veyra_net::Client;

use crate::page::{LoadOptions, Page};

pub struct Tab {
    pub page: Page,
    pub history: Vec<Url>,
    pub index: usize,
    pub scroll_positions: Vec<Point>,
    pub pinned: bool,
}

impl Tab {
    pub fn title(&self) -> String {
        self.page.title()
    }

    pub fn url(&self) -> String {
        self.page.url.to_string()
    }

    pub fn can_go_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }
}

pub struct Bookmark {
    pub title: String,
    pub url: Url,
}

pub struct HistoryEntry {
    pub title: String,
    pub url: Url,
    pub visits: u64,
}

pub struct Browser {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub bookmarks: Vec<Bookmark>,
    pub history: Vec<HistoryEntry>,
    pub viewport: Size,
    pub client: Rc<Client>,
    pub profile: Option<PathBuf>,
    pub downloads: Vec<String>,
}

impl Browser {
    pub fn new(viewport: Size) -> Browser {
        let client = Rc::new(Client::new().unwrap_or_else(|_| Client::default()));
        let mut browser = Browser {
            tabs: Vec::new(),
            active: 0,
            bookmarks: Vec::new(),
            history: Vec::new(),
            viewport,
            client,
            profile: default_profile(),
            downloads: Vec::new(),
        };
        browser.load_profile();
        browser.new_tab("veyra://newtab");
        browser
    }

    pub fn with_home(viewport: Size, home: &str) -> Browser {
        let mut b = Browser::new(viewport);
        b.navigate(home);
        b
    }

    fn options(&self) -> LoadOptions {
        LoadOptions {
            viewport: self.viewport,
            scripts: true,
            max_script_bytes: 64_000_000,
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn active_page_mut(&mut self) -> Option<&mut Page> {
        self.active_tab_mut().map(|t| &mut t.page)
    }

    pub fn open(&mut self, url: &str) -> &mut Tab {
        let page = Page::load_with_client(self.client.clone(), url, self.options());
        self.record_visit(&page);
        let tab = Tab {
            history: vec![page.url.clone()],
            page,
            index: 0,
            scroll_positions: vec![Point::ZERO],
            pinned: false,
        };
        if let Some(existing) = self.tabs.get_mut(self.active) {
            if existing.url() == "veyra://newtab" && existing.history.len() == 1 {
                *existing = tab;
                return &mut self.tabs[self.active];
            }
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        &mut self.tabs[self.active]
    }

    pub fn new_tab(&mut self, url: &str) {
        let page = Page::load_with_client(self.client.clone(), url, self.options());
        self.record_visit(&page);
        let tab = Tab {
            history: vec![page.url.clone()],
            page,
            index: 0,
            scroll_positions: vec![Point::ZERO],
            pinned: false,
        };
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.new_tab("veyra://newtab");
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
    }

    pub fn activate(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    pub fn navigate(&mut self, url: &str) {
        let page = Page::load_with_client(self.client.clone(), url, self.options());
        self.record_visit(&page);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.history.truncate(tab.index + 1);
            tab.history.push(page.url.clone());
            tab.index = tab.history.len() - 1;
            tab.scroll_positions.push(Point::ZERO);
            tab.page = page;
        } else {
            self.new_tab(url);
        }
        self.save_profile();
    }

    pub fn back(&mut self) {
        let url = {
            let Some(tab) = self.tabs.get(self.active) else { return };
            if tab.index == 0 {
                return;
            }
            tab.history[tab.index - 1].to_string()
        };
        let page = Page::load_with_client(self.client.clone(), &url, self.options());
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.index -= 1;
            tab.page = page;
        }
    }

    pub fn forward(&mut self) {
        let url = {
            let Some(tab) = self.tabs.get(self.active) else { return };
            if tab.index + 1 >= tab.history.len() {
                return;
            }
            tab.history[tab.index + 1].to_string()
        };
        let page = Page::load_with_client(self.client.clone(), &url, self.options());
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.index += 1;
            tab.page = page;
        }
    }

    pub fn reload(&mut self) {
        let url = self.active_tab().map(|t| t.page.url.to_string());
        if let Some(url) = url {
            let page = Page::load_with_client(self.client.clone(), &url, self.options());
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.page = page;
            }
        }
    }

    pub fn set_viewport(&mut self, viewport: Size) {
        self.viewport = viewport;
        for tab in &mut self.tabs {
            tab.page.resize(viewport);
        }
    }

    pub fn bookmark_current(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let url = tab.page.url.clone();
        if self.bookmarks.iter().any(|b| b.url == url) {
            self.bookmarks.retain(|b| b.url != url);
        } else {
            self.bookmarks.push(Bookmark { title: tab.page.title(), url });
        }
        self.save_profile();
    }

    pub fn is_bookmarked(&self) -> bool {
        self.tabs
            .get(self.active)
            .map(|t| self.bookmarks.iter().any(|b| b.url == t.page.url))
            .unwrap_or(false)
    }

    pub fn open_bookmark(&mut self, index: usize) {
        if let Some(url) = self.bookmarks.get(index).map(|b| b.url.to_string()) {
            self.navigate(&url);
        }
    }

    fn record_visit(&mut self, page: &Page) {
        let url = page.url.clone();
        if url.scheme == "veyra" {
            return;
        }
        let title = page.title();
        if let Some(last) = self.history.last_mut() {
            if last.url == url {
                last.visits += 1;
                last.title = title;
                self.save_profile();
                return;
            }
        }
        self.history.push(HistoryEntry { title, url, visits: 1 });
        if self.history.len() > 500 {
            self.history.remove(0);
        }
        self.save_profile();
    }

    pub fn history_for_url<'a>(&'a self, url: &str) -> Option<&'a HistoryEntry> {
        self.history.iter().find(|h| h.url.to_string() == url)
    }

    pub fn load_profile(&mut self) {
        let Some(path) = self.profile.clone() else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        for line in text.lines() {
            let mut parts = line.split('\t');
            match parts.next() {
                Some("B") => {
                    let title = unescape(parts.next().unwrap_or(""));
                    let url = parts.next().unwrap_or("");
                    if let Some(u) = Url::parse(url) {
                        self.bookmarks.push(Bookmark { title, url: u });
                    }
                }
                Some("H") => {
                    let title = unescape(parts.next().unwrap_or(""));
                    let url = parts.next().unwrap_or("");
                    let visits = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1);
                    if let Some(u) = Url::parse(url) {
                        self.history.push(HistoryEntry { title, url: u, visits });
                    }
                }
                _ => {}
            }
        }
    }

    pub fn save_profile(&self) {
        let Some(path) = self.profile.clone() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut out = String::new();
        for b in &self.bookmarks {
            out.push_str(&format!("B\t{}\t{}\n", escape(&b.title), b.url));
        }
        for h in &self.history {
            out.push_str(&format!("H\t{}\t{}\t{}\n", escape(&h.title), h.url, h.visits));
        }
        let _ = std::fs::write(path, out);
    }

    pub fn tab_summaries(&self) -> Vec<(String, String, bool)> {
        self.tabs
            .iter()
            .map(|t| (t.title(), t.url(), t.pinned))
            .collect()
    }
}

fn default_profile() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".veyra"))
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".veyra")))
        .map(|d| d.join("profile.tsv"))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', " ")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
