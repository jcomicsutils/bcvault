pub mod oklab;

use crate::motion::oklab::OkLab;
use egui::{Color32, Id, Vec2};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EffectsLevel {
    Off,
    Low,
    #[default]
    Full,
}

impl EffectsLevel {
    pub const ALL: [Self; 3] = [Self::Off, Self::Low, Self::Full];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Full => "full",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Full => "Full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "low" => Some(Self::Low),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringCfg {
    pub k: f32,
    pub c: f32,
    pub eps: f32,
}

impl SpringCfg {
    pub const SNAP: Self = Self {
        k: 900.0,
        c: 60.0,
        eps: 0.001,
    };
    pub const SMOOTH: Self = Self {
        k: 320.0,
        c: 34.0,
        eps: 0.001,
    };
    pub const SOFT: Self = Self {
        k: 140.0,
        c: 24.0,
        eps: 0.001,
    };
    pub const POP: Self = Self {
        k: 500.0,
        c: 26.0,
        eps: 0.001,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
    pub target: f32,
}

impl Spring {
    pub const fn new(val: f32) -> Self {
        Self {
            x: val,
            v: 0.0,
            target: val,
        }
    }

    pub const fn from_to(from: f32, target: f32) -> Self {
        Self {
            x: from,
            v: 0.0,
            target,
        }
    }

    /// Fixed 1/240 s sub-steps => identical result at 30, 60 or 144 fps. Returns true while moving.
    pub fn step(&mut self, cfg: SpringCfg, dt: f32) -> bool {
        let mut left = dt.clamp(0.0, 0.05);
        while left > 1e-6 {
            let h = if (left - (1.0 / 240.0)).abs() < 1e-5 {
                left
            } else {
                left.min(1.0 / 240.0)
            };
            let a = -cfg.k * (self.x - self.target) - cfg.c * self.v;
            self.v += a * h;
            self.x += self.v * h;
            left -= h;
        }
        if (self.x - self.target).abs() < cfg.eps && self.v.abs() < cfg.eps * 10.0 {
            self.x = self.target;
            self.v = 0.0;
            false
        } else {
            true
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum EntryKind {
    Scalar(Spring),
    Vec2([Spring; 2]),
    Color([Spring; 4]),
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    kind: EntryKind,
    last_touched_frame: u64,
}

pub struct Motion {
    entries: HashMap<Id, Entry>,
    frame: u64,
    dt: f32,
    active_count: usize,
    pub reduced_motion: bool,
    pub level: EffectsLevel,
}

impl Default for Motion {
    fn default() -> Self {
        Self::new()
    }
}

impl Motion {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            frame: 0,
            dt: 1.0 / 60.0,
            active_count: 0,
            reduced_motion: false,
            level: EffectsLevel::Full,
        }
    }

    pub fn is_instant(&self) -> bool {
        self.reduced_motion || self.level == EffectsLevel::Off
    }

    pub fn active_springs(&self) -> usize {
        self.active_count
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn has(&self, id: impl Hash) -> bool {
        self.entries.contains_key(&Id::new(id))
    }

    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 500.0, 0.05);
        self.begin_frame_dt(dt);
    }

    pub fn begin_frame_dt(&mut self, dt: f32) {
        self.frame = self.frame.wrapping_add(1);
        self.dt = dt.clamp(1.0 / 500.0, 0.05);
        self.active_count = 0;
        let current_frame = self.frame;
        self.entries
            .retain(|_, e| current_frame.saturating_sub(e.last_touched_frame) <= 120);
    }

    pub fn val(&mut self, id: impl Hash, target: f32, cfg: SpringCfg) -> f32 {
        let key = Id::new(id);
        let instant = self.is_instant();
        let frame = self.frame;
        let dt = self.dt;
        let entry = self.entries.entry(key).or_insert(Entry {
            kind: EntryKind::Scalar(Spring::new(target)),
            last_touched_frame: frame,
        });
        entry.last_touched_frame = frame;
        match &mut entry.kind {
            EntryKind::Scalar(sp) => {
                sp.target = target;
                if instant {
                    sp.x = target;
                    sp.v = 0.0;
                    target
                } else {
                    if sp.step(cfg, dt) {
                        self.active_count += 1;
                    }
                    sp.x
                }
            }
            _ => {
                entry.kind = EntryKind::Scalar(Spring::new(target));
                target
            }
        }
    }

    pub fn val_from(&mut self, id: impl Hash, from: f32, target: f32, cfg: SpringCfg) -> f32 {
        let key = Id::new(id);
        let instant = self.is_instant();
        let frame = self.frame;
        let dt = self.dt;
        let entry = self.entries.entry(key).or_insert(Entry {
            kind: EntryKind::Scalar(Spring::from_to(if instant { target } else { from }, target)),
            last_touched_frame: frame,
        });
        entry.last_touched_frame = frame;
        match &mut entry.kind {
            EntryKind::Scalar(sp) => {
                sp.target = target;
                if instant {
                    sp.x = target;
                    sp.v = 0.0;
                    target
                } else {
                    if sp.step(cfg, dt) {
                        self.active_count += 1;
                    }
                    sp.x
                }
            }
            _ => {
                entry.kind = EntryKind::Scalar(Spring::new(target));
                target
            }
        }
    }

    pub fn set_val(&mut self, id: impl Hash, value: f32) {
        let key = Id::new(id);
        let frame = self.frame;
        self.entries.insert(
            key,
            Entry {
                kind: EntryKind::Scalar(Spring::new(value)),
                last_touched_frame: frame,
            },
        );
    }

    pub fn vec2(&mut self, id: impl Hash, target: Vec2, cfg: SpringCfg) -> Vec2 {
        let key = Id::new(id);
        let instant = self.is_instant();
        let frame = self.frame;
        let dt = self.dt;
        let entry = self.entries.entry(key).or_insert(Entry {
            kind: EntryKind::Vec2([Spring::new(target.x), Spring::new(target.y)]),
            last_touched_frame: frame,
        });
        entry.last_touched_frame = frame;
        match &mut entry.kind {
            EntryKind::Vec2([sx, sy]) => {
                sx.target = target.x;
                sy.target = target.y;
                if instant {
                    sx.x = target.x;
                    sx.v = 0.0;
                    sy.x = target.y;
                    sy.v = 0.0;
                    target
                } else {
                    let mx = sx.step(cfg, dt);
                    let my = sy.step(cfg, dt);
                    if mx || my {
                        self.active_count += 1;
                    }
                    Vec2::new(sx.x, sy.x)
                }
            }
            _ => {
                entry.kind = EntryKind::Vec2([Spring::new(target.x), Spring::new(target.y)]);
                target
            }
        }
    }

    pub fn set_vec2(&mut self, id: impl Hash, value: Vec2) {
        let key = Id::new(id);
        let frame = self.frame;
        self.entries.insert(
            key,
            Entry {
                kind: EntryKind::Vec2([Spring::new(value.x), Spring::new(value.y)]),
                last_touched_frame: frame,
            },
        );
    }

    pub fn color(&mut self, id: impl Hash, target: Color32, cfg: SpringCfg) -> Color32 {
        let key = Id::new(id);
        let instant = self.is_instant();
        let frame = self.frame;
        let dt = self.dt;
        let lab = OkLab::from_color32(target);
        let entry = self.entries.entry(key).or_insert(Entry {
            kind: EntryKind::Color([
                Spring::new(lab.l),
                Spring::new(lab.a),
                Spring::new(lab.b),
                Spring::new(lab.alpha),
            ]),
            last_touched_frame: frame,
        });
        entry.last_touched_frame = frame;
        match &mut entry.kind {
            EntryKind::Color([sl, sa, sb, salpha]) => {
                sl.target = lab.l;
                sa.target = lab.a;
                sb.target = lab.b;
                salpha.target = lab.alpha;
                if instant {
                    sl.x = lab.l;
                    sl.v = 0.0;
                    sa.x = lab.a;
                    sa.v = 0.0;
                    sb.x = lab.b;
                    sb.v = 0.0;
                    salpha.x = lab.alpha;
                    salpha.v = 0.0;
                    target
                } else {
                    let m1 = sl.step(cfg, dt);
                    let m2 = sa.step(cfg, dt);
                    let m3 = sb.step(cfg, dt);
                    let m4 = salpha.step(cfg, dt);
                    if m1 || m2 || m3 || m4 {
                        self.active_count += 1;
                    }
                    OkLab {
                        l: sl.x,
                        a: sa.x,
                        b: sb.x,
                        alpha: salpha.x,
                    }
                    .to_color32()
                }
            }
            _ => {
                entry.kind = EntryKind::Color([
                    Spring::new(lab.l),
                    Spring::new(lab.a),
                    Spring::new(lab.b),
                    Spring::new(lab.alpha),
                ]);
                target
            }
        }
    }

    pub fn end_frame(&self, ctx: &egui::Context) {
        if self.active_count > 0 {
            ctx.request_repaint();
        }
    }
}

/// Computes stagger delay in seconds for item `index`, capped at `cap_ms`.
pub fn stagger_delay_s(index: usize, per_item_ms: u32, cap_ms: u32) -> f32 {
    let ms = (index as u32).saturating_mul(per_item_ms).min(cap_ms);
    (ms as f32) / 1000.0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spring_presets_converge_and_respect_overshoot_rules() {
        for (cfg, allow_overshoot) in [
            (SpringCfg::SNAP, false),
            (SpringCfg::SMOOTH, false),
            (SpringCfg::SOFT, false),
            (SpringCfg::POP, true),
        ] {
            let mut sp = Spring::from_to(0.0, 1.0);
            let mut max_val = 0.0_f32;
            let mut settled = false;
            for _ in 0..240 {
                let moving = sp.step(cfg, 1.0 / 60.0);
                max_val = max_val.max(sp.x);
                if !moving {
                    settled = true;
                    break;
                }
            }
            assert!(settled, "preset {cfg:?} did not settle within 4s");
            assert!((sp.x - 1.0).abs() < 1e-5);
            if !allow_overshoot {
                assert!(
                    max_val <= 1.005,
                    "preset {cfg:?} overshot: max_val = {max_val}"
                );
            } else {
                assert!(max_val > 1.01, "POP preset should overshoot slightly");
            }
        }
    }

    #[test]
    fn spring_is_frame_rate_independent() {
        let total_time = 0.5_f32;
        let run_at_fps = |fps: u32| -> f32 {
            let mut sp = Spring::from_to(0.0, 100.0);
            let steps = ((total_time * (fps as f32)).round()) as u32;
            let dt = total_time / (steps as f32);
            for _ in 0..steps {
                sp.step(SpringCfg::SMOOTH, dt);
            }
            sp.x
        };

        let v30 = run_at_fps(30);
        let v60 = run_at_fps(60);
        let v120 = run_at_fps(120);
        assert!((v30 - v60).abs() < 1e-3, "v30={v30} vs v60={v60}");
        assert!((v60 - v120).abs() < 1e-3, "v60={v60} vs v120={v120}");
    }

    #[test]
    fn spring_retargeting_preserves_velocity_and_reduced_motion_snaps() {
        let mut sp = Spring::from_to(0.0, 10.0);
        for _ in 0..5 {
            sp.step(SpringCfg::SMOOTH, 1.0 / 60.0);
        }
        let vel_before = sp.v;
        assert!(vel_before > 0.0);
        sp.target = 5.0;
        assert_eq!(sp.v, vel_before);

        let mut motion = Motion::new();
        motion.reduced_motion = true;
        motion.begin_frame_dt(1.0 / 60.0);
        let v = motion.val_from("k", 0.0, 42.0, SpringCfg::SMOOTH);
        assert_eq!(v, 42.0);
        assert_eq!(motion.active_springs(), 0);
    }

    #[test]
    fn motion_garbage_collects_untouched_entries() {
        let mut motion = Motion::new();
        motion.begin_frame_dt(1.0 / 60.0);
        let _ = motion.val("ephemeral", 1.0, SpringCfg::SNAP);
        assert_eq!(motion.entry_count(), 1);

        for _ in 0..125 {
            motion.begin_frame_dt(1.0 / 60.0);
        }
        assert_eq!(motion.entry_count(), 0);
    }

    #[test]
    fn headless_idle_repaint_delay_is_unlimited_once_settled() {
        let ctx = egui::Context::default();
        let mut motion = Motion::new();
        let mut target = 0.0_f32;

        let mut last_delay = Duration::ZERO;
        for frame_idx in 0..300 {
            if frame_idx == 1 {
                target = 100.0;
            }
            let raw_input = egui::RawInput {
                predicted_dt: 1.0 / 60.0,
                ..Default::default()
            };
            let full_output = ctx.run(raw_input, |ctx| {
                motion.begin_frame_dt(1.0 / 60.0);
                let _v = motion.val("anim", target, SpringCfg::SMOOTH);
                motion.end_frame(ctx);
            });
            if let Some(vp) = full_output.viewport_output.get(&egui::ViewportId::ROOT) {
                last_delay = vp.repaint_delay;
            }
        }

        assert_eq!(motion.active_springs(), 0);
        assert!(
            last_delay >= Duration::from_secs(3600),
            "expected unlimited repaint_delay at rest, got {last_delay:?}"
        );
    }
}
