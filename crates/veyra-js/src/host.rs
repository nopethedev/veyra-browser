//! Host services the interpreter needs from the browser shell.

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Services provided by the embedding browser.
pub trait Host {
    fn log(&mut self, level: &str, message: &str);
    fn fetch(
        &mut self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Result<FetchResponse, String>;
    fn now_ms(&mut self) -> f64;
    fn random(&mut self) -> f64;
}

/// A host for tests and headless evaluation.
#[derive(Default)]
pub struct NullHost {
    pub logs: Vec<String>,
    pub time: f64,
}

impl Host for NullHost {
    fn log(&mut self, _level: &str, message: &str) {
        self.logs.push(message.to_string());
    }
    fn fetch(
        &mut self,
        _method: &str,
        _url: &str,
        _headers: &[(String, String)],
        _body: Option<&str>,
    ) -> Result<FetchResponse, String> {
        Err("fetch is unavailable without a browser host".into())
    }
    fn now_ms(&mut self) -> f64 {
        self.time
    }
    fn random(&mut self) -> f64 {
        0.5
    }
}
