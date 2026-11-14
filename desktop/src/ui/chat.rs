//! The chat parts of the room view: message rows (Markdown, replies, files, reactions, the actions on hover, editing
//! and the delete confirm), the pins panel, and what sits above the composer (typing, the reply bar, the @ list, the
//! upload tray). Names, bodies and file names come from other people: drawn as text only.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use chrono::Local;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, ClipboardEntry, Context, Div, ElementId, FontWeight, ImageFormat, ImageSource,
    InteractiveElement, IntoElement, ParentElement, PathPromptOptions, RenderImage, SharedString,
    StatefulInteractiveElement, Styled, Window, div, img, px, rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, RopeExt};
use gpui_component::{ActiveTheme, Selectable, Sizable, h_flex, v_flex};
use vesper_protocol::{Attachment, AttachmentKind, UserInfo};

use super::room::time_label;
use super::sign_in::TextLine;
use super::{
    CancelEdit, ComposerDown, ComposerEscape, ComposerTab, ComposerUp, EDIT_BOX, NewLine,
    PasteFile, SaveEdit, Vesper, md, theme,
};
use crate::files::{self, IMAGE_MAX, Picture};
use crate::markdown::{parse, plain};
use crate::model::EMOJIS;
use crate::net::Cmd;
use crate::timeline::{BOX, RowHead, file_size, fit, mention_query, mentionable, typing_line};
use crate::uploads::UpState;

/// Decoded pictures kept for the timeline; the oldest goes first.
pub const IMAGE_CACHE: usize = 64;
pub const DELETE_ASK: &str =
    "Delete this message? It is removed for everyone in the room. This cannot be undone.";
const IMAGE_FAILED: &str = "The image did not load.";

/// An inline picture's state.
pub enum Img {
    Loading,
    Failed,
    Ready(Arc<RenderImage>),
}

fn small(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(color))
        .child(text.into())
}

/// An element the tests find by `selector`.
fn tagged(selector: String, el: impl IntoElement) -> Div {
    div().debug_selector(move || selector).child(el)
}

fn button(id: String, label: impl Into<SharedString>) -> Button {
    Button::new(ElementId::Name(id.into()))
        .xsmall()
        .ghost()
        .label(label)
}

impl Vesper {
    /// One message row, with its day row above when it starts a day.
    pub(super) fn message_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(msg) = self.model.log().and_then(|l| l.messages.get(ix)).cloned() else {
            return div().into_any_element();
        };
        let head = self.heads.get(ix).cloned().unwrap_or(RowHead {
            day: None,
            head: true,
        });
        let id = msg.id;
        let m = &self.model;
        let s = &m.state;
        let my_id = s.my_id().to_string();
        let mentioned = !msg.deleted && msg.mentions.contains(&my_id);
        let mut row = v_flex()
            .id(SharedString::from(format!("msg-{id}")))
            .group("msg")
            .relative()
            .debug_selector(|| format!("message-{id}"))
            .px_4()
            .py_0p5()
            .gap_0p5()
            .when(head.head, |r| r.pt_2())
            .hover(|s| s.bg(rgb(theme::BG_HOVER)))
            .when(mentioned, |r| {
                r.bg(rgb(theme::ACCENT_SUBTLE))
                    .border_l_2()
                    .border_color(rgb(theme::ACCENT))
            });
        if let Some(r) = &msg.reply_to {
            let ex = plain(&parse(&r.excerpt));
            let ex = if ex.is_empty() {
                "a deleted message".into()
            } else {
                ex
            };
            let target = r.id;
            row = row.child(
                div()
                    .id(SharedString::from(format!("quote-{id}")))
                    .debug_selector(|| format!("quote-{id}"))
                    .cursor_pointer()
                    .pl_2()
                    .border_l_2()
                    .border_color(rgb(theme::LINE))
                    .text_size(px(13.))
                    .text_color(rgb(theme::FG_3))
                    .truncate()
                    .child(format!("Reply to {}: {ex}", s.name_of(&r.author_id)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.model.jump_to(target) {
                            cx.notify();
                        }
                    })),
            );
        }
        let edited = msg.edited_at.is_some() && !msg.deleted;
        if head.head {
            row = row.child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(s.name_of(&msg.author_id)),
                    )
                    .child(small(time_label(msg.created_at), theme::FG_3))
                    .when(edited, |r| r.child(small("(edited)", theme::FG_3)))
                    .when(msg.pinned, |r| r.child(small("Pinned", theme::FG_3))),
            );
        }
        if msg.deleted {
            row = row.child(
                div()
                    .italic()
                    .text_color(rgb(theme::FG_3))
                    .child("Message deleted"),
            );
        } else if m.editing == Some(id) {
            row = row.child(self.edit_box(cx));
        } else {
            let names: Vec<&str> = msg.attachments.iter().map(|a| a.name.as_str()).collect();
            // A message of files only carries their names as its body: not shown twice.
            let filler = !msg.attachments.is_empty() && msg.body == names.join(", ");
            if !filler {
                let ctx = md::Ctx {
                    me: s
                        .me
                        .as_ref()
                        .map_or(String::new(), |m| m.user.username.clone()),
                    mono: cx.theme().mono_font_family.clone(),
                    id,
                };
                row = row.child(
                    v_flex()
                        .text_color(rgb(theme::FG))
                        .children(md::render(&parse(&msg.body), &ctx)),
                );
            }
            if !head.head && (edited || msg.pinned) {
                let note = match (edited, msg.pinned) {
                    (true, true) => "(edited) Pinned",
                    (true, false) => "(edited)",
                    _ => "Pinned",
                };
                row = row.child(small(note, theme::FG_3));
            }
            for a in &msg.attachments {
                let el = self.attachment(a, cx);
                row = row.child(el);
            }
            row = row.children(self.reactions(&msg, cx));
        }
        let m = &self.model;
        if m.picking == Some(id) {
            row = row.child(
                h_flex()
                    .gap_1()
                    .debug_selector(|| format!("picker-{id}"))
                    .children(EMOJIS.iter().enumerate().map(|(i, e)| {
                        tagged(
                            format!("pick-{id}-{i}"),
                            button(format!("pick-{id}-{i}"), *e).small().on_click(
                                cx.listener(move |this, _, _, cx| this.act(|m| m.react(id, e), cx)),
                            ),
                        )
                    })),
            );
        }
        if m.confirm_delete == Some(id) {
            row = row.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgb(theme::BG_RAISED))
                    .debug_selector(|| format!("confirm-{id}"))
                    .child(DELETE_ASK)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(tagged(
                                "confirm-cancel".into(),
                                Button::new("confirm-cancel")
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.model.cancel_delete();
                                        cx.notify();
                                    })),
                            ))
                            .child(tagged(
                                "confirm-delete".into(),
                                Button::new("confirm-delete")
                                    .small()
                                    .danger()
                                    .label("Delete")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.act(|m| m.delete(id, true), cx)
                                    })),
                            )),
                    ),
            );
        }
        if !msg.deleted && m.editing != Some(id) {
            row = row.child(self.toolbar(&msg, cx));
        }
        let day = head.day.map(|label| {
            let line = || div().flex_1().h(px(1.)).bg(rgb(theme::LINE_SUBTLE));
            h_flex()
                .debug_selector(|| format!("day-{id}"))
                .px_4()
                .pt_3()
                .gap_2()
                .child(line())
                .child(small(label, theme::FG_3))
                .child(line())
        });
        v_flex().children(day).child(row).into_any_element()
    }

    /// The actions on a message, shown while the pointer is over it (always there to click, like the web's).
    fn toolbar(&self, msg: &vesper_protocol::Message, cx: &mut Context<Self>) -> Div {
        let m = &self.model;
        let id = msg.id;
        let tool = |name: &str, label: &'static str| {
            let sel = format!("{name}-{id}");
            (sel.clone(), button(sel, label))
        };
        let (rs, react) = tool("react", "React");
        let (ps, reply) = tool("reply", "Reply");
        let mut bar = h_flex()
            .absolute()
            .top_0p5()
            .right_4()
            .gap_0p5()
            .p_0p5()
            .rounded(px(6.))
            .bg(rgb(theme::BG_RAISED))
            .border_1()
            .border_color(rgb(theme::LINE_SUBTLE))
            .opacity(0.)
            .group_hover("msg", |s| s.opacity(1.))
            .child(tagged(
                rs,
                react.on_click(cx.listener(move |this, _, _, cx| {
                    this.model.toggle_picker(id);
                    cx.notify();
                })),
            ))
            .child(tagged(
                ps,
                reply.on_click(cx.listener(move |this, _, window, cx| {
                    this.model.reply(id);
                    this.composer.update(cx, |i, cx| i.focus(window, cx));
                    cx.notify();
                })),
            ));
        if m.can_pin() {
            let (s, b) = tool("pin", if msg.pinned { "Unpin" } else { "Pin" });
            bar = bar.child(tagged(
                s,
                b.on_click(cx.listener(move |this, _, _, cx| this.act(|m| m.pin(id), cx))),
            ));
        }
        if m.mine(msg) {
            let (s, b) = tool("edit", "Edit");
            bar = bar.child(tagged(
                s,
                b.on_click(cx.listener(move |this, _, window, cx| this.start_edit(id, window, cx))),
            ));
        }
        if m.can_delete(msg) {
            let (s, b) = tool("delete", "Delete");
            bar = bar.child(tagged(
                s,
                b.on_click(
                    cx.listener(move |this, _, _, cx| this.act(|m| m.delete(id, false), cx)),
                ),
            ));
        }
        bar
    }

    fn edit_box(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .key_context(EDIT_BOX)
            .on_action(cx.listener(Self::save_edit))
            .on_action(cx.listener(Self::cancel_edit))
            .on_action(cx.listener(|this, _: &NewLine, window, cx| {
                this.edit.update(cx, |i, cx| i.insert("\n", window, cx));
            }))
            .gap_1()
            .debug_selector(|| "edit-box".into())
            .child(Input::new(&self.edit))
            .child(
                h_flex()
                    .gap_2()
                    .child(tagged(
                        "edit-save".into(),
                        Button::new("edit-save")
                            .small()
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_edit(&SaveEdit, window, cx)
                            })),
                    ))
                    .child(
                        Button::new("edit-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_edit(&CancelEdit, window, cx)
                            })),
                    )
                    .child(small("Enter saves, Esc cancels.", theme::FG_2)),
            )
            .when_some_text(&self.model.edit_error, theme::DANGER_FG, "edit-error")
    }

    fn reactions(&self, msg: &vesper_protocol::Message, cx: &mut Context<Self>) -> Option<Div> {
        if msg.reactions.is_empty() {
            return None;
        }
        let s = &self.model.state;
        let me = s.my_id();
        let id = msg.id;
        Some(
            h_flex()
                .flex_wrap()
                .gap_1()
                .debug_selector(|| format!("reactions-{id}"))
                .children(msg.reactions.iter().enumerate().map(|(i, r)| {
                    let on = r.user_ids.iter().any(|u| u == me);
                    let who: Vec<String> = r.user_ids.iter().map(|u| s.name_of(u)).collect();
                    let emoji = r.emoji.clone();
                    tagged(
                        format!("reaction-{id}-{i}"),
                        Button::new(ElementId::Name(format!("reaction-{id}-{i}").into()))
                            .xsmall()
                            .outline()
                            .selected(on)
                            .label(format!("{} {}", r.emoji, r.user_ids.len()))
                            .tooltip(who.join(", "))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.act(|m| m.react(id, &emoji), cx)
                            })),
                    )
                })),
        )
    }

    /// An image inline (fetched once, with the session cookie), anything else a card with Open or Save.
    fn attachment(&mut self, a: &Attachment, cx: &mut Context<Self>) -> AnyElement {
        if a.kind == AttachmentKind::Image && a.size as u64 <= IMAGE_MAX {
            if !self.images.contains_key(&a.id) {
                self.images.insert(a.id.clone(), Img::Loading);
                self.run(vec![Cmd::FetchImage(a.id.clone())]);
            }
            let open = {
                let a = a.clone();
                cx.listener(move |this, _, _, cx| this.open_attachment(&a, cx))
            };
            let pic = match &self.images[&a.id] {
                Img::Ready(i) => {
                    let s = i.size(0);
                    let (w, h) = fit(s.width.0 as u32, s.height.0 as u32);
                    img(ImageSource::Render(i.clone()))
                        .w(px(w as f32))
                        .h(px(h as f32))
                        .rounded(px(6.))
                        .into_any_element()
                }
                Img::Loading => div()
                    .w(px(BOX.0 as f32))
                    .h(px(BOX.1 as f32))
                    .rounded(px(6.))
                    .bg(rgb(theme::BG_RAISED))
                    .into_any_element(),
                Img::Failed => {
                    return v_flex()
                        .child(small(IMAGE_FAILED, theme::FG_3))
                        .child(self.card(a, cx))
                        .into_any_element();
                }
            };
            return div()
                .id(SharedString::from(format!("image-{}", a.id)))
                .debug_selector(|| format!("image-{}", a.id))
                .cursor_pointer()
                .mt_1()
                .child(pic)
                .on_click(open)
                .into_any_element();
        }
        self.card(a, cx).into_any_element()
    }

    fn card(&self, a: &Attachment, cx: &mut Context<Self>) -> Div {
        let kind = match a.kind {
            AttachmentKind::Image => "Image",
            AttachmentKind::Audio => "Audio",
            AttachmentKind::Video => "Video",
            AttachmentKind::File => "File",
        };
        let id = a.id.clone();
        let (open, save) = (a.clone(), a.clone());
        h_flex()
            .debug_selector(move || format!("file-{id}"))
            .mt_1()
            .gap_3()
            .p_2()
            .max_w(px(BOX.0 as f32))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(theme::LINE_SUBTLE))
            .bg(rgb(theme::BG_RAISED))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(a.name.clone()))
                    .child(small(
                        format!("{kind}, {}", file_size(a.size.max(0) as u64)),
                        theme::FG_3,
                    )),
            )
            // Media opens in the system's player; any other file only saves, so nothing sent in chat runs.
            .when(a.kind != AttachmentKind::File, |r| {
                r.child(tagged(
                    format!("open-{}", a.id),
                    Button::new(ElementId::Name(format!("open-{}", a.id).into()))
                        .small()
                        .label("Open")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_attachment(&open, cx)),
                        ),
                ))
            })
            .child(tagged(
                format!("save-{}", a.id),
                Button::new(ElementId::Name(format!("save-{}", a.id).into()))
                    .small()
                    .label("Save")
                    .on_click(cx.listener(move |this, _, _, cx| this.save_attachment(&save, cx))),
            ))
    }

    /// Hands a media file to the system's app. Downloaded first, named by its id and the type the server found.
    fn open_attachment(&mut self, a: &Attachment, cx: &mut Context<Self>) {
        let Some(ext) = files::media_ext(&a.mime).filter(|_| files::valid_id(&a.id)) else {
            return;
        };
        let path = std::env::temp_dir()
            .join("vesper-open")
            .join(format!("{}.{ext}", a.id));
        if path.exists() {
            cx.open_with_system(&path);
        } else {
            self.run(vec![Cmd::Download {
                id: a.id.clone(),
                to: path,
                open: true,
            }]);
        }
    }

    /// Saves where the user picks, starting in Downloads. With no dialog (no portal on Linux) it goes to Downloads.
    fn save_attachment(&mut self, a: &Attachment, cx: &mut Context<Self>) {
        if !files::valid_id(&a.id) {
            return;
        }
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        let name = files::safe_name(&a.name);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        let id = a.id.clone();
        cx.spawn(async move |this, cx| {
            let to = match rx.await {
                Ok(Ok(Some(p))) => p,
                Ok(Ok(None)) => return,
                _ => files::unique_path(&dir, &name),
            };
            let _ = this.update(cx, |v, _| {
                v.run(vec![Cmd::Download {
                    id,
                    to,
                    open: false,
                }])
            });
        })
        .detach();
    }

    /// A picture came from the network thread: kept for drawing, the oldest let go past `IMAGE_CACHE`.
    pub(super) fn image_loaded(&mut self, id: &str, picture: Option<Picture>, window: &mut Window) {
        let img = picture
            .and_then(|p| image::RgbaImage::from_raw(p.width, p.height, p.bgra))
            .map(|buf| Arc::new(RenderImage::new(vec![image::Frame::new(buf)])));
        self.images
            .insert(id.to_string(), img.map_or(Img::Failed, Img::Ready));
        self.image_order.push_back(id.to_string());
        while self.image_order.len() > IMAGE_CACHE {
            let Some(old) = self.image_order.pop_front() else {
                break;
            };
            if let Some(Img::Ready(i)) = self.images.remove(&old) {
                let _ = window.drop_image(i);
            }
        }
    }

    pub(super) fn start_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(body) = self.model.start_edit(id) {
            self.edit.update(cx, |i, cx| {
                let end = body.len();
                i.set_value(body, window, cx);
                // The caret at the end, as the web's edit box; this also focuses it.
                let pos = i.text().offset_to_position(end);
                i.set_cursor_position(pos, window, cx);
            });
        }
        cx.notify();
    }

    fn save_edit(&mut self, _: &SaveEdit, _: &mut Window, cx: &mut Context<Self>) {
        let body = self.edit.read(cx).value();
        self.act(|m| m.save_edit(&body), cx);
    }

    fn cancel_edit(&mut self, _: &CancelEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.model.cancel_edit();
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    /// The people the @ list offers now.
    pub(super) fn people(&self) -> Vec<UserInfo> {
        let s = &self.model.state;
        let (Some((_, q)), Some(room)) = (
            &self.mention,
            self.model.active.and_then(|id| s.find_room(id)),
        ) else {
            return vec![];
        };
        mentionable(q, s.users.values(), room, s.my_id())
            .into_iter()
            .cloned()
            .collect()
    }

    /// The composer's text changed: the @ list follows the caret, and typing goes out.
    pub(super) fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let (text, caret) = {
            let c = self.composer.read(cx);
            (c.value(), c.cursor())
        };
        let q = mention_query(&text, caret);
        if q.as_ref().map(|q| q.0) != self.mention.as_ref().map(|m| m.0) {
            self.mention_ix = 0;
        }
        // An @ list closed with Esc stays shut while that @ is typed.
        if q.is_some() && q.as_ref().map(|q| q.0) == self.mention_dismissed {
            self.mention = None;
        } else {
            (self.mention, self.mention_dismissed) = (q, None);
        }
        self.act(|m| m.typed(&text, Instant::now()), cx);
    }

    /// Puts `@username ` in place of the @ being typed.
    pub(super) fn pick_mention(
        &mut self,
        username: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start, _)) = self.mention.take() else {
            return;
        };
        self.composer.update(cx, |i, cx| {
            let (text, caret) = (i.value(), i.cursor());
            let ins = format!("@{username} ");
            let new = format!("{}{ins}{}", &text[..start], &text[caret.max(start)..]);
            i.set_value(new, window, cx);
            let pos = i.text().offset_to_position(start + ins.len());
            i.set_cursor_position(pos, window, cx);
        });
        cx.notify();
    }

    pub(super) fn composer_up(
        &mut self,
        _: &ComposerUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.people().len();
        if n > 0 {
            self.mention_ix = (self.mention_ix + n - 1) % n;
            return cx.notify();
        }
        let empty = self.composer.read(cx).value().is_empty();
        let files = self
            .model
            .active
            .is_some_and(|r| self.model.tray.of(r).next().is_some());
        if empty
            && !files
            && let Some((id, _)) = self.model.edit_last()
        {
            return self.start_edit(id, window, cx);
        }
        cx.propagate();
    }

    pub(super) fn composer_down(
        &mut self,
        _: &ComposerDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.people().len();
        if n == 0 {
            return cx.propagate();
        }
        self.mention_ix = (self.mention_ix + 1) % n;
        cx.notify();
    }

    pub(super) fn composer_tab(
        &mut self,
        _: &ComposerTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pick_active(window, cx) {
            cx.propagate();
        }
    }

    /// Enter or Tab with the @ list open takes the highlighted person.
    pub(super) fn pick_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let people = self.people();
        let Some(u) = people.get(self.mention_ix.min(people.len().saturating_sub(1))) else {
            return false;
        };
        let name = u.username.clone();
        self.pick_mention(&name, window, cx);
        true
    }

    pub(super) fn composer_escape(
        &mut self,
        _: &ComposerEscape,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((start, _)) = self.mention.take() {
            self.mention_dismissed = Some(start);
        } else if self.model.reply_to.is_some() {
            self.model.cancel_reply();
        } else {
            return cx.propagate();
        }
        cx.notify();
    }

    /// A picture on the clipboard goes to the tray as a file; anything else pastes as text.
    pub(super) fn paste_file(&mut self, _: &PasteFile, _: &mut Window, cx: &mut Context<Self>) {
        let image = cx.read_from_clipboard().and_then(|c| {
            c.entries().iter().find_map(|e| match e {
                ClipboardEntry::Image(i) => Some(i.clone()),
                _ => None,
            })
        });
        let Some(image) = image else {
            return cx.propagate();
        };
        let ext = match image.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
        };
        let dir = std::env::temp_dir().join("vesper-paste");
        let name = format!(
            "Pasted image {}.{ext}",
            Local::now().format("%Y-%m-%d %H.%M")
        );
        let path = files::unique_path(&dir, &name);
        let wrote = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &image.bytes));
        match wrote {
            Ok(()) => self.attach_paths(vec![path], cx),
            Err(_) => {
                self.model.error = crate::uploads::UNREADABLE.into();
                cx.notify();
            }
        }
    }

    /// Files from the button, a drop or a paste. Folders are left out.
    pub(super) fn attach_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let files: Vec<(PathBuf, u64)> = paths
            .into_iter()
            .filter_map(|p| {
                let meta = std::fs::metadata(&p).ok().filter(|m| m.is_file())?;
                Some((p, meta.len()))
            })
            .collect();
        self.act(|m| m.attach(files), cx);
    }

    pub(super) fn pick_files(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |v, cx| v.attach_paths(paths, cx));
            }
        })
        .detach();
    }

    /// What sits above the composer's text box: a saved file, who is typing, the reply, the @ list, the files.
    pub(super) fn above_composer(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![];
        let m = &self.model;
        let s = &m.state;
        let Some(room) = m.active else { return out };
        if let Some(p) = &m.saved {
            let path = p.clone();
            out.push(
                h_flex()
                    .debug_selector(|| "saved".into())
                    .gap_2()
                    .text_size(px(13.))
                    .text_color(rgb(theme::FG_2))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(format!("Saved to {}.", p.display())),
                    )
                    .child(
                        button("saved-show".into(), "Show in folder")
                            .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&path))),
                    )
                    .child(button("saved-ok".into(), "OK").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.model.saved = None;
                            cx.notify();
                        },
                    )))
                    .into_any_element(),
            );
        }
        let typing = typing_line(&m.typers(room, Instant::now()));
        if !typing.is_empty() {
            out.push(
                div()
                    .debug_selector(|| "typing".into())
                    .text_size(px(12.))
                    .text_color(rgb(theme::FG_3))
                    .child(typing)
                    .into_any_element(),
            );
        }
        if let Some(r) = &m.reply_to {
            let ex = plain(&parse(&r.body));
            let ex = if ex.is_empty() {
                "a message with a file".into()
            } else {
                ex
            };
            out.push(
                h_flex()
                    .debug_selector(|| "reply-bar".into())
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .bg(rgb(theme::BG_RAISED))
                    .text_size(px(13.))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(rgb(theme::FG_2))
                            .child(format!("Replying to {}: {ex}", s.name_of(&r.author_id))),
                    )
                    .child(tagged(
                        "reply-cancel".into(),
                        button("reply-cancel".into(), "Cancel reply").on_click(cx.listener(
                            |this, _, window, cx| {
                                this.model.cancel_reply();
                                this.composer.update(cx, |i, cx| i.focus(window, cx));
                                cx.notify();
                            },
                        )),
                    ))
                    .into_any_element(),
            );
        }
        let people = self.people();
        if !people.is_empty() {
            let active = self.mention_ix.min(people.len() - 1);
            out.push(
                v_flex()
                    .debug_selector(|| "mentions".into())
                    .p_1()
                    .rounded(px(6.))
                    .bg(rgb(theme::BG_RAISED))
                    .border_1()
                    .border_color(rgb(theme::LINE_SUBTLE))
                    .children(people.into_iter().enumerate().map(|(i, u)| {
                        let name = u.username.clone();
                        div()
                            .id(ElementId::Name(format!("mention-{}", u.username).into()))
                            .debug_selector(move || format!("mention-{}", u.username))
                            .px_2()
                            .py_0p5()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(i == active, |r| r.bg(rgb(theme::BG_SELECTED)))
                            .hover(|r| r.bg(rgb(theme::BG_HOVER)))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(u.display_name.clone())
                                    .child(small(format!("@{name}"), theme::FG_3)),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick_mention(&name, window, cx)
                            }))
                    }))
                    .into_any_element(),
            );
        }
        let items: Vec<_> = self.model.tray.of(room).cloned().collect();
        if !items.is_empty() {
            out.push(
                v_flex()
                    .debug_selector(|| "upload-tray".into())
                    .gap_1()
                    .children(items.into_iter().map(|i| {
                        let key = i.key;
                        let busy = matches!(i.state, UpState::Queued | UpState::Uploading);
                        let state: AnyElement = match i.state {
                            UpState::Uploading => {
                                let pct = (i.progress * 100.0).round();
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .h(px(4.))
                                            .rounded(px(2.))
                                            .bg(rgb(theme::LINE))
                                            .child(
                                                div()
                                                    .h_full()
                                                    .rounded(px(2.))
                                                    .bg(rgb(theme::ACCENT))
                                                    .w(px(0.8 * pct)),
                                            ),
                                    )
                                    .child(small(format!("{pct}%"), theme::FG_2))
                                    .into_any_element()
                            }
                            UpState::Queued => small("Waiting", theme::FG_2).into_any_element(),
                            UpState::Done => small("Ready", theme::FG_2).into_any_element(),
                            UpState::Failed => h_flex()
                                .gap_2()
                                .child(small(i.error.clone(), theme::DANGER_FG))
                                .when(i.retry, |r| {
                                    r.child(tagged(
                                        format!("upload-retry-{key}"),
                                        button(format!("upload-retry-{key}"), "Retry").on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.act(|m| m.retry_upload(key), cx)
                                            }),
                                        ),
                                    ))
                                })
                                .into_any_element(),
                        };
                        h_flex()
                            .debug_selector(move || format!("upload-{key}"))
                            .gap_3()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(rgb(theme::BG_RAISED))
                            .text_size(px(13.))
                            .child(div().flex_1().min_w_0().truncate().child(i.name.clone()))
                            .child(small(file_size(i.size), theme::FG_3))
                            .child(state)
                            .child(tagged(
                                format!("upload-remove-{key}"),
                                button(
                                    format!("upload-remove-{key}"),
                                    if busy { "Cancel" } else { "Remove" },
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.act(|m| m.remove_upload(key), cx),
                                )),
                            ))
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    /// The pins of the shown room, under the header, while open.
    pub(super) fn render_pins(&self, cx: &mut Context<Self>) -> Option<Div> {
        let m = &self.model;
        let (_, list) = m.pins.as_ref()?;
        let s = &m.state;
        let body: Vec<AnyElement> = match list {
            None => vec![small("Loading pins...", theme::FG_3).into_any_element()],
            Some(l) if l.is_empty() => {
                vec![small("No pinned messages yet.", theme::FG_3).into_any_element()]
            }
            Some(l) => l
                .iter()
                .map(|p| {
                    let id = p.id;
                    div()
                        .id(ElementId::Name(format!("pinned-{id}").into()))
                        .debug_selector(move || format!("pinned-{id}"))
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|r| r.bg(rgb(theme::BG_HOVER)))
                        .text_size(px(13.))
                        .truncate()
                        .child(format!(
                            "{}: {}",
                            s.name_of(&p.author_id),
                            plain(&parse(&p.body))
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.model.jump_to(id);
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect(),
        };
        Some(
            v_flex()
                .debug_selector(|| "pins-panel".into())
                .max_h(px(240.))
                .flex_none()
                .px_4()
                .py_2()
                .gap_0p5()
                .border_b_1()
                .border_color(rgb(theme::LINE_SUBTLE))
                .bg(rgb(theme::BG_SIDEBAR))
                .child(small("Pinned messages", theme::FG_3))
                .children(body),
        )
    }
}
