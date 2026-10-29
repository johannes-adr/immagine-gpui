//! Immagine: the Immich client app. `Immagine` is the root view; `login` and `timeline` are its screens.
//!
//! Sync runs off a `SyncEngine` that is moved into the sync task and handed back when it ends, so the
//! UI never borrows the store while it is being written. The UI reads a copy of the timeline.

mod albums;
mod login;
mod people;
mod icons;
mod tab_bar;
mod thumbs;
mod timeline;
mod viewer;

use crate::components::fps::{fps_badge, FpsMeter};
use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::timeline::TimelineItem;
use crate::logic::store::{MemoryStore, Store};
use crate::logic::sync::engine::SyncEngine;
use crate::theme::Palette;
use crate::web_adapters::{fetch::FetchTransport, idb, local_storage, mark, WebClock};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const DB: &str = "immagine";
const SNAPSHOT_KEY: &str = "store";
const TOKEN_KEY: &str = "immagine.token";

type Engine = SyncEngine<MemoryStore, FetchTransport, WebClock>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Loading,
    Login,
    Timeline,
}

pub(crate) struct Immagine {
    screen: Screen,
    origin: String,
    token: String,
    engine: Option<Engine>,
    items: Rc<Vec<TimelineItem>>,
    syncing: bool,
    status: String,
    // Login form.
    email: Entity<InputState>,
    password: Entity<InputState>,
    login_error: Option<String>,
    logging_in: bool,
    // Layout and appearance reported by the page.
    top: f32,
    bottom: f32,
    dark: bool,
    meter: Rc<FpsMeter>,
    fps: u32,
    worst_ms: u32,
    webgpu: bool,
    tab: usize,
    tab_gen: u32,
    // Tab bar thumb: drag in progress, where it started its last move (tab units) and the bubble size then.
    tab_drag: Option<tab_bar::TabDrag>,
    tab_from: f32,
    bubble_from: f32,
    thumb_gen: u32,
    /// Screen bounds of the tiles painted last, so the viewer can zoom out of the tapped one.
    tile_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    viewer: Option<viewer::Viewer>,
    viewer_serial: u32,
    /// Scroll position of each photo grid, kept here so a grid can stop rendering while the viewer covers it.
    grid_scroll: std::cell::RefCell<std::collections::HashMap<&'static str, UniformListScrollHandle>>,
    thumbs: thumbs::SharedThumbs,
    pinch: Option<crate::web_adapters::pinch::PinchHandle>,
    albums: albums::AlbumsState,
    people: people::PeopleState,
}

impl Immagine {
    pub(crate) fn new(top: f32, bottom: f32, dark: bool, webgpu: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| InputState::new(window, cx).placeholder("E-Mail"));
        let password = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Passwort");
            state.set_masked(true, window, cx);
            state
        });
        cx.subscribe_in(&password, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.log_in(cx);
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |t, cx| t.tick_fps(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
        let mut this = Self {
            screen: Screen::Loading,
            origin,
            token: String::new(),
            engine: None,
            items: Rc::new(Vec::new()),
            syncing: false,
            status: String::new(),
            email,
            password,
            login_error: None,
            logging_in: false,
            top,
            bottom,
            dark,
            meter: Rc::new(FpsMeter::default()),
            fps: 0,
            worst_ms: 0,
            webgpu,
            tab: 0,
            tab_gen: 0,
            tab_drag: None,
            tab_from: 0.,
            bubble_from: 0.,
            thumb_gen: 0,
            tile_bounds: Rc::new(RefCell::new(HashMap::new())),
            viewer: None,
            viewer_serial: 0,
            grid_scroll: Default::default(),
            thumbs: thumbs::new(cx.weak_entity()),
            pinch: None,
            albums: Default::default(),
            people: Default::default(),
        };
        this.resume(cx);
        this
    }

    fn tick_fps(&mut self, cx: &mut Context<Self>) {
        let Some((fps, worst)) = self.meter.reading() else {
            return;
        };
        if (fps, worst) != (self.fps, self.worst_ms) {
            self.fps = fps;
            self.worst_ms = worst;
            self.meter.skip_next_paint();
            cx.notify();
        }
    }

    /// Start-up: with a saved session, show the stored timeline at once and sync in the background.
    fn resume(&mut self, cx: &mut Context<Self>) {
        let Some(token) = local_storage::get(TOKEN_KEY).filter(|t| !t.is_empty()) else {
            self.screen = Screen::Login;
            return;
        };
        self.token = token;
        cx.spawn(async move |this, cx| {
            mark("resume: start");
            let bytes = idb::get(DB, SNAPSHOT_KEY).await;
            mark("resume: snapshot read");
            let store = match bytes {
                Ok(Some(bytes)) => {
                    mark(&format!("resume: restoring {} bytes", bytes.len()));
                    MemoryStore::restore(&bytes).unwrap_or_else(|_| MemoryStore::new())
                }
                _ => MemoryStore::new(),
            };
            mark("resume: restored");
            let _ = this.update(cx, |t, cx| {
                t.install_engine(store, cx);
                mark("resume: engine installed (timeline built)");
                t.sync(cx);
            });
        })
        .detach();
    }

    fn install_engine(&mut self, store: MemoryStore, cx: &mut Context<Self>) {
        match SyncEngine::new(store, FetchTransport, WebClock, &self.origin) {
            Ok(engine) => {
                self.items = Rc::new(engine.store().fetch_timeline());
                self.albums.refresh(engine.store());
                self.engine = Some(engine);
                self.screen = Screen::Timeline;
            }
            Err(e) => {
                self.login_error = Some(e.to_string());
                self.screen = Screen::Login;
            }
        }
        cx.notify();
    }

    fn log_in(&mut self, cx: &mut Context<Self>) {
        if self.logging_in {
            return;
        }
        let email = self.email.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        if email.is_empty() || password.is_empty() {
            return;
        }
        self.logging_in = true;
        self.login_error = None;
        let origin = self.origin.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let client = ImmichClient::new(&origin, FetchTransport)?;
                let auth = client.login(&email, &password).await?;
                auth.access_token.ok_or_else(|| ClientError::Decoding("Die Antwort enthält kein Token.".into()))
            }
            .await;
            let _ = this.update(cx, |t, cx| {
                t.logging_in = false;
                match result {
                    Ok(token) => {
                        local_storage::set(TOKEN_KEY, &token);
                        t.token = token;
                        t.install_engine(MemoryStore::new(), cx);
                        t.sync(cx);
                    }
                    Err(e) => t.login_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn log_out(&mut self, cx: &mut Context<Self>) {
        local_storage::remove(TOKEN_KEY);
        self.token.clear();
        self.engine = None;
        self.items = Rc::new(Vec::new());
        self.albums = Default::default();
        self.people = Default::default();
        self.viewer = None;
        self.tab = 0;
        self.screen = Screen::Login;
        cx.spawn(async move |_, _| {
            let _ = idb::put(DB, SNAPSHOT_KEY, b"").await;
        })
        .detach();
        cx.notify();
    }

    /// Runs one sync. The engine travels into the task and comes back with the result.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(mut engine) = self.engine.take() else {
            return;
        };
        self.syncing = true;
        self.status = "Synchronisiere …".into();
        let token = self.token.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            mark("sync: start");
            let outcome = engine.sync(&token).await;
            mark("sync: engine.sync done");
            let changed = match &outcome {
                Ok(run) => run.full_sync || run.stream.assets_changed,
                Err(_) => false,
            };
            let items = changed.then(|| engine.store().fetch_timeline());
            let snapshot = if changed { engine.store().snapshot().ok() } else { None };
            mark("sync: timeline + snapshot built");
            let _ = this.update(cx, |t, cx| {
                t.engine = Some(engine);
                if let Some(e) = &t.engine {
                    t.albums.refresh(e.store());
                }
                t.syncing = false;
                match &outcome {
                    Ok(_) => t.status.clear(),
                    Err(ClientError::Unauthorized) => {
                        t.log_out(cx);
                        t.login_error = Some(ClientError::Unauthorized.to_string());
                    }
                    Err(e) => t.status = e.to_string(),
                }
                if let Some(items) = items {
                    t.items = Rc::new(items);
                }
                cx.notify();
            });
            if let Some(bytes) = snapshot {
                let _ = idb::put(DB, SNAPSHOT_KEY, &bytes).await;
                mark("sync: snapshot stored");
            }
        })
        .detach();
    }
}

impl Render for Immagine {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::new(self.dark);
        let background = if self.dark { rgb(0x000000) } else { rgb(0xffffff) };
        let content = match self.screen {
            Screen::Loading => div().into_any_element(),
            Screen::Login => self.login_view(&p, cx).into_any_element(),
            Screen::Timeline if self.tab == 0 => self.timeline_view(&p, window, cx).into_any_element(),
            Screen::Timeline if self.tab == 1 => self.albums_view(&p, window, cx),
            Screen::Timeline => self.people_view(&p, window, cx),
        };
        // The renderer copies the glass backdrop once per frame at the first glass element; with the tab bar
        // gone while the viewer is open, the viewer's own glass sees the photo instead of the grid.
        let viewer_open = self.viewer.as_ref().is_some_and(|v| !v.closing);
        let chrome = (self.screen == Screen::Timeline && !viewer_open).then(|| self.tab_bar(&p, window, cx));
        let viewer = if self.screen == Screen::Timeline { self.viewer_overlay(window, cx) } else { None };
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(background)
            .text_color(p.label)
            .child(content)
            .children(chrome)
            .children(viewer)
            // FPS debug badge, re-enable when profiling:
            // .child(fps_badge(self.meter.clone(), self.top, self.fps, self.worst_ms, self.webgpu))
    }
}
