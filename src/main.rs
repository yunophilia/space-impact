//! Leptos shell: the LCD canvas, settings, on-screen controls and the game loop.

mod audio;
mod screen;

use leptos::ev;
use leptos::prelude::*;
use space_impact::game::{Game, Input, Phase};
use std::cell::{Cell, RefCell};
use wasm_bindgen::JsCast;
use web_sys::{KeyboardEvent, MouseEvent};

use screen::Palette;

thread_local! {
    static GAME: RefCell<Game> = RefCell::new(Game::new(seed(), load("si-hi").and_then(|s| s.parse().ok()).unwrap_or(0)));
    static KEYS: Cell<Input> = Cell::new(Input::default());
    static MOUSE: Cell<Input> = Cell::new(Input::default());
    static PAD: Cell<Input> = Cell::new(Input::default());
}

fn seed() -> u64 {
    (js_sys::Math::random() * u32::MAX as f64) as u64 ^ (js_sys::Date::now() as u64)
}

pub fn input() -> Input {
    let (k, p, m) = (KEYS.get(), PAD.get(), MOUSE.get());
    Input {
        up: k.up || p.up,
        down: k.down || p.down,
        left: k.left || p.left,
        right: k.right || p.right,
        fire: k.fire || p.fire || m.fire,
        special: k.special || p.special || m.special,
        select: k.select || p.select,
    }
}

pub fn with_game<R>(f: impl FnOnce(&mut Game) -> R) -> R {
    GAME.with(|g| f(&mut g.borrow_mut()))
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn load(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

pub fn save(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

/// Shared UI state the loop and the view both touch.
#[derive(Clone, Copy)]
pub struct Ui {
    pub phase: RwSignal<Phase>,
    pub paused: RwSignal<bool>,
    pub dark: RwSignal<bool>,
    pub palette: RwSignal<Palette>,
    pub sound: RwSignal<bool>,
    pub hi: RwSignal<u32>,
}

/// Start a new game from the page's Play button.
fn start_game() {
    audio::unlock();
    with_game(|g| g.start());
}

fn in_game(p: Phase) -> bool {
    matches!(p, Phase::Play)
}

fn on_key(e: KeyboardEvent, down: bool, ui: Ui) {
    if e.ctrl_key() || e.meta_key() || e.alt_key() {
        return;
    }
    let mut k = KEYS.get();
    let handled = match e.key().as_str() {
        "ArrowUp" | "w" | "W" => {
            k.up = down;
            true
        }
        "ArrowDown" | "s" | "S" => {
            k.down = down;
            true
        }
        "ArrowLeft" | "a" | "A" => {
            k.left = down;
            true
        }
        "ArrowRight" | "d" | "D" => {
            k.right = down;
            true
        }
        " " | "j" | "J" | "z" | "Z" => {
            k.fire = down;
            true
        }
        "x" | "X" | "k" | "K" | "c" | "C" => {
            k.special = down;
            true
        }
        "Enter" => {
            k.select = down;
            true
        }
        "p" | "P" | "Escape" => {
            if down && !e.repeat() && in_game(ui.phase.get_untracked()) {
                ui.paused.update(|p| *p = !*p);
            }
            true
        }
        _ => false,
    };
    if handled {
        e.prevent_default();
        if down {
            audio::unlock();
        }
    }
    KEYS.set(k);
}

/// Clicks on page controls (buttons, links, the touch pad) keep their normal meaning.
fn on_control(e: &MouseEvent) -> bool {
    e.target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| {
            el.closest("button, a, input, select, .touchpad")
                .ok()
                .flatten()
        })
        .is_some()
}

/// Left button fires, right button launches the special.
fn on_mouse(e: MouseEvent, down: bool) {
    let mut m = MOUSE.get();
    match e.button() {
        0 => m.fire = down,
        2 => m.special = down,
        _ => return,
    }
    if down {
        if on_control(&e) {
            return;
        }
        e.prevent_default();
        audio::unlock();
    }
    MOUSE.set(m);
}

fn pixel_scale() -> u32 {
    let Some(w) = web_sys::window() else { return 6 };
    let vw = w
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(900.0);
    let vh = w
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(700.0);
    let by_w = ((vw.min(960.0) - 56.0) / 84.0).floor();
    let by_h = ((vh - 150.0) / 48.0).floor();
    by_w.min(by_h.max(4.0)).clamp(2.0, 10.0) as u32
}

fn coarse_pointer() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(pointer: coarse)").ok().flatten())
        .map(|m| m.matches())
        .unwrap_or(false)
}

#[component]
fn App() -> impl IntoView {
    let ui = Ui {
        phase: RwSignal::new(Phase::Title),
        paused: RwSignal::new(false),
        dark: RwSignal::new(true),
        palette: RwSignal::new(
            load("si-palette")
                .map(|s| Palette::from_key(&s))
                .unwrap_or(Palette::Emulator),
        ),
        sound: RwSignal::new(load("si-sound").map(|s| s != "off").unwrap_or(true)),
        hi: RwSignal::new(load("si-hi").and_then(|s| s.parse().ok()).unwrap_or(0)),
    };
    let touch = RwSignal::new(
        load("si-touch")
            .map(|s| s == "on")
            .unwrap_or_else(coarse_pointer),
    );
    let scale = RwSignal::new(pixel_scale());

    window_event_listener(ev::resize, move |_| scale.set(pixel_scale()));
    window_event_listener(ev::keydown, move |e| on_key(e, true, ui));
    window_event_listener(ev::keyup, move |e| on_key(e, false, ui));
    window_event_listener(ev::mousedown, move |e| on_mouse(e, true));
    window_event_listener(ev::mouseup, move |e| on_mouse(e, false));
    // Right-click is the special weapon, so no context menu over the game.
    window_event_listener(ev::contextmenu, move |e| {
        if in_game(ui.phase.get_untracked())
            || !on_control(&e)
                && e.target().is_some_and(|t| {
                    t.dyn_into::<web_sys::Element>()
                        .ok()
                        .and_then(|el| el.closest(".screen").ok().flatten())
                        .is_some()
                })
        {
            e.prevent_default();
        }
    });
    window_event_listener(ev::blur, move |_| {
        KEYS.set(Input::default());
        MOUSE.set(Input::default());
        PAD.set(Input::default());
        if in_game(ui.phase.get_untracked()) {
            ui.paused.set(true);
        }
    });

    Effect::new(move |_| save("si-palette", ui.palette.get().key()));
    Effect::new(move |_| save("si-sound", if ui.sound.get() { "on" } else { "off" }));
    Effect::new(move |_| save("si-touch", if touch.get() { "on" } else { "off" }));

    let screen_style = move || {
        let s = scale.get();
        let (bg, _) = ui.palette.get().colors(ui.dark.get());
        format!(
            "width:{}px;height:{}px;--cell:{}px;--lcd-bg:rgb({},{},{})",
            84 * s,
            48 * s,
            s,
            bg[0],
            bg[1],
            bg[2]
        )
    };

    view! {
        <main class="app">
            <header class="masthead">
                <h1>"Space Impact"</h1>
                <p>"Rust · Leptos · WebAssembly"</p>
            </header>

            <section class="phone">
                <div class="bezel">
                    <div class="screen" class:grid=move || { scale.get() >= 4 } style=screen_style>
                        <screen::Lcd ui=ui />
                        <Show when=move || matches!(ui.phase.get(), Phase::Title | Phase::GameOver)>
                            <button class="screen-btn" on:click=move |_| start_game()>
                                {move || if ui.phase.get() == Phase::Title { "Play" } else { "Play again" }}
                            </button>
                        </Show>
                        <Show when=move || ui.paused.get() && in_game(ui.phase.get())>
                            <div class="pause-card">
                                <p>"Paused"</p>
                                <button class="screen-btn inline" on:click=move |_| ui.paused.set(false)>"Resume"</button>
                                <button
                                    class="screen-btn inline"
                                    on:click=move |_| {
                                        with_game(|g| g.quit());
                                        ui.paused.set(false);
                                    }
                                >
                                    "Quit"
                                </button>
                            </div>
                        </Show>
                    </div>
                </div>

                <p class="topscore">"Top score: "{move || format!("{:05}", ui.hi.get())}</p>
                <div class="toolbar">
                    <button
                        disabled=move || !in_game(ui.phase.get())
                        on:click=move |_| ui.paused.update(|p| *p = !*p)
                    >
                        {move || if ui.paused.get() { "Resume" } else { "Pause" }}
                    </button>
                    <button on:click=move |_| ui.sound.update(|s| *s = !*s) aria-pressed=move || ui.sound.get().to_string()>
                        {move || if ui.sound.get() { "Sound: on" } else { "Sound: off" }}
                    </button>
                    <button on:click=move |_| ui.palette.update(|p| *p = p.next())>
                        {move || format!("Screen: {}", ui.palette.get().label())}
                    </button>
                    <button on:click=move |_| touch.update(|t| *t = !*t) aria-pressed=move || touch.get().to_string()>
                        {move || if touch.get() { "Touch pad: on" } else { "Touch pad: off" }}
                    </button>
                </div>

                <Show when=move || touch.get()>
                    <screen::TouchPad />
                </Show>
            </section>

            <section class="help">
                <h2>"Controls"</h2>
                <dl>
                    <dt>"Move"</dt>
                    <dd><kbd>"←↑↓→"</kbd>" or "<kbd>"WASD"</kbd></dd>
                    <dt>"Fire"</dt>
                    <dd><kbd>"Space"</kbd>" / "<kbd>"J"</kbd>" / left click (hold to auto-fire)"</dd>
                    <dt>"Special"</dt>
                    <dd><kbd>"X"</kbd>" / "<kbd>"K"</kbd>" / right click"</dd>
                    <dt>"Start"</dt>
                    <dd><kbd>"Enter"</kbd>" / fire starts a game"</dd>
                    <dt>"Pause"</dt>
                    <dd><kbd>"P"</kbd>" / "<kbd>"Esc"</kbd></dd>
                </dl>
                <p>
                    "Shoot down the enemy waves and survive all 6 levels. Some enemies drop a sparkle when destroyed: fly into it for an "
                    "extra life or a special weapon: homing missiles, a laser beam, or a sweeping wall. You get a shield for a few "
                    "seconds after every respawn."
                </p>
            </section>

            <footer>
                "A fan recreation of Nokia's "<em>"Space Impact"</em>" as it shipped on the Nokia 3310. Every pixel, screen, level and "
                "enemy path was decoded from "
                <a href="https://www.youtube.com/watch?v=cv4ny5OLKGU">"gameplay footage of the original phone software"</a>
                ". Not affiliated with Nokia."
            </footer>
        </main>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
