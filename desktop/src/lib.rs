//! The Vesper desktop client. `net` talks to the server on its own thread, `model` holds what the app shows,
//! `ui` draws it with GPUI.

pub mod model;
pub mod net;
pub mod state;
pub mod store;
