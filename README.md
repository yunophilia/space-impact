# Space Impact (Nokia 3310) in Rust + Leptos + WASM

A fan recreation of Nokia's *Space Impact* as it shipped on the Nokia 3310: 84×48 1-bit LCD,
6 levels. It is a clean-room rebuild from footage: every bitmap (ship, shield, enemies, bosses,
scenery, HUD, intro, menus, instructions, Top score, victory and game-over screens) and every
level timeline was decoded pixel by pixel from
[gameplay footage of the original phone software](https://www.youtube.com/watch?v=cv4ny5OLKGU).
No code or data from other Space Impact clones is used.

- `src/game/data.rs`: generated assets and level scripts (do not edit by hand)
- `src/game/mod.rs`: the simulation (pure Rust, `cargo test --lib`)
- `src/main.rs`, `src/screen.rs`, `src/audio.rs`: the Leptos shell, LCD renderer, touch pad, beeps

## What is measured vs. inferred

Measured from the footage: all graphics; screen layout; tick rate (~11 Hz); ship/bullet speeds
(2 px/tick); fire rate; scenery scroll (1 px/tick, stopping for bosses); every enemy's spawn time,
entry point and recorded path; boss paths; shield length (30 ticks); scoring (+5 per hit, +5 per
kill, +100 per boss); power-up drops and what each gives; the level-exit fly-off; the intro, menu, Top score
and ending screens.

Estimated or inferred: enemy and boss hit points, enemy/boss firing timing, what enemies do after
the point where the footage shows them destroyed (they keep their last speed), the final boss's
attack, the big score digits 1, 3, 4 and 7 (never shown on screen), the in-game "Continue" menu
(not implemented; Esc returns to the main menu), and sound.

## Running

`trunk serve` (open http://127.0.0.1:8087).
Pushing to `main` deploys to GitHub Pages via `.github/workflows/deploy.yml`.
