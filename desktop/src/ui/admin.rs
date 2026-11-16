//! The admin pages (`web/src/routes/admin/`): Invites and Members for moderators, Rooms, Disk use and Server for the
//! owner too, through the same `/api` routes. The server enforces the same rules; this only keeps a member from pages
//! that would answer 403. A new invite or reset link is shown once and lives in this state only: never stored, never
//! logged (`Json`'s `Debug` hides it), gone with Done or when the session ends.
//! Not here: the person card from a member's name, Alt+Up and Alt+Down on a room row, the member search box.

use std::collections::HashMap;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use reqwest::Method;
use serde_json::{Value, json};
use vesper_protocol::{
    ApiErrorCode, DiskUse, InviteCreated, InviteInfo, ResetCreated, ResetInfo, Role, RoomInfo,
    ServerStatus, UserInfo,
};

use super::Vesper;
use super::account::{Load, now_ms, rel_time};
use super::c::{
    self, Avatar, Button, Dialog, Ellipsis, Field, IconButton, PasswordInput, StatusKind, Switch,
    Variant, text_input,
};
use super::settings::{At, fail_text, h3, on_bool, section, sections, text};
use super::theme;
use crate::net::Fail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Invites,
    Members,
    Rooms,
    Disk,
    Server,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::Invites => "Invites",
            Section::Members => "Members",
            Section::Rooms => "Rooms",
            Section::Disk => "Disk use",
            Section::Server => "Server",
        }
    }
    /// The web's section id: the name's first word, lower case (`disk`).
    pub fn id(self) -> String {
        let n = self.name();
        n.split(' ').next().unwrap_or(n).to_lowercase()
    }
    pub fn from_id(id: &str) -> Option<Section> {
        [
            Section::Invites,
            Section::Members,
            Section::Rooms,
            Section::Disk,
            Section::Server,
        ]
        .into_iter()
        .find(|s| s.name().to_lowercase().starts_with(id))
    }
}

/// The admin sections a role opens (`ADMIN_SECTIONS` less `ADMIN_OWNER_ONLY` for a moderator); none for a member.
pub fn sections_for(role: Role) -> &'static [Section] {
    match role {
        Role::Owner => &[
            Section::Invites,
            Section::Members,
            Section::Rooms,
            Section::Disk,
            Section::Server,
        ],
        Role::Moderator => &[Section::Invites, Section::Members],
        Role::Member => &[],
    }
}

/// `admin.ts` inviteStatus: revoked, then used up, then expired.
pub fn invite_status(i: &InviteInfo, now: i64) -> &'static str {
    if i.revoked {
        "Revoked"
    } else if i.uses >= i.max_uses {
        "Used up"
    } else if i.expires_at <= now {
        "Expired"
    } else {
        "Active"
    }
}

/// `canRevoke`: an active invite; a moderator only member invites.
pub fn can_revoke(i: &InviteInfo, viewer: Role, now: i64) -> bool {
    invite_status(i, now) == "Active" && (viewer == Role::Owner || i.role == Role::Member)
}

/// `canReset`: never yourself; the owner for anyone else, a moderator for members.
pub fn can_reset(me: &UserInfo, them: &UserInfo) -> bool {
    me.user_id != them.user_id
        && (me.role == Role::Owner || (me.role == Role::Moderator && them.role == Role::Member))
}

/// `untilText`: "in 7 days", "in 3 h", "in 20 min", "in less than a minute"; past times as relTime.
pub fn until_text(ms: i64, now: i64) -> String {
    if ms <= now {
        return rel_time(ms, now);
    }
    let min = (ms - now) / 60_000;
    if min < 1 {
        "in less than a minute".into()
    } else if min < 60 {
        format!("in {min} min")
    } else if min < 24 * 60 {
        format!("in {} h", min / 60)
    } else {
        let d = min / (24 * 60);
        format!("in {d} {}", if d == 1 { "day" } else { "days" })
    }
}

/// `uptimeText`: two units at most.
pub fn uptime_text(s: i64) -> String {
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min")
    } else {
        "under a minute".into()
    }
}

/// `bytes`: 1024 steps, one decimal under 10.
pub fn bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut n, mut u) = (n as f64, 0);
    while n >= 1024. && u < UNITS.len() - 1 {
        (n, u) = (n / 1024., u + 1);
    }
    if u > 0 && n < 10. {
        let r = (n * 10.).round() / 10.;
        format!("{r} {}", UNITS[u])
    } else {
        format!("{} {}", n.round(), UNITS[u])
    }
}

fn role_name(r: Role) -> &'static str {
    match r {
        Role::Member => "Member",
        Role::Moderator => "Moderator",
        Role::Owner => "Owner",
    }
}

fn rank(r: Role) -> u8 {
    match r {
        Role::Owner => 0,
        Role::Moderator => 1,
        Role::Member => 2,
    }
}

const EXPIRES: [(u32, &str); 4] = [
    (1, "1 hour"),
    (24, "1 day"),
    (168, "7 days"),
    (720, "30 days"),
];

/// What a dialog's confirm button sends.
type Confirm = Box<dyn Fn(&mut Vesper, &mut Window, &mut Context<Vesper>)>;

/// A dialog of the admin pages.
#[derive(Clone)]
pub enum Ask {
    Role(UserInfo, Role),
    Ban(UserInfo),
    Reset(UserInfo),
    Revoke(InviteInfo),
    Archive(RoomInfo),
    Members(RoomInfo, Vec<String>),
}

#[derive(Default)]
pub struct State {
    now: i64,
    invites: Load<Vec<InviteInfo>>,
    invite_role: usize,
    invite_hours: usize,
    creating: bool,
    create_error: String,
    /// The new link, shown once: (url, uses, expires_at).
    created: Option<(String, u32, i64)>,
    copied: &'static str,
    members: Load<()>,
    member_error: String,
    busy: String,
    resets: HashMap<String, Vec<ResetInfo>>,
    /// A reset link made: (url, expires_at, name).
    made: Option<(String, i64, String)>,
    room_error: String,
    created_room: String,
    list_error: String,
    editing: Option<(i64, String)>,
    disk: Load<DiskUse>,
    server: Load<ServerStatus>,
    loading: bool,
    pub ask: Option<Ask>,
    ask_error: String,
    ask_busy: bool,
    /// Invites was opened once: its Expires starts at 7 days.
    shown: bool,
    shown_pw: bool,
}

/// One table cell: `w` px wide, or the rest.
fn cell(w: Option<f32>, first: bool, child: impl IntoElement) -> Div {
    let d = div()
        .flex()
        .items_center()
        .min_w_0()
        .pr(px(8.))
        .pl(px(if first { 12. } else { 8. }))
        .child(child);
    match w {
        Some(w) => d.w(px(w)).flex_none(),
        None => d.flex_1(),
    }
}

/// `.tbl` of app.css: a 32 px head row over a `--line` rule, in `--fg-2`, medium.
fn head(cols: &[(Option<f32>, &'static str)], cx: &App) -> Div {
    let t = theme::tokens(cx);
    h_flex()
        .h(px(32.))
        .border_b_1()
        .border_color(t.line)
        .text_color(t.fg_2)
        .font_weight(t.weight_ui)
        .children(
            cols.iter()
                .enumerate()
                .map(|(i, (w, s))| cell(*w, i == 0, *s)),
        )
}

/// A body row: at least 48 px, 4 px top and bottom, over a `--line-subtle` rule.
fn row(cells: Vec<(Option<f32>, AnyElement)>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    h_flex()
        .min_h(px(48.))
        .py(px(4.))
        .border_b_1()
        .border_color(t.line_subtle)
        .children(
            cells
                .into_iter()
                .enumerate()
                .map(|(i, (w, c))| cell(w, i == 0, c)),
        )
}

impl Vesper {
    pub(super) fn open_admin(&mut self, s: Section, _: &mut Window, _: &mut Context<Self>) {
        let role = self.my_role();
        let a = &mut self.page.admin;
        a.now = now_ms();
        a.ask = None;
        if !sections_for(role).contains(&s) {
            return;
        }
        match s {
            Section::Invites => {
                if a.invite_hours == 0 && a.invite_role == 0 && !a.shown {
                    a.invite_hours = 2; // 7 days
                }
                a.shown = true;
                self.http("admin:invites", Method::GET, "/api/invites", None);
                self.http("admin:users", Method::GET, "/api/users", None);
            }
            Section::Members => {
                self.http("admin:users", Method::GET, "/api/users", None);
                if role == Role::Owner {
                    self.http("admin:resets", Method::GET, "/api/resets", None);
                }
            }
            Section::Disk => {
                a.loading = true;
                self.http("admin:disk", Method::GET, "/api/admin/disk", None);
            }
            Section::Server => {
                a.loading = true;
                self.http("admin:server", Method::GET, "/api/admin/server", None);
            }
            Section::Rooms => {}
        }
    }

    /// Esc: an admin dialog closes first.
    pub(super) fn admin_escape(&mut self) -> bool {
        self.page.admin.ask.take().is_some()
    }

    pub(super) fn admin_answer(
        &mut self,
        tag: &str,
        r: Result<Value, Fail>,
        cx: &mut Context<Self>,
    ) {
        let a = &mut self.page.admin;
        a.now = now_ms();
        let (kind, arg) = tag.split_once(':').unwrap_or((tag, ""));
        match kind {
            "invites" => {
                let got: Load<Vec<InviteInfo>> = Load::from(r);
                // A failed reload keeps the table.
                if !matches!((&got, &a.invites), (Load::Failed, Load::Ready(_))) {
                    a.invites = got;
                }
            }
            "users" => match r.map(serde_json::from_value::<Vec<UserInfo>>) {
                Ok(Ok(list)) => {
                    a.members = Load::Ready(());
                    for u in list {
                        self.model.state.users.insert(u.user_id.clone(), u);
                    }
                }
                _ if matches!(a.members, Load::Ready(())) => {}
                _ => a.members = Load::Failed,
            },
            "resets" => {
                if let Ok(Ok(list)) = r.map(serde_json::from_value::<Vec<ResetInfo>>) {
                    a.resets.clear();
                    for x in list {
                        a.resets.entry(x.user_id.clone()).or_default().push(x);
                    }
                }
            }
            "create" => {
                a.creating = false;
                match r.map(serde_json::from_value::<InviteCreated>) {
                    Ok(Ok(c)) => {
                        a.created = Some((c.url.0, c.info.max_uses, c.info.expires_at));
                        a.copied = "";
                        if let Load::Ready(l) = &mut a.invites {
                            l.insert(0, c.info);
                        }
                    }
                    Ok(Err(_)) => {
                        a.create_error = "The server sent an answer this app cannot read.".into()
                    }
                    Err(f) => a.create_error = fail_text(f),
                }
            }
            "revoke" => {
                a.ask_busy = false;
                match r {
                    Ok(_) => {
                        let id: i64 = arg.parse().unwrap_or(0);
                        if let Load::Ready(l) = &mut a.invites {
                            l.iter_mut()
                                .filter(|i| i.id == id)
                                .for_each(|i| i.revoked = true);
                        }
                        a.ask = None;
                    }
                    Err(f) => a.ask_error = fail_text(f),
                }
            }
            "user" => {
                a.busy.clear();
                a.ask_busy = false;
                match r.map(serde_json::from_value::<UserInfo>) {
                    Ok(Ok(u)) => {
                        self.model.state.users.insert(u.user_id.clone(), u);
                        self.page.admin.ask = None;
                    }
                    Ok(Err(_)) => self.page.admin.ask = None,
                    Err(f) if self.page.admin.ask.is_some() => {
                        self.page.admin.ask_error = fail_text(f)
                    }
                    Err(f) => self.page.admin.member_error = fail_text(f),
                }
            }
            "reset" => {
                a.ask_busy = false;
                match r.map(serde_json::from_value::<ResetCreated>) {
                    Ok(Ok(made)) => {
                        a.made = Some((made.url.0, made.expires_at, arg.to_string()));
                        a.copied = "";
                        a.ask = None;
                        if self.my_role() == Role::Owner {
                            self.http("admin:resets", Method::GET, "/api/resets", None);
                        }
                    }
                    Ok(Err(_)) => a.ask = None,
                    Err(Fail::Api {
                        code: ApiErrorCode::Forbidden,
                        ..
                    }) => a.ask_error = "Your password is not right.".into(),
                    Err(f) => a.ask_error = fail_text(f),
                }
            }
            "room-new" => match r.map(serde_json::from_value::<RoomInfo>) {
                Ok(Ok(room)) => {
                    a.created_room = room.name;
                }
                Ok(Err(_)) => {}
                Err(f) => a.room_error = room_name_error(f),
            },
            "room" => {
                a.ask_busy = false;
                match r {
                    Ok(_) => {
                        a.editing = None;
                        a.ask = None;
                    }
                    Err(f) if arg == "rename" => {
                        if let Some(e) = &mut a.editing {
                            e.1 = room_name_error(f);
                        }
                    }
                    Err(Fail::Api { status: 409, .. }) if arg == "order" => {
                        a.list_error = "The room list changed. Try again.".into();
                    }
                    Err(f) if a.ask.is_some() => a.ask_error = fail_text(f),
                    Err(f) => a.list_error = fail_text(f),
                }
            }
            "disk" => {
                a.loading = false;
                a.disk = Load::from(r);
            }
            "server" => {
                a.loading = false;
                a.server = Load::from(r);
            }
            _ => {}
        }
        cx.notify();
    }

    fn copy_link(&mut self, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.page.admin.copied = "Link copied.";
        cx.notify();
    }

    /// The box with a new link, shown once (`.created`): the link, Copy link, the note, Done.
    fn link_box(
        &self,
        label: String,
        url: &str,
        note: String,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Vesper) + 'static,
    ) -> Div {
        let t = theme::tokens(cx).clone();
        let copy = url.to_string();
        v_flex()
            .debug_selector(|| "admin-link".into())
            .max_w(px(560.))
            .gap(px(12.))
            .p(px(16.))
            .border_1()
            .border_color(t.line)
            .rounded(t.radius)
            .bg(t.bg_raised)
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(div().font_weight(t.weight_ui).child(label))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(32.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .border_1()
                                    .border_color(t.line)
                                    .rounded(t.radius)
                                    .bg(t.bg_element)
                                    .font_family(t.font_mono.clone())
                                    .text_xs()
                                    .child(div().min_w_0().ellipsis().child(url.to_string())),
                            )
                            .child(Button::new("copy-link", "Copy link").on_click(
                                cx.listener(move |this, _, _, cx| this.copy_link(copy.clone(), cx)),
                            )),
                    ),
            )
            .child(text(note, t.fg_2, false))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .child(text(self.page.admin.copied, t.fg_2, false)),
                    )
                    .child(
                        div().debug_selector(|| "link-done".into()).child(
                            Button::new("link-done", "Done")
                                .variant(Variant::Ghost)
                                .small()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    done(this);
                                    cx.notify();
                                })),
                        ),
                    ),
            )
    }

    pub(super) fn render_admin(
        &mut self,
        s: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<&'static str>, Div) {
        let role = self.my_role();
        let t = theme::tokens(cx).clone();
        if role == Role::Member || !sections_for(role).contains(&s) {
            let words = if role == Role::Member {
                "You do not have access to admin pages."
            } else {
                "Only the owner can open this page."
            };
            return (
                None,
                v_flex()
                    .debug_selector(|| "admin-denied".into())
                    .items_center()
                    .gap(px(16.))
                    .pt(px(96.))
                    .child(div().text_color(t.fg).font_weight(t.weight_ui).child(words))
                    .child(
                        Button::new("admin-home", "Go to the rooms").on_click(
                            cx.listener(|this, _, window, cx| this.close_page(window, cx)),
                        ),
                    ),
            );
        }
        let body = match s {
            Section::Invites => self.render_invites(window, cx),
            Section::Members => self.render_members(cx),
            Section::Rooms => self.render_rooms(window, cx),
            Section::Disk => self.render_disk(cx),
            Section::Server => self.render_server(cx),
        };
        (Some(s.name()), body)
    }

    fn render_invites(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let owner = self.my_role() == Role::Owner;
        let uses = self.page_input("invite-uses", false, window, cx);
        if self.input_text("invite-uses", cx).is_empty()
            && !self.page.admin.creating
            && self.page.admin.created.is_none()
            && self.page.admin.create_error.is_empty()
        {
            self.set_input("invite-uses", "1", window, cx);
        }
        let a = &self.page.admin;
        let roles: Vec<SharedString> = if owner {
            vec!["Member".into(), "Moderator".into()]
        } else {
            vec!["Member".into()]
        };
        let role_select = self
            .select(
                "invite-role",
                roles,
                Some(a.invite_role),
                176.,
                cx,
                |this, i, _, _| this.page.admin.invite_role = i,
            )
            .disabled(!owner);
        let hours = self.select(
            "invite-expires",
            EXPIRES.iter().map(|e| e.1.into()).collect(),
            Some(a.invite_hours),
            176.,
            cx,
            |this, i, _, _| this.page.admin.invite_hours = i,
        );
        let a = &self.page.admin;
        let form = v_flex()
            .gap(px(16.))
            .child(
                h_flex()
                    .max_w(px(560.))
                    .gap(px(16.))
                    .items_start()
                    .child(
                        div().flex_1().min_w_0().child(
                            Field::new("Role")
                                .help(if owner {
                                    ""
                                } else {
                                    "Only the owner can invite moderators."
                                })
                                .child(role_select),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Field::new("Uses")
                                .help("1 to 50 people.")
                                .child(text_input(&uses, cx)),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Field::new("Expires").child(hours)),
                    ),
            )
            .child(
                h_flex().child(div().debug_selector(|| "invite-create".into()).child(
                    Button::new("invite-create", "Create invite")
                        .primary()
                        .loading(a.creating)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let a = &this.page.admin;
                            if a.creating {
                                return;
                            }
                            let owner = this.my_role() == Role::Owner;
                            let role = if owner && a.invite_role == 1 {
                                "moderator"
                            } else {
                                "member"
                            };
                            let uses: u32 = this
                                .input_text("invite-uses", cx)
                                .trim()
                                .parse()
                                .unwrap_or(0);
                            let hours = EXPIRES[a.invite_hours.min(3)].0;
                            let a = &mut this.page.admin;
                            a.create_error.clear();
                            if !(1..=50).contains(&uses) {
                                a.create_error = "Uses must be a whole number from 1 to 50.".into();
                                return cx.notify();
                            }
                            a.creating = true;
                            let body =
                                json!({ "role": role, "max_uses": uses, "expires_hours": hours });
                            this.http("admin:create", Method::POST, "/api/invites", Some(body));
                            cx.notify();
                        })),)
                ),
            )
            .when(!a.create_error.is_empty(), |d| {
                d.child(text(a.create_error.clone(), t.danger_fg, false))
            });
        let created = a.created.clone().map(|(url, n, until)| {
            let note = format!(
                "This link is shown only now. It works for {n} {} until {}.",
                if n == 1 { "person" } else { "people" },
                date_time(until)
            );
            self.link_box("Invite link".into(), &url, note, cx, |this| {
                this.page.admin.created = None
            })
        });
        let a = &self.page.admin;
        let now = a.now;
        let list = match &a.invites {
            Load::Failed => text(
                "Could not load the invites. Check your connection and try again.",
                t.fg_2,
                false,
            ),
            Load::Loading => text("Loading invites...", t.fg_2, false),
            Load::Ready(l) if l.is_empty() => text(
                "No invites yet. Create one to bring a friend in.",
                t.fg_2,
                false,
            ),
            Load::Ready(list) => {
                let cols = [
                    (Some(110.), "Role"),
                    (Some(90.), "Uses"),
                    (Some(130.), "Expires"),
                    (Some(170.), "Created by"),
                    (Some(90.), "Status"),
                    (None, ""),
                ];
                let me_role = self.my_role();
                let rows = list.iter().map(|i| {
                    let st = invite_status(i, now);
                    let who = match &i.created_by {
                        None => "Command line".to_string(),
                        Some(id) => self
                            .model
                            .state
                            .users
                            .get(id)
                            .map_or("-".into(), |u| u.display_name.clone()),
                    };
                    let inv = i.clone();
                    let revoke = can_revoke(i, me_role, now).then(|| {
                        Button::new(SharedString::from(format!("revoke-{}", i.id)), "Revoke")
                            .variant(Variant::Ghost)
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.page.admin.ask = Some(Ask::Revoke(inv.clone()));
                                this.page.admin.ask_error.clear();
                                cx.notify();
                            }))
                    });
                    row(
                        vec![
                            (Some(110.), role_name(i.role).into_any_element()),
                            (
                                Some(90.),
                                format!("{} of {}", i.uses, i.max_uses).into_any_element(),
                            ),
                            (Some(130.), until_text(i.expires_at, now).into_any_element()),
                            (
                                Some(170.),
                                div().min_w_0().ellipsis().child(who).into_any_element(),
                            ),
                            (
                                Some(90.),
                                div()
                                    .text_color(if st == "Active" { t.fg } else { t.fg_2 })
                                    .child(st)
                                    .into_any_element(),
                            ),
                            (
                                None,
                                h_flex()
                                    .flex_1()
                                    .justify_end()
                                    .children(revoke)
                                    .into_any_element(),
                            ),
                        ],
                        cx,
                    )
                });
                v_flex().child(head(&cols, cx)).children(rows)
            }
        };
        sections(
            vec![
                section()
                    .child(h3("New invite", cx))
                    .child(form)
                    .children(created),
                section().child(h3("All invites", cx)).child(list),
            ],
            cx,
        )
    }

    fn render_members(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let Some(me) = self.my_user().cloned() else {
            return div();
        };
        let owner = me.role == Role::Owner;
        let a = &self.page.admin;
        let body = match a.members {
            Load::Failed => h_flex()
                .gap(px(12.))
                .child(text(
                    "Could not load the members. Check your connection and try again.",
                    t.fg_2,
                    false,
                ))
                .child(
                    Button::new("members-retry", "Try again")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.http("admin:users", Method::GET, "/api/users", None);
                            cx.notify();
                        })),
                ),
            Load::Loading => text("Loading members...", t.fg_2, false),
            Load::Ready(()) => {
                let mut people: Vec<UserInfo> = self.model.state.users.values().cloned().collect();
                people.sort_by(|a, b| {
                    rank(a.role).cmp(&rank(b.role)).then_with(|| {
                        a.display_name
                            .to_lowercase()
                            .cmp(&b.display_name.to_lowercase())
                    })
                });
                let cols = [
                    (None, "Name"),
                    (Some(144.), "Role"),
                    (Some(80.), "Status"),
                    (Some(160.), ""),
                ];
                let mut rows = vec![];
                for u in people {
                    let editable = owner && u.user_id != me.user_id && u.role != Role::Owner;
                    let resets: Vec<Div> = self
                        .page
                        .admin
                        .resets
                        .get(&u.user_id)
                        .into_iter()
                        .flatten()
                        .map(|r| {
                            text(self.reset_line(r), t.fg_2, true)
                                .pl(px(32.))
                                .mt(px(2.))
                        })
                        .collect();
                    let name = v_flex()
                        .w_full()
                        .min_w_0()
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .min_w_0()
                                .child(Avatar::new(
                                    u.user_id.clone(),
                                    u.display_name.clone(),
                                    u.accent.clone(),
                                    20,
                                ))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(div().ellipsis().child(u.display_name.clone()))
                                        .child(
                                            div().text_xs().text_color(t.fg_2).ellipsis().child(
                                                format!(
                                                    "@{}{}",
                                                    u.username,
                                                    if u.user_id == me.user_id {
                                                        " (you)"
                                                    } else {
                                                        ""
                                                    }
                                                ),
                                            ),
                                        ),
                                ),
                        )
                        .children(resets);
                    let role_cell = if editable {
                        let who = u.clone();
                        let sel = self.select_dyn(
                            format!("role-{}", u.username).into(),
                            vec!["Member".into(), "Moderator".into()],
                            Some(usize::from(u.role == Role::Moderator)),
                            144.,
                            cx,
                            move |this, i, _, _| {
                                let to = if i == 1 {
                                    Role::Moderator
                                } else {
                                    Role::Member
                                };
                                if to != who.role {
                                    this.page.admin.ask = Some(Ask::Role(who.clone(), to));
                                    this.page.admin.ask_error.clear();
                                }
                            },
                        );
                        div().w_full().child(sel).into_any_element()
                    } else {
                        role_name(u.role).into_any_element()
                    };
                    let mut actions = h_flex().flex_1().justify_end().gap(px(8.));
                    if can_reset(&me, &u) {
                        let who = u.clone();
                        actions = actions.child(
                            Button::new(
                                SharedString::from(format!("reset-{}", u.username)),
                                "Reset link",
                            )
                            .variant(Variant::Ghost)
                            .small()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.page.admin.ask = Some(Ask::Reset(who.clone()));
                                    this.page.admin.ask_error.clear();
                                    cx.notify();
                                },
                            )),
                        );
                    }
                    if editable {
                        let who = u.clone();
                        actions = actions.child(if u.banned {
                            Button::new(
                                SharedString::from(format!("unban-{}", u.username)),
                                "Unban",
                            )
                            .small()
                            .loading(self.page.admin.busy == u.user_id)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if this.page.admin.busy.is_empty() {
                                        this.page.admin.busy = who.user_id.clone();
                                        this.page.admin.member_error.clear();
                                        this.http(
                                            "admin:user",
                                            Method::POST,
                                            format!("/api/users/{}/unban", who.user_id),
                                            None,
                                        );
                                    }
                                    cx.notify();
                                },
                            ))
                        } else {
                            Button::new(SharedString::from(format!("ban-{}", u.username)), "Ban")
                                .variant(Variant::Ghost)
                                .small()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.page.admin.ask = Some(Ask::Ban(who.clone()));
                                    this.page.admin.ask_error.clear();
                                    cx.notify();
                                }))
                        });
                    }
                    let sel = format!("member-{}", u.username);
                    rows.push(
                        row(
                            vec![
                                (None, name.into_any_element()),
                                (Some(144.), role_cell),
                                (
                                    Some(80.),
                                    div()
                                        .text_color(t.fg_2)
                                        .child(if u.banned { "Banned" } else { "" })
                                        .into_any_element(),
                                ),
                                (Some(160.), actions.into_any_element()),
                            ],
                            cx,
                        )
                        .debug_selector(move || sel),
                    );
                }
                let made = self.page.admin.made.clone().map(|(url, until, name)| {
                    let note = format!(
                        "This link is shown only now. It works once, until {}.",
                        date_time(until)
                    );
                    self.link_box(format!("Reset link for {name}"), &url, note, cx, |this| {
                        this.page.admin.made = None
                    })
                    .mt(px(16.))
                });
                let err = self.page.admin.member_error.clone();
                v_flex()
                    .child(head(&cols, cx))
                    .children(rows)
                    .children(made)
                    .when(!err.is_empty(), |d| {
                        d.child(text(err, t.danger_fg, false).mt(px(12.)))
                    })
            }
        };
        v_flex()
            .when(!owner, |d| {
                d.child(
                    text(
                        "Only the owner can change roles and ban people.",
                        t.fg_2,
                        false,
                    )
                    .mb(px(16.)),
                )
            })
            .child(body)
    }

    /// `resetLine`: "Password reset by Ann (@ann) on Nov 2 (used)".
    fn reset_line(&self, r: &ResetInfo) -> String {
        let by = match &r.created_by {
            None => "from the command line".to_string(),
            Some(id) => match self.model.state.users.get(id) {
                Some(u) => format!("by {} (@{})", u.display_name, u.username),
                None => "by someone".into(),
            },
        };
        let status = if r.used_at.is_some() {
            "used"
        } else if r.cancelled {
            "cancelled"
        } else if r.expires_at <= now_ms() {
            "expired"
        } else {
            "unused"
        };
        let day = {
            use chrono::TimeZone;
            chrono::Local
                .timestamp_millis_opt(r.used_at.unwrap_or(r.created_at))
                .single()
                .map_or_else(String::new, |d| d.format("%b %-d").to_string())
        };
        format!("Password reset {by} on {day} ({status})")
    }

    fn render_rooms(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let new_name = self.page_input("room-new", false, window, cx);
        let rename = self.page_input("room-rename", false, window, cx);
        let st = &self.model.state;
        let mut rooms: Vec<RoomInfo> = st.rooms.iter().chain(&st.archived).cloned().collect();
        rooms.sort_by_key(|r| r.position);
        let last_open = st.rooms.len() <= 1;
        let a = &self.page.admin;
        let create = section().child(h3("New room", cx)).child(
            v_flex()
                .max_w(px(480.))
                .gap(px(12.))
                .child(
                    Field::new("Room name").help("1 to 32 characters.").child(
                        h_flex()
                            .gap(px(8.))
                            // A flex column, so the input fills it (a plain div left it padding-wide).
                            .child(v_flex().flex_1().min_w_0().child(text_input(&new_name, cx)))
                            .child(
                                div().debug_selector(|| "room-create".into()).child(
                                    Button::new("room-create", "Create room")
                                        .primary()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let name = this.input_text("room-new", cx);
                                            let a = &mut this.page.admin;
                                            (a.room_error, a.created_room) =
                                                (String::new(), String::new());
                                            this.http(
                                                "admin:room-new",
                                                Method::POST,
                                                "/api/rooms",
                                                Some(json!({ "name": name })),
                                            );
                                            this.set_input("room-new", "", window, cx);
                                            cx.notify();
                                        })),
                                ),
                            ),
                    ),
                )
                // The web's status line is always there: empty, it still adds its gap less 4 px.
                .map(|d| {
                    if !a.room_error.is_empty() {
                        d.child(text(a.room_error.clone(), t.danger_fg, false).mt(px(-4.)))
                    } else if !a.created_room.is_empty() {
                        d.child(
                            text(format!("Created {}.", a.created_room), t.fg_2, false).mt(px(-4.)),
                        )
                    } else {
                        d.pb(px(8.))
                    }
                }),
        );
        let ids: Vec<i64> = rooms.iter().map(|r| r.id).collect();
        let n = rooms.len();
        let cols = [(None, "Name"), (Some(176.), "Restricted"), (Some(224.), "")];
        let mut rows = vec![];
        for (i, r) in rooms.into_iter().enumerate() {
            let id = r.id;
            let editing = a
                .editing
                .as_ref()
                .filter(|e| e.0 == id)
                .map(|e| e.1.clone());
            let dim = if r.archived { 0.55 } else { 1. };
            let name = match &editing {
                Some(err) => v_flex()
                    .w_full()
                    .child(text_input(&rename, cx))
                    .when(!err.is_empty(), |d| {
                        d.child(text(err.clone(), t.danger_fg, true).mt(px(4.)))
                    }),
                None => v_flex()
                    .min_w_0()
                    .opacity(dim)
                    .child(div().ellipsis().child(r.name.clone()))
                    .when(r.archived, |d| d.child(text("Archived", t.fg_2, true))),
            };
            let restricted = r.restricted;
            let members_btn = restricted.then(|| {
                let room = r.clone();
                Button::new(
                    SharedString::from(format!("room-members-{id}")),
                    format!("Members {}", r.members.len()),
                )
                .small()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page.admin.ask = Some(Ask::Members(room.clone(), room.members.clone()));
                    this.page.admin.ask_error.clear();
                    this.http("admin:users", Method::GET, "/api/users", None);
                    cx.notify();
                }))
            });
            let restrict = h_flex()
                .gap(px(12.))
                .opacity(dim)
                .child(
                    // `-ml-4` on the web.
                    div().ml(px(-16.)).w(px(40.)).child(
                        Switch::new(SharedString::from(format!("restrict-{id}")), "", restricted)
                            .on_change(on_bool(cx, move |this, on, _, cx| {
                                this.page.admin.list_error.clear();
                                this.http(
                                    "admin:room:restrict",
                                    Method::PATCH,
                                    format!("/api/rooms/{id}"),
                                    Some(json!({ "restricted": on })),
                                );
                                cx.notify();
                            })),
                    ),
                )
                .children(members_btn);
            let mut actions = h_flex().flex_1().justify_end().gap(px(4.));
            if editing.is_some() {
                actions = actions
                    .child(
                        Button::new("rename-cancel", "Cancel")
                            .variant(Variant::Ghost)
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page.admin.editing = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rename-save", "Save")
                            .primary()
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let name = this.input_text("room-rename", cx);
                                this.http(
                                    "admin:room:rename",
                                    Method::PATCH,
                                    format!("/api/rooms/{id}"),
                                    Some(json!({ "name": name })),
                                );
                                cx.notify();
                            })),
                    );
            } else {
                if r.archived {
                    actions = actions.child(
                        Button::new(SharedString::from(format!("unarchive-{id}")), "Unarchive")
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.http(
                                    "admin:room:unarchive",
                                    Method::PATCH,
                                    format!("/api/rooms/{id}"),
                                    Some(json!({ "archived": false })),
                                );
                                cx.notify();
                            })),
                    );
                } else {
                    let room = r.clone();
                    actions = actions.child(
                        Button::new(SharedString::from(format!("archive-{id}")), "Archive")
                            .variant(Variant::Ghost)
                            .small()
                            .disabled(last_open)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.page.admin.ask = Some(Ask::Archive(room.clone()));
                                this.page.admin.ask_error.clear();
                                cx.notify();
                            })),
                    );
                }
                let old = r.name.clone();
                actions = actions
                    .child(
                        Button::new(SharedString::from(format!("rename-{id}")), "Rename")
                            .variant(Variant::Ghost)
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.page.admin.editing = Some((id, String::new()));
                                this.set_input("room-rename", old.clone(), window, cx);
                                cx.notify();
                            })),
                    )
                    .children(
                        [(-1i64, "chevronUp"), (1, "chevronDown")].map(|(by, icon)| {
                            let ids = ids.clone();
                            let edge = (by < 0 && i == 0) || (by > 0 && i + 1 == n);
                            IconButton::new(SharedString::from(format!("move-{id}-{by}")), icon)
                                .disabled(edge)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut next = ids.clone();
                                    let j = (i as i64 + by) as usize;
                                    next.swap(i, j);
                                    this.page.admin.list_error.clear();
                                    this.http(
                                        "admin:room:order",
                                        Method::PUT,
                                        "/api/rooms/order",
                                        Some(json!({ "ids": next })),
                                    );
                                    cx.notify();
                                }))
                        }),
                    );
            }
            rows.push(row(
                vec![
                    (None, name.into_any_element()),
                    (Some(176.), restrict.into_any_element()),
                    (Some(224.), actions.into_any_element()),
                ],
                cx,
            ));
        }
        let a = &self.page.admin;
        let list = section()
            .child(h3("All rooms", cx))
            .child(v_flex().child(head(&cols, cx)).children(rows))
            .when(!a.list_error.is_empty(), |d| d.child(text(a.list_error.clone(), t.danger_fg, false)))
            .child(
                v_flex()
                    .gap(px(4.))
                    .child(text("Only listed people, moderators and the owner can see a restricted room.", t.fg_2, false))
                    .child(text(
                        "Rooms cannot be deleted. Archive one to hide it from everyone. The last room that is not \
                         archived cannot be archived.",
                        t.fg_2,
                        false,
                    )),
            );
        sections(vec![create, list], cx)
    }

    fn refresh_button(&self, tag: &'static str, path: &'static str, cx: &mut Context<Self>) -> Div {
        h_flex().child(
            Button::new("admin-refresh", "Refresh")
                .loading(self.page.admin.loading)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page.admin.loading = true;
                    this.http(tag, Method::GET, path, None);
                    cx.notify();
                })),
        )
    }

    fn render_disk(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let body = match &self.page.admin.disk {
            Load::Failed => text(
                "Could not load the disk use. Check your connection and try again.",
                t.fg_2,
                false,
            ),
            Load::Loading => text("Loading disk use...", t.fg_2, false),
            Load::Ready(d) => {
                let used = d.fs_total - d.fs_free;
                let rows = [
                    (
                        "Uploads",
                        d.upload_bytes,
                        d.quota_bytes,
                        format!("{} of {}", bytes(d.upload_bytes), bytes(d.quota_bytes)),
                    ),
                    ("Database", d.db_bytes, 0, bytes(d.db_bytes)),
                    (
                        "Disk",
                        used,
                        d.fs_total,
                        format!("{} of {}", bytes(used), bytes(d.fs_total)),
                    ),
                ];
                v_flex()
                    .gap(px(24.))
                    .when(d.warn, |v| {
                        v.child(c::status(
                            StatusKind::Warn,
                            "Uploads or the disk are over 80 % full. Delete old files or raise the quota \
                             (VESPER_UPLOAD_QUOTA_MB).",
                            cx,
                        ))
                    })
                    .child(v_flex().gap(px(20.)).children(rows.into_iter().map(|(label, used, of, words)| {
                        let p = if of > 0 { ((used as f64 / of as f64) * 100.).round().min(100.) as u32 } else { 0 };
                        v_flex()
                            .gap(px(8.))
                            .child(
                                h_flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(div().font_weight(theme::tokens(cx).weight_strong).child(label))
                                    .child(text(
                                        if of > 0 { format!("{words}, {p} %") } else { words },
                                        t.fg_2,
                                        false,
                                    )),
                            )
                            .when(of > 0, |v| {
                                v.child(
                                    div()
                                        .debug_selector(move || format!("disk-track-{label}"))
                                        // A flex row: in GPUI's default block layout the bar's relative width came out 0.
                                        .flex()
                                        .h(px(8.))
                                        .rounded(px(4.))
                                        .bg(t.bg_selected)
                                        .border_1()
                                        .border_color(t.line_strong)
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .debug_selector(move || format!("disk-bar-{label}"))
                                                .h_full()
                                                .min_w(px(2.))
                                                .w(gpui::relative(p as f32 / 100.))
                                                .rounded(px(4.))
                                                .bg(if p >= 80 { t.warn_fg } else { t.fg_2 }),
                                        ),
                                )
                            })
                    })))
            }
        };
        v_flex()
            .max_w(px(560.))
            .gap(px(24.))
            .child(body)
            .child(self.refresh_button("admin:disk", "/api/admin/disk", cx))
    }

    fn render_server(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let body = match &self.page.admin.server {
            Load::Failed => text(
                "Could not load the server status. Check your connection and try again.",
                t.fg_2,
                false,
            ),
            Load::Loading => text("Loading the server status...", t.fg_2, false),
            Load::Ready(s) => {
                let backup = match &s.last_backup {
                    Some(b) => format!("{}, {}", date_time(b.at), b.name),
                    None => "None in the backups folder".into(),
                };
                let rows = [
                    (
                        "Version",
                        format!("{}, build {}", s.server.version, s.server.build),
                    ),
                    ("Running for", uptime_text(s.uptime_s)),
                    ("Database", bytes(s.disk.db_bytes)),
                    (
                        "Disk free",
                        format!("{} of {}", bytes(s.disk.fs_free), bytes(s.disk.fs_total)),
                    ),
                    ("In voice now", s.voice.to_string()),
                    ("Failed pushes, last 24 h", s.push_failures.to_string()),
                    ("Last backup", backup),
                ];
                let key_w = 168.;
                v_flex()
                    .gap(px(24.))
                    .child(
                        v_flex()
                            .gap(px(12.))
                            .children(rows.into_iter().map(|(k, v)| {
                                h_flex()
                                    .items_start()
                                    .gap(px(24.))
                                    .child(
                                        div().w(px(key_w)).flex_none().text_color(t.fg_2).child(k),
                                    )
                                    .child(div().flex_1().min_w_0().child(v))
                            })),
                    )
                    .child(
                        h_flex()
                            .child(text("Uploads and the quota: ", t.fg_2, false))
                            .child(
                                div()
                                    .id("server-disk")
                                    .text_color(t.fg)
                                    .underline()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_page(At::Admin(Section::Disk), window, cx)
                                    }))
                                    .child("Disk use"),
                            )
                            .child(text(".", t.fg_2, false)),
                    )
            }
        };
        v_flex()
            .max_w(px(560.))
            .gap(px(24.))
            .child(body)
            .child(self.refresh_button("admin:server", "/api/admin/server", cx))
    }

    pub(super) fn admin_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Dialog> {
        let ask = self.page.admin.ask.clone()?;
        if !matches!(self.page.at, At::Admin(_)) {
            return None;
        }
        let t = theme::tokens(cx).clone();
        let busy = self.page.admin.ask_busy;
        let err = self.page.admin.ask_error.clone();
        let cancel = Button::new("ask-cancel", "Cancel")
            .variant(Variant::Ghost)
            .on_click(cx.listener(|this, _, _, cx| {
                this.page.admin.ask = None;
                cx.notify();
            }));
        let go = |label: &'static str, v: Variant, cx: &mut Context<Self>, f: Confirm| {
            div().debug_selector(|| "ask-confirm".into()).child(
                Button::new("ask-confirm", label)
                    .variant(v)
                    .loading(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.page.admin.ask_busy {
                            return;
                        }
                        this.page.admin.ask_busy = true;
                        this.page.admin.ask_error.clear();
                        f(this, window, cx);
                        cx.notify();
                    })),
            )
        };
        let close = cx.listener(|this, _, _, cx| {
            this.page.admin.ask = None;
            cx.notify();
        });
        let error = (!err.is_empty()).then(|| text(err, t.danger_fg, false).mt(px(12.)));
        let d = match ask {
            Ask::Role(u, role) => {
                let id = u.user_id.clone();
                let r = if role == Role::Moderator {
                    "moderator"
                } else {
                    "member"
                };
                Dialog::new(format!("Make {} a {}?", u.display_name, r))
                    .child(if role == Role::Moderator {
                        "Moderators can create member invites, see every room, and mute or remove members in voice."
                    } else {
                        "They lose the moderator tools and the restricted rooms they were not listed in."
                    })
                    .footer(h_flex().gap_2().child(cancel).child(go(
                        "Change role",
                        Variant::Primary,
                        cx,
                        Box::new(move |this, _, _| {
                            this.http("admin:user", Method::PATCH, format!("/api/users/{id}/role"), Some(json!({ "role": r })))
                        }),
                    )))
            }
            Ask::Ban(u) => {
                let id = u.user_id.clone();
                Dialog::new(format!("Ban {}?", u.display_name))
                    .child("They are signed out everywhere and cannot sign in.")
                    .footer(h_flex().gap_2().child(cancel).child(go(
                        "Ban",
                        Variant::Danger,
                        cx,
                        Box::new(move |this, _, _| {
                            this.http(
                                "admin:user",
                                Method::POST,
                                format!("/api/users/{id}/ban"),
                                None,
                            )
                        }),
                    )))
            }
            Ask::Reset(u) => {
                let pw = self.page_input("reset-password", true, window, cx);
                let shown = self.page.admin.shown_pw();
                let view = cx.entity().downgrade();
                let (id, name) = (u.user_id.clone(), u.display_name.clone());
                Dialog::new(format!("Create a reset link for {}?", u.display_name))
                    .child(
                        "You get a link to send them. It works once, for 24 hours. When they use it, they set a new \
                         password and are signed out everywhere, and they are told who made the link. An older reset \
                         link for them stops working.",
                    )
                    .child(
                        div().mt(px(16.)).child(
                            Field::new("Your password").help("To check it is you.").child(PasswordInput::new(
                                &pw,
                                shown,
                                move |on, _, cx| {
                                    let _ = view.update(cx, |v, cx| {
                                        v.page.admin.set_shown_pw(on);
                                        cx.notify();
                                    });
                                },
                            )),
                        ),
                    )
                    .footer(h_flex().gap_2().child(cancel).child(go(
                        "Create link",
                        Variant::Primary,
                        cx,
                        Box::new(move |this, window, cx| {
                            let password = this.input_text("reset-password", cx);
                            this.set_input("reset-password", "", window, cx);
                            this.http(
                                format!("admin:reset:{name}"),
                                Method::POST,
                                format!("/api/users/{id}/reset"),
                                Some(json!({ "password": password })),
                            );
                        }),
                    )))
            }
            Ask::Revoke(i) => {
                let id = i.id;
                Dialog::new("Revoke this invite?")
                    .child(format!(
                        "This {} link ({} of {} used) stops working at once. People who already joined with it keep \
                         their accounts.",
                        role_name(i.role).to_lowercase(),
                        i.uses,
                        i.max_uses
                    ))
                    .footer(h_flex().gap_2().child(cancel).child(go(
                        "Revoke",
                        Variant::Danger,
                        cx,
                        Box::new(move |this, _, _| {
                            this.http(format!("admin:revoke:{id}"), Method::DELETE, format!("/api/invites/{id}"), None)
                        }),
                    )))
            }
            Ask::Archive(r) => {
                let id = r.id;
                Dialog::new(format!("Archive {}?", r.name))
                    .child(text(
                        "It leaves the room list, the switcher and search for everyone. Only this page still lists it.",
                        t.fg_2,
                        false,
                    ))
                    .child(text(
                        "Its messages and files are kept. Unarchive brings the room back with all of them.",
                        t.fg_2,
                        false,
                    ).mt(px(8.)))
                    .child(text(
                        "Everyone in its voice call, you included, is taken out of it. Nobody can join it or post in \
                         it while it is archived.",
                        t.fg_2,
                        false,
                    ).mt(px(8.)))
                    .footer(h_flex().gap_2().child(cancel).child(go(
                        "Archive",
                        Variant::Danger,
                        cx,
                        Box::new(move |this, _, _| {
                            this.http(
                                "admin:room:archive",
                                Method::PATCH,
                                format!("/api/rooms/{id}"),
                                Some(json!({ "archived": true })),
                            )
                        }),
                    )))
            }
            Ask::Members(r, picked) => {
                let id = r.id;
                let mut people: Vec<UserInfo> = self
                    .model
                    .state
                    .users
                    .values()
                    .filter(|u| !u.banned)
                    .cloned()
                    .collect();
                people.sort_by(|a, b| {
                    rank(a.role)
                        .cmp(&rank(b.role))
                        .then_with(|| a.display_name.cmp(&b.display_name))
                });
                let hover = t.bg_hover;
                let rows = people.into_iter().map(|u| {
                    let on = picked.contains(&u.user_id);
                    let uid = u.user_id.clone();
                    let note = format!(
                        "@{}{}",
                        u.username,
                        if u.role != Role::Member {
                            format!(", {}, sees every room", role_name(u.role).to_lowercase())
                        } else {
                            String::new()
                        }
                    );
                    h_flex()
                        .id(SharedString::from(format!("pick-{}", u.username)))
                        .h(px(36.))
                        .gap(px(12.))
                        .px(px(8.))
                        .rounded(t.radius_sm)
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Ask::Members(_, p)) = &mut this.page.admin.ask {
                                if let Some(i) = p.iter().position(|x| *x == uid) {
                                    p.remove(i);
                                } else {
                                    p.push(uid.clone());
                                }
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .size(px(14.))
                                .flex_none()
                                .rounded(px(3.))
                                .border_1()
                                .border_color(if on { t.accent } else { t.line_strong })
                                .when(on, |d| {
                                    d.bg(t.accent).child(
                                        super::icons::icon("check")
                                            .size(px(12.))
                                            .text_color(t.on_accent),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .ellipsis()
                                .text_color(t.fg)
                                .child(u.display_name.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .ellipsis()
                                .text_xs()
                                .text_color(t.fg_2)
                                .child(note),
                        )
                });
                let ids = picked.clone();
                Dialog::new(format!("Who can see {}?", r.name))
                    .child(
                        v_flex()
                            .id("pick-list")
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .children(rows),
                    )
                    .footer(h_flex().gap_2().child(cancel).child(go(
                        "Save",
                        Variant::Primary,
                        cx,
                        Box::new(move |this, _, _| {
                            let ids = match &this.page.admin.ask {
                                Some(Ask::Members(_, p)) => p.clone(),
                                _ => ids.clone(),
                            };
                            this.http(
                                "admin:room:members",
                                Method::PUT,
                                format!("/api/rooms/{id}/members"),
                                Some(json!({ "user_ids": ids })),
                            )
                        }),
                    )))
            }
        };
        Some(d.on_close(close).children(error))
    }
}

impl State {
    #[cfg(test)]
    pub fn links_gone(&self) -> bool {
        self.created.is_none() && self.made.is_none()
    }
    fn shown_pw(&self) -> bool {
        self.shown_pw
    }
    fn set_shown_pw(&mut self, on: bool) {
        self.shown_pw = on;
    }
}

/// A 400 on a room name is the name rule.
fn room_name_error(f: Fail) -> String {
    match f {
        Fail::Api {
            code: ApiErrorCode::BadRequest,
            ..
        } => "Room names are 1 to 32 characters, with no hidden characters.".into(),
        f => fail_text(f),
    }
}

/// `toLocaleString` with day, short month, hour and minute, as en-US writes it: "Nov 22, 7:41 PM".
fn date_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or_else(String::new, |d| d.format("%b %-d, %-I:%M %p").to_string())
}
