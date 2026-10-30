//! Owner: people agent. Swift sources: `../immagine/Immagine/UI/People/*.swift`, `People/PeopleStore.swift`.
//! People tab: faces recognized by Immich; person page with their photos (`photo_grid`).
//! People are not in the `Store`; they are fetched on demand through `ImmichClient` (`people`, `person_assets`, ...).

use super::{thumbs, Immagine};
use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::person::{PeopleOrder, Person};
use crate::logic::models::timeline::TimelineItem;
use crate::theme::{Palette, BLUE, RED};
use crate::host::{Current as H, Host};
use gpui_kit::*;
use std::rc::Rc;

/// People per request; the server pages through all of them (hidden ones included).
const PEOPLE_PAGE: u32 = 200;
/// Photos per request for a person page.
const PHOTO_PAGE: u32 = 500;
/// Upper bound on requests per listing, so a broken paging answer cannot loop forever.
const MAX_PAGES: u32 = 100;
/// Space between two portraits in a row.
const GAP: f32 = 14.;
/// Side gutter of the grid.
const PAD: f32 = 16.;
/// Height of the name label under a portrait, including the space above it.
const LABEL_H: f32 = 26.;
/// Vertical space between two rows of portraits.
const ROW_GAP: f32 = 18.;

/// Loading state of a list.
#[derive(Clone, Default, PartialEq)]
pub(super) enum Load {
    #[default]
    Idle,
    Loading,
    Done,
    Failed(String),
}

/// The person page: the person and the photos they appear on, newest first.
pub(super) struct PersonPage {
    pub(super) person: Person,
    pub(super) items: Rc<Vec<TimelineItem>>,
    pub(super) status: Load,
    pub(super) scroll: UniformListScrollHandle,
}

/// UI state of the People tab.
#[derive(Default)]
pub(super) struct PeopleState {
    /// Visible people in overview order: named ones alphabetically, unnamed ones last.
    pub(super) list: Rc<Vec<Person>>,
    pub(super) status: Load,
    /// The open person page, `None` on the overview.
    pub(super) open: Option<PersonPage>,
    pub(super) scroll: UniformListScrollHandle,
}

/// Every person the server knows, paging until the end.
async fn all_people(origin: &str, token: &str) -> Result<Vec<Person>, ClientError> {
    let client = ImmichClient::new(origin, H::transport())?;
    let mut all: Vec<Person> = Vec::new();
    for page in 1..=MAX_PAGES {
        let answer = client.people(page, PEOPLE_PAGE, token).await?;
        let got = answer.people.len();
        let more = match answer.has_next_page {
            Some(more) => more,
            None => got > 0 && ((all.len() + got) as i64) < answer.total,
        };
        all.extend(answer.people);
        if !more || got == 0 {
            break;
        }
    }
    Ok(all)
}

/// Every photo of one person, newest first; photos without a capture date sort last.
async fn person_photos(origin: &str, person_id: &str, token: &str) -> Result<Vec<TimelineItem>, ClientError> {
    let client = ImmichClient::new(origin, H::transport())?;
    let mut items: Vec<TimelineItem> = Vec::new();
    for page in 1..=MAX_PAGES {
        let answer = client.person_assets(person_id, page, PHOTO_PAGE, token).await?;
        let got = answer.assets.items.len();
        items.extend(answer.assets.items.iter().map(|asset| asset.timeline_item()));
        if answer.assets.next_page.is_none() || got == 0 {
            break;
        }
    }
    items.retain(|item| !item.is_trashed);
    items.sort_by(|a, b| b.sort_date.cmp(&a.sort_date));
    Ok(items)
}

impl Immagine {
    pub(super) fn people_view(&mut self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.people.open.is_some() {
            return self.person_page(p, window, cx);
        }
        // The first visit loads the list; later visits show what is there.
        if matches!(self.people.status, Load::Idle) {
            self.load_people(cx);
        }
        self.people_list(p, window, cx)
    }

    /// Fetches all people in the background. Does not notify, so it is safe to call while rendering.
    fn load_people(&mut self, cx: &mut Context<Self>) {
        if matches!(self.people.status, Load::Loading) {
            return;
        }
        self.people.status = Load::Loading;
        let origin = self.origin.clone();
        let token = self.token.clone();
        cx.spawn(async move |this, cx| {
            let result = all_people(&origin, &token).await;
            let _ = this.update(cx, |t, cx| {
                match result {
                    Ok(all) => {
                        t.people.list = Rc::new(PeopleOrder::for_picking(&PeopleOrder::overview(&all, "", false)));
                        t.people.status = Load::Done;
                    }
                    Err(ClientError::Unauthorized) => {
                        t.log_out(cx);
                        t.people = Default::default();
                        t.login_error = Some(ClientError::Unauthorized.to_string());
                    }
                    Err(e) => t.people.status = Load::Failed(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_person(&mut self, person: Person, cx: &mut Context<Self>) {
        let id = person.id.clone();
        self.people.open = Some(PersonPage { person, items: Rc::new(Vec::new()), status: Load::Loading, scroll: UniformListScrollHandle::new() });
        self.load_person_photos(id, cx);
        cx.notify();
    }

    fn load_person_photos(&mut self, id: String, cx: &mut Context<Self>) {
        let origin = self.origin.clone();
        let token = self.token.clone();
        cx.spawn(async move |this, cx| {
            let result = person_photos(&origin, &id, &token).await;
            let _ = this.update(cx, |t, cx| {
                // The user may have left the page or opened another person meanwhile.
                match result {
                    Ok(items) => {
                        if let Some(page) = t.people.open.as_mut().filter(|page| page.person.id == id) {
                            page.items = Rc::new(items);
                            page.status = Load::Done;
                        }
                    }
                    Err(ClientError::Unauthorized) => {
                        t.log_out(cx);
                        t.people = Default::default();
                        t.login_error = Some(ClientError::Unauthorized.to_string());
                    }
                    Err(e) => {
                        if let Some(page) = t.people.open.as_mut().filter(|page| page.person.id == id) {
                            page.status = Load::Failed(e.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn people_list(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let vw: f32 = window.viewport_size().width.into();
        let columns = if vw < 520. { 3 } else { ((vw / 140.).floor() as usize).clamp(3, 6) };
        let cell = ((vw - 2. * PAD - GAP * (columns as f32 - 1.)) / columns as f32).floor();
        let row_h = cell + LABEL_H + ROW_GAP;
        let footer = self.bottom.max(10.) + 84.;
        let list = self.people.list.clone();
        let rows = list.len().div_ceil(columns);
        let failed = matches!(self.people.status, Load::Failed(_));

        let (subtitle, color) = match &self.people.status {
            Load::Failed(e) => (e.clone(), rgb(RED).into()),
            Load::Done => (format!("{} Personen", list.len()), p.secondary),
            _ => ("Lädt …".to_string(), p.secondary),
        };

        let body = if list.is_empty() {
            let text = match &self.people.status {
                Load::Done => "Keine Personen".to_string(),
                Load::Failed(e) => format!("{e}\nZum Wiederholen tippen."),
                _ => "Lädt …".to_string(),
            };
            let mut empty = div()
                .id("people-empty")
                .flex_1()
                .px(px(PAD))
                .flex()
                .items_center()
                .justify_center()
                .text_center()
                .text_size(px(17.))
                .text_color(if failed { rgb(RED).into() } else { p.secondary })
                .child(text);
            if failed {
                empty = empty.active(|s| s.opacity(0.6)).on_click(cx.listener(|this, _, _, cx| {
                    this.load_people(cx);
                    cx.notify();
                }));
            }
            empty.into_any_element()
        } else {
            let view = cx.weak_entity();
            let store = self.thumbs.clone();
            let origin = self.origin.clone();
            let chip = p.chip;
            let label = p.label;
            let secondary = p.secondary;
            uniform_list("people", rows + 1, move |range, window, cx| {
                store.borrow_mut().begin_frame();
                for evicted in store.borrow_mut().take_evicted() {
                    let _ = window.drop_image(evicted);
                }
                range
                    .map(|row| {
                        if row >= rows {
                            return div().h(px(footer)).into_any_element();
                        }
                        let tiles = (0..columns)
                            .filter_map(|c| list.get(row * columns + c).map(|person| (row * columns + c, person)))
                            .map(|(idx, person)| {
                                let view = view.clone();
                                let key = format!("person:{}", person.id);
                                let url = format!("{origin}/api/people/{}/thumbnail", person.id);
                                let picture = thumbs::get(&store, &key, || url, cx);

                                let mut portrait = div()
                                    .size(px(cell))
                                    .flex_none()
                                    .rounded_full()
                                    .overflow_hidden()
                                    .bg(chip)
                                    .flex()
                                    .items_center()
                                    .justify_center();
                                portrait = match picture {
                                    Some(image) => portrait.child(img(image).size_full().rounded_full().object_fit(ObjectFit::Cover)),
                                    None => {
                                        let initial = person.name.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
                                        portrait.text_size(px(cell * 0.4)).text_color(secondary).child(initial)
                                    }
                                };

                                let name = person.display_name().to_string();
                                let target = person.clone();
                                div()
                                    .id(("person", idx))
                                    .w(px(cell))
                                    .flex_none()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .active(|s| s.opacity(0.6))
                                    .child(portrait)
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(LABEL_H))
                                            .pt(px(6.))
                                            .text_center()
                                            .truncate()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(label)
                                            .child(name),
                                    )
                                    .on_click(move |_, window, cx| {
                                        if window.tap_caught_fling() {
                                            return;
                                        }
                                        view.update(cx, |this, cx| this.open_person(target.clone(), cx)).ok();
                                    })
                            })
                            .collect::<Vec<_>>();
                        div().h(px(row_h)).px(px(PAD)).flex().gap(px(GAP)).children(tiles).into_any_element()
                    })
                    .collect()
            })
            .track_scroll(&self.people.scroll)
            .flex_1()
            .into_any_element()
        };

        let mut status = div().id("people-status").text_size(px(15.)).text_color(color).child(subtitle);
        if failed {
            status = status.active(|s| s.opacity(0.6)).on_click(cx.listener(|this, _, _, cx| {
                this.load_people(cx);
                cx.notify();
            }));
        }

        self.rubber
            .shift(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .pt(px(self.top.max(12.)))
                    .child(
                        div()
                            .px_4()
                            .pt(px(40.))
                            .pb_2()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(34.)).font_weight(FontWeight::BOLD).child("Personen"))
                            .child(status),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn person_page(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(page) = self.people.open.as_ref() else {
            return div().into_any_element();
        };
        let (subtitle, color) = match &page.status {
            Load::Done => (format!("{} Fotos", page.items.len()), p.secondary),
            Load::Failed(e) => (e.clone(), rgb(RED).into()),
            _ => ("Lädt …".to_string(), p.secondary),
        };
        let name = page.person.display_name().to_string();

        let back = div().flex().flex_row().child(super::icons::back_chip(p.glass_tint, "person-back", rgb(BLUE).into()).on_click(cx.listener(|this, _, _, cx| {
            this.people.open = None;
            cx.notify();
        })));

        let header = div()
            .px_4()
            .pt(px(40.))
            .pb_2()
            .flex()
            .flex_col()
            .child(back)
            .child(div().pt_3().text_size(px(34.)).font_weight(FontWeight::BOLD).child(name))
            .child(div().text_size(px(15.)).text_color(color).child(subtitle));

        let grid = self.photo_grid("person", page.items.clone(), 0., (&page.scroll, None), p, window, cx);

        self.rubber.shift(div().size_full().flex().flex_col().pt(px(self.top.max(12.))).child(header).child(grid)).into_any_element()
    }
}
