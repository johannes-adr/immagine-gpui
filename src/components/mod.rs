//! Reusable GPUI building blocks. Nothing in here knows the app: no Immich, no German strings, no `crate::immagine`,
//! `crate::logic`, `crate::host`, `crate::theme` or `crate::util`. Text, colors and ids come in as parameters;
//! the one outside input is the time (see `clock`), which the host installs once.
//!
//! **Using it in another project:** copy this folder, add it as `mod components;` (module paths are all
//! `crate::components::...`), depend on `gpui_kit` (GPUI) and call `clock::install(my_now_ms)` once at startup.
//! Build a `theme::Theme` per render (`Theme::new(dark)` gives iOS-like colors, or fill the fields yourself).
//!
//! **The one non-portable dependency is glass.** `glass_div*` (and so `button::pill/circle/chip/back`, the sheet's
//! close circle and `morph`'s card) call `Window::paint_glass`, which only exists in this repository's vendored
//! `gpui-pre` (see AGENTS.md, "GPUI is vendored and patched"). Without that patch, replace `glass::glass_div_with`
//! by a translucent `div().bg(..)` and everything else works unchanged.
//!
//! API style: free functions returning an element you finish (`Stateful<Div>`/`Div`/`Canvas`), a small `Copy`
//! parameter struct where there are several knobs (`button::Surface`, `morph::Spec`, `sheet::Spec`), and a stateful
//! struct only where there is state (`Sheet`, `Morph`, `RubberBand`, `FpsMeter`).
//!
//! | module        | what                                                                                    | glass |
//! |---------------|-----------------------------------------------------------------------------------------|-------|
//! | `clock`       | installable millisecond clock (wasm has no `Instant`)                                   |       |
//! | `ease`        | `lerp`, `ease_out`, `anim` (quint), `spring_out`                                        |       |
//! | `theme`       | `Theme`: label/secondary/separator/glass/bar/background/card/accent/... + light/dark    |       |
//! | `glass`       | `glass_div`, `glass_div_faded`, `glass_div_with(tint, radius, k, fresh_backdrop)`, `glass_blur` | yes |
//! | `button`      | `Surface`, `pill`, `circle`, `chip`, `back`, `filled`, `dim` (pressed style)            | pill, circle, chip, back |
//! | `icon`        | `Glyph` set + `glyph`, `chevron(Dir)`, `heart`, `progress_ring`; all stroked in a given color |  |
//! | `list`        | `Group` (caption + card; `row`, `action`, `toggle`, `raw`), `caption`, `section`        |       |
//! | `scrim`       | the dimmed layer behind sheets and dialogs                                              |       |
//! | `sheet`       | `Sheet` (motion + scroll + own rubber band), `overlay`, `Pointer`; new sheet = state field + tick + routing + one `overlay` call | close circle |
//! | `morph`       | `Morph` + `overlay`: a button turns into a card; `confirm_content` for destructive confirmations | card |
//! | `rubber_band` | iOS overscroll for any scroller: `RubberBand`, `input`                                  |       |
//! | `tap`         | click handlers that ignore the tap which stopped a fling (`listener`, `weak`)           |       |
//! | `measure`     | `bounds` (invisible layer reporting its rect), `rect`                                   |       |
//! | `fps`         | `FpsMeter` and `fps_badge`                                                              |       |
//!
//! Rules a component must keep: buttons and rows never occlude by themselves (the caller adds `.occlude()` where a
//! tappable layer lies below) while overlays (`sheet`, `morph`) occlude their scrim and card; nothing reads a clock
//! but `clock`; glass semantics stay (`k <= 0` paints nothing, `fresh` samples the frame painted so far).

pub(crate) mod button;
pub(crate) mod clock;
pub(crate) mod ease;
pub(crate) mod fps;
pub(crate) mod glass;
pub(crate) mod icon;
pub(crate) mod list;
pub(crate) mod measure;
pub(crate) mod morph;
pub(crate) mod rubber_band;
pub(crate) mod scrim;
pub(crate) mod sheet;
pub(crate) mod tap;
pub(crate) mod theme;
