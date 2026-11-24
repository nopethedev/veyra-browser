//! Event objects and dispatch state shared by the UI, the DOM and scripts.

use crate::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    None,
    Capturing,
    AtTarget,
    Bubbling,
}

#[derive(Debug, Clone, Default)]
pub struct MouseEventData {
    pub client_x: f32,
    pub client_y: f32,
    pub page_x: f32,
    pub page_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub button: u8,
    pub buttons: u8,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Default)]
pub struct KeyEventData {
    pub key: String,
    pub code: String,
    pub key_code: u32,
    pub repeat: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub default_prevented: bool,
    pub propagation_stopped: bool,
    pub immediate_stopped: bool,
    pub target: Option<NodeId>,
    pub current_target: Option<NodeId>,
    pub phase: EventPhase,
    pub time_stamp: f64,
    pub mouse: Option<MouseEventData>,
    pub key: Option<KeyEventData>,
    pub data: Option<String>,
    pub trusted: bool,
    /// Set by JS to stop the browser default action of this specific event.
    pub prevent_default_called: bool,
}

impl Event {
    pub fn new(event_type: &str, bubbles: bool, cancelable: bool) -> Event {
        Event {
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            default_prevented: false,
            propagation_stopped: false,
            immediate_stopped: false,
            target: None,
            current_target: None,
            phase: EventPhase::None,
            time_stamp: 0.0,
            mouse: None,
            key: None,
            data: None,
            trusted: false,
            prevent_default_called: false,
        }
    }

    pub fn click(x: f32, y: f32) -> Event {
        let mut e = Event::new("click", true, true);
        e.mouse = Some(MouseEventData {
            client_x: x,
            client_y: y,
            page_x: x,
            page_y: y,
            offset_x: x,
            offset_y: y,
            button: 0,
            buttons: 0,
            ..Default::default()
        });
        e
    }

    pub fn key(key: &str, code: &str, key_code: u32) -> Event {
        let mut e = Event::new("keydown", true, true);
        e.key = Some(KeyEventData {
            key: key.to_string(),
            code: code.to_string(),
            key_code,
            ..Default::default()
        });
        e
    }

    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.default_prevented = true;
            self.prevent_default_called = true;
        }
    }

    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    pub fn stop_immediate_propagation(&mut self) {
        self.propagation_stopped = true;
        self.immediate_stopped = true;
    }
}
