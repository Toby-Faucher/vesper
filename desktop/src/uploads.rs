//! The files waiting above the composer, like the web's `tray.svelte.ts`. Each file goes up on its own request, at most
//! `LIMIT` at a time, the rest wait in order. Cancel stops the request (the server deletes the partial file), Retry
//! starts a failed one again, and the next send takes the finished ones of its room. Names are shown as text only.

use std::path::PathBuf;

use vesper_protocol::Attachment;

use crate::net::{Cmd, Fail};

/// Uploads in flight at once: the server takes 10, then 1 every 30 s.
pub const LIMIT: usize = 3;

pub const UPLOAD_QUOTA: &str =
    "The server has no room for more files. Ask the owner to free some space.";
pub const UPLOAD_RATE: &str = "Too many uploads in a short time. Wait a minute and try again.";
pub const UPLOAD_NETWORK: &str = "The upload stopped. Check your connection and try again.";
pub const UNREADABLE: &str = "That file could not be read.";

pub fn too_large(max: i64) -> String {
    format!("Files can be up to {} MB.", max / 1024 / 1024)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpState {
    Queued,
    Uploading,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub key: u64,
    pub room: i64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub state: UpState,
    /// 0 to 1.
    pub progress: f32,
    pub error: String,
    /// False when trying again cannot help (too large).
    pub retry: bool,
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Default)]
pub struct Tray {
    pub items: Vec<Item>,
    next: u64,
}

impl Tray {
    pub fn of(&self, room: i64) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.room == room)
    }

    /// An upload of this room is queued or running: sending waits.
    pub fn busy(&self, room: i64) -> bool {
        self.of(room)
            .any(|i| matches!(i.state, UpState::Queued | UpState::Uploading))
    }

    /// The finished files of a room, as (key, upload id).
    pub fn done(&self, room: i64) -> Vec<(u64, String)> {
        self.of(room)
            .filter_map(|i| Some((i.key, i.attachment.as_ref()?.id.clone())))
            .collect()
    }

    /// Adds files (path, size). One past `max` bytes fails here and is never sent.
    pub fn add(&mut self, room: i64, files: Vec<(PathBuf, u64)>, max: i64) -> Vec<Cmd> {
        for (path, size) in files {
            self.next += 1;
            let big = size > max.max(0) as u64;
            let name = path
                .file_name()
                .map_or("file".into(), |n| n.to_string_lossy().into_owned());
            self.items.push(Item {
                key: self.next,
                room,
                path,
                name,
                size,
                state: if big {
                    UpState::Failed
                } else {
                    UpState::Queued
                },
                progress: 0.0,
                error: if big { too_large(max) } else { String::new() },
                retry: !big,
                attachment: None,
            });
        }
        self.pump()
    }

    /// Cancel (stops a running upload) or Remove: the file leaves the tray.
    pub fn remove(&mut self, key: u64) -> Vec<Cmd> {
        let mut cmds = vec![];
        if let Some(i) = self.items.iter().position(|i| i.key == key) {
            if self.items[i].state == UpState::Uploading {
                cmds.push(Cmd::CancelUpload(key));
            }
            self.items.remove(i);
        }
        cmds.extend(self.pump());
        cmds
    }

    pub fn retry(&mut self, key: u64) -> Vec<Cmd> {
        if let Some(i) = self
            .items
            .iter_mut()
            .find(|i| i.key == key && i.state == UpState::Failed && i.retry)
        {
            (i.state, i.progress, i.error) = (UpState::Queued, 0.0, String::new());
        }
        self.pump()
    }

    /// After a send: the files it attached leave. A file that finished meanwhile stays for the next one.
    pub fn sent(&mut self, keys: &[u64]) {
        self.items.retain(|i| !keys.contains(&i.key));
    }

    /// Sign out: every upload stops and goes.
    pub fn clear(&mut self) -> Vec<Cmd> {
        let cmds = self
            .items
            .iter()
            .filter(|i| i.state == UpState::Uploading)
            .map(|i| Cmd::CancelUpload(i.key))
            .collect();
        self.items.clear();
        cmds
    }

    pub fn progress(&mut self, key: u64, p: f32) {
        if let Some(i) = self.find(key, UpState::Uploading) {
            i.progress = p;
        }
    }

    pub fn finished(&mut self, key: u64, a: Attachment) -> Vec<Cmd> {
        if let Some(i) = self.find(key, UpState::Uploading) {
            (i.state, i.progress, i.attachment) = (UpState::Done, 1.0, Some(a));
        }
        self.pump()
    }

    /// `fail` is `None` when the file could not be read.
    pub fn failed(&mut self, key: u64, fail: Option<Fail>, max: i64) -> Vec<Cmd> {
        if let Some(i) = self.find(key, UpState::Uploading) {
            let (error, retry) = match fail {
                None => (UNREADABLE.to_string(), true),
                Some(Fail::Api { status: 413, .. }) => (too_large(max), false),
                Some(Fail::Api { status: 507, .. }) => (UPLOAD_QUOTA.into(), true),
                Some(Fail::Api { status: 429, .. }) => (UPLOAD_RATE.into(), true),
                Some(Fail::Network) => (UPLOAD_NETWORK.into(), true),
                Some(f) => (f.text().into(), true),
            };
            (i.state, i.error, i.retry) = (UpState::Failed, error, retry);
        }
        self.pump()
    }

    fn find(&mut self, key: u64, state: UpState) -> Option<&mut Item> {
        self.items
            .iter_mut()
            .find(|i| i.key == key && i.state == state)
    }

    fn pump(&mut self) -> Vec<Cmd> {
        let mut running = self
            .items
            .iter()
            .filter(|i| i.state == UpState::Uploading)
            .count();
        let mut cmds = vec![];
        for i in &mut self.items {
            if running >= LIMIT {
                break;
            }
            if i.state != UpState::Queued {
                continue;
            }
            running += 1;
            i.state = UpState::Uploading;
            cmds.push(Cmd::Upload {
                key: i.key,
                room: i.room,
                path: i.path.clone(),
            });
        }
        cmds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vesper_protocol::{ApiErrorCode, AttachmentKind};

    fn att(id: &str) -> Attachment {
        Attachment {
            id: id.into(),
            name: "a.txt".into(),
            mime: "application/octet-stream".into(),
            size: 1,
            kind: AttachmentKind::File,
        }
    }

    fn uploads(cmds: &[Cmd]) -> Vec<u64> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Upload { key, .. } => Some(*key),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn three_at_a_time_in_order_with_cancel_retry_and_send() {
        let mut t = Tray::default();
        let files = (1..=5)
            .map(|i| (PathBuf::from(format!("/f{i}")), 10))
            .collect();
        assert_eq!(uploads(&t.add(1, files, 100)), [1, 2, 3]);
        assert!(t.busy(1) && !t.busy(2));
        // Cancel a running one: the next waiting one starts.
        let cmds = t.remove(2);
        assert!(matches!(cmds[0], Cmd::CancelUpload(2)));
        assert_eq!(uploads(&cmds), [4]);
        t.progress(1, 0.5);
        assert_eq!(t.items[0].progress, 0.5);
        assert_eq!(uploads(&t.finished(1, att("u1"))), [5]);
        let rate = Fail::Api {
            status: 429,
            code: ApiErrorCode::RateLimited,
        };
        assert!(t.failed(3, Some(rate), 100).is_empty());
        let f = t.items.iter().find(|i| i.key == 3).unwrap();
        assert_eq!(
            (f.state, f.error.as_str(), f.retry),
            (UpState::Failed, UPLOAD_RATE, true)
        );
        assert_eq!(uploads(&t.retry(3)), [3], "two run, so it starts again");
        t.finished(4, att("u4"));
        assert_eq!(t.done(1), [(1, "u1".to_string()), (4, "u4".to_string())]);
        t.sent(&[1, 4]);
        assert_eq!(t.items.iter().map(|i| i.key).collect::<Vec<_>>(), [3, 5]);
        assert!(matches!(
            t.clear().as_slice(),
            [Cmd::CancelUpload(3), Cmd::CancelUpload(5)]
        ));
    }

    #[test]
    fn too_large_fails_at_once_without_retry() {
        let mut t = Tray::default();
        assert!(
            t.add(1, vec![(PathBuf::from("/big.bin"), 26 << 20)], 25 << 20)
                .is_empty()
        );
        let i = &t.items[0];
        assert_eq!((i.state, i.retry), (UpState::Failed, false));
        assert_eq!(i.error, "Files can be up to 25 MB.");
        assert_eq!(i.name, "big.bin");
        assert!(t.retry(i.key).is_empty());
        assert!(!t.busy(1));
    }
}
