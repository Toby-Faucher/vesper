//! The Vesper desktop client. `net` talks to the server on its own thread, `model` holds what the app shows,
//! `ui` draws it with GPUI, `voice` runs a call.

pub mod files;
pub mod highlight;
pub mod link;
pub mod logfile;
pub mod markdown;
pub mod model;
pub mod net;
pub mod notify;
pub mod searchq;
pub mod settings;
pub mod single;
pub mod state;
pub mod store;
pub mod switcher;
pub mod timeline;
pub mod ui;
pub mod update;
pub mod uploads;
pub mod voice;
