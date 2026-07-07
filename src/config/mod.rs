//! KDL configuration, niri-flavored.
//!
//! Example config:
//!
//! ```kdl
//! input {
//!     keyboard-shortcuts {
//!         Mod="Alt"   // which physical key acts as the Mod key
//!     }
//! }
//!
//! layout {
//!     gaps 8
//!     edge-padding 8
//!     default-column-width { proportion 0.25; }
//!     center-focused-column "on-overflow"   // never | on-overflow | always
//! }
//!
//! animations {
//!     window-movement { duration-ms 250; easing "ease-out-cubic"; }
//!     view-offset    { duration-ms 250; easing "ease-out-expo"; }
//!     // off
//! }
//!
//! binds {
//!     Mod+Left  { focus-column-left; }
//!     Mod+Right { focus-column-right; }
//!     Mod+H     { focus-column-left; }
//!     Mod+L     { focus-column-right; }
//!     Mod+J     { focus-window-down; }
//!     Mod+K     { focus-window-up; }
//!     Mod+Q     { close-window; }
//!     Mod+Shift+E { quit; }
//! }
//! ```
//!
//! Config lives at `%APPDATA%\yumi-wini\config.kdl` (overridable with
//! `YUMI_WINI_CONFIG`). Missing file = defaults.

// The input module consumes this in the next commits; silence interim
// dead-code warnings.
#![allow(dead_code)]

use std::path::PathBuf;

use crate::anim::{AnimParams, Easing};
use crate::layout::geometry::{CenterFocused, LayoutParams};
use crate::layout::ColumnWidth;

use kdl::{KdlDocument, KdlNode};

/// A parsed key binding: key combo -> action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    /// e.g. "Mod+Shift+Left", lowercase key names.
    pub combo: String,
    pub action: Action,
}

/// Everything the WM can do from a key press. Mirrors the niri action
/// names we intend to support (a growing subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    CloseWindow,
    Spawn(String),
    FocusColumnLeft,
    FocusColumnRight,
    FocusColumnFirst,
    FocusColumnLast,
    FocusWindowDown,
    FocusWindowUp,
    MoveColumnLeft,
    MoveColumnRight,
    MoveWindowDown,
    MoveWindowUp,
    MoveWindowToColumnLeft,
    MoveWindowToColumnRight,
    ConsumeOrExpelWindowLeft,
    ConsumeOrExpelWindowRight,
    SetColumnWidth(String),
    ToggleFullWidth,
    MaximizeColumn,
    ToggleWindowedFullscreen,
    SetWindowHeight(String),
    FocusWorkspace(u8),
    WorkspaceSwitch(u8),
    MoveWindowToWorkspace(u8),
    MoveColumnToWorkspace(u8),
    ToggleWindowFloating,
    ToggleOverview,
    Unknown(String),
}

/// Which physical key acts as "Mod".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModKey {
    Alt,
    Super,
    Ctrl,
}

/// Animation tuning, per animation kind (niri-style `animations` node).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationsConfig {
    /// Master switch (`animations { off; }` disables everything by
    /// forcing zero-duration animations).
    pub enabled: bool,
    pub window_movement: AnimParams,
    pub window_open: AnimParams,
    pub view_offset: AnimParams,
}

impl Default for AnimationsConfig {
    fn default() -> Self {
        AnimationsConfig {
            enabled: true,
            window_movement: AnimParams::default(),
            window_open: AnimParams {
                duration: std::time::Duration::from_millis(150),
                easing: Easing::EaseOutCubic,
            },
            view_offset: AnimParams {
                duration: std::time::Duration::from_millis(250),
                easing: Easing::EaseOutExpo,
            },
        }
    }
}

/// Full configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub mod_key: ModKey,
    pub layout: LayoutParams,
    pub animations: AnimationsConfig,
    pub binds: Vec<Bind>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mod_key: ModKey::Alt,
            layout: LayoutParams::default(),
            animations: AnimationsConfig::default(),
            // niri's default binds (navigation subset we implement).
            binds: vec![
                bind("Mod+Left", Action::FocusColumnLeft),
                bind("Mod+Down", Action::FocusWindowDown),
                bind("Mod+Up", Action::FocusWindowUp),
                bind("Mod+Right", Action::FocusColumnRight),
                bind("Mod+H", Action::FocusColumnLeft),
                bind("Mod+J", Action::FocusWindowDown),
                bind("Mod+K", Action::FocusWindowUp),
                bind("Mod+L", Action::FocusColumnRight),
                bind("Mod+Shift+Left", Action::MoveColumnLeft),
                bind("Mod+Shift+Down", Action::MoveWindowDown),
                bind("Mod+Shift+Up", Action::MoveWindowUp),
                bind("Mod+Shift+Right", Action::MoveColumnRight),
                bind("Mod+Shift+H", Action::MoveColumnLeft),
                bind("Mod+Shift+J", Action::MoveWindowDown),
                bind("Mod+Shift+K", Action::MoveWindowUp),
                bind("Mod+Shift+L", Action::MoveColumnRight),
                bind("Mod+Ctrl+Left", Action::MoveWindowToColumnLeft),
                bind("Mod+Ctrl+Right", Action::MoveWindowToColumnRight),
                bind("Mod+Ctrl+H", Action::MoveWindowToColumnLeft),
                bind("Mod+Ctrl+L", Action::MoveWindowToColumnRight),
                bind("Mod+Ctrl+J", Action::ConsumeOrExpelWindowLeft),
                bind("Mod+Ctrl+K", Action::ConsumeOrExpelWindowRight),
                bind("Mod+Page_Down", Action::FocusColumnRight),
                bind("Mod+Page_Up", Action::FocusColumnLeft),
                bind("Mod+Home", Action::FocusColumnFirst),
                bind("Mod+End", Action::FocusColumnLast),
                bind("Mod+F", Action::MaximizeColumn),
                bind("Mod+Shift+F", Action::ToggleWindowedFullscreen),
                bind("Mod+Q", Action::CloseWindow),
                bind("Mod+Shift+Slash", Action::ToggleOverview),
                bind("Mod+Shift+E", Action::Quit),
            ],
        }
    }
}

fn bind(combo: &str, action: Action) -> Bind {
    Bind {
        combo: combo.to_string(),
        action,
    }
}

/// Locate the config file without reading it.
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("YUMI_WINI_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("yumi-wini").join("config.kdl")
}

/// Load and parse the config; falls back to defaults on any problem
/// (with a logged warning). Never fails hard — a broken config should
/// not brick the WM.
pub fn load() -> Config {
    let path = config_path();
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            log::info!(
                "no config at {} — using defaults (drop one there to customize)",
                path.display()
            );
            return Config::default();
        }
    };
    match parse(&source) {
        Ok(cfg) => {
            log::info!("loaded config from {}", path.display());
            cfg
        }
        Err(err) => {
            log::warn!("config parse error ({err}); using defaults");
            Config::default()
        }
    }
}

/// Parse KDL text into a Config. Unknown nodes/fields are ignored
/// (forward compatibility, like niri).
pub fn parse(source: &str) -> Result<Config, String> {
    let doc: KdlDocument = source
        .parse()
        .map_err(|e| format!("KDL syntax: {e}"))?;

    let mut config = Config::default();
    config.binds.clear(); // Config-provided binds replace the defaults.

    for node in doc.nodes() {
        match node.name().value() {
            "input" => parse_input(node, &mut config),
            "layout" => parse_layout(node, &mut config),
            "animations" => parse_animations(node, &mut config),
            "binds" => parse_binds(node, &mut config),
            other => log::debug!("ignoring unknown config node {other:?}"),
        }
    }
    Ok(config)
}

fn parse_input(node: &KdlNode, config: &mut Config) {
    let Some(doc) = node.children() else { return };
    for n in doc.nodes() {
        if n.name().value() == "keyboard-shortcuts" {
            // `Mod "Super"` inside keyboard-shortcuts is a child node
            // whose first argument names the modifier key.
            let mod_name = n
                .children()
                .and_then(|c| {
                    c.nodes()
                        .iter()
                        .find(|m| m.name().value() == "Mod")
                        .and_then(first_string_arg)
                })
                .unwrap_or_else(|| "Alt".to_string());
            config.mod_key = match mod_name.as_str() {
                "Super" | "Super_L" | "Logo" => ModKey::Super,
                "Ctrl" | "Control" => ModKey::Ctrl,
                _ => ModKey::Alt,
            };
        }
    }
}

fn parse_layout(node: &KdlNode, config: &mut Config) {
    let Some(doc) = node.children() else { return };
    for n in doc.nodes() {
        match n.name().value() {
            "gaps" => {
                if let Some(v) = first_float_arg(n) {
                    config.layout.gaps = v;
                }
            }
            "edge-padding" => {
                if let Some(v) = first_float_arg(n) {
                    config.layout.edge_padding = v;
                }
            }
            "center-focused-column" => {
                if let Some(v) = first_string_arg(n) {
                    config.layout.center_focused_column = match v.as_str() {
                        "never" => CenterFocused::Never,
                        "always" => CenterFocused::Always,
                        _ => CenterFocused::OnOverflow,
                    };
                }
            }
            "default-column-width" => {
                config.layout.default_column_width = parse_width_node(n);
            }
            other => log::debug!("ignoring layout node {other:?}"),
        }
    }
}

/// `default-column-width { proportion 0.5; }` or `{ fixed 1280; }` or
/// a bare `default-column-width 800`.
fn parse_width_node(n: &KdlNode) -> ColumnWidth {
    if let Some(f) = first_float_arg(n) {
        return ColumnWidth::Fixed(f);
    }
    let Some(doc) = n.children() else {
        return ColumnWidth::Proportion(0.25);
    };
    for child in doc.nodes() {
        match child.name().value() {
            "proportion" => {
                if let Some(p) = first_float_arg(child) {
                    return ColumnWidth::Proportion(p.clamp(0.01, 100.0));
                }
            }
            "fixed" => {
                if let Some(f) = first_float_arg(child) {
                    return ColumnWidth::Fixed(f.clamp(1.0, 100_000.0));
                }
            }
            _ => {}
        }
    }
    ColumnWidth::Proportion(0.25)
}

/// `animations { off; window-movement { duration-ms 200; easing "ease-out-expo"; } }`
fn parse_animations(node: &KdlNode, config: &mut Config) {
    // `animations "off";` as an argument, or `animations { off; }`
    // as a child node — either form disables everything.
    let off_arg = node
        .entries()
        .iter()
        .any(|e| e.name().is_none() && e.value().as_string() == Some("off"));
    let off_node = node
        .children()
        .is_some_and(|c| c.nodes().iter().any(|n| n.name().value() == "off"));
    if off_arg || off_node {
        config.animations.enabled = false;
    }
    let Some(doc) = node.children() else { return };
    for n in doc.nodes() {
        let params = match n.name().value() {
            "window-movement" => &mut config.animations.window_movement,
            "window-open" => &mut config.animations.window_open,
            "view-offset" => &mut config.animations.view_offset,
            other => {
                log::debug!("ignoring animations node {other:?}");
                continue;
            }
        };
        let Some(children) = n.children() else { continue };
        for c in children.nodes() {
            match c.name().value() {
                "duration-ms" => {
                    if let Some(ms) = first_float_arg(c) {
                        params.duration = std::time::Duration::from_millis(ms.max(0.0) as u64);
                    }
                }
                "easing" => {
                    if let Some(name) = first_string_arg(c) {
                        match easing_from_name(&name) {
                            Some(e) => params.easing = e,
                            None => log::warn!(
                                "unknown easing {name:?}; expected one of: linear, \
                                 ease-out-quad, ease-out-cubic, ease-out-expo, ease-out-back"
                            ),
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Parse a niri-style easing name.
fn easing_from_name(name: &str) -> Option<Easing> {
    Some(match name {
        "linear" => Easing::Linear,
        "ease-out-quad" => Easing::EaseOutQuad,
        "ease-out-cubic" => Easing::EaseOutCubic,
        "ease-out-expo" => Easing::EaseOutExpo,
        "ease-out-back" => Easing::EaseOutBack(1.70158),
        _ => return None,
    })
}

fn parse_binds(node: &KdlNode, config: &mut Config) {
    let Some(doc) = node.children() else { return };
    for n in doc.nodes() {
        // Node name is the key combo, child nodes are actions.
        let combo = n.name().value().to_string();
        if let Some(actions) = n.children() {
            for action in actions.nodes() {
                if let Some(a) = parse_action(action) {
                    config.binds.push(Bind {
                        combo: combo.clone(),
                        action: a,
                    });
                }
            }
        }
    }
}

/// Map an action node like `focus-column-left;` or
/// `set-column-width "+10";` to an Action.
fn parse_action(node: &KdlNode) -> Option<Action> {
    let name = node.name().value();
    let arg = first_string_arg(node);
    let action = match name {
        "quit" => Action::Quit,
        "close-window" => Action::CloseWindow,
        "spawn" | "spawn-sh" => {
            let cmd = arg.unwrap_or_default();
            Action::Spawn(cmd)
        }
        "focus-column-left" => Action::FocusColumnLeft,
        "focus-column-right" => Action::FocusColumnRight,
        "focus-column-first" => Action::FocusColumnFirst,
        "focus-column-last" => Action::FocusColumnLast,
        "focus-window-down" => Action::FocusWindowDown,
        "focus-window-up" => Action::FocusWindowUp,
        "move-column-left" => Action::MoveColumnLeft,
        "move-column-right" => Action::MoveColumnRight,
        "move-window-down" => Action::MoveWindowDown,
        "move-window-up" => Action::MoveWindowUp,
        "move-window-to-column-left" => Action::MoveWindowToColumnLeft,
        "move-window-to-column-right" => Action::MoveWindowToColumnRight,
        "consume-or-expel-window-left" => Action::ConsumeOrExpelWindowLeft,
        "consume-or-expel-window-right" => Action::ConsumeOrExpelWindowRight,
        "set-column-width" => Action::SetColumnWidth(arg.unwrap_or_default()),
        "set-window-height" => Action::SetWindowHeight(arg.unwrap_or_default()),
        "toggle-full-width" => Action::ToggleFullWidth,
        "maximize-column" => Action::MaximizeColumn,
        "toggle-windowed-fullscreen" => Action::ToggleWindowedFullscreen,
        "toggle-window-floating" => Action::ToggleWindowFloating,
        "toggle-overview" => Action::ToggleOverview,
        "focus-workspace" | "workspace-switch" => {
            Action::FocusWorkspace(arg.and_then(|s| s.parse().ok()).unwrap_or(1))
        }
        "move-window-to-workspace" => {
            Action::MoveWindowToWorkspace(arg.and_then(|s| s.parse().ok()).unwrap_or(1))
        }
        "move-column-to-workspace" => {
            Action::MoveColumnToWorkspace(arg.and_then(|s| s.parse().ok()).unwrap_or(1))
        }
        other => {
            log::debug!("ignoring unknown action {other:?}");
            Action::Unknown(other.to_string())
        }
    };
    // Keep unknown actions in the bind table (they no-op) so the combo
    // still shadows defaults sensibly.
    Some(action)
}

// --- small KDL helpers -------------------------------------------------

fn first_string_arg(node: &KdlNode) -> Option<String> {
    // Arguments are entries without a property name.
    node.entries()
        .iter()
        .filter(|e| e.name().is_none())
        .find_map(|e| e.value().as_string().map(|s| s.to_string()))
}

fn first_float_arg(node: &KdlNode) -> Option<f64> {
    node.entries()
        .iter()
        .filter(|e| e.name().is_none())
        .find_map(|e| {
            e.value()
                .as_integer()
                .map(|v| v as f64)
                .or_else(|| e.value().as_float())
        })
}

#[allow(dead_code)]
fn has_property(node: &KdlNode, name: &str) -> bool {
    node.entries()
        .iter()
        .any(|e| e.name().map(|n| n.value() == name).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let cfg = Config::default();
        assert!(matches!(cfg.mod_key, ModKey::Alt));
        assert_eq!(cfg.layout.gaps, 8.0);
        assert!(cfg
            .binds
            .iter()
            .any(|b| b.combo == "Mod+H" && b.action == Action::FocusColumnLeft));
    }

    #[test]
    fn parse_full_example() {
        let src = r#"
            input {
                keyboard-shortcuts {
                    Mod "Super"
                }
            }
            layout {
                gaps 12
                edge-padding 4
                center-focused-column "always"
                default-column-width { fixed 1000; }
            }
            binds {
                Mod+H { focus-column-left; }
                Mod+L { focus-column-right; }
                Mod+W { set-column-width "+100"; }
            }
        "#;
        let cfg = parse(src).unwrap();
        assert!(matches!(cfg.mod_key, ModKey::Super));
        assert_eq!(cfg.layout.gaps, 12.0);
        assert_eq!(cfg.layout.edge_padding, 4.0);
        assert_eq!(cfg.layout.center_focused_column, CenterFocused::Always);
        assert_eq!(cfg.layout.default_column_width, ColumnWidth::Fixed(1000.0));
        assert_eq!(cfg.binds.len(), 3);
        assert!(cfg
            .binds
            .iter()
            .any(|b| b.action == Action::SetColumnWidth("+100".into())));
    }

    #[test]
    fn parse_errors_are_soft() {
        assert!(parse("this is ((( not kdl").is_err());
        // Unknown nodes are fine.
        let cfg = parse("favorites { color \"blue\"; }").unwrap();
        assert_eq!(cfg.layout.gaps, 8.0);
    }

    #[test]
    fn animations_config() {
        let cfg = parse(
            "animations {
                window-movement { duration-ms 500; easing \"ease-out-expo\"; }
                view-offset { duration-ms 0; }
            }",
        )
        .unwrap();
        assert!(cfg.animations.enabled);
        assert_eq!(cfg.animations.window_movement.duration, std::time::Duration::from_millis(500));
        assert_eq!(cfg.animations.window_movement.easing, Easing::EaseOutExpo);
        assert_eq!(cfg.animations.view_offset.duration, std::time::Duration::ZERO);
        // Defaults intact for untouched kinds.
        assert_eq!(cfg.animations.window_open.duration, std::time::Duration::from_millis(150));

        let cfg = parse("animations { off; }").unwrap();
        assert!(!cfg.animations.enabled);

        // Unknown easing keeps the old value (soft failure).
        let cfg = parse("animations { window-open { easing \"bogus\"; } }").unwrap();
        assert_eq!(cfg.animations.window_open.easing, Easing::EaseOutCubic);
    }

    #[test]
    fn proportion_clamped() {
        let cfg = parse("layout { default-column-width { proportion 5000; } }").unwrap();
        assert_eq!(cfg.layout.default_column_width, ColumnWidth::Proportion(100.0));
    }
}
