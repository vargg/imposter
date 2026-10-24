//! Workspace layout preferences: versioned, sanitized field by field, stored as UI state.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Narrower than this, side by side is never used.
pub const MIN_SIDE_WIDTH: f32 = 900.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    #[default]
    Auto,
    SideBySide,
    Stacked,
}

/// Request and response either side by side or stacked; `Auto` picks by width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct WorkspaceLayout {
    pub version: u32,
    pub mode: LayoutMode,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    /// Request share of the width when side by side.
    pub side_ratio: f32,
    /// Request share of the height when stacked.
    pub stack_ratio: f32,
    /// List share of the history screen.
    pub history_ratio: f32,
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        Self {
            version: 1,
            mode: LayoutMode::Auto,
            sidebar_visible: true,
            sidebar_width: 240.0,
            side_ratio: 0.5,
            stack_ratio: 0.45,
            history_ratio: 0.5,
        }
    }
}

pub const SIDEBAR_WIDTH: (f32, f32) = (200.0, 400.0);
pub const RATIO: (f32, f32) = (0.15, 0.85);

impl WorkspaceLayout {
    /// Read stored state, keeping every valid field and defaulting each invalid one, so a
    /// damaged or older payload never loses the rest. Unknown versions start over.
    pub fn from_stored(value: &Value) -> Self {
        let default = Self::default();
        let Some(object) = value.as_object() else {
            return default;
        };
        if object.get("version").is_some_and(|v| v.as_u64() != Some(1)) {
            return default;
        }
        let number = |key: &str, fallback: f32, (low, high): (f32, f32)| {
            #[allow(clippy::cast_possible_truncation)]
            // layout values are small screen sizes and ratios
            let value = object
                .get(key)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .map_or(fallback, |v| v as f32);
            value.clamp(low, high)
        };
        Self {
            version: 1,
            mode: object
                .get("mode")
                .and_then(|mode| serde_json::from_value(mode.clone()).ok())
                .unwrap_or(default.mode),
            sidebar_visible: object
                .get("sidebar_visible")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            sidebar_width: number("sidebar_width", default.sidebar_width, SIDEBAR_WIDTH).round(),
            side_ratio: number("side_ratio", default.side_ratio, RATIO),
            stack_ratio: number("stack_ratio", default.stack_ratio, RATIO),
            history_ratio: number("history_ratio", default.history_ratio, RATIO),
        }
    }

    pub fn to_stored(self) -> Value {
        // Plain numbers and booleans always serialize.
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn ratio(&self, orientation: Orientation) -> f32 {
        match orientation {
            Orientation::SideBySide => self.side_ratio,
            Orientation::Stacked => self.stack_ratio,
        }
    }

    pub fn set_ratio(&mut self, orientation: Orientation, ratio: f32) {
        let ratio = ratio.clamp(RATIO.0, RATIO.1);
        match orientation {
            Orientation::SideBySide => self.side_ratio = ratio,
            Orientation::Stacked => self.stack_ratio = ratio,
        }
    }
}

/// The orientation for `width`, with hysteresis in `Auto` so a window near the threshold does
/// not flip back and forth. A manual side-by-side choice still stacks when it cannot fit.
pub fn effective_orientation(
    width: f32,
    mode: LayoutMode,
    previous: Option<Orientation>,
) -> Orientation {
    match mode {
        LayoutMode::SideBySide if width >= MIN_SIDE_WIDTH => Orientation::SideBySide,
        LayoutMode::Stacked | LayoutMode::SideBySide => Orientation::Stacked,
        LayoutMode::Auto => {
            let threshold = match previous {
                None => 1060.0,
                Some(Orientation::SideBySide) => 1020.0,
                Some(Orientation::Stacked) => 1100.0,
            };
            if width >= threshold {
                Orientation::SideBySide
            } else {
                Orientation::Stacked
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stored_state_is_sanitized_field_by_field() {
        let layout = WorkspaceLayout::from_stored(&json!({
            "version": 1, "mode": "stacked", "sidebar_visible": "yes",
            "sidebar_width": 9999, "side_ratio": 0.01, "stack_ratio": 0.6, "history_ratio": null
        }));
        assert_eq!(layout.mode, LayoutMode::Stacked);
        assert!(layout.sidebar_visible, "a non-boolean keeps the default");
        assert!((layout.sidebar_width - 400.0).abs() < f32::EPSILON);
        assert!((layout.side_ratio - 0.15).abs() < f32::EPSILON);
        assert!((layout.stack_ratio - 0.6).abs() < f32::EPSILON);
        assert!((layout.history_ratio - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn unknown_versions_and_shapes_start_over_and_missing_version_is_accepted() {
        assert_eq!(
            WorkspaceLayout::from_stored(&json!({"version": 2, "mode": "stacked"})),
            WorkspaceLayout::default()
        );
        assert_eq!(
            WorkspaceLayout::from_stored(&json!([1, 2])),
            WorkspaceLayout::default()
        );
        assert_eq!(
            WorkspaceLayout::from_stored(&json!({"mode": "side_by_side"})).mode,
            LayoutMode::SideBySide
        );
        assert_eq!(
            WorkspaceLayout::from_stored(&json!({"mode": "diagonal"})).mode,
            LayoutMode::Auto
        );
    }

    #[test]
    fn stored_state_round_trips() {
        let layout = WorkspaceLayout {
            mode: LayoutMode::SideBySide,
            sidebar_visible: false,
            side_ratio: 0.3,
            ..WorkspaceLayout::default()
        };
        assert_eq!(WorkspaceLayout::from_stored(&layout.to_stored()), layout);
    }

    #[test]
    fn orientation_follows_width_with_hysteresis() {
        assert_eq!(
            effective_orientation(1059.0, LayoutMode::Auto, None),
            Orientation::Stacked
        );
        assert_eq!(
            effective_orientation(1060.0, LayoutMode::Auto, None),
            Orientation::SideBySide
        );
        assert_eq!(
            effective_orientation(1030.0, LayoutMode::Auto, Some(Orientation::SideBySide)),
            Orientation::SideBySide
        );
        assert_eq!(
            effective_orientation(1090.0, LayoutMode::Auto, Some(Orientation::Stacked)),
            Orientation::Stacked
        );
        assert_eq!(
            effective_orientation(950.0, LayoutMode::SideBySide, None),
            Orientation::SideBySide
        );
        assert_eq!(
            effective_orientation(800.0, LayoutMode::SideBySide, None),
            Orientation::Stacked
        );
        assert_eq!(
            effective_orientation(3000.0, LayoutMode::Stacked, None),
            Orientation::Stacked
        );
    }

    #[test]
    fn ratios_are_clamped_per_orientation() {
        let mut layout = WorkspaceLayout::default();
        layout.set_ratio(Orientation::SideBySide, 0.99);
        layout.set_ratio(Orientation::Stacked, 0.3);
        assert!((layout.ratio(Orientation::SideBySide) - 0.85).abs() < f32::EPSILON);
        assert!((layout.ratio(Orientation::Stacked) - 0.3).abs() < f32::EPSILON);
    }
}
