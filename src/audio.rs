//! Nokia-style square-wave beeps through Web Audio.

use space_impact::game::Sfx;
use std::cell::RefCell;
use web_sys::{AudioContext, AudioContextState, OscillatorType};

thread_local! {
    static CTX: RefCell<Option<AudioContext>> = const { RefCell::new(None) };
}

/// Browsers only allow audio after a user gesture; call this from input handlers.
pub fn unlock() {
    CTX.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = AudioContext::new().ok();
        }
        if let Some(ctx) = c.as_ref() {
            if ctx.state() == AudioContextState::Suspended {
                let _ = ctx.resume();
            }
        }
    });
}

/// One note: start Hz, end Hz (slide), offset and length in seconds.
type Note = (f32, f32, f64, f64);

fn notes(s: Sfx) -> &'static [Note] {
    match s {
        Sfx::Shoot => &[(1800.0, 1400.0, 0.0, 0.03)],
        Sfx::Hit => &[(320.0, 260.0, 0.0, 0.025)],
        Sfx::Explode => &[(260.0, 50.0, 0.0, 0.16)],
        Sfx::PowerUp => &[
            (880.0, 880.0, 0.0, 0.06),
            (1175.0, 1175.0, 0.06, 0.06),
            (1568.0, 1568.0, 0.12, 0.09),
        ],
        Sfx::Special => &[(500.0, 1500.0, 0.0, 0.12)],
        Sfx::Die => &[(500.0, 60.0, 0.0, 0.5)],
        Sfx::BossDie => &[(400.0, 40.0, 0.0, 0.45), (300.0, 30.0, 0.45, 0.5)],
        Sfx::LevelClear => &[
            (523.0, 523.0, 0.0, 0.08),
            (659.0, 659.0, 0.09, 0.08),
            (784.0, 784.0, 0.18, 0.08),
            (1047.0, 1047.0, 0.27, 0.16),
        ],
        Sfx::GameOver => &[
            (392.0, 392.0, 0.0, 0.2),
            (330.0, 330.0, 0.22, 0.2),
            (262.0, 196.0, 0.44, 0.45),
        ],
    }
}

pub fn play(s: Sfx) {
    CTX.with(|c| {
        let c = c.borrow();
        let Some(ctx) = c.as_ref() else { return };
        if ctx.state() != AudioContextState::Running {
            return;
        }
        let now = ctx.current_time();
        for &(f0, f1, at, len) in notes(s) {
            let _ = beep(ctx, f0, f1, now + at, len);
        }
    });
}

fn beep(
    ctx: &AudioContext,
    f0: f32,
    f1: f32,
    t: f64,
    len: f64,
) -> Result<(), wasm_bindgen::JsValue> {
    let osc = ctx.create_oscillator()?;
    osc.set_type(OscillatorType::Square);
    osc.frequency().set_value_at_time(f0, t)?;
    if f1 != f0 {
        osc.frequency()
            .exponential_ramp_to_value_at_time(f1, t + len)?;
    }
    let gain = ctx.create_gain()?;
    gain.gain().set_value_at_time(0.045, t)?;
    gain.gain().linear_ramp_to_value_at_time(0.0, t + len)?;
    osc.connect_with_audio_node(&gain)?;
    gain.connect_with_audio_node(&ctx.destination())?;
    osc.start_with_when(t)?;
    osc.stop_with_when(t + len + 0.02)?;
    Ok(())
}
