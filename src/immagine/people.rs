//! Owner: people agent. Swift sources: `../immagine/Immagine/UI/People/*.swift`, `People/PeopleStore.swift`.
//! People tab: faces recognized by Immich; person page with their photos (`photo_grid`).
//! People are not in the `Store`; they are fetched on demand through `ImmichClient` (`people`, `person_assets`, ...).

use super::Immagine;
use crate::logic::models::person::Person;
use crate::theme::Palette;
use gpui_kit::*;

/// UI state of the People tab; extend as needed.
#[derive(Default)]
pub(super) struct PeopleState {
    pub(super) people: Vec<Person>,
}

impl Immagine {
    pub(super) fn people_view(&mut self, p: &Palette, _window: &Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(17.))
            .text_color(p.secondary)
            .child("Personen folgt bald")
            .into_any_element()
    }
}
