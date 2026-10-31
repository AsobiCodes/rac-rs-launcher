//! Fits the UI to small screens. The pages are laid out for a window of at least `MIN_W` x `MIN_H` points. A Steam Deck in Gaming Mode
//! (gamescope) gives the launcher a 1280 x 800 pixel window, often with a scale factor of 1.5 or 2, which is only 853 x 533 or
//! 640 x 400 points: the sidebar overlapped, the page ran off the right edge and the Play button was gone. Here the effective zoom
//! is lowered (never raised) until the window holds `MIN_W` x `MIN_H` points again, and the operating system's minimum window size is
//! limited to a margin below the monitor so that it cannot ask for a window bigger than the screen. Windows that are large enough are
//! not touched. The effective zoom is `user zoom * fit`: the user's own zoom (Ctrl +, Ctrl -, Ctrl 0, handled here instead of by egui,
//! which would write the fitted zoom back as the user's zoom) stays separate from the fit and survives a change of the window size.
use egui::{Context, Vec2, ViewportCommand};

/// Smallest window (in points) the pages are laid out for.
pub const MIN_W: f32 = 860.0;
pub const MIN_H: f32 = 580.0;
/// Minimum size (in points) asked for at start-up, before the monitor is known; `Fit::apply` raises it after the first frame.
pub const BOOT_MIN: [f32; 2] = [480.0, 320.0];
/// The window size (in points) the launcher opens with: the size it was closed at, which the operating system's own minimum already
/// bounded while it was saved (`min_window_px`: on a small laptop that is smaller than `MIN_W` x `MIN_H`, and a window the player made
/// smaller to fit the screen must come back that small), but never below `BOOT_MIN`. A first start has the default size of `LauncherCfg`.
pub fn start_size(saved_w: f32, saved_h: f32) -> [f32; 2] {
    [saved_w.max(BOOT_MIN[0]), saved_h.max(BOOT_MIN[1])]
}

/// The UI is never shrunk below this (a 0.4 zoom of a 1280 x 800 window is far more than any real screen needs).
const MIN_ZOOM: f32 = 0.4;
/// Below this many pixels the window is minimized or not laid out yet: leave the zoom alone.
const MIN_REAL_PX: Vec2 = Vec2::new(300.0, 200.0);
/// The OS minimum window is at most this share of the monitor (width, height): a decorated window also needs a title bar and the
/// taskbar, and the monitor size is all the window code reports (no work area), so the rest is left to the fit zoom.
pub const MONITOR_SHARE: Vec2 = Vec2::new(0.90, 0.85);
/// The user's own zoom (Ctrl +, Ctrl -) stays inside egui's limits for it.
const USER_ZOOM_MIN: f32 = 0.2;
const USER_ZOOM_MAX: f32 = 5.0;

/// The zoom factor (at most 1.0) that makes a window of `win_px` pixels hold `MIN_W` x `MIN_H` points when the system scale is
/// `native` pixels per point. 1.0 whenever the window is big enough (every desktop size).
pub fn fit_zoom(win_px: Vec2, native: f32) -> f32 {
    if !(native.is_finite() && native > 0.0) || !(win_px.x.is_finite() && win_px.y.is_finite()) || win_px.x <= 0.0 || win_px.y <= 0.0 {
        return 1.0;
    }
    let z = (win_px.x / (native * MIN_W)).min(win_px.y / (native * MIN_H));
    if z >= 0.999 {
        1.0
    } else {
        z.max(MIN_ZOOM)
    }
}

/// Smallest window (in pixels) the operating system should allow: the design minimum at the system scale, but never more than
/// `MONITOR_SHARE` of the monitor (room for the title bar and the taskbar; the fit zoom handles the rest). Unknown monitor = `BOOT_MIN`
/// (no promise we could keep).
pub fn min_window_px(native: f32, monitor_px: Option<Vec2>) -> Vec2 {
    let native = if native.is_finite() && native > 0.0 { native } else { 1.0 };
    match monitor_px {
        Some(m) if m.x >= MIN_REAL_PX.x && m.y >= MIN_REAL_PX.y => {
            Vec2::new((MIN_W * native).min(m.x * MONITOR_SHARE.x), (MIN_H * native).min(m.y * MONITOR_SHARE.y)).floor()
        }
        _ => Vec2::new(BOOT_MIN[0] * native, BOOT_MIN[1] * native).floor(),
    }
}

/// A zoom key press (egui's own shortcuts: Ctrl +, Ctrl =, Ctrl -, Ctrl 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomKey {
    In,
    Out,
    Reset,
}

/// The user's zoom factor if it is usable, else 1.0.
fn sane_user_zoom(user: f32) -> f32 {
    if user.is_finite() && user > 0.0 {
        user.clamp(USER_ZOOM_MIN, USER_ZOOM_MAX)
    } else {
        1.0
    }
}

/// The user's zoom after one key press, with egui's own rules: steps of 0.1 rounded to tenths, limits 0.2 .. 5.0, Ctrl 0 = 1.0.
pub fn step_user_zoom(user: f32, key: ZoomKey) -> f32 {
    let step = match key {
        ZoomKey::Reset => return 1.0,
        ZoomKey::In => 0.1,
        ZoomKey::Out => -0.1,
    };
    ((sane_user_zoom(user) + step).clamp(USER_ZOOM_MIN, USER_ZOOM_MAX) * 10.0).round() / 10.0
}

/// The zoom factor the UI really runs at: the user's zoom times the fit. The fit only ever lowers (a value above 1.0 or nonsense = 1.0).
pub fn effective_zoom(user: f32, fit: f32) -> f32 {
    let fit = if fit.is_finite() && fit > 0.0 { fit.min(1.0) } else { 1.0 };
    sane_user_zoom(user) * fit
}

/// What the app keeps between frames: the user's zoom (Ctrl +, Ctrl -), the last minimum size it sent (in whole pixels) and the zoom of
/// the previous frame.
pub struct Fit {
    user: f32,
    keys_taken: bool,
    sent_min: Option<(i32, i32)>,
    last_zoom: Option<f32>,
}

impl Default for Fit {
    fn default() -> Self {
        Fit { user: 1.0, keys_taken: false, sent_min: None, last_zoom: None }
    }
}

impl Fit {
    /// The user's own zoom factor (1.0 until Ctrl + / Ctrl - is used); the zoom the UI runs at is this times the fit.
    pub fn user_zoom(&self) -> f32 {
        self.user
    }

    /// Ctrl +, Ctrl = , Ctrl - and Ctrl 0 change the user's zoom, as egui's own handler did before (one step per key per frame). That
    /// handler is switched off: it would take the fitted zoom for the user's zoom and snap back to the fit one frame later.
    fn take_zoom_keys(&mut self, ctx: &Context) {
        use egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};
        if !self.keys_taken {
            self.keys_taken = true;
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
        }
        let (reset, zoom_in, zoom_out) = ctx.input_mut(|i| {
            if i.consume_shortcut(&ZOOM_RESET) {
                (true, false, false)
            } else {
                (false, i.consume_shortcut(&ZOOM_IN) | i.consume_shortcut(&ZOOM_IN_SECONDARY), i.consume_shortcut(&ZOOM_OUT))
            }
        });
        if reset {
            self.user = step_user_zoom(self.user, ZoomKey::Reset);
        }
        if zoom_in {
            self.user = step_user_zoom(self.user, ZoomKey::In);
        }
        if zoom_out {
            self.user = step_user_zoom(self.user, ZoomKey::Out);
        }
    }

    /// Call once per frame before the pages are drawn.
    pub fn apply(&mut self, ctx: &Context) {
        self.take_zoom_keys(ctx);
        let (info, ppp, content) = ctx.input(|i| (i.viewport().clone(), i.pixels_per_point(), i.content_rect()));
        if info.minimized == Some(true) {
            return;
        }
        let zoom = ctx.zoom_factor();
        let native = info.native_pixels_per_point.unwrap_or(ppp / zoom);
        let win_px = content.size() * ppp;
        if win_px.x < MIN_REAL_PX.x || win_px.y < MIN_REAL_PX.y {
            return;
        }
        // the fit depends on the window size and the system scale only (not on the zoom): no zoom <-> window size feedback
        let want = effective_zoom(self.user, fit_zoom(win_px, native));
        if (want - zoom).abs() > 0.002 {
            ctx.set_zoom_factor(want);
        }
        // in the frame where a new zoom starts, the window code still measured the monitor with the old zoom: wait one frame
        let zoom_just_changed = self.last_zoom.is_some_and(|z| (z - zoom).abs() > 1e-6);
        self.last_zoom = Some(zoom);
        if zoom_just_changed {
            return;
        }
        let monitor_px = info.monitor_size.map(|m| m * ppp);
        let min = min_window_px(native, monitor_px);
        let key = (min.x as i32, min.y as i32);
        if self.sent_min != Some(key) {
            self.sent_min = Some(key);
            // points at the zoom of this frame: the window code converts them with the same zoom (a new zoom starts next frame)
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(min / ppp));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn a_saved_window_smaller_than_the_design_minimum_comes_back_as_it_was() {
        // 1366 x 768 at 150 %: the OS minimum is 1229 x 652 px = 819 x 435 points; the player shrank the window to that and closed it.
        assert_eq!(start_size(819.0, 435.0), [819.0, 435.0]);
        assert_eq!(start_size(1000.0, 640.0), [1000.0, 640.0], "the default size is untouched");
        assert_eq!(start_size(1111.0, 700.0), [1111.0, 700.0]);
        // Nothing below the boot minimum, whatever the file said.
        assert_eq!(start_size(10.0, 5.0), BOOT_MIN);
        assert_eq!(start_size(f32::NAN, f32::NAN), BOOT_MIN);
        assert_eq!(start_size(0.0, 9000.0), [BOOT_MIN[0], 9000.0]);
    }

    fn zoom_at(w: f32, h: f32, native: f32) -> f32 {
        fit_zoom(vec2(w, h), native)
    }

    #[test]
    fn desktop_sizes_are_untouched() {
        for (w, h, n) in [(1920.0, 1080.0, 1.0), (2560.0, 1440.0, 1.5), (1366.0, 768.0, 1.0), (1000.0, 640.0, 1.0), (860.0, 580.0, 1.0), (1290.0, 870.0, 1.5), (1075.0, 725.0, 1.25)] {
            assert_eq!(zoom_at(w, h, n), 1.0, "{w}x{h} at {n}");
        }
    }

    #[test]
    fn deck_sizes_shrink_to_fit() {
        // 1280 x 800 pixels with the scale factors a Deck may report
        assert_eq!(zoom_at(1280.0, 800.0, 1.0), 1.0);
        assert_eq!(zoom_at(1280.0, 800.0, 1.25), 1.0);
        for native in [1.5f32, 2.0] {
            let z = zoom_at(1280.0, 800.0, native);
            assert!(z < 1.0 && z > 0.6, "{native}: {z}");
            let pts = vec2(1280.0, 800.0) / (native * z);
            assert!(pts.x >= MIN_W - 0.5 && pts.y >= MIN_H - 0.5, "{native}: {pts:?}");
        }
        // 1.5: the height decides (800 / 1.5 = 533 < 580), 2.0: the height too
        assert!((zoom_at(1280.0, 800.0, 2.0) - 800.0 / (2.0 * 580.0)).abs() < 1e-6);
    }

    #[test]
    fn small_laptops() {
        for (w, h) in [(1280.0, 720.0), (1024.0, 600.0), (800.0, 600.0), (1366.0, 768.0)] {
            let z = zoom_at(w, h, 1.0);
            let pts = vec2(w, h) / z;
            assert!(pts.x >= MIN_W - 0.5 && pts.y >= MIN_H - 0.5, "{w}x{h}: {pts:?}");
        }
        assert!(zoom_at(800.0, 600.0, 1.0) < 1.0);
    }

    #[test]
    fn nonsense_input_keeps_the_zoom() {
        assert_eq!(fit_zoom(vec2(0.0, 0.0), 1.0), 1.0);
        assert_eq!(fit_zoom(vec2(f32::NAN, 800.0), 1.0), 1.0);
        assert_eq!(fit_zoom(vec2(1280.0, 800.0), 0.0), 1.0);
        assert_eq!(fit_zoom(vec2(1280.0, 800.0), f32::NAN), 1.0);
        // tiny windows are limited, not shrunk to nothing
        assert_eq!(fit_zoom(vec2(100.0, 100.0), 1.0), MIN_ZOOM);
    }

    #[test]
    fn minimum_window_keeps_a_margin_below_the_monitor() {
        // desktop: the design minimum at the system scale (well inside the monitor)
        assert_eq!(min_window_px(1.0, Some(vec2(1920.0, 1080.0))), vec2(860.0, 580.0));
        assert_eq!(min_window_px(1.5, Some(vec2(2560.0, 1440.0))), vec2(1290.0, 870.0));
        assert_eq!(min_window_px(1.0, Some(vec2(1366.0, 768.0))), vec2(860.0, 580.0));
        // 1366 x 768 at 150 %: 1290 x 870 pixels are taller than the screen; with title bar and taskbar the window must stay below it
        assert_eq!(min_window_px(1.5, Some(vec2(1366.0, 768.0))), vec2(1229.0, 652.0));
        // Deck at 2.0 / 1.5 / 1.25: 1280 x 800 pixels (the share is 90 % x 85 %)
        assert_eq!(min_window_px(2.0, Some(vec2(1280.0, 800.0))), vec2(1152.0, 680.0));
        assert_eq!(min_window_px(1.5, Some(vec2(1280.0, 800.0))), vec2(1152.0, 680.0));
        assert_eq!(min_window_px(1.25, Some(vec2(1280.0, 800.0))), vec2(1075.0, 680.0));
        // never above the share, whatever the monitor and scale
        for (mw, mh) in [(800.0f32, 600.0f32), (1024.0, 600.0), (1280.0, 720.0), (1366.0, 768.0), (1280.0, 800.0), (1920.0, 1080.0)] {
            for native in [1.0f32, 1.25, 1.5, 2.0, 3.0] {
                let m = min_window_px(native, Some(vec2(mw, mh)));
                assert!(m.x <= mw * MONITOR_SHARE.x && m.y <= mh * MONITOR_SHARE.y, "{mw}x{mh} at {native}: {m:?}");
                assert!(m.x <= MIN_W * native && m.y <= MIN_H * native, "{mw}x{mh} at {native}: {m:?}");
                assert!(m.x > 0.0 && m.y > 0.0);
            }
        }
        // monitor unknown or nonsense: only the boot minimum; scale nonsense = 1
        assert_eq!(min_window_px(2.0, None), vec2(960.0, 640.0));
        assert_eq!(min_window_px(2.0, Some(vec2(0.0, 0.0))), vec2(960.0, 640.0));
        assert_eq!(min_window_px(f32::NAN, Some(vec2(1920.0, 1080.0))), vec2(860.0, 580.0));
    }

    #[test]
    fn user_zoom_steps_like_egui() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(near(step_user_zoom(1.0, ZoomKey::In), 1.1));
        assert!(near(step_user_zoom(1.0, ZoomKey::Out), 0.9));
        assert!(near(step_user_zoom(2.3, ZoomKey::Reset), 1.0));
        assert!(near(step_user_zoom(0.4, ZoomKey::Reset), 1.0));
        // limits of egui's zoom
        let (mut hi, mut lo) = (1.0f32, 1.0f32);
        for _ in 0..80 {
            hi = step_user_zoom(hi, ZoomKey::In);
            lo = step_user_zoom(lo, ZoomKey::Out);
        }
        assert!(near(hi, USER_ZOOM_MAX) && near(lo, USER_ZOOM_MIN), "{hi} {lo}");
        // nonsense in = a normal step from 1.0
        assert!(near(step_user_zoom(f32::NAN, ZoomKey::In), 1.1));
        assert!(near(step_user_zoom(0.0, ZoomKey::Out), 0.9));
    }

    #[test]
    fn user_zoom_survives_a_fit_change() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let user = step_user_zoom(step_user_zoom(1.0, ZoomKey::In), ZoomKey::In); // 1.2
        assert!(near(user, 1.2));
        let deck = fit_zoom(vec2(1280.0, 800.0), 2.0); // 0.689...
        assert!(deck < 0.7);
        // the window gets smaller (fit lowers) and bigger again (fit back to 1): the user's 1.2 is in both
        assert!(near(effective_zoom(user, 1.0), 1.2));
        assert!(near(effective_zoom(user, deck), 1.2 * deck));
        assert!(near(effective_zoom(user, 1.0), 1.2));
        // Ctrl 0 puts the user's zoom back to 1.0, i.e. the effective zoom back to the bare fit (not to 1.0)
        let reset = step_user_zoom(user, ZoomKey::Reset);
        assert!(near(effective_zoom(reset, deck), deck));
        assert!(near(effective_zoom(reset, 1.0), 1.0));
    }

    #[test]
    fn the_fit_never_raises_the_zoom() {
        assert_eq!(effective_zoom(1.0, 3.0), 1.0);
        assert_eq!(effective_zoom(1.0, f32::NAN), 1.0);
        assert_eq!(effective_zoom(1.0, -2.0), 1.0);
        assert_eq!(effective_zoom(1.0, 0.0), 1.0);
        assert_eq!(effective_zoom(f32::NAN, 0.5), 0.5);
        // fit_zoom itself: never above 1.0, whatever the window and scale
        for w in [100.0f32, 640.0, 860.0, 1280.0, 1920.0, 3840.0, 7680.0] {
            for h in [100.0f32, 400.0, 580.0, 800.0, 1080.0, 2160.0] {
                for n in [0.5f32, 1.0, 1.25, 1.5, 2.0, 3.0] {
                    let z = fit_zoom(vec2(w, h), n);
                    assert!(z <= 1.0 && z >= MIN_ZOOM, "{w}x{h} at {n}: {z}");
                }
            }
        }
    }

    /// Plays the part of the window code: every frame the window has the same pixel size, and the screen rectangle handed to egui is
    /// that size divided by native x zoom. The zoom must settle after one change and then stay put.
    struct Sim {
        ctx: Context,
        fit: Fit,
        native: f32,
        monitor_px: Vec2,
        mins: Vec<Vec2>,
    }

    impl Sim {
        fn new(native: f32, monitor_px: Vec2) -> Sim {
            Sim { ctx: Context::default(), fit: Fit::default(), native, monitor_px, mins: Vec::new() }
        }

        /// One frame; `keys` are pressed with Ctrl.
        fn frame(&mut self, win_px: Vec2, keys: &[egui::Key]) {
            let ppp = self.native * self.ctx.zoom_factor();
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, win_px / ppp));
            let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
            for &key in keys {
                raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: ctrl });
            }
            let vp = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
            vp.native_pixels_per_point = Some(self.native);
            vp.monitor_size = Some(self.monitor_px / ppp);
            let Sim { ctx, fit, .. } = self;
            let mut out = ctx.run_ui(raw, |ui| fit.apply(ui.ctx()));
            out.textures_delta.clear();
            for c in out.viewport_output.values().flat_map(|v| v.commands.iter()) {
                if let ViewportCommand::MinInnerSize(s) = c {
                    self.mins.push(*s * ppp);
                }
            }
        }

        fn run(&mut self, win_px: Vec2, frames: usize) {
            for _ in 0..frames {
                self.frame(win_px, &[]);
            }
        }
    }

    fn settle(win_px: Vec2, native: f32, monitor_px: Vec2) -> (f32, Vec<Vec2>) {
        let mut sim = Sim::new(native, monitor_px);
        sim.run(win_px, 8);
        (sim.ctx.zoom_factor(), sim.mins)
    }

    #[test]
    fn zoom_settles_and_stays_for_every_deck_scale() {
        for native in [1.0f32, 1.25, 1.5, 2.0] {
            let (z, mins) = settle(vec2(1280.0, 800.0), native, vec2(1280.0, 800.0));
            let pts = vec2(1280.0, 800.0) / (native * z);
            assert!(pts.x >= MIN_W - 0.5 && pts.y >= MIN_H - 0.5, "native {native}: zoom {z} gives {pts:?}");
            assert!((z - fit_zoom(vec2(1280.0, 800.0), native)).abs() < 0.003, "native {native}: zoom {z}");
            // one minimum size, sent once, a margin below the screen
            assert_eq!(mins.len(), 1, "native {native}: {mins:?}");
            assert!(mins[0].x <= 1280.0 * MONITOR_SHARE.x + 0.5 && mins[0].y <= 800.0 * MONITOR_SHARE.y + 0.5, "native {native}: {:?}", mins[0]);
        }
    }

    #[test]
    fn a_desktop_window_keeps_zoom_one() {
        let (z, mins) = settle(vec2(1290.0, 870.0), 1.5, vec2(2560.0, 1440.0));
        assert_eq!(z, 1.0);
        assert_eq!(mins.len(), 1);
        assert!((mins[0] - vec2(1290.0, 870.0)).length() < 1.0, "{:?}", mins[0]);
    }

    #[test]
    fn a_small_decorated_laptop_gets_a_minimum_below_the_screen() {
        // 1366 x 768 at 150 %: the design minimum (1290 x 870 px) is taller than the screen
        let mut sim = Sim::new(1.5, vec2(1366.0, 768.0));
        sim.run(vec2(1229.0, 652.0), 8);
        assert_eq!(sim.mins.len(), 1);
        assert!(sim.mins[0].x <= 1229.5 && sim.mins[0].y <= 652.5, "{:?}", sim.mins[0]);
        // a window at that minimum still gets a zoom that holds the pages (fit < 1)
        let z = sim.ctx.zoom_factor();
        let pts = vec2(1229.0, 652.0) / (1.5 * z);
        assert!(z < 1.0 && pts.x >= MIN_W - 0.5 && pts.y >= MIN_H - 0.5, "zoom {z}, {pts:?}");
    }

    #[test]
    fn keyboard_zoom_works_on_top_of_the_fit() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.003;
        let deck = vec2(1280.0, 800.0);
        let mut sim = Sim::new(2.0, deck);
        sim.run(deck, 6);
        let fit = fit_zoom(deck, 2.0);
        assert!(fit < 0.7 && near(sim.ctx.zoom_factor(), fit));
        // Ctrl + : one step of the USER zoom, the fit stays under it, and it holds (no snap back to the bare fit)
        sim.frame(deck, &[egui::Key::Plus]);
        sim.run(deck, 6);
        assert!(near(sim.fit.user_zoom(), 1.1) && near(sim.ctx.zoom_factor(), 1.1 * fit), "user {} zoom {}", sim.fit.user_zoom(), sim.ctx.zoom_factor());
        sim.frame(deck, &[egui::Key::Equals]);
        sim.run(deck, 6);
        assert!(near(sim.fit.user_zoom(), 1.2) && near(sim.ctx.zoom_factor(), 1.2 * fit), "user {} zoom {}", sim.fit.user_zoom(), sim.ctx.zoom_factor());
        // the window grows to a desktop size (fit 1.0): the user's 1.2 survives
        let big = vec2(2560.0, 1600.0);
        sim.monitor_px = big;
        sim.run(big, 6);
        assert!(near(sim.fit.user_zoom(), 1.2) && near(sim.ctx.zoom_factor(), 1.2), "user {} zoom {}", sim.fit.user_zoom(), sim.ctx.zoom_factor());
        // and shrinks again: 1.2 x fit
        sim.run(deck, 6);
        assert!(near(sim.ctx.zoom_factor(), 1.2 * fit), "zoom {}", sim.ctx.zoom_factor());
        // Ctrl - : one step down
        sim.frame(deck, &[egui::Key::Minus]);
        sim.run(deck, 6);
        assert!(near(sim.fit.user_zoom(), 1.1) && near(sim.ctx.zoom_factor(), 1.1 * fit), "user {} zoom {}", sim.fit.user_zoom(), sim.ctx.zoom_factor());
        // Ctrl 0 : the user's zoom is 1.0 again, the UI is at the bare fit
        sim.frame(deck, &[egui::Key::Num0]);
        sim.run(deck, 6);
        assert!(near(sim.fit.user_zoom(), 1.0) && near(sim.ctx.zoom_factor(), fit), "user {} zoom {}", sim.fit.user_zoom(), sim.ctx.zoom_factor());
        // the zoom keys do not feed back into the window size: the OS minimum is sent for the deck monitor and once more for the big one
        assert!(sim.mins.len() <= 2, "{:?}", sim.mins);
    }

    #[test]
    fn keyboard_zoom_on_a_desktop_is_the_old_behaviour() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.003;
        let win = vec2(1920.0, 1080.0);
        let mut sim = Sim::new(1.0, win);
        sim.run(win, 4);
        assert_eq!(sim.ctx.zoom_factor(), 1.0);
        sim.frame(win, &[egui::Key::Plus]);
        sim.run(win, 4);
        assert!(near(sim.ctx.zoom_factor(), 1.1));
        sim.frame(win, &[egui::Key::Minus]);
        sim.frame(win, &[egui::Key::Minus]);
        sim.run(win, 4);
        assert!(near(sim.ctx.zoom_factor(), 0.9), "{}", sim.ctx.zoom_factor());
        sim.frame(win, &[egui::Key::Num0]);
        sim.run(win, 4);
        assert_eq!(sim.ctx.zoom_factor(), 1.0);
        assert_eq!(sim.mins.len(), 1, "{:?}", sim.mins);
    }

    #[test]
    fn a_minimized_window_is_left_alone() {
        let ctx = Context::default();
        let mut fit = Fit::default();
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(20.0, 20.0)));
        let vp = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        vp.native_pixels_per_point = Some(1.0);
        vp.minimized = Some(true);
        ctx.run_ui(raw, |ui| fit.apply(ui.ctx())).textures_delta.clear();
        ctx.run_ui(egui::RawInput::default(), |ui| fit.apply(ui.ctx())).textures_delta.clear();
        assert_eq!(ctx.zoom_factor(), 1.0);
    }
}
