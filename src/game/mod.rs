//! Space Impact (Nokia 3310) simulation. Pure Rust, no web dependencies.
//!
//! Every bitmap, screen and level timeline lives in `data.rs`, decoded from
//! footage of the original ROM. Enemies and bosses replay the paths recorded
//! there; where the footage ends (the enemy was shot down) they carry on at
//! their last speed. One tick is one game frame, about 11 per second.

#[rustfmt::skip]
pub mod data;
pub mod gfx;

use data::{Shot as Recorded, LEVELS};
use gfx::{Frame, Sprite, W};
use std::sync::OnceLock;

/// Milliseconds per game tick, measured from the footage.
pub const TICK_MS: f32 = 1000.0 / 11.1;

const PF_H: i32 = 43; // playfield height; the 5 other rows hold the HUD
const SHIP_W: i32 = 10;
const SHIP_H: i32 = 7;
const SHIP_MAX_X: i32 = 74;
const SHIELD_TICKS: u32 = 30;
const START_LIVES: u32 = 3;
const MAX_LIVES: u32 = 5;
const FIRE_EVERY: u32 = 2;
const EXIT_HOLD: u32 = 15;
const EXIT_STEPS: [i32; 9] = [3, 3, 4, 5, 6, 7, 8, 10, 12];
const LASER_TICKS: u32 = 4;
const FINAL_BOSS_HP: u16 = 150;
/// Level 2's clouds are background: the ship flies through them.
const SOLID_SCENERY: [bool; 6] = [true, false, true, true, true, true];

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Input {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub fire: bool,
    pub special: bool,
    /// Enter: starts a game from the title or game-over screen.
    pub select: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The intro animation, then its last frame (the logo) until a game starts.
    Title,
    Play,
    Victory,
    Defeat,
    GameOver,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weapon {
    Missile,
    Laser,
    Wall,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    Shoot,
    Special,
    SpecialWall,
    Hit,
    Explode,
    PowerUp,
    Die,
    BossDie,
    LevelClear,
    GameOver,
}

struct Art {
    ship: Sprite,
    shield: [Sprite; 2],
    explosion: [Sprite; 3],
    missile: Sprite,
    item: [Sprite; 2],
    digits: Vec<Sprite>,
    big_digits: Vec<Sprite>,
    heart: Sprite,
    icons: [Sprite; 3],
    types: Vec<Vec<Sprite>>,
    bosses: Vec<Vec<Vec<Sprite>>>,
    final_boss: Vec<Sprite>,
    strips: Vec<Option<Sprite>>,
    game_over: Sprite,
}

fn art() -> &'static Art {
    static A: OnceLock<Art> = OnceLock::new();
    A.get_or_init(|| {
        let p = Sprite::parse;
        Art {
            ship: p(&data::SHIP),
            shield: [p(&data::SHIELD[0]), p(&data::SHIELD[1])],
            explosion: [
                p(&data::EXPLOSION[0]),
                p(&data::EXPLOSION[1]),
                p(&data::EXPLOSION[2]),
            ],
            missile: p(&data::MISSILE),
            item: [p(&data::ITEM[0]), p(&data::ITEM[1])],
            digits: data::DIGITS.iter().map(p).collect(),
            big_digits: data::BIG_DIGITS.iter().map(p).collect(),
            heart: p(&data::HEART),
            icons: [
                p(&data::ICON_MISSILE),
                p(&data::ICON_LASER),
                p(&data::ICON_WALL),
            ],
            types: data::TYPES
                .iter()
                .map(|t| t.frames.iter().map(p).collect())
                .collect(),
            bosses: LEVELS
                .iter()
                .map(|l| {
                    l.bosses
                        .iter()
                        .map(|b| b.frames.iter().map(p).collect())
                        .collect()
                })
                .collect(),
            final_boss: data::FINAL_BOSS.iter().map(p).collect(),
            strips: LEVELS
                .iter()
                .map(|l| (!l.strip.is_empty()).then(|| Sprite::from_rows(l.strip)))
                .collect(),
            game_over: p(&data::GAME_OVER),
        }
    })
}

/// The recorded full-screen animations.
#[derive(Clone, Copy)]
enum Seq {
    Intro,
    Victory,
    Defeat,
}

/// Recorded screen sequences, parsed once.
fn shots(seq: Seq) -> &'static [(Sprite, u32)] {
    static CACHE: OnceLock<Vec<Vec<(Sprite, u32)>>> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        let parse = |s: &[Recorded]| {
            s.iter()
                .map(|f| (Sprite::parse(&f.bmp), f.ms as u32))
                .collect()
        };
        vec![
            parse(data::INTRO),
            parse(data::VICTORY),
            parse(data::DEFEAT),
        ]
    });
    &all[seq as usize]
}

/// Which frame of a recorded sequence is up after `ms` (clamped or looped).
fn shot_at(seq: Seq, ms: u32, looped: bool) -> (&'static Sprite, bool) {
    let s = shots(seq);
    let total: u32 = s.iter().map(|(_, d)| d).sum::<u32>().max(1);
    let mut t = if looped { ms % total } else { ms };
    for (sp, d) in s {
        if t < *d {
            return (sp, false);
        }
        t -= d;
    }
    (&s[s.len() - 1].0, true)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
}

/// A sprite at a playfield position, for pixel-exact collisions.
#[derive(Clone, Copy)]
struct Body<'a> {
    s: &'a Sprite,
    x: i32,
    y: i32,
}

impl Body<'_> {
    fn touches(&self, o: &Body) -> bool {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (
            (self.x + self.s.w).min(o.x + o.s.w),
            (self.y + self.s.h).min(o.y + o.s.h),
        );
        (y0..y1).any(|y| {
            (x0..x1).any(|x| self.s.get(x - self.x, y - self.y) && o.s.get(x - o.x, y - o.y))
        })
    }

    fn hits_rect(&self, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
        let (x0, y0) = (rx.max(self.x), ry.max(self.y));
        let (x1, y1) = (
            (rx + rw).min(self.x + self.s.w),
            (ry + rh).min(self.y + self.s.h),
        );
        (y0..y1).any(|y| (x0..x1).any(|x| self.s.get(x - self.x, y - self.y)))
    }
}

struct Enemy {
    spawn: usize,
    ty: usize,
    x: i32,
    y: i32,
    hp: u16,
    step: usize,
    vx: i32,
    hit_by: Vec<u32>,
    dead: bool,
}

struct Boss {
    /// Index into the level's bosses, or `None` for the final boss.
    def: Option<usize>,
    x: i32,
    y: i32,
    frame: usize,
    hp: u16,
    born: u32,
    step: usize,
    fire_t: u32,
    hit_by: Vec<u32>,
}

struct Bullet {
    x: i32,
    y: i32,
    vx: i32,
    enemy: bool,
    dead: bool,
}

#[derive(Clone, Copy)]
enum SpecialKind {
    Laser { ttl: u32 },
    Wall,
    Missile,
}

struct Special {
    id: u32,
    kind: SpecialKind,
    x: i32,
    y: i32,
    dead: bool,
}

impl Special {
    fn hits(&self, body: &Body, missile: &Sprite) -> bool {
        match self.kind {
            SpecialKind::Laser { .. } => body.hits_rect(self.x, self.y + 1, W - self.x, 1),
            SpecialKind::Wall => body.hits_rect(self.x, 0, 1, PF_H),
            SpecialKind::Missile => body.touches(&Body {
                s: missile,
                x: self.x,
                y: self.y,
            }),
        }
    }
}

struct Item {
    x: i32,
    y: i32,
    gift: u8,
    dead: bool,
}

struct Blast {
    x: i32,
    y: i32,
    t: u32,
}

pub struct Game {
    phase: Phase,
    phase_ms: u32,
    level: usize,
    lt: u32,
    scroll: i32,
    ship_x: i32,
    ship_y: i32,
    ship_alive: bool,
    respawn_t: u32,
    shield: u32,
    pending_death: bool,
    pub lives: u32,
    pub score: u32,
    pub hi: u32,
    weapon: Weapon,
    ammo: u32,
    enemies: Vec<Enemy>,
    spawn_i: usize,
    bosses: Vec<Boss>,
    next_boss: usize,
    cleared_t: Option<u32>,
    exit_step: usize,
    bullets: Vec<Bullet>,
    specials: Vec<Special>,
    items: Vec<Item>,
    blasts: Vec<Blast>,
    fire_cool: u32,
    prev: Input,
    rng: Rng,
    next_id: u32,
    sfx: Vec<Sfx>,
}

impl Game {
    pub fn new(seed: u64, hi: u32) -> Game {
        Game {
            phase: Phase::Title,
            phase_ms: 0,
            level: 0,
            lt: 0,
            scroll: 0,
            ship_x: data::START_POS.0 as i32,
            ship_y: data::START_POS.1 as i32,
            ship_alive: true,
            respawn_t: 0,
            shield: 0,
            pending_death: false,
            lives: START_LIVES,
            score: 0,
            hi,
            weapon: Weapon::Missile,
            ammo: 3,
            enemies: vec![],
            spawn_i: 0,
            bosses: vec![],
            next_boss: 0,
            cleared_t: None,
            exit_step: 0,
            bullets: vec![],
            specials: vec![],
            items: vec![],
            blasts: vec![],
            fire_cool: 0,
            prev: Input::default(),
            rng: Rng(seed | 1),
            next_id: 0,
            sfx: vec![],
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn level(&self) -> usize {
        self.level + 1
    }
    pub fn in_game(&self) -> bool {
        self.phase == Phase::Play
    }
    pub fn drain_sfx(&mut self) -> Vec<Sfx> {
        std::mem::take(&mut self.sfx)
    }

    /// Start a new game at `level` (1-6).
    pub fn start_at(&mut self, level: usize) {
        self.lives = START_LIVES;
        self.score = 0;
        self.weapon = Weapon::Missile;
        self.ammo = 3;
        self.load_level(level.clamp(1, 6) - 1);
        self.set_phase(Phase::Play);
    }

    pub fn start(&mut self) {
        self.start_at(1);
    }

    /// Leave the current game for the title screen.
    pub fn quit(&mut self) {
        self.hi = self.hi.max(self.score);
        self.set_phase(Phase::Title);
        // Skip straight to the logo rather than replaying the intro.
        self.phase_ms = u32::MAX / 2;
    }

    /// Test hook: keep the spawn shield up for a long time.
    pub fn debug_invulnerable(&mut self) {
        self.shield = 1_000_000;
    }

    fn set_phase(&mut self, p: Phase) {
        self.phase = p;
        self.phase_ms = 0;
    }

    fn load_level(&mut self, i: usize) {
        self.level = i;
        self.lt = 0;
        self.scroll = 0;
        self.ship_x = data::START_POS.0 as i32;
        self.ship_y = data::START_POS.1 as i32;
        self.ship_alive = true;
        self.shield = SHIELD_TICKS;
        self.enemies.clear();
        self.spawn_i = 0;
        self.bosses.clear();
        self.next_boss = 0;
        self.cleared_t = None;
        self.exit_step = 0;
        self.bullets.clear();
        self.specials.clear();
        self.items.clear();
        self.blasts.clear();
        self.fire_cool = 0;
    }

    fn end_game(&mut self, won: bool) {
        self.hi = self.hi.max(self.score);
        self.set_phase(if won { Phase::Victory } else { Phase::Defeat });
        self.sfx.push(Sfx::GameOver);
    }

    pub fn tick(&mut self, input: Input) {
        self.phase_ms += TICK_MS as u32;
        let edge = |now: bool, before: bool| now && !before;
        let select = edge(
            input.select || input.fire,
            self.prev.select || self.prev.fire,
        );
        match self.phase {
            Phase::Title => {
                if select {
                    self.start();
                }
            }
            Phase::Victory | Phase::Defeat => {
                let seq = if self.phase == Phase::Victory {
                    Seq::Victory
                } else {
                    Seq::Defeat
                };
                if shot_at(seq, self.phase_ms, false).1 || (select && self.phase_ms > 600) {
                    self.set_phase(Phase::GameOver);
                    self.sfx.push(Sfx::GameOver);
                }
            }
            Phase::GameOver => {
                if select && self.phase_ms > 600 {
                    self.start();
                }
            }
            Phase::Play => self.step_play(input),
        }
        self.prev = input;
    }

    fn level_def(&self) -> &'static data::LevelDef {
        &LEVELS[self.level]
    }

    fn step_play(&mut self, input: Input) {
        let def = self.level_def();
        self.lt += 1;

        // The scenery scrolls one pixel per tick until a boss turns up.
        let strip_w = art().strips[self.level].as_ref().map_or(0, |s| s.w);
        if self.bosses.is_empty() && self.cleared_t.is_none() && self.scroll + W < strip_w {
            self.scroll += 1;
        }

        while self.spawn_i < def.spawns.len() && def.spawns[self.spawn_i].tick as u32 <= self.lt {
            let s = &def.spawns[self.spawn_i];
            let (t0, x, y) = s.path[0];
            // Speed to keep once the recording runs out: the path's average,
            // always leftward so nothing hangs on screen or drifts back.
            let (t1, x1, _) = s.path[s.path.len() - 1];
            let avg = (x1 as f32 - x as f32) / (t1 as f32 - t0 as f32).max(1.0);
            self.enemies.push(Enemy {
                spawn: self.spawn_i,
                ty: s.ty as usize,
                x: x as i32,
                y: y as i32,
                hp: if s.hp > 0 {
                    s.hp
                } else {
                    data::TYPES[s.ty as usize].hp.max(1)
                },
                step: 0,
                vx: (avg.round() as i32).min(-1),
                hit_by: vec![],
                dead: false,
            });
            self.spawn_i += 1;
        }
        if self.cleared_t.is_none() {
            if self.next_boss < def.bosses.len() {
                let b = &def.bosses[self.next_boss];
                if self.lt >= b.path[0].0 as u32 {
                    let (_, x, y, f) = b.path[0];
                    self.bosses.push(Boss {
                        def: Some(self.next_boss),
                        x: x as i32,
                        y: y as i32,
                        frame: f as usize,
                        hp: b.hp,
                        born: self.lt,
                        step: 0,
                        fire_t: 0,
                        hit_by: vec![],
                    });
                    self.next_boss += 1;
                }
            } else if def.bosses.is_empty()
                && self.next_boss == 0
                && self.spawn_i >= def.spawns.len()
                && self.enemies.is_empty()
            {
                // The last level: the giant eye slides in once the final wave is gone.
                self.bosses.push(Boss {
                    def: None,
                    x: W,
                    y: data::FINAL_BOSS_AT.1 as i32,
                    frame: 0,
                    hp: FINAL_BOSS_HP,
                    born: self.lt,
                    step: 0,
                    fire_t: 0,
                    hit_by: vec![],
                });
                self.next_boss = 1;
            }
        }

        self.step_ship(input);
        self.step_enemies();
        self.step_bosses();
        // Bullets stop where they meet solid scenery (every pixel they sweep through).
        let solid = if SOLID_SCENERY[self.level] {
            art().strips[self.level].as_ref()
        } else {
            None
        };
        for s in &mut self.bullets {
            let x0 = s.x;
            s.x += s.vx;
            s.dead |= s.x < -2 || s.x > W;
            if let Some(strip) = solid {
                let (lo, hi) = (x0.min(s.x), x0.max(s.x) + 1);
                s.dead |= (lo..=hi).any(|x| (0..W).contains(&x) && strip.get(self.scroll + x, s.y));
            }
        }
        self.step_specials();
        for it in &mut self.items {
            it.x -= 1;
            it.dead |= it.x < -8;
        }
        self.collide();
        for b in &mut self.blasts {
            b.t += 1;
        }
        self.blasts.retain(|b| b.t < 3);
        self.enemies.retain(|e| !e.dead);
        self.bullets.retain(|s| !s.dead);
        self.specials.retain(|s| !s.dead);
        self.items.retain(|i| !i.dead);

        // All bosses down: the level is cleared.
        let all_bosses_spawned = self.next_boss >= def.bosses.len().max(1);
        if self.cleared_t.is_none() && all_bosses_spawned && self.bosses.is_empty() && self.lt > 1 {
            self.cleared_t = Some(self.lt);
            self.bullets.retain(|s| !s.enemy);
        }
    }

    fn step_ship(&mut self, input: Input) {
        self.shield = self.shield.saturating_sub(1);
        if !self.ship_alive {
            if self.respawn_t > 0 {
                self.respawn_t -= 1;
                if self.respawn_t == 0 {
                    if self.lives == 0 {
                        self.end_game(false);
                    } else {
                        self.ship_alive = true;
                        self.shield = SHIELD_TICKS;
                    }
                }
            }
            return;
        }
        if let Some(t) = self.cleared_t {
            // Level cleared: hold for a moment, then accelerate off to the right.
            if self.lt - t > EXIT_HOLD {
                self.ship_x += EXIT_STEPS.get(self.exit_step).copied().unwrap_or(14);
                self.exit_step += 1;
                if self.ship_x >= W {
                    self.sfx.push(Sfx::LevelClear);
                    if self.level + 1 == LEVELS.len() {
                        self.end_game(true);
                    } else {
                        let next = self.level + 1;
                        self.load_level(next);
                    }
                }
            }
            return;
        }
        if input.left {
            self.ship_x -= 2;
        }
        if input.right {
            self.ship_x += 2;
        }
        if input.up {
            self.ship_y -= 2;
        }
        if input.down {
            self.ship_y += 2;
        }
        self.ship_x = self.ship_x.clamp(0, SHIP_MAX_X);
        self.ship_y = self.ship_y.clamp(1, PF_H - SHIP_H - 1);

        self.fire_cool = self.fire_cool.saturating_sub(1);
        if input.fire && self.fire_cool == 0 {
            self.bullets.push(Bullet {
                x: self.ship_x + SHIP_W,
                y: self.ship_y + 3,
                vx: 2,
                enemy: false,
                dead: false,
            });
            self.fire_cool = FIRE_EVERY;
            self.sfx.push(Sfx::Shoot);
        }
        if input.special && !self.prev.special && self.ammo > 0 {
            self.ammo -= 1;
            self.next_id += 1;
            let kind = match self.weapon {
                Weapon::Laser => SpecialKind::Laser { ttl: LASER_TICKS },
                Weapon::Wall => SpecialKind::Wall,
                Weapon::Missile => SpecialKind::Missile,
            };
            let (x, y) = (self.ship_x + SHIP_W, self.ship_y + 2);
            self.specials.push(Special {
                id: self.next_id,
                kind,
                x,
                y,
                dead: false,
            });
            self.sfx.push(if self.weapon == Weapon::Wall {
                Sfx::SpecialWall
            } else {
                Sfx::Special
            });
        }
    }

    fn step_enemies(&mut self) {
        let def = self.level_def();
        let lt = self.lt as i32;
        for e in &mut self.enemies {
            let path = def.spawns[e.spawn].path;
            while e.step + 1 < path.len() && (path[e.step + 1].0 as i32) <= lt {
                e.step += 1;
            }
            let (t0, x0, y0) = path[e.step];
            if e.step + 1 < path.len() {
                // Interpolate across ticks the footage did not observe.
                let (t1, x1, y1) = path[e.step + 1];
                let span = (t1 as i32 - t0 as i32).max(1);
                let f = (lt - t0 as i32) as f32 / span as f32;
                e.x = (x0 as f32 + (x1 as i32 - x0 as i32) as f32 * f).round() as i32;
                e.y = (y0 as f32 + (y1 as i32 - y0 as i32) as f32 * f).round() as i32;
            } else if lt > t0 as i32 {
                // Beyond the footage: carry on at the path's average speed.
                e.x += e.vx;
            }
            let w = art().types[e.ty].first().map_or(8, |s| s.w);
            e.dead |= e.x + w < -2 || e.x > W + 40;
        }
    }

    fn step_bosses(&mut self) {
        let lt = self.lt;
        let level = self.level;
        for b in &mut self.bosses {
            b.fire_t += 1;
            let Some(i) = b.def else {
                let (fx, _) = data::FINAL_BOSS_AT;
                if b.x > fx as i32 {
                    b.x -= 1;
                }
                b.frame = ((lt / 2) % 4) as usize;
                if b.fire_t >= 10 && b.x <= fx as i32 {
                    b.fire_t = 0;
                    let y = 12 + (self.rng.next() % 16) as i32;
                    self.bullets.push(Bullet {
                        x: b.x + 4,
                        y,
                        vx: -2,
                        enemy: true,
                        dead: false,
                    });
                }
                continue;
            };
            let def = &LEVELS[level].bosses[i];
            let path = def.path;
            let rel = |k: usize| path[k].0 as u32 - path[0].0 as u32;
            // Replay the recorded path; once it runs out, loop its last two thirds.
            let span = rel(path.len() - 1).max(1);
            let from = rel(path.len() / 3);
            let age = lt - b.born;
            let t = if age <= span {
                age
            } else {
                from + (age - span) % (span - from).max(1)
            };
            while b.step + 1 < path.len() && rel(b.step + 1) <= t {
                b.step += 1;
            }
            while b.step > 0 && rel(b.step) > t {
                b.step -= 1;
            }
            let (_, x, y, f) = path[b.step];
            b.x = x as i32;
            b.y = y as i32;
            b.frame = f as usize;
            if b.fire_t >= 12 {
                b.fire_t = 0;
                let h = art().bosses[level][i].first().map_or(10, |s| s.h);
                self.bullets.push(Bullet {
                    x: b.x - 2,
                    y: b.y + h / 2,
                    vx: -2,
                    enemy: true,
                    dead: false,
                });
            }
        }
    }

    fn step_specials(&mut self) {
        let spawns = self.level_def().spawns;
        let targets: Vec<(i32, i32)> = self
            .enemies
            .iter()
            .filter(|e| !spawns[e.spawn].decor)
            .map(|e| (e.x, e.y))
            .collect();
        for sp in &mut self.specials {
            match &mut sp.kind {
                SpecialKind::Laser { ttl } => {
                    sp.dead = *ttl == 0;
                    *ttl = ttl.saturating_sub(1);
                }
                SpecialKind::Wall => {
                    sp.x += 2;
                    sp.dead = sp.x >= W;
                }
                SpecialKind::Missile => {
                    sp.x += 2;
                    let ahead = targets
                        .iter()
                        .filter(|(tx, _)| *tx > sp.x)
                        .min_by_key(|(tx, _)| *tx);
                    if let Some(&(_, ty)) = ahead {
                        sp.y += (ty + 2 - sp.y).signum();
                    }
                    sp.dead = sp.x >= W;
                }
            }
        }
    }

    fn kill_ship(&mut self) {
        if !self.ship_alive || self.shield > 0 || self.cleared_t.is_some() {
            return;
        }
        self.ship_alive = false;
        self.lives = self.lives.saturating_sub(1);
        self.respawn_t = 3;
        self.blasts.push(Blast {
            x: self.ship_x + 1,
            y: self.ship_y,
            t: 0,
        });
        self.sfx.push(Sfx::Die);
    }

    fn boss_sprite(&self, b: &Boss) -> &'static Sprite {
        let a = art();
        match b.def {
            Some(i) => {
                let fr = &a.bosses[self.level][i];
                &fr[b.frame % fr.len().max(1)]
            }
            None => &a.final_boss[b.frame % a.final_boss.len()],
        }
    }

    fn collide(&mut self) {
        let a = art();
        let ship = Body {
            s: &a.ship,
            x: self.ship_x,
            y: self.ship_y,
        };
        let vulnerable = self.ship_alive && self.shield == 0;
        let mut score = 0u32;
        let mut hit = false;
        let mut boom = false;
        let mut drops = vec![];
        let frame = self.lt as usize;
        let spawns = self.level_def().spawns;

        for e in &mut self.enemies {
            let fr = &a.types[e.ty];
            // Background decorations (level 2's clouds) never collide with anything.
            if fr.is_empty() || spawns[e.spawn].decor {
                continue;
            }
            let body = Body {
                s: &fr[frame % fr.len()],
                x: e.x,
                y: e.y,
            };
            for s in self.bullets.iter_mut().filter(|s| !s.enemy && !s.dead) {
                if body.hits_rect(s.x, s.y, 2, 1) {
                    s.dead = true;
                    e.hp = e.hp.saturating_sub(1);
                    score += 5;
                    hit = true;
                }
            }
            for sp in self.specials.iter_mut().filter(|s| !s.dead) {
                if !e.hit_by.contains(&sp.id) && sp.hits(&body, &a.missile) {
                    e.hit_by.push(sp.id);
                    e.hp = e.hp.saturating_sub(10);
                    score += 5;
                    sp.dead |= matches!(sp.kind, SpecialKind::Missile);
                }
            }
            if e.hp == 0 {
                e.dead = true;
                score += 5;
                boom = true;
                self.blasts.push(Blast {
                    x: e.x + body.s.w / 2 - 3,
                    y: e.y + body.s.h / 2 - 3,
                    t: 0,
                });
                let gift = spawns[e.spawn].gift;
                if gift != 0 {
                    drops.push(Item {
                        x: e.x,
                        y: e.y,
                        gift,
                        dead: false,
                    });
                }
            } else if self.ship_alive && body.touches(&ship) {
                if self.shield > 0 {
                    boom = true;
                } else {
                    self.pending_death = true;
                }
                e.dead = true;
                self.blasts.push(Blast {
                    x: e.x,
                    y: e.y,
                    t: 0,
                });
            }
        }
        self.items.extend(drops);

        let mut killed = vec![];
        for bi in 0..self.bosses.len() {
            let s = self.boss_sprite(&self.bosses[bi]);
            let b = &mut self.bosses[bi];
            let body = Body { s, x: b.x, y: b.y };
            for sh in self.bullets.iter_mut().filter(|s| !s.enemy && !s.dead) {
                if body.hits_rect(sh.x, sh.y, 2, 1) {
                    sh.dead = true;
                    b.hp = b.hp.saturating_sub(1);
                    score += 5;
                    hit = true;
                }
            }
            for sp in self.specials.iter_mut().filter(|s| !s.dead) {
                if !b.hit_by.contains(&sp.id) && sp.hits(&body, &a.missile) {
                    b.hit_by.push(sp.id);
                    b.hp = b.hp.saturating_sub(5);
                    score += 5;
                    sp.dead |= matches!(sp.kind, SpecialKind::Missile);
                }
            }
            if b.hp == 0 {
                killed.push(bi);
                score += 100;
                for k in 0..3 {
                    self.blasts.push(Blast {
                        x: b.x + 4 + k * 5,
                        y: b.y + 4 + k * 3,
                        t: 0,
                    });
                }
            } else if vulnerable && body.touches(&ship) {
                self.pending_death = true;
            }
        }
        for bi in killed.into_iter().rev() {
            self.bosses.remove(bi);
            self.sfx.push(Sfx::BossDie);
        }

        if self.ship_alive {
            for s in self.bullets.iter_mut().filter(|s| s.enemy && !s.dead) {
                if ship.hits_rect(s.x, s.y, 2, 1) {
                    s.dead = true;
                    self.pending_death |= self.shield == 0;
                }
            }
        }

        if vulnerable && SOLID_SCENERY[self.level] {
            if let Some(strip) = a.strips[self.level].as_ref() {
                let crash = (0..SHIP_H).any(|y| {
                    (0..SHIP_W).any(|x| {
                        a.ship.get(x, y)
                            && strip.get(self.scroll + self.ship_x + x, self.ship_y + y)
                    })
                });
                self.pending_death |= crash;
            }
        }

        // Items: bullets score off them, flying into one collects it.
        for it in &mut self.items {
            let body = Body {
                s: &a.item[0],
                x: it.x,
                y: it.y,
            };
            for s in self.bullets.iter_mut().filter(|s| !s.enemy && !s.dead) {
                if body.hits_rect(s.x, s.y, 2, 1) {
                    s.dead = true;
                    score += 5;
                }
            }
            if self.ship_alive && body.touches(&ship) {
                it.dead = true;
                self.sfx.push(Sfx::PowerUp);
                let gift = match it.gift {
                    2 => Some(Weapon::Laser),
                    3 => Some(Weapon::Wall),
                    4 => Some(Weapon::Missile),
                    _ => None,
                };
                match gift {
                    None => self.lives = (self.lives + 1).min(MAX_LIVES),
                    Some(w) if w == self.weapon => {
                        self.ammo = (self.ammo + if w == Weapon::Wall { 1 } else { 3 }).min(99)
                    }
                    Some(w) => {
                        self.weapon = w;
                        self.ammo = if w == Weapon::Wall { 1 } else { 3 };
                    }
                }
            }
        }

        if std::mem::take(&mut self.pending_death) {
            self.kill_ship();
        }
        if hit {
            self.sfx.push(Sfx::Hit);
        }
        if boom {
            self.sfx.push(Sfx::Explode);
        }
        self.score = (self.score + score).min(99_999);
    }

    // ---------------------------------------------------------------- drawing

    pub fn render(&self, f: &mut Frame) {
        let a = art();
        f.clear();
        match self.phase {
            Phase::Title => f.paste(
                shot_at(Seq::Intro, self.phase_ms.min(1 << 30), false).0,
                0,
                0,
            ),
            Phase::Victory => f.paste(shot_at(Seq::Victory, self.phase_ms, false).0, 0, 0),
            Phase::Defeat => f.paste(shot_at(Seq::Defeat, self.phase_ms, false).0, 0, 0),
            Phase::GameOver => {
                f.paste(&a.game_over, 0, 0);
                let (x, y) = data::GAME_OVER_DIGITS_AT;
                draw_big_number(f, self.score, x as i32, y as i32);
            }
            Phase::Play => {
                self.draw_play(f);
                if self.level_def().dark {
                    f.invert();
                }
            }
        }
    }

    fn draw_play(&self, f: &mut Frame) {
        let a = art();
        let def = self.level_def();
        let (pf, hud) = if def.hud_bottom { (0, 43) } else { (5, 0) };
        for i in 0..self.lives.min(MAX_LIVES) as i32 {
            f.blit(&a.heart, i * 6, hud);
        }
        let icon = match self.weapon {
            Weapon::Missile => &a.icons[0],
            Weapon::Laser => &a.icons[1],
            Weapon::Wall => &a.icons[2],
        };
        f.blit(icon, 36, hud);
        f.blit(&a.digits[(self.ammo / 10 % 10) as usize], 43, hud);
        f.blit(&a.digits[(self.ammo % 10) as usize], 47, hud);
        for (i, c) in format!("{:05}", self.score.min(99_999)).bytes().enumerate() {
            f.blit(&a.digits[(c - b'0') as usize], 57 + i as i32 * 4, hud);
        }

        let (y0, y1) = (pf, pf + PF_H);
        if let Some(strip) = a.strips[self.level].as_ref() {
            for y in 0..PF_H {
                for x in 0..W {
                    if strip.get(self.scroll + x, y) {
                        f.set(x, pf + y, true);
                    }
                }
            }
        }
        let frame = self.lt as usize;
        for it in &self.items {
            f.blit_clip(&a.item[frame % 2], it.x, pf + it.y, y0, y1);
        }
        for e in &self.enemies {
            let fr = &a.types[e.ty];
            if !fr.is_empty() {
                f.blit_clip(&fr[frame % fr.len()], e.x, pf + e.y, y0, y1);
            }
        }
        for b in &self.bosses {
            f.blit_clip(self.boss_sprite(b), b.x, pf + b.y, y0, y1);
        }
        if self.ship_alive {
            if self.shield > 0 {
                let (dx, dy) = data::SHIELD_SHIP_OFFSET;
                f.blit_clip(
                    &a.shield[frame % 2],
                    self.ship_x - dx as i32,
                    pf + self.ship_y - dy as i32,
                    y0,
                    y1,
                );
            } else {
                f.blit_clip(&a.ship, self.ship_x, pf + self.ship_y, y0, y1);
            }
        }
        for s in &self.bullets {
            f.rect(s.x, pf + s.y, 2, 1, true);
        }
        for sp in &self.specials {
            match sp.kind {
                SpecialKind::Laser { .. } => f.rect(sp.x, pf + sp.y + 1, W - sp.x, 1, true),
                SpecialKind::Wall => f.rect(sp.x, pf, 1, PF_H, true),
                SpecialKind::Missile => f.blit_clip(&a.missile, sp.x, pf + sp.y, y0, y1),
            }
        }
        for b in &self.blasts {
            f.blit_clip(&a.explosion[b.t as usize % 3], b.x, pf + b.y, y0, y1);
        }
    }
}

/// The large score digits of the Top score and Game over screens, 8 px apart.
fn draw_big_number(f: &mut Frame, n: u32, x: i32, y: i32) {
    for (i, c) in n.to_string().bytes().enumerate() {
        f.blit(&art().big_digits[(c - b'0') as usize], x + i as i32 * 8, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_parse() {
        let a = art();
        assert_eq!((a.ship.w, a.ship.h), (10, 7));
        assert!(a.strips[1].as_ref().unwrap().w > W);
        assert_eq!(a.final_boss.len(), 4);
        for s in [Seq::Intro, Seq::Victory, Seq::Defeat] {
            assert!(!shots(s).is_empty());
        }
    }

    /// Every level can be cleared by a shielded, constantly firing ship.
    #[test]
    fn levels_complete_with_autopilot() {
        for lvl in 1..=6 {
            let mut g = Game::new(7, 0);
            g.start_at(lvl);
            let mut t = 0;
            while g.level() == lvl && g.phase == Phase::Play && t < 12_000 {
                g.shield = 5;
                g.lives = 3;
                let up = (t / 20) % 2 == 0;
                g.tick(Input {
                    fire: t % 2 == 0,
                    up,
                    down: !up,
                    ..Default::default()
                });
                t += 1;
            }
            assert!(
                g.level() == lvl + 1 || g.phase == Phase::Victory,
                "level {lvl} stuck after {t} ticks (bosses {:?})",
                g.bosses.iter().map(|b| b.hp).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn clouds_are_not_solid() {
        let mut g = Game::new(1, 0);
        g.start_at(2);
        g.shield = 0;
        // Park the ship on the bottom row, inside the cloud strip.
        g.ship_y = PF_H - SHIP_H - 1;
        // Drop real enemies and their shots, but keep the floating clouds.
        let spawns = LEVELS[1].spawns;
        let mut clouds_seen = 0;
        for _ in 0..600 {
            g.tick(Input {
                down: true,
                ..Default::default()
            });
            g.shield = 0;
            g.enemies.retain(|e| spawns[e.spawn].decor);
            clouds_seen += g.enemies.len();
            g.bullets.clear();
        }
        assert!(clouds_seen > 0, "no floating clouds passed by");
        assert_eq!(g.lives, START_LIVES);
    }

    #[test]
    fn bullets_stop_at_terrain() {
        let mut g = Game::new(1, 0);
        g.start_at(3);
        g.debug_invulnerable();
        let strip = art().strips[2].as_ref().unwrap();
        // A terrain pixel in the right half of the screen, with open space to its left.
        let (tx, ty) = (40..W)
            .flat_map(|x| (0..PF_H).map(move |y| (x, y)))
            .find(|&(x, y)| {
                strip.get(g.scroll + x, y) && !(20..x).any(|c| strip.get(g.scroll + c, y))
            })
            .expect("no terrain on screen");
        g.enemies.clear();
        g.bullets.clear();
        g.bullets.push(Bullet {
            x: 20,
            y: ty,
            vx: 2,
            enemy: false,
            dead: false,
        });
        let mut scroll0 = g.scroll;
        for _ in 0..40 {
            g.tick(Input::default());
            g.enemies.clear();
            g.bullets.retain(|b| !b.enemy);
            if g.bullets.is_empty() {
                break;
            }
            let b = &g.bullets[0];
            assert!(
                b.x + (g.scroll - scroll0) <= tx + 1,
                "bullet passed through terrain"
            );
            scroll0 = scroll0.min(g.scroll);
        }
        assert!(g.bullets.is_empty(), "bullet never stopped");
    }

    #[test]
    fn level3_miniboss_holds_the_stage() {
        // The jellyfish mini-boss soaks up hits instead of dying like a grunt.
        let s = LEVELS[2]
            .spawns
            .iter()
            .find(|s| s.hp > 0)
            .expect("no mini-boss on level 3");
        assert!(s.hp >= 20);
        // Bosses glide in from the right edge.
        for l in &LEVELS {
            for b in l.bosses {
                assert!(b.path[0].1 >= 80, "boss pops in at x {}", b.path[0].1);
            }
        }
    }

    #[test]
    fn enemies_never_move_backwards() {
        for lvl in 1..=6 {
            let mut g = Game::new(5, 0);
            g.start_at(lvl);
            let mut last: std::collections::HashMap<usize, i32> = Default::default();
            for _ in 0..3000 {
                g.debug_invulnerable();
                g.tick(Input::default());
                if g.phase != Phase::Play {
                    break;
                }
                for e in &g.enemies {
                    if let Some(&px) = last.get(&e.spawn) {
                        assert!(
                            e.x <= px,
                            "level {lvl}: enemy {} moved right {} -> {}",
                            e.spawn,
                            px,
                            e.x
                        );
                    }
                    last.insert(e.spawn, e.x);
                }
            }
        }
    }

    #[test]
    fn title_flow() {
        let mut g = Game::new(1, 0);
        assert_eq!(g.phase, Phase::Title);
        g.tick(Input {
            select: true,
            ..Default::default()
        });
        assert_eq!(g.phase, Phase::Play);
        g.quit();
        assert_eq!(g.phase, Phase::Title);
        g.tick(Input::default());
        g.tick(Input {
            fire: true,
            ..Default::default()
        });
        assert_eq!(g.phase, Phase::Play);
    }
}
