//! Settings > Account (`Account.svelte`, `Passkeys.svelte`, `RecentCalls.svelte`): the name, colour and bio, the
//! password, the devices signed in, the passkeys and your recent calls, through the same `/api` routes as the web.
//! Not here: the profile picture, adding or renaming a passkey (WebAuthn is the browser's) and the welcome steps.

use std::collections::HashMap;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, Div, InteractiveElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vesper_protocol::{ApiErrorCode, CallMode, CallRecord, Device, PasskeyList, UserInfo};

use super::Vesper;
use super::c::{
    Avatar, Button, Ellipsis, Field, PasswordInput, SLOT_HEX, Variant, avatar_slot, text_input,
};
use super::settings::{fail_text, h3, section, sections, text};
use super::theme::{self, TEXT_BASE, TextStepExt};
use crate::net::Fail;

pub const MAX_BIO: usize = 160;

/// A list the page fetched.
#[derive(Debug, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Failed,
    Ready(T),
}

impl<T: DeserializeOwned> Load<T> {
    pub fn from(r: Result<Value, Fail>) -> Load<T> {
        match r.ok().and_then(|v| serde_json::from_value(v).ok()) {
            Some(v) => Load::Ready(v),
            None => Load::Failed,
        }
    }
}

#[derive(Default)]
pub struct State {
    filled: bool,
    pub accent_error: String,
    pub accent_prev: Option<String>,
    slot: usize,
    saving: bool,
    saved: bool,
    profile_error: String,
    changing: bool,
    pw_done: bool,
    /// Which field it is under (`old`, `new`, `confirm` or `form`) and the words.
    pw_error: Option<(&'static str, String)>,
    shown: HashMap<&'static str, bool>,
    devices: Load<Vec<Device>>,
    revoking: Option<i64>,
    revoke_error: String,
    passkeys: Load<PasskeyList>,
    passkey_error: String,
    calls: Load<Vec<CallRecord>>,
    now: i64,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `relTime` of messages.ts: "just now", "5 min ago", "3 h ago", "2 days ago", else the date.
pub fn rel_time(ms: i64, now: i64) -> String {
    let min = (now - ms).div_euclid(60_000);
    if min < 1 {
        return "just now".into();
    }
    if min < 60 {
        return format!("{min} min ago");
    }
    if min < 24 * 60 {
        return format!("{} h ago", min / 60);
    }
    let days = min / (24 * 60);
    if days < 7 {
        return format!("{days} {} ago", if days == 1 { "day" } else { "days" });
    }
    date_of(ms)
}

/// `toLocaleDateString` with day, short month and year, as en-US writes it: "Nov 2, 2026".
pub fn date_of(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or_else(String::new, |d| d.format("%b %-d, %Y").to_string())
}

/// `callLine` of account.ts: how long, the filter it ended on, and the figures (a dash when never measured).
pub fn call_line(c: &CallRecord) -> (String, String) {
    let s = &c.summary;
    let min = (s.duration_s as f64 / 60.).round() as u32;
    let length = if s.duration_s < 60 {
        format!("{} s", s.duration_s)
    } else if min < 60 {
        format!("{min} min")
    } else {
        format!("{} h {} min", min / 60, min % 60)
    };
    let mode = match s.mode {
        CallMode::Deepfilter => "DeepFilterNet",
        CallMode::Rnnoise => "RNNoise",
        CallMode::Browser => "browser filter",
        CallMode::Off => "no filter",
    };
    let why = if s.fallback_reason.is_empty() {
        String::new()
    } else {
        format!(" ({})", s.fallback_reason)
    };
    let loading = if s.browser_ns_loading {
        ", browser filter while it loaded"
    } else {
        ""
    };
    let fig = |v: Option<f64>, unit: &str, digits: usize| {
        v.map_or("–".to_string(), |v| format!("{v:.digits$} {unit}"))
    };
    (
        format!("{length}, {mode}{why}{loading}"),
        format!(
            "Loss {}, jitter {}, round trip {}, concealed {}",
            fig(s.loss_pct, "%", 1),
            fig(s.jitter_ms, "ms", 0),
            fig(s.rtt_ms, "ms", 0),
            fig(s.concealed_pct, "%", 1)
        ),
    )
}

/// `pwCheck`: the length in characters and that the two new passwords match.
pub fn pw_check(new: &str, confirm: &str) -> Option<(&'static str, String)> {
    let n = new.chars().count();
    if !(8..=128).contains(&n) {
        return Some(("new", "Use 8 to 128 characters.".into()));
    }
    (new != confirm).then(|| ("confirm", "The two new passwords are not the same.".into()))
}

/// A bordered list (`.devices`, `.keys`, `.calls`): rows split by `--line-subtle` rules.
fn list_box(rows: Vec<Div>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    v_flex()
        .border_1()
        .border_color(t.line)
        .rounded(t.radius)
        .children(
            rows.into_iter()
                .enumerate()
                .map(|(i, r)| r.when(i > 0, |d| d.border_t_1().border_color(t.line_subtle))),
        )
}

/// The `This device` pill.
fn this_device(cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex_none()
        .px(px(6.))
        .rounded(t.radius_sm)
        .bg(t.bg_selected)
        .text_xs()
        .text_color(t.fg_2)
        .child("This device")
}

impl Vesper {
    pub(super) fn open_account(&mut self, _: &mut Window, _: &mut Context<Self>) {
        let Some(me) = self.my_user().cloned() else {
            return;
        };
        let a = &mut self.page.account;
        a.slot = avatar_slot(&me.user_id, &me.accent);
        (a.saved, a.profile_error, a.pw_done, a.pw_error) = (false, String::new(), false, None);
        (a.devices, a.passkeys, a.calls) = (Load::Loading, Load::Loading, Load::Loading);
        a.now = now_ms();
        a.filled = false;
        self.http("account:devices", Method::GET, "/api/sessions", None);
        self.http("account:passkeys", Method::GET, "/api/passkeys", None);
        self.http("account:calls", Method::GET, "/api/me/calls", None);
    }

    pub(super) fn account_answer(
        &mut self,
        tag: &str,
        r: Result<Value, Fail>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let a = &mut self.page.account;
        a.now = now_ms();
        match tag {
            "devices" => {
                a.devices = Load::from(r);
                if let Load::Ready(d) = &mut a.devices {
                    d.sort_by_key(|d| !d.current); // this device first, then the server's order
                }
            }
            "passkeys" => a.passkeys = Load::from(r),
            "calls" => a.calls = Load::from(r),
            "profile" => {
                a.saving = false;
                match r.map(serde_json::from_value::<UserInfo>) {
                    Ok(Ok(u)) => {
                        a.saved = true;
                        self.model.state.users.insert(u.user_id.clone(), u);
                    }
                    Ok(Err(_)) => a.saved = true,
                    Err(f) => a.profile_error = fail_text(f),
                }
            }
            "password" => {
                a.changing = false;
                match r {
                    Ok(_) => {
                        a.pw_done = true;
                        self.http("account:devices", Method::GET, "/api/sessions", None);
                    }
                    // A wrong current password is a 403; the desktop's Origin is always right.
                    Err(Fail::Api {
                        code: ApiErrorCode::Forbidden,
                        ..
                    }) => a.pw_error = Some(("old", "Your current password is not right.".into())),
                    Err(f) => a.pw_error = Some(("form", fail_text(f))),
                }
            }
            t if t.starts_with("revoke:") => {
                let id: i64 = t["revoke:".len()..].parse().unwrap_or(0);
                a.revoking = None;
                match r {
                    Ok(_) => {
                        if let Load::Ready(d) = &mut a.devices {
                            d.retain(|d| d.id != id);
                        }
                    }
                    Err(_) => a.revoke_error = "Could not sign out that device. Try again.".into(),
                }
            }
            t if t.starts_with("passkey-remove") => match r {
                Ok(_) => self.http("account:passkeys", Method::GET, "/api/passkeys", None),
                Err(f) => a.passkey_error = fail_text(f),
            },
            _ => {}
        }
        let _ = (window, cx);
    }

    fn save_profile(&mut self, cx: &mut Context<Self>) {
        let Some(me) = self.my_user().cloned() else {
            return;
        };
        if self.page.account.saving {
            return;
        }
        let (name, bio) = (
            self.input_text("account-name", cx),
            self.input_text("account-bio", cx),
        );
        let slot = self.page.account.slot.clamp(1, 8);
        // `profilePatch`: only what changed.
        let mut patch = serde_json::Map::new();
        if name.trim() != me.display_name {
            patch.insert("display_name".into(), json!(name));
        }
        if SLOT_HEX[slot - 1] != me.accent.trim().to_lowercase() {
            patch.insert("accent".into(), json!(SLOT_HEX[slot - 1]));
        }
        if bio.split_whitespace().collect::<Vec<_>>().join(" ") != me.bio {
            patch.insert("bio".into(), json!(bio));
        }
        let a = &mut self.page.account;
        a.profile_error.clear();
        if patch.is_empty() {
            a.saved = true;
            return;
        }
        a.saving = true;
        self.http(
            "account:profile",
            Method::PATCH,
            "/api/me",
            Some(Value::Object(patch)),
        );
    }

    fn change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page.account.changing {
            return;
        }
        let (old, new, confirm) = (
            self.input_text("account-old", cx),
            self.input_text("account-new", cx),
            self.input_text("account-confirm", cx),
        );
        let a = &mut self.page.account;
        a.pw_done = false;
        a.pw_error = pw_check(&new, &confirm);
        if a.pw_error.is_none() {
            a.changing = true;
            let body = json!({ "old": old, "new": new });
            self.http(
                "account:password",
                Method::POST,
                "/api/me/password",
                Some(body),
            );
        }
        // Done or failed, no password stays in the form.
        for k in ["account-old", "account-new", "account-confirm"] {
            self.set_input(k, "", window, cx);
        }
    }

    fn password_field(
        &mut self,
        key: &'static str,
        label: &'static str,
        help: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Field {
        let input = self.page_input(key, true, window, cx);
        let shown = self.page.account.shown.get(key).copied().unwrap_or(false);
        let at = key.trim_start_matches("account-");
        let err = self
            .page
            .account
            .pw_error
            .as_ref()
            .filter(|e| e.0 == at)
            .map(|e| e.1.clone())
            .unwrap_or_default();
        let view = cx.entity().downgrade();
        Field::new(label)
            .help(help)
            .error(err)
            .child(PasswordInput::new(&input, shown, move |on, _, cx| {
                let _ = view.update(cx, |v, cx| {
                    v.page.account.shown.insert(key, on);
                    cx.notify();
                });
            }))
    }

    pub(super) fn render_account(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let Some(me) = self.my_user().cloned() else {
            return div();
        };
        if !self.page.account.filled {
            // The fields start from your profile once, then keep what you type.
            self.page.account.filled = true;
            self.set_input("account-name", me.display_name.clone(), window, cx);
            self.set_input("account-bio", me.bio.clone(), window, cx);
        }
        let name_input = self.page_input("account-name", false, window, cx);
        let bio_input = self.page_input("account-bio", false, window, cx);
        let name = self.input_text("account-name", cx);
        let bio_len = self.input_text("account-bio", cx).chars().count();
        let a = &self.page.account;
        let slot = a.slot.clamp(1, 8);
        let shown_name = if name.trim().is_empty() {
            me.username.clone()
        } else {
            name.clone()
        };
        let card = h_flex()
            .max_w(px(320.))
            .gap(px(12.))
            .p(px(12.))
            .rounded(t.radius)
            .bg(t.bg_raised)
            .child(Avatar::new(
                me.user_id.clone(),
                shown_name.clone(),
                SLOT_HEX[slot - 1],
                40,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_step(TEXT_BASE)
                            .font_weight(theme::tokens(cx).weight_strong)
                            .ellipsis()
                            .child(shown_name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.fg_2)
                            .ellipsis()
                            .child(format!("Signed in as {}", me.username)),
                    ),
            );
        let swatches = h_flex().gap(px(10.)).children((1..=8usize).map(|i| {
            let on = i == slot;
            div()
                .id(SharedString::from(format!("swatch-{i}")))
                .debug_selector(move || format!("swatch-{i}"))
                .size(px(28.))
                .flex_none()
                .rounded_full()
                .border_2()
                .border_color(if on {
                    t.fg
                } else {
                    gpui::transparent_black().into()
                })
                .p(px(2.))
                .child(div().size_full().rounded_full().bg(t.avatar[i - 1]))
                .on_click(cx.listener(move |this, _, _, cx| {
                    (this.page.account.slot, this.page.account.saved) = (i, false);
                    cx.notify();
                }))
        }));
        let profile = section().child(h3("Profile", cx)).child(card).child(
            v_flex()
                .max_w(px(320.))
                .gap(px(20.))
                .child(
                    Field::new("Display name")
                        .help("1 to 32 characters.")
                        .error(a.profile_error.clone())
                        .child(text_input(&name_input, cx)),
                )
                .child(
                    v_flex()
                        .child(div().mb(px(8.)).font_weight(t.weight_ui).child("Colour"))
                        .child(swatches),
                )
                .child(
                    Field::new("Bio")
                        .value(format!("{bio_len} / {MAX_BIO}"))
                        .help(format!(
                            "Up to {MAX_BIO} characters, shown on your person card."
                        ))
                        .child(text_input(&bio_input, cx)),
                )
                .child(
                    h_flex()
                        .gap(px(12.))
                        .child(
                            div().debug_selector(|| "account-save".into()).child(
                                Button::new("account-save", "Save")
                                    .loading(a.saving)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_profile(cx);
                                        cx.notify();
                                    })),
                            ),
                        )
                        .child(text(if a.saved { "Saved." } else { "" }, t.fg_2, false)),
                ),
        );

        let (pw_done, changing) = (a.pw_done, a.changing);
        let form_error = a
            .pw_error
            .as_ref()
            .filter(|e| e.0 == "form")
            .map(|e| e.1.clone());
        let old = self.password_field("account-old", "Current password", "", window, cx);
        let new = self.password_field(
            "account-new",
            "New password",
            "8 to 128 characters.",
            window,
            cx,
        );
        let confirm =
            self.password_field("account-confirm", "Confirm new password", "", window, cx);
        let password = section().child(h3("Password", cx)).child(
            v_flex()
                .gap(px(16.))
                .child(
                    v_flex()
                        .max_w(px(320.))
                        .gap(px(16.))
                        .child(old)
                        .child(new)
                        .child(confirm),
                )
                .child(
                    h_flex()
                        .gap(px(12.))
                        .child(
                            Button::new("account-password", "Change password")
                                .loading(changing)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.change_password(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(text(
                            if pw_done {
                                "Password changed. Your other devices were signed out."
                            } else {
                                ""
                            },
                            t.fg_2,
                            false,
                        )),
                )
                .children(form_error.map(|e| text(e, t.danger_fg, false))),
        );

        let a = &self.page.account;
        let now = a.now;
        let devices_body = match &a.devices {
            Load::Failed => text(
                "Could not load your devices. Check your connection and try again.",
                t.fg_2,
                false,
            ),
            Load::Loading => text("Loading devices...", t.fg_2, false),
            Load::Ready(d) if d.is_empty() => text("No devices are signed in.", t.fg_2, false),
            Load::Ready(list) => {
                let rows = list
                    .iter()
                    .map(|d| {
                        let (id, current) = (d.id, d.current);
                        h_flex()
                            .gap(px(16.))
                            .px(px(12.))
                            .py(px(10.))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        h_flex()
                                            .gap(px(8.))
                                            .font_weight(theme::tokens(cx).weight_strong)
                                            .child(
                                                div().min_w_0().ellipsis().child(d.label.clone()),
                                            )
                                            .when(current, |r| r.child(this_device(cx))),
                                    )
                                    .child(text(
                                        format!(
                                            "Last seen {}. Signed in {}.",
                                            rel_time(d.last_seen_at, now),
                                            date_of(d.created_at)
                                        ),
                                        t.fg_2,
                                        true,
                                    )),
                            )
                            .child(
                                Button::new(SharedString::from(format!("device-{id}")), "Sign out")
                                    .small()
                                    .loading(a.revoking == Some(id))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if current {
                                            return this.sign_out(window, cx);
                                        }
                                        if this.page.account.revoking.is_some() {
                                            return;
                                        }
                                        this.page.account.revoke_error.clear();
                                        this.page.account.revoking = Some(id);
                                        this.http(
                                            format!("account:revoke:{id}"),
                                            Method::DELETE,
                                            format!("/api/sessions/{id}"),
                                            None,
                                        );
                                        cx.notify();
                                    })),
                            )
                    })
                    .collect();
                v_flex()
                    .gap(px(16.))
                    .child(list_box(rows, cx))
                    .when(!a.revoke_error.is_empty(), |d| {
                        d.child(text(a.revoke_error.clone(), t.danger_fg, false))
                    })
            }
        };
        let devices = section()
            .child(
                v_flex().child(h3("Devices", cx)).child(
                    text(
                        "Browsers and apps signed in to this account. Signing one out ends its session there.",
                        t.fg_2,
                        false,
                    )
                    .mt(px(4.)),
                ),
            )
            .child(devices_body);

        let passkeys_body = match &a.passkeys {
            Load::Failed => text(
                "Your passkeys could not be loaded. Open this page again to try again.",
                t.fg_2,
                false,
            ),
            Load::Loading => text("Loading your passkeys.", t.fg_2, false),
            Load::Ready(list) => {
                let rows: Vec<Div> = list
                    .passkeys
                    .iter()
                    .map(|p| {
                        let id = p.id;
                        let used = p.last_used_at.map_or("Not used yet.".to_string(), |u| {
                            format!("Last used {}.", rel_time(u, now))
                        });
                        h_flex()
                            .gap(px(16.))
                            .px(px(12.))
                            .py(px(10.))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_weight(theme::tokens(cx).weight_strong)
                                            .ellipsis()
                                            .child(p.name.clone()),
                                    )
                                    .child(text(
                                        format!("Added {}. {used}", date_of(p.created_at)),
                                        t.fg_2,
                                        true,
                                    )),
                            )
                            .child(
                                Button::new(SharedString::from(format!("passkey-{id}")), "Remove")
                                    .small()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.page.account.passkey_error.clear();
                                        this.http(
                                            format!("account:passkey-remove:{id}"),
                                            Method::DELETE,
                                            format!("/api/passkeys/{id}"),
                                            None,
                                        );
                                        cx.notify();
                                    })),
                            )
                    })
                    .collect();
                let empty = rows.is_empty();
                v_flex()
                    .gap(px(16.))
                    .when(!empty, |d| d.child(list_box(rows, cx)))
                    .when(empty, |d| d.child(text("No passkeys yet.", t.fg_2, false)))
                    .child(text(
                        if list.enabled {
                            "To add a passkey, open Vesper in your browser: Settings, Account."
                        } else {
                            "Passkeys are off on this server: its address is an IP number, and passkeys need a domain name."
                        },
                        t.fg_2,
                        false,
                    ))
            }
        };
        let passkeys = section()
            .child(
                v_flex().child(h3("Passkeys", cx)).child(
                    text(
                        "Sign in with your fingerprint, face, screen lock or a security key instead of typing a password. \
                         Your password keeps working and is how you get back in if you lose a passkey.",
                        t.fg_2,
                        false,
                    )
                    .mt(px(4.)),
                ),
            )
            .child(passkeys_body)
            .when(!a.passkey_error.is_empty(), |d| {
                d.child(text(a.passkey_error.clone(), t.danger_fg, false))
            });

        let calls_body = match &a.calls {
            Load::Failed => text(
                "Your recent calls could not be loaded. Open this page again to try again.",
                t.fg_2,
                false,
            ),
            Load::Loading => text("Loading your recent calls.", t.fg_2, false),
            Load::Ready(c) if c.is_empty() => text(
                "No calls yet. A call shows here after you leave it.",
                t.fg_2,
                false,
            ),
            Load::Ready(calls) => list_box(
                calls
                    .iter()
                    .map(|c| {
                        let (head, figures) = call_line(c);
                        v_flex()
                            .px(px(12.))
                            .py(px(8.))
                            .child(text(
                                format!("{}: {head}", rel_time(c.ended_at, now)),
                                t.fg,
                                false,
                            ))
                            .child(text(figures, t.fg_2, true))
                    })
                    .collect(),
                cx,
            ),
        };
        let calls = section()
            .child(
                v_flex().child(h3("Your recent calls", cx)).child(
                    text(
                        "How your last calls went, as Vesper measured them. Only you can see this list, not the owner or \
                         anyone else. Vesper keeps your last 20 calls for 30 days.",
                        t.fg_2,
                        false,
                    )
                    .mt(px(4.)),
                ),
            )
            .child(calls_body);

        let _ = Variant::Ghost;
        sections(vec![profile, password, devices, passkeys, calls], cx)
    }
}
