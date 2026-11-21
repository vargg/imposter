//! Workspace layout preferences: versioned, sanitized field by field, stored as UI state.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The smallest window the app is laid out for (logical points).
pub const MIN_WINDOW: [f32; 2] = [720.0, 480.0];
pub const RAIL_WIDTH: f32 = 48.0;

/// Request and response side by side or stacked; only the user switches it, never the width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    SideBySide,
    Stacked,
}

impl Orientation {
    #[must_use]
    pub fn switched(self) -> Self {
        match self {
            Self::SideBySide => Self::Stacked,
            Self::Stacked => Self::SideBySide,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct WorkspaceLayout {
    pub version: u32,
    pub orientation: Orientation,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    /// Variables and Settings follow the window width until the column is resized.
    pub list_width: Option<f32>,
    pub mock_list_width: f32,
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
            orientation: Orientation::SideBySide,
            sidebar_visible: true,
            sidebar_width: 240.0,
            list_width: None,
            mock_list_width: 260.0,
            side_ratio: 0.5,
            stack_ratio: 0.45,
            history_ratio: 0.5,
        }
    }
}

pub const SIDEBAR_WIDTH: (f32, f32) = (184.0, 400.0);
pub(crate) const LIST_WIDTH: (f32, f32) = (144.0, 320.0);
pub(crate) const MOCK_LIST_WIDTH: (f32, f32) = (200.0, 480.0);
pub const RATIO: (f32, f32) = (0.15, 0.85);

/// The least a request or response pane gets beside the other: room for the tab row and
/// tools (side by side), or for the tab row and a few lines (stacked).
pub const MIN_PANE_WIDE: f32 = 220.0;
pub const MIN_PANE_TALL: f32 = 160.0;
/// The least the History list (three rows and its header) and the details below it get.
pub const MIN_HISTORY_LIST: f32 = 110.0;
pub const MIN_HISTORY_DETAILS: f32 = 160.0;

/// The size of the first of two panes sharing `total` points by `ratio`, keeping each at
/// its minimum; when both minimums do not fit, they share in proportion to them.
pub fn split(total: f32, ratio: f32, min_first: f32, min_second: f32) -> f32 {
    let total = total.max(0.0);
    if total <= min_first + min_second {
        return total * min_first / (min_first + min_second);
    }
    (total * ratio).clamp(min_first, total - min_second)
}

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
        let number = |key: &str, (low, high): (f32, f32)| {
            #[allow(clippy::cast_possible_truncation)]
            // layout values are small screen sizes and ratios
            object
                .get(key)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .map(|v| (v as f32).clamp(low, high))
        };
        Self {
            version: 1,
            // Before the switch there was a `mode` with an automatic choice: only an explicit
            // "stacked" stays stacked.
            orientation: object
                .get("orientation")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_else(|| {
                    if object.get("mode").and_then(Value::as_str) == Some("stacked") {
                        Orientation::Stacked
                    } else {
                        default.orientation
                    }
                }),
            sidebar_visible: object
                .get("sidebar_visible")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            sidebar_width: number("sidebar_width", SIDEBAR_WIDTH)
                .map_or(default.sidebar_width, f32::round),
            list_width: number("list_width", LIST_WIDTH).map(f32::round),
            mock_list_width: number("mock_list_width", MOCK_LIST_WIDTH)
                .map_or(default.mock_list_width, f32::round),
            side_ratio: number("side_ratio", RATIO).unwrap_or(default.side_ratio),
            stack_ratio: number("stack_ratio", RATIO).unwrap_or(default.stack_ratio),
            history_ratio: number("history_ratio", RATIO).unwrap_or(default.history_ratio),
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_split_keeps_each_pane_at_its_minimum() {
        assert_eq!(split(1000.0, 0.5, 160.0, 160.0), 500.0);
        assert_eq!(split(1000.0, 0.05, 160.0, 160.0), 160.0, "first kept");
        assert_eq!(split(1000.0, 0.95, 160.0, 160.0), 840.0, "second kept");
        assert_eq!(split(200.0, 0.9, 100.0, 300.0), 50.0, "in proportion");
        assert_eq!(split(-5.0, 0.5, 160.0, 160.0), 0.0);
    }

    #[test]
    fn stored_state_is_sanitized_field_by_field() {
        let layout = WorkspaceLayout::from_stored(&json!({
            "version": 1, "orientation": "stacked", "sidebar_visible": "yes",
            "sidebar_width": 9999, "side_ratio": 0.01, "stack_ratio": 0.6, "history_ratio": null
        }));
        assert_eq!(layout.orientation, Orientation::Stacked);
        assert!(layout.sidebar_visible, "a non-boolean keeps the default");
        assert!((layout.sidebar_width - 400.0).abs() < f32::EPSILON);
        assert!((layout.side_ratio - 0.15).abs() < f32::EPSILON);
        assert!((layout.stack_ratio - 0.6).abs() < f32::EPSILON);
        assert!((layout.history_ratio - 0.5).abs() < f32::EPSILON);
        assert_eq!(layout.list_width, None);
        assert_eq!(layout.mock_list_width, 260.0);

        let layout = WorkspaceLayout::from_stored(&json!({
            "list_width": 100, "mock_list_width": 600
        }));
        assert_eq!(layout.list_width, Some(144.0));
        assert_eq!(layout.mock_list_width, 480.0);

        let layout = WorkspaceLayout::from_stored(&json!({
            "list_width": "invalid", "mock_list_width": "invalid"
        }));
        assert_eq!(layout.list_width, None);
        assert_eq!(layout.mock_list_width, 260.0);
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
        let orientation = |value| WorkspaceLayout::from_stored(&value).orientation;
        assert_eq!(
            orientation(json!({"orientation": "stacked"})),
            Orientation::Stacked
        );
        assert_eq!(
            orientation(json!({"orientation": "diagonal"})),
            Orientation::SideBySide
        );
    }

    #[test]
    fn the_old_automatic_mode_keeps_only_an_explicit_stacked_choice() {
        let orientation = |mode| WorkspaceLayout::from_stored(&json!({"mode": mode})).orientation;
        assert_eq!(orientation("stacked"), Orientation::Stacked);
        assert_eq!(orientation("side_by_side"), Orientation::SideBySide);
        assert_eq!(orientation("auto"), Orientation::SideBySide);
        assert_eq!(Orientation::SideBySide.switched(), Orientation::Stacked);
        assert_eq!(Orientation::Stacked.switched(), Orientation::SideBySide);
    }

    #[test]
    fn stored_state_round_trips() {
        let layout = WorkspaceLayout {
            orientation: Orientation::Stacked,
            sidebar_visible: false,
            side_ratio: 0.3,
            list_width: Some(276.0),
            mock_list_width: 350.0,
            ..WorkspaceLayout::default()
        };
        assert_eq!(WorkspaceLayout::from_stored(&layout.to_stored()), layout);
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
