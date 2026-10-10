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

/// D-pad plus fire/special buttons, multi-touch friendly via pointer events.
#[component]
pub fn TouchPad() -> impl IntoView {
    let dpad = NodeRef::<html::Div>::new();
    let steer = move |e: &web_sys::PointerEvent| {
        let Some(el) = dpad.get_untracked() else {
            return;
        };
        let r = el.get_bounding_client_rect();
        let dx = (e.client_x() as f64 - (r.left() + r.width() / 2.0)) / (r.width() / 2.0);
        let dy = (e.client_y() as f64 - (r.top() + r.height() / 2.0)) / (r.height() / 2.0);
        const DEAD: f64 = 0.3;
        set_pad(|p| {
            p.left = dx < -DEAD;
            p.right = dx > DEAD;
            p.up = dy < -DEAD;
            p.down = dy > DEAD;
        });
    };
    let release = move |_| {
        set_pad(|p| {
            p.left = false;
            p.right = false;
            p.up = false;
            p.down = false;
        })
    };
    let button = move |label: &'static str, class: &'static str, set: fn(&mut Input, bool)| {
        view! {
            <button
                class=format!("pad-btn {class}")
                on:pointerdown=move |e: web_sys::PointerEvent| {
                    e.prevent_default();
                    audio::unlock();
                    // Capture the pointer so a held finger that drifts off the
                    // button keeps firing; only lifting it releases the button.
                    if let Some(el) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = el.set_pointer_capture(e.pointer_id());
                    }
                    set_pad(|p| set(p, true));
                }
                on:pointerup=move |_| set_pad(|p| set(p, false))
                on:pointercancel=move |_| set_pad(|p| set(p, false))
                on:lostpointercapture=move |_| set_pad(|p| set(p, false))
                on:contextmenu=move |e| e.prevent_default()
            >
                {label}
            </button>
        }
    };
    on_cleanup(|| PAD.set(Input::default()));

    view! {
        <div class="touchpad">
            <div
                class="dpad"
                node_ref=dpad
                on:pointerdown=move |e: web_sys::PointerEvent| {
                    e.prevent_default();
                    audio::unlock();
                    if let Some(el) = dpad.get_untracked() {
                        let _ = el.set_pointer_capture(e.pointer_id());
                    }
                    steer(&e);
                }
                on:pointermove=move |e: web_sys::PointerEvent| {
                    if e.buttons() != 0 || e.pointer_type() == "touch" {
                        steer(&e);
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
                {button("Special", "special", |p, v| p.special = v)}
                {button("Fire", "fire", |p, v| p.fire = v)}
            </div>
        </div>
    }
}
