// Mouse sensitivity scaling with layered overrides.
// Priority: hotkey > mode > base config value.
//
// Delta-based scaling modelled on toolscreen's GetRawInputData hook:
// compute the raw input delta per frame, scale it, and accumulate into
// the output position the game sees. At sens=1.0 we pass the raw delta
// through unchanged. The sub-pixel accumulator and its reset paths
// (on sens change / direction flip) mirror toolscreen's behaviour.

use tracing::trace;

pub struct SensitivityState {
    base: f32,
    // (uniform, optional per-axis (x, y))
    mode_override: Option<(f32, Option<(f32, f32)>)>,
    hotkey_override: Option<(f32, Option<(f32, f32)>)>,

    // Last raw GLFW input position (absolute virtual cursor pos in
    // CURSOR_DISABLED mode; window-relative in CURSOR_NORMAL).
    raw_x: f64,
    raw_y: f64,

    // Last output position we sent to the game. MC reads this as its
    // Mouse.x and computes its own frame-to-frame delta for camera turn.
    out_x: f64,
    out_y: f64,

    // Sub-pixel fractional residual carried across frames. Parallel to
    // toolscreen's `xAccum`/`yAccum`: holds the scaled-but-unconsumed
    // portion of the delta. For f64 output we consume fully each frame,
    // but the reset paths (sens change, direction flip) still matter.
    x_accum: f64,
    y_accum: f64,

    // Last sensitivity we applied. Used to detect a sens change and drop
    // any stale accumulator residual (otherwise old-sens fractional bits
    // would bleed into new-sens output).
    last_sx: f32,
    last_sy: f32,

    inited: bool,
}

impl SensitivityState {
    pub fn new() -> Self {
        Self {
            base: 1.0,
            mode_override: None,
            hotkey_override: None,
            raw_x: 0.0,
            raw_y: 0.0,
            out_x: 0.0,
            out_y: 0.0,
            x_accum: 0.0,
            y_accum: 0.0,
            last_sx: 1.0,
            last_sy: 1.0,
            inited: false,
        }
    }

    pub fn set_base_sensitivity(&mut self, s: f32) {
        tracing::debug!(sensitivity = s, "sensitivity: set_base_sensitivity");
        self.base = s;
    }

    pub fn set_mode_override(&mut self, s: f32, separate: Option<(f32, f32)>) {
        self.mode_override = Some((s, separate));
    }

    pub fn clear_mode_override(&mut self) {
        self.mode_override = None;
    }

    // toggles the hotkey override on/off
    pub fn toggle_hotkey_override(&mut self, s: f32, separate: Option<(f32, f32)>) {
        if self.hotkey_override.is_some() {
            self.hotkey_override = None;
            trace!("hotkey sensitivity override cleared");
        } else {
            self.hotkey_override = Some((s, separate));
            trace!(
                sensitivity = s,
                ?separate,
                "hotkey sensitivity override enabled"
            );
        }
    }

    pub fn has_hotkey_override(&self) -> bool {
        self.hotkey_override.is_some()
    }

    pub fn scale_cursor(&mut self, x: f64, y: f64) -> (f64, f64) {
        let (sx, sy) = self.effective_sensitivity();

        if !self.inited {
            self.raw_x = x;
            self.raw_y = y;
            self.out_x = x;
            self.out_y = y;
            self.x_accum = 0.0;
            self.y_accum = 0.0;
            self.last_sx = sx;
            self.last_sy = sy;
            self.inited = true;
            return (x, y);
        }

        let raw_dx = x - self.raw_x;
        let raw_dy = y - self.raw_y;
        self.raw_x = x;
        self.raw_y = y;

        // Sensitivity changed -- drop stale fractional residual. Without
        // this, the old sens's accumulator would corrupt output under the
        // new one (e.g. switching from godsens to normal sens shouldn't
        // spill godsens-scaled fractional bits into the first normal frame).
        if (sx - self.last_sx).abs() > f32::EPSILON
            || (sy - self.last_sy).abs() > f32::EPSILON
        {
            self.x_accum = 0.0;
            self.y_accum = 0.0;
            self.last_sx = sx;
            self.last_sy = sy;
        }

        // Direction-flip reset per axis. Stale same-sign residual would
        // delay opposite-direction output (matches toolscreen).
        if raw_dx != 0.0
            && self.x_accum != 0.0
            && (raw_dx > 0.0) != (self.x_accum > 0.0)
        {
            self.x_accum = 0.0;
        }
        if raw_dy != 0.0
            && self.y_accum != 0.0
            && (raw_dy > 0.0) != (self.y_accum > 0.0)
        {
            self.y_accum = 0.0;
        }

        // sens=1.0 passthrough: forward raw delta untouched and zero the
        // accumulator so a later non-identity sens starts clean.
        let is_identity =
            (sx - 1.0).abs() < f32::EPSILON && (sy - 1.0).abs() < f32::EPSILON;

        let (out_dx, out_dy) = if is_identity {
            self.x_accum = 0.0;
            self.y_accum = 0.0;
            (raw_dx, raw_dy)
        } else {
            // Scale delta and accumulate. f64 output has no integer
            // rounding, so we consume the full accumulator each frame --
            // sub-pixel precision is preserved natively.
            self.x_accum += raw_dx * sx as f64;
            self.y_accum += raw_dy * sy as f64;
            let dx = self.x_accum;
            let dy = self.y_accum;
            self.x_accum = 0.0;
            self.y_accum = 0.0;
            (dx, dy)
        };

        self.out_x += out_dx;
        self.out_y += out_dy;

        (self.out_x, self.out_y)
    }

    pub fn reset_tracking(&mut self) {
        self.inited = false;
    }

    fn effective_sensitivity(&self) -> (f32, f32) {
        // hotkey takes priority over everything
        if let Some((uniform, separate)) = self.hotkey_override {
            return match separate {
                Some((x, y)) => (x, y),
                None => (uniform, uniform),
            };
        }

        if let Some((uniform, separate)) = self.mode_override {
            return match separate {
                Some((x, y)) => (x, y),
                None => (uniform, uniform),
            };
        }

        (self.base, self.base)
    }

    pub fn get_effective_sensitivity(&self) -> (f32, f32) {
        self.effective_sensitivity()
    }
}

impl Default for SensitivityState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sensitivity_is_identity() {
        let mut s = SensitivityState::new();

        let (x, y) = s.scale_cursor(100.0, 200.0);
        assert_eq!((x, y), (100.0, 200.0));

        let (x, y) = s.scale_cursor(110.0, 220.0);
        assert!((x - 110.0).abs() < 1e-10);
        assert!((y - 220.0).abs() < 1e-10);
    }

    #[test]
    fn base_sensitivity_scales_delta() {
        let mut s = SensitivityState::new();
        s.set_base_sensitivity(2.0);

        s.scale_cursor(100.0, 100.0);

        let (x, y) = s.scale_cursor(110.0, 120.0);
        assert!((x - 120.0).abs() < 1e-10, "frame1 x: {x}"); // 100 + 10*2
        assert!((y - 140.0).abs() < 1e-10, "frame1 y: {y}"); // 100 + 20*2

        let (x2, y2) = s.scale_cursor(120.0, 140.0);
        assert!((x2 - 140.0).abs() < 1e-10, "frame2 x: {x2}"); // 120 + 10*2
        assert!((y2 - 180.0).abs() < 1e-10, "frame2 y: {y2}"); // 140 + 20*2

        let (x3, y3) = s.scale_cursor(130.0, 160.0);
        assert!((x3 - 160.0).abs() < 1e-10, "frame3 x: {x3}");
        assert!((y3 - 220.0).abs() < 1e-10, "frame3 y: {y3}");
    }

    #[test]
    fn mode_override_takes_precedence() {
        let mut s = SensitivityState::new();
        s.set_base_sensitivity(2.0);
        s.set_mode_override(0.5, None);

        s.scale_cursor(100.0, 100.0);

        let (x, y) = s.scale_cursor(110.0, 120.0);
        assert!((x - 105.0).abs() < 1e-10); // 100 + 10*0.5
        assert!((y - 110.0).abs() < 1e-10); // 100 + 20*0.5
    }

    #[test]
    fn hotkey_override_takes_top_priority() {
        let mut s = SensitivityState::new();
        s.set_base_sensitivity(2.0);
        s.set_mode_override(0.5, None);
        s.toggle_hotkey_override(3.0, None);

        s.scale_cursor(100.0, 100.0);

        let (x, y) = s.scale_cursor(110.0, 120.0);
        assert!((x - 130.0).abs() < 1e-10); // 100 + 10*3
        assert!((y - 160.0).abs() < 1e-10); // 100 + 20*3
    }

    #[test]
    fn toggle_hotkey_override_on_off() {
        let mut s = SensitivityState::new();
        s.set_base_sensitivity(1.0);

        assert!(!s.has_hotkey_override());

        s.toggle_hotkey_override(0.5, None);
        assert!(s.has_hotkey_override());

        s.toggle_hotkey_override(0.5, None);
        assert!(!s.has_hotkey_override());
    }

    #[test]
    fn separate_xy_sensitivity() {
        let mut s = SensitivityState::new();
        s.set_mode_override(1.0, Some((0.5, 2.0)));

        s.scale_cursor(100.0, 100.0);

        let (x, y) = s.scale_cursor(120.0, 110.0);
        assert!((x - 110.0).abs() < 1e-10); // 100 + 20*0.5
        assert!((y - 120.0).abs() < 1e-10); // 100 + 10*2.0
    }

    #[test]
    fn reset_tracking_reinitializes() {
        let mut s = SensitivityState::new();
        s.set_base_sensitivity(2.0);

        s.scale_cursor(100.0, 100.0);
        s.scale_cursor(110.0, 110.0);

        s.reset_tracking();

        let (x, y) = s.scale_cursor(200.0, 200.0);
        assert_eq!((x, y), (200.0, 200.0));
    }
}
