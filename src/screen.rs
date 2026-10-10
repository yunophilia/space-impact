//! The LCD canvas with its frame loop, and the on-screen touch pad.

use crate::{audio, input, save, with_game, Ui, PAD};
use leptos::html;
use leptos::prelude::*;
use space_impact::game::gfx::{Frame, H, W};
use space_impact::game::{Input, Phase, TICK_MS as DT};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, ImageData};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Palette {
    Emulator,
    Nokia,
    Mono,
    Classic,
}

impl Palette {
    pub fn from_key(s: &str) -> Palette {
        match s {
            "mono" => Palette::Mono,
            "classic" => Palette::Classic,
            "nokia" => Palette::Nokia,
            _ => Palette::Emulator,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Palette::Emulator => "n3310",
            Palette::Nokia => "nokia",
            Palette::Mono => "mono",
            Palette::Classic => "classic",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Palette::Emulator => "Nokia 3310",
            Palette::Nokia => "mint green",
            Palette::Mono => "black & white",
            Palette::Classic => "web classic",
        }
    }
    pub fn next(self) -> Palette {
        match self {
            Palette::Emulator => Palette::Nokia,
            Palette::Nokia => Palette::Mono,
            Palette::Mono => Palette::Classic,
            Palette::Classic => Palette::Emulator,
        }
    }
    /// (background, ink).
    pub fn colors(self, dark: bool) -> ([u8; 3], [u8; 3]) {
        let (bg, fg) = match self {
            Palette::Emulator => ([0xad, 0xbd, 0x8b], [0x27, 0x33, 0x12]),
            Palette::Nokia => ([0xc7, 0xf0, 0xd8], [0x43, 0x52, 0x3d]),
            Palette::Mono => ([0xf2, 0xf2, 0xee], [0x14, 0x14, 0x14]),
            Palette::Classic => ([0xaa, 0xd6, 0x9c], [0x28, 0x28, 0x28]),
        };
        if dark {
            (fg, bg)
        } else {
            (bg, fg)
        }
    }
}

/// How far each LCD pixel moves toward its target per rendered frame;
/// below 1.0 leaves a faint trail, like a slow LCD.
const LCD_RESPONSE: f32 = 0.6;

struct LcdState {
    ctx: CanvasRenderingContext2d,
    frame: Frame,
    lum: Vec<f32>,
    rgba: Vec<u8>,
    last: Option<f64>,
    acc: f64,
}

impl LcdState {
    fn step(&mut self, ts: f64, ui: Ui) {
        let dt = self.last.map(|l| (ts - l).clamp(0.0, 250.0)).unwrap_or(0.0);
        self.last = Some(ts);
        if !ui.paused.get_untracked() {
            self.acc += dt;
        }
        let input = input();
        with_game(|g| {
            while self.acc >= DT as f64 {
                g.tick(input);
                self.acc -= DT as f64;
            }
        });
        self.present(ui);
    }

    /// Draws the current game state and syncs the UI signals with it.
    fn present(&mut self, ui: Ui) {
        let (phase, dark, hi, sfx) = with_game(|g| {
            g.render(&mut self.frame);
            (g.phase(), false, g.hi, g.drain_sfx())
        });

        // Signals are set outside the game borrow so effects may touch the game.
        if phase != ui.phase.get_untracked() {
            if !matches!(phase, Phase::Play) {
                ui.paused.set(false);
                save("si-hi", &hi.to_string());
            }
            if hi != ui.hi.get_untracked() {
                ui.hi.set(hi);
            }
            ui.phase.set(phase);
        }
        if dark != ui.dark.get_untracked() {
            ui.dark.set(dark);
        }
        if ui.sound.get_untracked() {
            for s in sfx {
                audio::play(s);
            }
        }

        let (bg, fg) = ui.palette.get_untracked().colors(dark);
        for (i, &on) in self.frame.px.iter().enumerate() {
            let l = &mut self.lum[i];
            *l += (on as f32 - *l) * LCD_RESPONSE;
            let o = i * 4;
            for c in 0..3 {
                self.rgba[o + c] = (bg[c] as f32 + (fg[c] as f32 - bg[c] as f32) * *l) as u8;
            }
            self.rgba[o + 3] = 255;
        }
        if let Ok(img) =
            ImageData::new_with_u8_clamped_array_and_sh(Clamped(&self.rgba), W as u32, H as u32)
        {
            let _ = self.ctx.put_image_data(&img, 0.0, 0.0);
        }
    }
}

thread_local! {
    static LCD: RefCell<Option<(LcdState, Ui)>> = const { RefCell::new(None) };
}

fn with_lcd(f: impl FnOnce(&mut LcdState, Ui)) {
    LCD.with(|l| {
        if let Some((lcd, ui)) = l.borrow_mut().as_mut() {
            f(lcd, *ui)
        }
    });
}

/// Test hook: run `ticks` simulation steps with a fixed input bitmask
/// (1 up, 2 down, 4 left, 8 right, 16 fire, 32 special, 64 select) and redraw.
/// Works even when the tab is hidden and animation frames are paused.
#[wasm_bindgen(js_name = siDebugStep)]
pub fn debug_step(ticks: u32, keys: u32) {
    let input = Input {
        up: keys & 1 != 0,
        down: keys & 2 != 0,
        left: keys & 4 != 0,
        right: keys & 8 != 0,
        fire: keys & 16 != 0,
        special: keys & 32 != 0,
        select: keys & 64 != 0,
    };
    with_game(|g| (0..ticks).for_each(|_| g.tick(input)));
    with_lcd(|lcd, ui| lcd.present(ui));
}

/// Test hook: start a fresh game at `level`.
#[wasm_bindgen(js_name = siDebugStart)]
pub fn debug_start(level: u32) {
    with_game(|g| g.start_at(level as usize));
    with_lcd(|lcd, ui| lcd.present(ui));
}

type FrameCallback = Closure<dyn FnMut(f64)>;

/// Test hook: make the ship invulnerable for this level.
#[wasm_bindgen(js_name = siDebugGod)]
pub fn debug_god() {
    with_game(|g| g.debug_invulnerable());
}

fn request_frame(cb: &FrameCallback) {
    if let Some(w) = web_sys::window() {
        let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
    }
}

#[component]
pub fn Lcd(ui: Ui) -> impl IntoView {
    let canvas = NodeRef::<html::Canvas>::new();
    canvas.on_load(move |el| {
        let Some(ctx) = el
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
        else {
            return;
        };
        let lcd = LcdState {
            ctx,
            frame: Frame::default(),
            lum: vec![0.0; (W * H) as usize],
            rgba: vec![0; (W * H * 4) as usize],
            last: None,
            acc: 0.0,
        };
        LCD.with(|l| *l.borrow_mut() = Some((lcd, ui)));
        // The loop runs for the life of the page, so the closure is leaked on purpose.
        let cb: Rc<RefCell<Option<FrameCallback>>> = Rc::new(RefCell::new(None));
        let next = cb.clone();
        *cb.borrow_mut() = Some(Closure::new(move |ts: f64| {
            with_lcd(|lcd, ui| lcd.step(ts, ui));
            if let Some(c) = next.borrow().as_ref() {
                request_frame(c);
            }
        }));
        if let Some(c) = cb.borrow().as_ref() {
            request_frame(c);
        };
    });
    view! { <canvas node_ref=canvas width=W height=H aria-label="Game screen"></canvas> }
}

fn set_pad(f: impl FnOnce(&mut Input)) {
    let mut p = PAD.get();
    f(&mut p);
    PAD.set(p);
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Dpad,
    Fire,
    Special,
}

thread_local! {
    /// Which control each finger on the pad started on, by touch identifier.
    static FINGERS: RefCell<Vec<(i32, Role)>> = const { RefCell::new(Vec::new()) };
}

/// Centre and half-size of an element on screen.
type Zone = (f64, f64, f64, f64);

fn zone(el: &web_sys::Element) -> Zone {
    let r = el.get_bounding_client_rect();
    (
        r.left() + r.width() / 2.0,
        r.top() + r.height() / 2.0,
        r.width() / 2.0,
        r.height() / 2.0,
    )
}

/// Adds the d-pad direction for a point to `p`.
fn steer(p: &mut Input, (cx, cy, rx, ry): Zone, x: f64, y: f64) {
    const DEAD: f64 = 0.3;
    let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
    p.left |= dx < -DEAD;
    p.right |= dx > DEAD;
    p.up |= dy < -DEAD;
    p.down |= dy > DEAD;
}

fn clear_dirs(p: &mut Input) {
    (p.left, p.right, p.up, p.down) = (false, false, false, false);
}

/// D-pad plus fire/special buttons.
///
/// Touch input is rebuilt from scratch on every touch event out of the list of
/// fingers currently on the screen, so a cancelled or missed event can never
/// leave a button released while a finger still holds it. Each finger keeps
/// the control it first landed on, however far it drifts. Pointer events still
/// drive the pad for a mouse.
#[component]
pub fn TouchPad() -> impl IntoView {
    let pad = NodeRef::<html::Div>::new();
    let dpad = NodeRef::<html::Div>::new();
    let fire = NodeRef::<html::Button>::new();
    let special = NodeRef::<html::Button>::new();
    let (fire_on, set_fire_on) = signal(false);
    let (special_on, set_special_on) = signal(false);

    let on_touch = move |e: web_sys::TouchEvent| {
        e.prevent_default();
        let (Some(pd), Some(d), Some(f), Some(s)) = (
            pad.get_untracked(),
            dpad.get_untracked(),
            fire.get_untracked(),
            special.get_untracked(),
        ) else {
            return;
        };
        if e.type_() == "touchstart" {
            audio::unlock();
        }
        let (dz, fz, sz) = (zone(&d), zone(&f), zone(&s));
        let touches = e.touches();
        let mut p = Input::default();
        FINGERS.with(|fingers| {
            let mut fingers = fingers.borrow_mut();
            let mut seen = vec![];
            for i in 0..touches.length() {
                let Some(t) = touches.get(i) else { continue };
                let (x, y) = (t.client_x() as f64, t.client_y() as f64);
                let role = match fingers.iter().find(|(id, _)| *id == t.identifier()) {
                    Some(&(_, r)) => r,
                    None => {
                        // Fingers elsewhere on the page are not ours.
                        let on_pad = t
                            .target()
                            .and_then(|tg| tg.dyn_into::<web_sys::Node>().ok())
                            .is_some_and(|n| pd.contains(Some(&n)));
                        if !on_pad {
                            continue;
                        }
                        // A new finger takes the control whose centre is nearest.
                        let dist = |(cx, cy, rx, ry): Zone| ((x - cx) / rx).hypot((y - cy) / ry);
                        let r = [
                            (Role::Dpad, dist(dz)),
                            (Role::Fire, dist(fz)),
                            (Role::Special, dist(sz)),
                        ]
                        .into_iter()
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map_or(Role::Dpad, |(r, _)| r);
                        fingers.push((t.identifier(), r));
                        r
                    }
                };
                seen.push(t.identifier());
                match role {
                    Role::Dpad => steer(&mut p, dz, x, y),
                    Role::Fire => p.fire = true,
                    Role::Special => p.special = true,
                }
            }
            fingers.retain(|(id, _)| seen.contains(id));
        });
        PAD.set(p);
        set_fire_on.set(p.fire);
        set_special_on.set(p.special);
    };
    pad.on_load(move |el| {
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_passive(false);
        let cb = Closure::<dyn FnMut(web_sys::TouchEvent)>::new(on_touch);
        for ev in ["touchstart", "touchmove", "touchend", "touchcancel"] {
            let _ = el.add_event_listener_with_callback_and_add_event_listener_options(
                ev,
                cb.as_ref().unchecked_ref(),
                &opts,
            );
        }
        // Lives as long as the element; the pad is only created when it is switched on.
        cb.forget();
    });

    // The pointer handlers below are for a mouse; touch is handled above.
    let mouse = |e: &web_sys::PointerEvent| e.pointer_type() != "touch";
    let mouse_steer = move |e: &web_sys::PointerEvent| {
        if let Some(el) = dpad.get_untracked() {
            let z = zone(&el);
            set_pad(|p| {
                clear_dirs(p);
                steer(p, z, e.client_x() as f64, e.client_y() as f64);
            });
        }
    };
    let release = move |e: web_sys::PointerEvent| {
        if mouse(&e) {
            set_pad(clear_dirs);
        }
    };
    let button = move |label: &'static str,
                       class: &'static str,
                       node: NodeRef<html::Button>,
                       on: ReadSignal<bool>,
                       set: fn(&mut Input, bool)| {
        view! {
            <button
                class=format!("pad-btn {class}")
                class:on=on
                node_ref=node
                on:pointerdown=move |e: web_sys::PointerEvent| {
                    if !mouse(&e) {
                        return;
                    }
                    e.prevent_default();
                    audio::unlock();
                    if let Some(el) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = el.set_pointer_capture(e.pointer_id());
                    }
                    set_pad(|p| set(p, true));
                }
                on:pointerup=move |e: web_sys::PointerEvent| {
                    if mouse(&e) {
                        set_pad(|p| set(p, false))
                    }
                }
                on:pointercancel=move |e: web_sys::PointerEvent| {
                    if mouse(&e) {
                        set_pad(|p| set(p, false))
                    }
                }
                on:contextmenu=move |e| e.prevent_default()
            >
                {label}
            </button>
        }
    };
    on_cleanup(|| {
        PAD.set(Input::default());
        FINGERS.with(|f| f.borrow_mut().clear());
    });

    view! {
        <div class="touchpad" node_ref=pad>
            <div
                class="dpad"
                node_ref=dpad
                on:pointerdown=move |e: web_sys::PointerEvent| {
                    if !mouse(&e) {
                        return;
                    }
                    e.prevent_default();
                    audio::unlock();
                    if let Some(el) = dpad.get_untracked() {
                        let _ = el.set_pointer_capture(e.pointer_id());
                    }
                    mouse_steer(&e);
                }
                on:pointermove=move |e: web_sys::PointerEvent| {
                    if mouse(&e) && e.buttons() != 0 {
                        mouse_steer(&e);
                    }
                }
                on:pointerup=release
                on:pointercancel=release
                on:contextmenu=move |e| e.prevent_default()
            >
                <span class="arrow up"></span>
                <span class="arrow down"></span>
                <span class="arrow left"></span>
                <span class="arrow right"></span>
            </div>
            <div class="actions">
                {button("Special", "special", special, special_on, |p, v| p.special = v)}
                {button("Fire", "fire", fire, fire_on, |p, v| p.fire = v)}
            </div>
        </div>
    }
}
