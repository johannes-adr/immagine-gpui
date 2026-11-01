//! Immagine: the Immich client app. `Immagine` is the root view; `login` and `timeline` are its screens.
//!
//! Sync runs off a `SyncEngine` that is moved into the sync task and handed back when it ends, so the
//! UI never borrows the store while it is being written. The UI reads a copy of the timeline.

mod albums;
mod downloads;
mod login;
mod people;
mod range_views;
mod slider;
mod icons;
mod search;
mod selection;
mod scrollbar;
mod jobs;
mod settings;
mod tab_bar;
mod thumbs;
mod timeline;
mod video_player;
mod viewer;

use crate::components::rubber_band::{self, RubberBand};
use crate::components::fps::{fps_badge, FpsMeter};
use crate::logic::client::{ClientError, ImmichClient};
use crate::logic::models::timeline::TimelineItem;
use crate::logic::store::{MemoryStore, Store};
use crate::logic::sync::engine::SyncEngine;
use crate::theme::Palette;
use crate::host::{Current as H, GestureHandle, Host, SystemClock, Transport};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

pub(super) const DB: &str = "immagine";
pub(super) const SNAPSHOT_KEY: &str = "store";
const TOKEN_KEY: &str = "immagine.token";
/// Persisted debug switch of the settings sheet ("1" = on): shows the FPS badge.
const DEBUG_KEY: &str = "immagine.debug";
/// The server typed at login; the hosts read it to know where to send requests.
const SERVER_KEY: &str = "immagine.server";

type Engine = SyncEngine<MemoryStore, Transport, SystemClock>;

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
    server: Entity<InputState>,
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
    albums_repaired: bool,
    debug: bool,
    webgpu: bool,
    tab: usize,
    tab_gen: u32,
    /// Glass thumbs of the tab bar and of the Jahre / Monate / Alle bar.
    tabs_slider: slider::Slider,
    range_slider: slider::Slider,
    /// Scrolled away from the newest photos, the tab bar is replaced by the Jahre / Monate / Alle bar.
    range_bar: bool,
    range_mode: usize,
    /// Where the list was (px from its end) when the Mediathek button was tapped; the tab bar stays meanwhile.
    range_latch: Option<f32>,
    /// Jahre / Monate lists: their scroll handles, whether they were put at the newest end, the groups of the
    /// current items, and rows other views ask them (or the photo grid) to bring into view once.
    years_scroll: UniformListScrollHandle,
    months_scroll: UniformListScrollHandle,
    years_placed: std::cell::Cell<bool>,
    months_placed: std::cell::Cell<bool>,
    months_jump: std::cell::Cell<Option<usize>>,
    timeline_jump: std::cell::Cell<Option<usize>>,
    range_cache: RefCell<Option<(Rc<Vec<TimelineItem>>, Rc<crate::logic::range_groups::Groups>)>>,
    /// When the timeline was last told to jump to its end; until it has, its scroll position says nothing.
    scroll_settle: std::cell::Cell<f64>,
    swap_at: f64,
    swap_gen: u32,
    /// The timeline shows the newest photos at the bottom and starts scrolled there.
    timeline_scroll: UniformListScrollHandle,
    timeline_placed: std::cell::Cell<bool>,
    /// Columns of the timeline grid (a step of `ZOOM_LEVELS`), the pinch scale gathered since the last step, the
    /// photos (first, last index) the grid built last, and the span the header shows.
    /// Settled columns of the timeline grid (0 = not chosen yet: 3 on phones, 5 on wide screens).
    grid_cols: usize,
    /// Columns shown right now (fractional while pinching or settling), see `timeline.rs`.
    grid_f: f32,
    grid_from: usize,
    /// The photo held still while zooming, and where in its row (0..1) the middle of the screen was when the
    /// pinch began.
    grid_anchor: (usize, f32),
    /// (row height, empty rows above the photos, middle of the screen in the list's box) of the last timeline layout.
    grid_geom: std::cell::Cell<(f32, usize, f32)>,
    grid_pinching: bool,
    /// The user zoomed the grid: the width-based default no longer applies.
    grid_user: bool,
    /// (start, columns at start, target level) of the settle animation after a pinch.
    grid_anim: Option<(f64, f32, usize)>,
    /// Last ctrl+wheel (trackpad pinch in browsers); a pause settles the gesture.
    grid_wheel_at: f64,
    /// Opacity of the grid's photos (dips while the layout switches to another column level).
    grid_fade: f32,
    grid_fade_at: f64,
    visible: Rc<std::cell::Cell<(usize, usize)>>,
    header_span: std::cell::Cell<(usize, usize)>,
    /// Rubber band shared by every scroll view (timeline, albums, people, their pages).
    rubber: RubberBand,
    albums_scroll: ScrollHandle,
    /// Screen bounds of the tiles painted last, so the viewer can zoom out of the tapped one.
    tile_bounds: Rc<RefCell<HashMap<usize, TileRect>>>,
    grid_bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    viewer: Option<viewer::Viewer>,
    viewer_serial: u32,
    thumbs: thumbs::SharedThumbs,
    pinch: Option<GestureHandle>,
    albums: albums::AlbumsState,
    people: people::PeopleState,
    search: search::SearchState,
    settings: settings::SettingsState,
    jobs: jobs::JobsState,
    downloads: downloads::Downloads,
    select: selection::SelectState,
    scrollbar: scrollbar::ScrollbarState,
}

/// The timeline is stored newest first; the grid reads oldest first, so the newest photos sit at the bottom.
fn oldest_first(mut items: Vec<TimelineItem>) -> Vec<TimelineItem> {
    items.reverse();
    items
}

impl Immagine {
    /// Height the on-screen keyboard covers at the bottom of the window (0 without one). Web reports it through
    /// the visual viewport, UIKit through the window insets.
    pub(crate) fn keyboard_height(window: &Window) -> f32 {
        let viewport: f32 = window.viewport_size().height.into();
        let visible = window.visual_viewport_bounds();
        let by_viewport = viewport - f32::from(visible.origin.y + visible.size.height);
        by_viewport.max(f32::from(window.insets().ime.bottom)).max(0.)
    }

    pub(crate) fn new(top: f32, bottom: f32, dark: bool, webgpu: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_server = H::origin();
        let server = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Server, z. B. https://immich.example.com");
            state.set_value(initial_server, window, cx);
            state
        });
        let email = cx.new(|cx| InputState::new(window, cx).placeholder("E-Mail"));
        let password = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Passwort");
            state.set_masked(true, window, cx);
            state
        });
        cx.subscribe_in(&password, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                window.dismiss_virtual_keyboard();
                this.log_in(cx);
            }
        })
        .detach();

        // Follows the OS appearance while running (the web host passes it once at start-up).
        let weak = cx.weak_entity();
        window
            .observe_window_appearance(move |window, cx| {
                let dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
                weak.update(cx, |t, cx| {
                    t.dark = dark;
                    cx.notify();
                })
                .ok();
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

        let origin = H::origin();
        let downloads = downloads::Downloads::default();
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
            server,
            top,
            bottom,
            dark,
            meter: Rc::new(FpsMeter::default()),
            fps: 0,
            albums_repaired: false,
            debug: H::setting_get(DEBUG_KEY).as_deref() == Some("1"),
            worst_ms: 0,
            webgpu,
            tab: 0,
            tab_gen: 0,
            tabs_slider: Default::default(),
            range_slider: Default::default(),
            range_bar: false,
            range_mode: 2,
            range_latch: None,
            years_scroll: UniformListScrollHandle::new(),
            months_scroll: UniformListScrollHandle::new(),
            years_placed: std::cell::Cell::new(false),
            months_placed: std::cell::Cell::new(false),
            months_jump: std::cell::Cell::new(None),
            timeline_jump: std::cell::Cell::new(None),
            range_cache: RefCell::new(None),
            scroll_settle: std::cell::Cell::new(f64::NEG_INFINITY),
            swap_at: f64::NEG_INFINITY,
            swap_gen: 0,
            timeline_scroll: UniformListScrollHandle::new(),
            timeline_placed: std::cell::Cell::new(false),
            grid_cols: 0,
            grid_f: 3.,
            grid_from: 3,
            grid_anchor: (0, 0.5),
            grid_geom: std::cell::Cell::new((0., 0, 0.)),
            grid_pinching: false,
            grid_user: false,
            grid_anim: None,
            grid_wheel_at: 0.,
            grid_fade: 1.,
            grid_fade_at: 0.,
            visible: Rc::new(std::cell::Cell::new((0, 0))),
            header_span: std::cell::Cell::new((0, 0)),
            rubber: Default::default(),
            albums_scroll: ScrollHandle::new(),
            tile_bounds: Rc::new(RefCell::new(HashMap::new())),
            grid_bounds: Rc::new(std::cell::Cell::new(Bounds::default())),
            viewer: None,
            viewer_serial: 0,
            thumbs: thumbs::new(cx.weak_entity(), downloads.ids.clone()),
            pinch: None,
            albums: Default::default(),
            people: Default::default(),
            search: Default::default(),
            settings: Default::default(),
            jobs: Default::default(),
            downloads,
            select: Default::default(),
            scrollbar: Default::default(),
        };
        this.resume(cx);
        this.ensure_gestures(cx);
        this.load_downloads(cx);
        thumbs::init_cache(&this.thumbs, cx);
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
        let Some(token) = H::setting_get(TOKEN_KEY).filter(|t| !t.is_empty()) else {
            self.screen = Screen::Login;
            return;
        };
        self.token = token;
        cx.spawn(async move |this, cx| {
            H::mark("resume: start");
            let bytes = H::blob_get(DB, SNAPSHOT_KEY).await;
            H::mark("resume: snapshot read");
            let store = match bytes {
                Ok(Some(bytes)) => {
                    H::mark(&format!("resume: restoring {} bytes", bytes.len()));
                    MemoryStore::restore(&bytes).unwrap_or_else(|_| MemoryStore::new())
                }
                _ => MemoryStore::new(),
            };
            H::mark("resume: restored");
            let _ = this.update(cx, |t, cx| {
                t.install_engine(store, cx);
                H::mark("resume: engine installed (timeline built)");
                t.sync(cx);
            });
        })
        .detach();
    }

    fn install_engine(&mut self, store: MemoryStore, cx: &mut Context<Self>) {
        match SyncEngine::new(store, H::transport(), H::clock(), &self.origin) {
            Ok(engine) => {
                self.items = Rc::new(oldest_first(engine.store().fetch_timeline()));
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
        let mut server = self.server.read(cx).value().trim().trim_end_matches('/').to_string();
        if !server.is_empty() && !server.contains("://") {
            server = format!("https://{server}");
        }
        if server.is_empty() {
            self.login_error = Some("Bitte eine Server-Adresse eingeben.".into());
            cx.notify();
            return;
        }
        self.logging_in = true;
        self.login_error = None;
        // Saved before the first request: the transport resolves its paths against the saved server.
        H::setting_set(SERVER_KEY, &server);
        self.origin = server.clone();
        let origin = server;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let client = ImmichClient::new(&origin, H::transport())?;
                let auth = client.login(&email, &password).await?;
                auth.access_token.ok_or_else(|| ClientError::Decoding("Die Antwort enthält kein Token.".into()))
            }
            .await;
            let _ = this.update(cx, |t, cx| {
                t.logging_in = false;
                match result {
                    Ok(token) => {
                        H::setting_set(TOKEN_KEY, &token);
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

    pub(super) fn log_out(&mut self, cx: &mut Context<Self>) {
        H::setting_remove(TOKEN_KEY);
        self.token.clear();
        self.engine = None;
        self.items = Rc::new(Vec::new());
        self.timeline_placed.set(false);
        self.albums = Default::default();
        self.people = Default::default();
        self.search = Default::default();
        self.viewer = None;
        self.select_end();
        self.settings.sheet.close();
        self.jobs.reset();
        self.tab = 0;
        self.screen = Screen::Login;
        self.downloads.ids.borrow_mut().clear();
        self.downloads.busy.clear();
        thumbs::clear_cache(&self.thumbs);
        cx.spawn(async move |_, _| {
            let _ = H::blob_clear(jobs::JOBS_DB).await;
            let _ = H::blob_clear(downloads::DOWNLOAD_DB).await;
            let _ = H::blob_clear(thumbs::THUMB_CACHE_DB).await;
            let _ = H::blob_put(DB, SNAPSHOT_KEY, b"").await;
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
            H::mark("sync: start");
            let outcome = engine.sync(&token).await;
            H::mark("sync: engine.sync done");
            let changed = match &outcome {
                Ok(run) => run.full_sync || run.stream.assets_changed,
                Err(_) => false,
            };
            let items = changed.then(|| engine.store().fetch_timeline());
            let snapshot = if changed { engine.store().snapshot().ok() } else { None };
            H::mark("sync: timeline + snapshot built");
            let _ = this.update(cx, |t, cx| {
                t.engine = Some(engine);
                if let Some(e) = &t.engine {
                    t.albums.refresh(e.store());
                }
                t.syncing = false;
                // Once per launch, after the first sync: fill album links the stream never delivered (see `repair_albums`).
                if outcome.is_ok() && !std::mem::replace(&mut t.albums_repaired, true) {
                    let ids = t.albums.album_ids();
                    t.repair_albums(ids, cx);
                }
                match &outcome {
                    Ok(_) => t.status.clear(),
                    Err(ClientError::Unauthorized) => {
                        t.log_out(cx);
                        t.login_error = Some(ClientError::Unauthorized.to_string());
                    }
                    Err(e) => t.status = e.to_string(),
                }
                if let Some(items) = items {
                    t.items = Rc::new(oldest_first(items));
                }
                cx.notify();
            });
            if let Some(bytes) = snapshot {
                let _ = H::blob_put(DB, SNAPSHOT_KEY, &bytes).await;
                H::mark("sync: snapshot stored");
            }
        })
        .detach();
    }
}

/// A grid tile as it was last painted: its screen bounds and the scroll offset of its list at that time.
#[derive(Clone)]
pub(super) struct TileRect {
    bounds: Bounds<Pixels>,
    offset: Pixels,
    scroll: ScrollHandle,
}

impl TileRect {
    /// Where the tile is now: the painted bounds moved by how far its list scrolled since, so the answer stays
    /// right for a tile that scrolled out of the rows the list builds.
    pub(super) fn now(&self) -> Bounds<Pixels> {
        let mut b = self.bounds;
        b.origin.y += self.scroll.offset().y - self.offset;
        b
    }
}

impl Immagine {
    /// The scroll handle of whatever scrolls on screen now, for the rubber band; `None` while the viewer is open
    /// or on screens without a scroll view.
    fn active_scroll(&self) -> Option<ScrollHandle> {
        if self.screen != Screen::Timeline || self.viewer.as_ref().is_some_and(|v| !v.closing) || self.settings.sheet.visible() || self.jobs.sheet.visible() {
            return None;
        }
        let list = |h: &UniformListScrollHandle| h.0.borrow().base_handle.clone();
        Some(match self.tab {
            0 if self.search.open => list(&self.search.scroll),
            0 => match self.range_mode {
                0 => list(&self.years_scroll),
                1 => list(&self.months_scroll),
                _ => list(&self.timeline_scroll),
            },
            1 => match self.albums.open_scroll() {
                Some(scroll) => list(scroll),
                None => self.albums_scroll.clone(),
            },
            _ => match self.people.open.as_ref() {
                Some(page) => list(&page.scroll),
                None => list(&self.people.scroll),
            },
        })
    }
}

impl Render for Immagine {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::new(self.dark);
        // Native hosts know the safe area only once the window exists and report changes later (rotation,
        // keyboard); the web host passes it once through `new`, and reports none here.
        let safe = window.insets().safe_area;
        let (safe_top, safe_bottom) = (f32::from(safe.top), f32::from(safe.bottom));
        if safe_top > 0. || safe_bottom > 0. {
            self.top = safe_top;
            self.bottom = safe_bottom;
        }
        self.rubber.tick(window);
        self.settings.sheet.tick(window);
        self.settings.rubber.tick(window);
        self.jobs.sheet.tick(window);
        self.jobs.rubber.tick(window);
        self.grid_default(window.viewport_size().width.into());
        self.grid_tick(window);
        self.select_check();
        self.sel_tick(window);
        self.sb_check();
        let background = if self.dark { rgb(0x000000) } else { rgb(0xffffff) };
        let content = match self.screen {
            Screen::Loading => div().into_any_element(),
            Screen::Login => self.login_view(&p, window, cx).into_any_element(),
            Screen::Timeline if self.tab == 0 && self.search.open => self.search_view(&p, window, cx),
            Screen::Timeline if self.tab == 0 => match self.range_mode {
                0 => self.years_view(&p, window, cx),
                1 => self.months_view(&p, window, cx),
                _ => self.timeline_view(&p, window, cx).into_any_element(),
            },
            Screen::Timeline if self.tab == 1 => self.albums_view(&p, window, cx),
            Screen::Timeline => self.people_view(&p, window, cx),
        };
        // The renderer copies the glass backdrop once per frame at the first glass element; with the tab bar
        // gone while the viewer is open, the viewer's own glass sees the photo instead of the grid.
        let viewer_open = self.viewer.as_ref().is_some_and(|v| !v.closing);
        self.sync_range_bar(viewer_open, cx);
        let searching = self.tab == 0 && self.search.open;
        let chrome = (self.screen == Screen::Timeline && !viewer_open).then(|| {
            if searching {
                vec![self.search_bar(&p, window, cx)]
            } else if self.select.on {
                vec![self.sel_toolbar(&p, cx)]
            } else {
                self.bars(&p, window, cx)
            }
        });
        let scrollbar = if self.screen == Screen::Timeline { self.sb_layer(&p, cx) } else { None };
        let viewer = if self.screen == Screen::Timeline { self.viewer_overlay(window, cx) } else { None };
        let select_overlay = if self.screen == Screen::Timeline && self.select.on { self.sel_overlay(&p, cx) } else { None };
        let settings = if self.screen == Screen::Timeline { self.settings_overlay(&p, window, cx) } else { None };
        let jobs = if self.screen == Screen::Timeline { self.jobs_overlay(&p, window, cx) } else { None };
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(background)
            .text_color(p.label)
            .child(content)
            .children(scrollbar)
            .child(self.grid_input(cx))
            .child(self.sel_input(cx))
            .child(self.sb_input(cx))
            .child(rubber_band::input(cx.weak_entity(), |this, dy, phase, window| {
                // While the grid zooms, the first finger of the pinch must not also scroll the list or start a fling.
                if this.grid_zooming() {
                    window.cancel_touch_momentum();
                    return (true, false);
                }
                // Swipe-to-select: the finger paints, the list must not scroll along (it auto-scrolls itself).
                if this.select.painting() || this.scrollbar.dragging() {
                    window.cancel_touch_momentum();
                    return (true, false);
                }
                if this.select.overlay_open() {
                    return (false, false);
                }
                // The settings sheet covers the page: its list takes the gesture, with its own band.
                if this.settings.sheet.visible() {
                    return this.settings_scroll_event(dy, phase, window);
                }
                if this.jobs.sheet.visible() {
                    return this.jobs_scroll_event(dy, phase, window);
                }
                let scroll = this.active_scroll();
                this.rubber.event(dy, phase, scroll.as_ref(), window)
            }))
            // Glass is ordered by what it overlaps, and the backdrop it samples is copied at the first glass batch.
            // A tab bar over an empty footer (list scrolled to its end) would be ordered before the photos and see
            // none of them. This almost invisible layer above the content lifts everything painted after it.
            .child(div().absolute().top_0().left_0().size_full().bg(rgba(0x00000001)))
            .children(viewer)
            .children(chrome.into_iter().flatten())
            .children(select_overlay)
            .children(settings)
            .children(jobs)
            .children(self.debug.then(|| fps_badge(self.meter.clone(), self.top, self.fps, self.worst_ms, H::renderer_name(self.webgpu))))
    }
}
