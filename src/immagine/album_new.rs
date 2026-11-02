//! The album form, one sheet for two jobs. "Neues Album" (plus button in the Alben header) asks for name and
//! description; "Album bearbeiten" (the "..." button on an album page) changes both and deletes the album.
//! Inside a folder a new album gets the folder's path as prefix (`AlbumPath::join`), so it appears there; renaming
//! keeps the album's folder. Everything goes to the server first (`POST`/`PATCH`/`DELETE /api/albums`), the answer
//! is merged into the store, so nothing is shown that the server refused. The cover is set from the viewer's menu
//! (`set_cover`).

use super::Immagine;
use crate::components::button;
use crate::components::sheet::{self, Sheet};
use crate::host::{Current as H, Host};
use crate::logic::client::{AlbumUpdate, ImmichClient};
use crate::logic::models::album_tree::AlbumPath;
use crate::theme::{Palette, BLUE, RED};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;

#[derive(Default, Clone, PartialEq)]
enum Mode {
    #[default]
    Create,
    /// The album's id.
    Edit(String),
}

#[derive(Default)]
pub(super) struct NewAlbumState {
    pub(super) sheet: Sheet,
    name: Option<Entity<InputState>>,
    description: Option<Entity<InputState>>,
    mode: Mode,
    busy: bool,
    /// "Album löschen" was tapped: the sheet asks once more.
    confirm_delete: bool,
    error: Option<String>,
}

impl Immagine {
    fn ensure_album_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_album.name.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Albumname"));
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    window.dismiss_virtual_keyboard();
                    this.save_album(cx);
                }
            })
            .detach();
            self.new_album.name = Some(input);
        }
        if self.new_album.description.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Beschreibung (optional)"));
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    window.dismiss_virtual_keyboard();
                    this.save_album(cx);
                }
            })
            .detach();
            self.new_album.description = Some(input);
        }
    }

    fn open_album_form(&mut self, mode: Mode, name: &str, description: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_album_inputs(window, cx);
        if let Some(input) = self.new_album.name.clone() {
            input.update(cx, |state, cx| state.set_value(name, window, cx));
            input.update(cx, |state, cx| state.focus(window, cx));
            window.request_virtual_keyboard();
        }
        if let Some(input) = self.new_album.description.clone() {
            input.update(cx, |state, cx| state.set_value(description, window, cx));
        }
        self.new_album.mode = mode;
        self.new_album.error = None;
        self.new_album.busy = false;
        self.new_album.confirm_delete = false;
        self.new_album.sheet.open();
        cx.notify();
    }

    pub(super) fn open_new_album(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_album_form(Mode::Create, "", "", window, cx);
    }

    /// The "..." button of an album page.
    pub(super) fn open_edit_album(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(album) = self.albums.albums.iter().find(|a| a.id == id).cloned() else { return };
        let leaf = AlbumPath::leaf_of_album_name(&album.album_name);
        self.open_album_form(Mode::Edit(id.to_string()), &leaf, album.description.as_deref().unwrap_or(""), window, cx);
    }

    fn close_new_album(&mut self, cx: &mut Context<Self>) {
        self.new_album.sheet.close();
        cx.notify();
    }

    fn form_values(&self, cx: &App) -> (String, String) {
        let read = |input: &Option<Entity<InputState>>| input.as_ref().map(|i| i.read(cx).value().trim().to_string()).unwrap_or_default();
        (read(&self.new_album.name), read(&self.new_album.description))
    }

    fn save_album(&mut self, cx: &mut Context<Self>) {
        if self.new_album.busy {
            return;
        }
        let (name, description) = self.form_values(cx);
        if name.is_empty() {
            self.new_album.error = Some("Bitte einen Namen eingeben.".into());
            cx.notify();
            return;
        }
        self.new_album.busy = true;
        self.new_album.error = None;
        let (origin, token) = (self.origin.clone(), self.token.clone());
        let mode = self.new_album.mode.clone();
        // New: below the folder that is open. Existing: the album stays in its folder.
        let full = match &mode {
            Mode::Create => AlbumPath::join(&self.albums.path, &name),
            Mode::Edit(id) => {
                let old = self.albums.albums.iter().find(|a| &a.id == id).map(|a| a.album_name.clone()).unwrap_or_default();
                AlbumPath::join(&AlbumPath::parent_of(&old), &name)
            }
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match ImmichClient::new(&origin, H::transport()) {
                Ok(client) => match &mode {
                    Mode::Create => client.create_album(&full, &description, &[], &token).await,
                    Mode::Edit(id) => client.update_album(id, &AlbumUpdate { album_name: Some(&full), description: Some(&description), ..Default::default() }, &token).await,
                }
                .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = this.update(cx, |t, cx| {
                t.new_album.busy = false;
                match result {
                    Ok(album) => {
                        t.new_album.sheet.close();
                        t.merge_album(album, cx);
                        if mode == Mode::Create {
                            t.sync(cx);
                        }
                    }
                    Err(e) => t.new_album.error = Some(format!("Album konnte nicht gespeichert werden: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Takes the server's answer for one album into the store and the open page, and stores the snapshot.
    pub(super) fn merge_album(&mut self, album: crate::logic::models::album::Album, cx: &mut Context<Self>) {
        use crate::logic::store::Store;
        let Some(engine) = self.engine.as_mut() else { return };
        engine.store_mut().upsert_albums(&[album]);
        self.albums.refresh(engine.store());
        let snapshot = engine.store().snapshot().ok();
        cx.notify();
        if let Some(bytes) = snapshot {
            cx.spawn(async move |_, _| {
                let _ = H::blob_put(super::DB, super::SNAPSHOT_KEY, &bytes).await;
            })
            .detach();
        }
    }

    fn delete_album(&mut self, cx: &mut Context<Self>) {
        let Mode::Edit(id) = self.new_album.mode.clone() else { return };
        if self.new_album.busy {
            return;
        }
        self.new_album.busy = true;
        self.new_album.error = None;
        let (origin, token) = (self.origin.clone(), self.token.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match ImmichClient::new(&origin, H::transport()) {
                Ok(client) => client.delete_album(&id, &token).await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = this.update(cx, |t, cx| {
                use crate::logic::store::Store;
                t.new_album.busy = false;
                match result {
                    Ok(()) => {
                        t.new_album.sheet.close();
                        t.new_album.confirm_delete = false;
                        t.leave_album_page();
                        if let Some(engine) = t.engine.as_mut() {
                            engine.store_mut().delete_album(&id);
                            t.albums.refresh(engine.store());
                            let snapshot = engine.store().snapshot().ok();
                            if let Some(bytes) = snapshot {
                                cx.spawn(async move |_, _| {
                                    let _ = H::blob_put(super::DB, super::SNAPSHOT_KEY, &bytes).await;
                                })
                                .detach();
                            }
                        }
                    }
                    Err(e) => t.new_album.error = Some(format!("Album konnte nicht gelöscht werden: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn new_album_overlay(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = &self.new_album;
        if !state.sheet.visible() {
            return None;
        }
        let (name, description) = (state.name.as_ref()?, state.description.as_ref()?);
        let editing = matches!(state.mode, Mode::Edit(_));
        let field = |input: &Entity<InputState>| {
            div()
                .h(px(52.))
                .px(px(16.))
                .rounded(px(14.))
                .bg(p.ui.card)
                .flex()
                .items_center()
                .child(div().flex_1().child(Input::new(input).appearance(false).bordered(false).cleanable(false)))
        };
        let mut body = div().px_4().pt_2().pb(px(self.bottom.max(10.) + 16.)).flex().flex_col().gap_3();
        if state.confirm_delete {
            body = body
                .child(div().text_size(px(17.)).child("Das Album wird gelöscht. Die Fotos darin bleiben in deiner Mediathek."))
                .child(
                    button::filled("album-delete-confirm", rgb(RED), 50., 14., if state.busy { "Wird gelöscht …" } else { "Album löschen" })
                        .text_size(px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0xffffff))
                        .on_click(cx.listener(|this, _, _, cx| this.delete_album(cx))),
                )
                .child(
                    button::filled("album-delete-cancel", p.ui.card, 50., 14., "Abbrechen")
                        .text_size(px(17.))
                        .text_color(p.label)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.new_album.confirm_delete = false;
                            cx.notify();
                        })),
                );
        } else {
            body = body.child(field(name)).child(field(description));
            let label = match (state.busy, editing) {
                (true, _) => "Wird gespeichert …",
                (false, true) => "Sichern",
                (false, false) => "Erstellen",
            };
            body = body.child(
                button::filled("new-album-create", rgb(BLUE), 50., 14., label)
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xffffff))
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.dismiss_virtual_keyboard();
                        this.save_album(cx)
                    })),
            );
            if editing {
                body = body.child(
                    button::filled("album-delete", p.ui.card, 50., 14., "Album löschen")
                        .text_size(px(17.))
                        .text_color(rgb(RED))
                        .on_click(cx.listener(|this, _, window, cx| {
                            window.dismiss_virtual_keyboard();
                            this.new_album.confirm_delete = true;
                            cx.notify();
                        })),
                );
            }
        }
        if let Some(error) = &state.error {
            body = body.child(div().text_size(px(14.)).text_color(rgb(RED)).child(error.clone()));
        }
        let title = if editing { "Album bearbeiten" } else { "Neues Album" };
        Some(sheet::overlay(
            &state.sheet,
            |this: &Immagine| &this.new_album.sheet,
            cx,
            window,
            sheet::Spec { id: "new-album", title, top_margin: self.top.max(12.) + 10., theme: &p.ui },
            body,
            |this, cx| this.close_new_album(cx),
        ))
    }
}
