//! The phone's buzzer, through Web Audio.
//!
//! Every sound is a sequence of (pitch, duration) steps measured from the
//! original game's audio, played on a waveform with the buzzer's harmonic mix
//! (odd harmonics only). Like the phone, it plays one sound at a time: a new
//! sound cuts the previous one off.

use space_impact::game::Sfx;
use std::cell::RefCell;
use web_sys::{AudioContext, AudioContextState, GainNode, OscillatorNode, PeriodicWave};

struct Buzzer {
    ctx: AudioContext,
    wave: PeriodicWave,
    playing: Option<(OscillatorNode, GainNode)>,
}

thread_local! {
    static BUZZER: RefCell<Option<Buzzer>> = const { RefCell::new(None) };
}

/// Harmonic amplitudes relative to the fundamental, measured from a held A5.
const HARMONICS: [f32; 7] = [1.0, 0.0, 0.247, 0.0, 0.086, 0.0, 0.041];
const VOLUME: f32 = 0.05;

/// Browsers only allow audio after a user gesture; call this from input handlers.
pub fn unlock() {
    BUZZER.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_none() {
            *b = make().ok();
        }
        if let Some(bz) = b.as_ref() {
            if bz.ctx.state() == AudioContextState::Suspended {
                let _ = bz.ctx.resume();
            }
        }
    });
}

fn make() -> Result<Buzzer, wasm_bindgen::JsValue> {
    let ctx = AudioContext::new()?;
    let mut real = vec![0.0f32; HARMONICS.len() + 1];
    let mut imag = vec![0.0f32; HARMONICS.len() + 1];
    imag[1..].copy_from_slice(&HARMONICS);
    real[0] = 0.0;
    let wave = ctx.create_periodic_wave(&mut real, &mut imag)?;
    Ok(Buzzer {
        ctx,
        wave,
        playing: None,
    })
}

/// (Hz, ms) steps; 0 Hz is a rest.
type Steps = &'static [(f32, u16)];

fn steps(s: Sfx) -> Steps {
    match s {
        // A4 A#4 B4 C5
        Sfx::Shoot => &[(440.0, 45), (466.2, 40), (493.9, 35), (523.3, 50)],
        // C8, then a falling chromatic run down to E7
        Sfx::Special => &[
            (4186.0, 65),
            (3951.1, 15),
            (3729.3, 15),
            (3520.0, 15),
            (3322.4, 15),
            (3136.0, 20),
            (2960.0, 15),
            (2793.8, 15),
            (2637.0, 25),
        ],
        // F6 alternating with a falling run from B5 to D5
        Sfx::SpecialWall => &[
            (1396.9, 25),
            (987.8, 15),
            (1396.9, 15),
            (932.3, 15),
            (1396.9, 15),
            (880.0, 20),
            (1396.9, 15),
            (830.6, 15),
            (1396.9, 15),
            (784.0, 15),
            (1396.9, 15),
            (740.0, 15),
            (1396.9, 15),
            (698.5, 15),
            (1396.9, 15),
            (659.3, 20),
            (1396.9, 15),
            (622.3, 15),
            (1396.9, 15),
            (587.3, 20),
        ],
        // E7 blip
        Sfx::PowerUp => &[(2637.0, 25)],
        // A5, A#5, B5 interleaved with C8
        Sfx::Die => &[
            (880.0, 25),
            (4186.0, 40),
            (932.3, 20),
            (4186.0, 45),
            (987.8, 20),
            (4186.0, 50),
        ],
        // D5 D5 D5 A5 D5 A5: plays with the ending animation and again on the Game over screen
        Sfx::GameOver => &[
            (587.3, 150),
            (0.0, 130),
            (587.3, 130),
            (0.0, 10),
            (587.3, 130),
            (880.0, 415),
            (0.0, 20),
            (587.3, 130),
            (0.0, 10),
            (880.0, 450),
        ],
        // The original makes no sound for hits, explosions, boss kills or level ends.
        Sfx::Hit | Sfx::Explode | Sfx::BossDie | Sfx::LevelClear => &[],
    }
}

pub fn play(s: Sfx) {
    let seq = steps(s);
    if seq.is_empty() {
        return;
    }
    BUZZER.with(|b| {
        let mut b = b.borrow_mut();
        let Some(bz) = b.as_mut() else { return };
        if bz.ctx.state() != AudioContextState::Running {
            return;
        }
        if let Some((osc, _)) = bz.playing.take() {
            let _ = osc.stop();
        }
        if let Ok(pair) = schedule(bz, seq) {
            bz.playing = Some(pair);
        }
    });
}

fn schedule(bz: &Buzzer, seq: Steps) -> Result<(OscillatorNode, GainNode), wasm_bindgen::JsValue> {
    let ctx = &bz.ctx;
    let osc = ctx.create_oscillator()?;
    osc.set_periodic_wave(&bz.wave);
    let gain = ctx.create_gain()?;
    let now = ctx.current_time();
    let mut t = now;
    gain.gain().set_value_at_time(0.0, t)?;
    for &(hz, ms) in seq {
        if hz > 0.0 {
            osc.frequency().set_value_at_time(hz, t)?;
            gain.gain().set_value_at_time(VOLUME, t)?;
        } else {
            gain.gain().set_value_at_time(0.0, t)?;
        }
        t += ms as f64 / 1000.0;
    }
    gain.gain().set_value_at_time(0.0, t)?;
    osc.connect_with_audio_node(&gain)?;
    gain.connect_with_audio_node(&ctx.destination())?;
    osc.start_with_when(now)?;
    osc.stop_with_when(t + 0.01)?;
    Ok((osc, gain))
}
