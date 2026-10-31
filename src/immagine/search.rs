//! Search: opened from the lupe of the range bar. Modelled on Immich's search page: a text field, "Kontext" (smart /
//! CLIP search, `POST /search/smart`) or "Dateiname" (`POST /search/metadata` with `originalFileName`), and a
//! media-type filter. Results are fetched on demand through `ImmichClient`, not stored, and shown in `photo_grid`.
//! Smart search results keep the server's relevance order; filename results are newest first.
//! The field lives in a glass bar at the bottom (Apple Photos look) that rides above the keyboard.

use super::icons::{glyph, Glyph};
use super::people::Load;
use super::Immagine;
use crate::components::glass::glass_div;
use crate::host::{Current as H, Host};
use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::timeline::TimelineItem;
use crate::theme::{Palette, BLUE, RED};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use std::rc::Rc;
use std::time::Duration;

/// Results per request, and the cap on requests per query.
const PAGE: u32 = 200;
const MAX_PAGES: u32 = 3;
/// Typing pauses this long before a search starts.
const DEBOUNCE_MS: u64 = 400;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Mode {
    #[default]
    Context,
    Filename,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Kind {
    #[default]
    All,
    Photos,
    Videos,
}

impl Kind {
    fn api(self) -> Option<&'static str> {
        match self {
            Kind::All => None,
            Kind::Photos => Some("IMAGE"),
            Kind::Videos => Some("VIDEO"),
        }
    }
}

#[derive(Default)]
pub(super) struct SearchState {
    pub(super) open: bool,
    input: Option<Entity<InputState>>,
    mode: Mode,
    kind: Kind,
    items: Rc<Vec<TimelineItem>>,
    status: Load,
    /// Bumped by every typed character and every new search; an answer or timer for an older value is dropped.
    serial: u32,
    pub(super) scroll: UniformListScrollHandle,
}

async fn run_query(origin: &str, token: &str, query: &str, mode: Mode, kind: Kind) -> Result<Vec<TimelineItem>, ClientError> {
    let client = ImmichClient::new(origin, H::transport())?;
    let mut items: Vec<TimelineItem> = Vec::new();
    for page in 1..=MAX_PAGES {
        let answer = match mode {
            Mode::Context => client.smart_search(query, kind.api(), page, PAGE, token).await?,
            Mode::Filename => client.filename_search(query, kind.api(), page, PAGE, token).await?,
        };
        let got = answer.assets.items.len();
        items.extend(answer.assets.items.iter().map(|asset| asset.timeline_item()));
        if answer.assets.next_page.is_none() || got == 0 {
            break;
        }
    }
    items.retain(|item| !item.is_trashed);
    if mode == Mode::Filename {
        items.sort_by(|a, b| b.sort_date.cmp(&a.sort_date));
    }
    Ok(items)
}

impl Immagine {
    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Mediathek durchsuchen …"));
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.search_typed(cx),
                InputEvent::PressEnter { .. } => {
                    window.dismiss_virtual_keyboard();
                    this.start_search(cx)
                }
                _ => {}
            })
            .detach();
            self.search.input = Some(input);
        }
        self.search.open = true;
        self.focus_search(window, cx);
        cx.notify();
    }

    pub(super) fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search.open = false;
        self.search.serial += 1;
        cx.notify();
    }

    /// Focuses the field and asks for the software keyboard. Called from tap handlers: browsers only show the
    /// keyboard for a focus that happens inside a user gesture, and the web platform only recognizes a tap on the
    /// text itself as a tap on a text input, not one on the glass pill around it or on the lupe that opened this.
    fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.search.input.clone() {
            input.update(cx, |state, cx| state.focus(window, cx));
            window.request_virtual_keyboard();
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.input.as_ref().map(|i| i.read(cx).value().trim().to_string()).unwrap_or_default()
    }

    /// Typing: search once the text has been still for `DEBOUNCE_MS`.
    fn search_typed(&mut self, cx: &mut Context<Self>) {
        self.search.serial += 1;
        let serial = self.search.serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(DEBOUNCE_MS)).await;
            let _ = this.update(cx, |t, cx| {
                if t.search.serial == serial {
                    t.start_search(cx);
                }
            });
        })
        .detach();
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.search.serial += 1;
        let serial = self.search.serial;
        if query.is_empty() {
            self.search.items = Rc::new(Vec::new());
            self.search.status = Load::Idle;
            cx.notify();
            return;
        }
        self.search.status = Load::Loading;
        let (origin, token, mode, kind) = (self.origin.clone(), self.token.clone(), self.search.mode, self.search.kind);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = run_query(&origin, &token, &query, mode, kind).await;
            let _ = this.update(cx, |t, cx| {
                if t.search.serial != serial {
                    return;
                }
                match result {
                    Ok(items) => {
                        t.search.items = Rc::new(items);
                        t.search.status = Load::Done;
                    }
                    Err(ClientError::Unauthorized) => {
                        t.log_out(cx);
                        t.login_error = Some(ClientError::Unauthorized.to_string());
                    }
                    Err(e) => t.search.status = Load::Failed(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn search_view(&mut self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.search;
        let (hint, hint_color): (String, Rgba) = match &s.status {
            Load::Idle => ("Nach Motiven, Orten oder Dateinamen suchen.".into(), p.secondary),
            Load::Loading => ("Suche …".into(), p.secondary),
            Load::Failed(e) => (e.clone(), rgb(RED).into()),
            Load::Done if s.items.is_empty() => ("Keine Ergebnisse".into(), p.secondary),
            Load::Done => (format!("{} Ergebnisse", s.items.len()), p.secondary),
        };

        // Glass pills like the suggestion buttons of Apple's search; the selected one is blue.
        let chip = |id: &'static str, label: &'static str, on: bool| {
            glass_div(p.glass_tint, 18.)
                .id(id)
                .h(px(40.))
                .px_4()
                .flex()
                .items_center()
                .text_size(px(16.))
                .font_weight(if on { FontWeight::BOLD } else { FontWeight::MEDIUM })
                .text_color(if on { rgb(BLUE).into() } else { p.label })
                .active(|s| s.opacity(0.6))
                .child(label)
        };
        let mode_chip = |id: &'static str, label: &'static str, mode: Mode| {
            chip(id, label, s.mode == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.search.mode = mode;
                this.start_search(cx);
            }))
        };
        let kind_chip = |id: &'static str, label: &'static str, kind: Kind| {
            chip(id, label, s.kind == kind).on_click(cx.listener(move |this, _, _, cx| {
                this.search.kind = kind;
                this.start_search(cx);
            }))
        };

        let header = div()
            .px_4()
            .pt(px(40.))
            .pb_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_size(px(34.)).font_weight(FontWeight::BOLD).child("Suchen"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(mode_chip("search-mode-ctx", "Kontext", Mode::Context))
                    .child(mode_chip("search-mode-name", "Dateiname", Mode::Filename))
                    .child(kind_chip("search-kind-all", "Alle", Kind::All))
                    .child(kind_chip("search-kind-img", "Fotos", Kind::Photos))
                    .child(kind_chip("search-kind-vid", "Videos", Kind::Videos)),
            )
            .child(div().text_size(px(14.)).text_color(hint_color).child(hint));

        let grid = self.photo_grid("search", self.search.items.clone(), 0., (&self.search.scroll, None), p, window, cx);
        self.rubber.shift(div().size_full().flex().flex_col().pt(px(self.top.max(12.))).child(header).child(grid)).into_any_element()
    }

    /// The bottom bar of the search: a glass field with a lupe and a round close button, riding above the keyboard.
    pub(super) fn search_bar(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        const H: f32 = 56.;
        let keyboard = Self::keyboard_height(window);
        let bottom = if keyboard > 0. { keyboard + 8. } else { self.bottom.max(10.) };

        let field = self.search.input.as_ref().map(|input| {
            glass_div(p.glass_tint, H / 2.)
                .id("search-field")
                .h(px(H))
                .flex_1()
                .px_4()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
                .child(glyph(Glyph::Search, 22., p.secondary))
                .child(div().flex_1().child(Input::new(input).appearance(false).bordered(false).cleanable(false)))
        });
        let close = glass_div(p.glass_tint, H / 2.)
            .id("search-close")
            .size(px(H))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .active(|s| s.opacity(0.6))
            .on_click(cx.listener(|this, _, window, cx| {
                window.dismiss_virtual_keyboard();
                this.close_search(cx);
            }))
            .child(glyph(Glyph::Close, 22., p.label));

        div()
            .id("search-bar")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .pb(px(bottom))
            .px(px(16.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .occlude()
            .children(field)
            .child(close)
            .into_any_element()
    }
}
