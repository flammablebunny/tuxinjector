use serde::{Deserialize, Serialize};

// Where the game content actually sits inside the output surface.
// Differs from screen size when letterboxing or custom viewports are involved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameViewportGeometry {
    pub game_w: i32,
    pub game_h: i32,
    pub final_x: i32,
    pub final_y: i32,
    pub final_w: i32,
    pub final_h: i32,
}

impl Default for GameViewportGeometry {
    fn default() -> Self {
        Self {
            game_w: 0, game_h: 0,
            final_x: 0, final_y: 0,
            final_w: 0, final_h: 0,
        }
    }
}

// Anchor point for positioning overlays relative to screen or viewport edges
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeTo {
    TopLeftScreen,
    TopCenterScreen,
    TopRightScreen,
    CenterScreen,
    BottomLeftScreen,
    BottomCenterScreen,
    BottomRightScreen,

    TopLeftViewport,
    TopCenterViewport,
    TopRightViewport,
    CenterViewport,
    BottomLeftViewport,
    BottomCenterViewport,
    BottomRightViewport,

    // F3+S pie chart anchors. These magic numbers come from MC's
    // debug screen layout. Don't ask me why it's 92 and 36.
    // Left edge of pie chart. Origin: (game_w - 92, game_h - 220)
    PieLeft,
    // Right edge of pie chart. Origin: (game_w - 36, game_h - 220)
    PieRight,
}

impl Default for RelativeTo {
    fn default() -> Self {
        Self::TopLeftScreen
    }
}

pub fn is_viewport_relative(anchor: &RelativeTo) -> bool {
    matches!(
        anchor,
        RelativeTo::TopLeftViewport
            | RelativeTo::TopCenterViewport
            | RelativeTo::TopRightViewport
            | RelativeTo::CenterViewport
            | RelativeTo::BottomLeftViewport
            | RelativeTo::BottomCenterViewport
            | RelativeTo::BottomRightViewport
            | RelativeTo::PieLeft
            | RelativeTo::PieRight
    )
}

// Resolve anchor + offset into absolute screen coordinates.
// This is the big match statement that all overlay positioning funnels through.
// sw/sh = screen dims, ew/eh = element size, vp = game viewport, gui_scale = MC's GUI scale.
// display_scale = Retina content scale (2.0 on macOS Retina, 1.0 elsewhere).
pub fn resolve_relative_position(
    anchor: RelativeTo,
    x: i32, y: i32,
    sw: i32, sh: i32,
    vp: &GameViewportGeometry,
    ew: i32, eh: i32,
    gui_scale: u32,
    display_scale: f32,
) -> (i32, i32) {
    let vx = vp.final_x;
    let vy = vp.final_y;
    let vw = vp.final_w;
    let vh = vp.final_h;

    match anchor {
        // top-left
        RelativeTo::TopLeftScreen => (x, y),
        RelativeTo::TopLeftViewport => (vx + x, vy + y),

        // top-center
        RelativeTo::TopCenterScreen => ((sw - ew) / 2 + x, y),
        RelativeTo::TopCenterViewport => (vx + (vw - ew) / 2 + x, vy + y),

        // top-right
        RelativeTo::TopRightScreen => (sw - ew - x, y),
        RelativeTo::TopRightViewport => (vx + vw - ew - x, vy + y),

        // center
        RelativeTo::CenterScreen => ((sw - ew) / 2 + x, (sh - eh) / 2 + y),
        RelativeTo::CenterViewport => (
            vx + (vw - ew) / 2 + x,
            vy + (vh - eh) / 2 + y,
        ),

        // bottom-left
        RelativeTo::BottomLeftScreen => (x, sh - eh - y),
        RelativeTo::BottomLeftViewport => (vx + x, vy + vh - eh - y),

        // bottom-center
        RelativeTo::BottomCenterScreen => ((sw - ew) / 2 + x, sh - eh - y),
        RelativeTo::BottomCenterViewport => (
            vx + (vw - ew) / 2 + x,
            vy + vh - eh - y,
        ),

        // bottom-right
        RelativeTo::BottomRightScreen => (sw - ew - x, sh - eh - y),
        RelativeTo::BottomRightViewport => (vx + vw - ew - x, vy + vh - eh - y),

        // pie chart anchors - the magic numbers (92, 36, 220) come from
        // MC's debug screen at guiScale=3; we scale them for other scales.
        // On macOS Retina the framebuffer is 2x, so multiply by display_scale.
        RelativeTo::PieLeft => {
            let s = gui_scale as f32 / 3.0 * display_scale;
            let px = (92.0 * s).round() as i32;
            let py = (220.0 * s).round() as i32;
            (vx + vw - px + x, vy + vh - py + y)
        }
        RelativeTo::PieRight => {
            let s = gui_scale as f32 / 3.0 * display_scale;
            let px = (36.0 * s).round() as i32;
            let py = (220.0 * s).round() as i32;
            (vx + vw - px + x, vy + vh - py + y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_viewport() -> GameViewportGeometry {
        GameViewportGeometry {
            game_w: 1280,
            game_h: 720,
            final_x: 100,
            final_y: 50,
            final_w: 1600,
            final_h: 900,
        }
    }

    #[test]
    fn top_left_screen_is_identity() {
        let vp = test_viewport();
        let (px, py) = resolve_relative_position(RelativeTo::TopLeftScreen, 10, 20, 1920, 1080, &vp, 0, 0, 3, 1.0);
        assert_eq!((px, py), (10, 20));
    }

    #[test]
    fn center_screen_with_element() {
        let vp = test_viewport();
        // 100x50 element on 1920x1080 screen
        let (px, py) = resolve_relative_position(RelativeTo::CenterScreen, 0, 0, 1920, 1080, &vp, 100, 50, 3, 1.0);
        assert_eq!((px, py), ((1920 - 100) / 2, (1080 - 50) / 2));
    }

    #[test]
    fn bottom_right_viewport() {
        let vp = test_viewport();
        let (px, py) =
            resolve_relative_position(RelativeTo::BottomRightViewport, 10, 10, 1920, 1080, &vp, 200, 100, 3, 1.0);
        assert_eq!((px, py), (100 + 1600 - 200 - 10, 50 + 900 - 100 - 10));
    }

    #[test]
    fn viewport_relative_check() {
        assert!(!is_viewport_relative(&RelativeTo::TopLeftScreen));
        assert!(is_viewport_relative(&RelativeTo::CenterViewport));
        assert!(is_viewport_relative(&RelativeTo::PieLeft));
        assert!(is_viewport_relative(&RelativeTo::PieRight));
    }

    #[test]
    fn pie_left_anchor() {
        let vp = GameViewportGeometry {
            game_w: 1920, game_h: 1080,
            final_x: 0, final_y: 0,
            final_w: 1920, final_h: 1080,
        };
        // default guiScale=3, display_scale=1.0 -> offsets 92, 220
        let (px, py) = resolve_relative_position(RelativeTo::PieLeft, 0, 0, 1920, 1080, &vp, 0, 0, 3, 1.0);
        assert_eq!((px, py), (1920 - 92, 1080 - 220));
    }

    #[test]
    fn pie_left_anchor_gui_scale_2() {
        let vp = GameViewportGeometry {
            game_w: 1920, game_h: 1080,
            final_x: 0, final_y: 0,
            final_w: 1920, final_h: 1080,
        };
        // guiScale=2 -> 92*2/3=61, 220*2/3=147
        let (px, py) = resolve_relative_position(RelativeTo::PieLeft, 0, 0, 1920, 1080, &vp, 0, 0, 2, 1.0);
        assert_eq!((px, py), (1920 - 61, 1080 - 147));
    }

    #[test]
    fn pie_left_anchor_retina() {
        // macOS Retina 2x: framebuffer is 3840x2160, display_scale=2.0
        let vp = GameViewportGeometry {
            game_w: 3840, game_h: 2160,
            final_x: 0, final_y: 0,
            final_w: 3840, final_h: 2160,
        };
        // guiScale=3, display_scale=2.0 -> offsets 92*2=184, 220*2=440
        let (px, py) = resolve_relative_position(RelativeTo::PieLeft, 0, 0, 3840, 2160, &vp, 0, 0, 3, 2.0);
        assert_eq!((px, py), (3840 - 184, 2160 - 440));
    }

    #[test]
    fn pie_right_anchor() {
        let vp = GameViewportGeometry {
            game_w: 2560, game_h: 1440,
            final_x: 0, final_y: 0,
            final_w: 2560, final_h: 1440,
        };
        let (px, py) = resolve_relative_position(RelativeTo::PieRight, 0, 0, 2560, 1440, &vp, 0, 0, 3, 1.0);
        assert_eq!((px, py), (2560 - 36, 1440 - 220));
    }
}
