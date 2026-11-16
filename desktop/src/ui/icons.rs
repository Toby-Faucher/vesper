//! The web app's icons (`web/src/lib/ui/icons.ts`, Lucide paths, ISC licence there), read from that file at build time
//! so the two never drift. [`Assets`] serves each as `vesper/<name>.svg` in the stroke style of `c/Icon.svelte`;
//! [`icon`] draws one in the current text colour.

use std::borrow::Cow;
use std::sync::OnceLock;

use gpui::{AssetSource, SharedString, Styled, Svg, px, svg};

const ICONS_TS: &str = include_str!("../../../web/src/lib/ui/icons.ts");
/// The video card's open icon, `expandIcon`, lives here on the web (to keep it out of the first load).
const ATTACHMENTS_TS: &str = include_str!("../../../web/src/lib/ui/attachments.ts");
/// The screen share's icons (`eyeOff`, `monitorUp`, ...), kept out of the web's first load in their own file.
const SHARE_TS: &str = include_str!("../../../web/src/lib/share/icons.ts");

/// The desktop's own window buttons (chrome.rs), drawn like the others: minimize, maximize, restore.
const WINDOW: &[(&str, &str)] = &[
    ("winMin", "M6 12h12"),
    ("winMax", "M6 6h12v12H6z"),
    ("winRestore", "M6 9h9v9H6z M9 6h9v9"),
];

/// Every icon: its name in icons.ts (`micOff`) and its one path.
pub fn all() -> &'static [(&'static str, &'static str)] {
    static ALL: OnceLock<Vec<(&str, &str)>> = OnceLock::new();
    ALL.get_or_init(|| {
        ICONS_TS
            .lines()
            .chain(ATTACHMENTS_TS.lines())
            .chain(SHARE_TS.lines())
            .filter_map(|l| {
                let rest = l.strip_prefix("export const ")?;
                let (name, rest) = rest.split_once(" = '")?;
                let (d, _) = rest.split_once("' as IconPath")?;
                Some((name, d))
            })
            .chain(WINDOW.iter().copied())
            .collect()
    })
}

/// The SVG file of one icon, as `c/Icon.svelte` draws it (24 px box, stroke 1.5, round caps and joins).
pub fn svg_of(name: &str) -> Option<String> {
    let d = all().iter().find(|i| i.0 == name)?.1;
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="{d}"/></svg>"##
    ))
}

/// The Button's loading spinner (`c/Button.svelte`): a faint ring and a quarter arc, 16 px box.
const SPINNER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none"><circle cx="8" cy="8" r="6" stroke="#000" stroke-opacity="0.3" stroke-width="2"/><path d="M14 8a6 6 0 0 0-6-6" stroke="#000" stroke-width="2" stroke-linecap="round"/></svg>"##;

/// The app's assets: the icons and the spinner. GPUI tints an SVG with the element's text colour.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let Some(file) = path.strip_prefix("vesper/") else {
            return Ok(None);
        };
        if file == "spinner.svg" {
            return Ok(Some(Cow::Borrowed(SPINNER.as_bytes())));
        }
        let name = file.strip_suffix(".svg").unwrap_or(file);
        Ok(svg_of(name).map(|s| Cow::Owned(s.into_bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(if path.trim_end_matches('/') == "vesper" {
            all()
                .iter()
                .map(|(n, _)| format!("vesper/{n}.svg").into())
                .collect()
        } else {
            vec![]
        })
    }
}

/// An icon by its icons.ts name (`icon("mic")`), 16 px like `<Icon>`; `.size(px(20.))` for the large one. Always set
/// `.text_color()` on the icon itself: a GPUI svg does not inherit its parent's colour (unset, it is not drawn).
pub fn icon(name: &'static str) -> Svg {
    debug_assert!(all().iter().any(|i| i.0 == name), "no icon {name}");
    svg()
        .path(format!("vesper/{name}.svg"))
        .size(px(16.))
        .flex_none()
}

/// The 16 px loading spinner (turn it with `with_animation`).
pub fn spinner() -> Svg {
    svg().path("vesper/spinner.svg").size(px(16.)).flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_of_icons_ts_is_served() {
        let n = ICONS_TS
            .lines()
            .filter(|l| l.starts_with("export const "))
            .count();
        let share = SHARE_TS
            .lines()
            .filter(|l| l.starts_with("export const "))
            .count();
        // and expandIcon from attachments.ts, and the window buttons
        assert_eq!(
            all().len(),
            n + 1 + share + WINDOW.len(),
            "a line of icons.ts did not parse"
        );
        assert!(Assets.load("vesper/eyeOff.svg").unwrap().is_some());
        assert!(Assets.load("vesper/expandIcon.svg").unwrap().is_some());
        assert!(Assets.load("vesper/winRestore.svg").unwrap().is_some());
        assert!(n >= 40);
        let mic = Assets.load("vesper/mic.svg").unwrap().unwrap();
        assert!(
            std::str::from_utf8(&mic)
                .unwrap()
                .contains(r#"<path d="M12 2a3"#)
        );
        assert!(Assets.load("vesper/nope.svg").unwrap().is_none());
        assert!(Assets.load("vesper/spinner.svg").unwrap().is_some());
        assert_eq!(
            Assets.list("vesper").unwrap().len(),
            n + 1 + share + WINDOW.len()
        );
    }
}
