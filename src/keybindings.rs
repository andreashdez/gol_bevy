//! Keyboard shortcuts: every binding as a constant with its keys, key caps, and
//! action, the characters typed each frame, and the Help sections and README
//! controls list generated from the same constants.
use crate::{app::EditTool, shared::WEB_BUILD};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub struct KeyBinding {
    /// Characters matched on the logical key, so the shortcut follows the active
    /// layout (`/` is Shift+7 on a German keyboard). Matched case-insensitively.
    pub chars: &'static [&'static str],
    /// Physical keys, for bindings that depend on position (WASD) or type no
    /// character.
    pub keys: &'static [KeyCode],
    /// Key caps shown in Help and tooltips, one per alternative.
    pub caps: &'static [&'static str],
    pub action: &'static str,
}

impl KeyBinding {
    pub fn just_pressed(self, input: &ButtonInput<KeyCode>, typed: &TypedKeys) -> bool {
        self.keys.iter().copied().any(|key| input.just_pressed(key))
            || self
                .chars
                .iter()
                .any(|character| typed.just_typed(character))
    }

    /// Held state is only tracked for physical keys.
    pub fn pressed(self, input: &ButtonInput<KeyCode>) -> bool {
        self.keys.iter().copied().any(|key| input.pressed(key))
    }
}

/// Characters typed this frame, lowercased. Rebuilt from key events every frame
/// instead of using `ButtonInput<Key>`, whose held state sticks when a modifier
/// changes the character between press and release (Shift+= pressed as `+`,
/// released as `=`), which would swallow the next press.
#[derive(Resource, Default)]
pub struct TypedKeys(Vec<String>);

impl TypedKeys {
    pub fn just_typed(&self, character: &str) -> bool {
        self.0.iter().any(|typed| typed == character)
    }

    pub fn press(&mut self, character: &str) {
        self.0.push(character.to_lowercase());
    }
}

/// Records the characters typed this frame. On layouts whose letter keys type
/// no Latin letters (Russian, Greek, Hebrew, Arabic), a letter key also counts
/// as the letter it has on a US layout, so letter shortcuts and Cmd/Ctrl chords
/// still work there. Latin layouts keep following the typed character.
pub fn collect_typed_keys(
    mut events: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut typed: ResMut<TypedKeys>,
) {
    if !typed.0.is_empty() {
        typed.0.clear();
    }
    for event in events.read() {
        if event.state.is_pressed()
            && !event.repeat
            && let bevy::input::keyboard::Key::Character(character) = &event.logical_key
        {
            typed.press(character);
            if !character.is_ascii()
                && let Some(letter) = us_letter(event.key_code)
            {
                typed.press(letter);
            }
        }
    }
}

/// The letter a physical letter key types on a US layout.
fn us_letter(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::KeyA => "a",
        KeyCode::KeyB => "b",
        KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",
        KeyCode::KeyE => "e",
        KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",
        KeyCode::KeyH => "h",
        KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",
        KeyCode::KeyK => "k",
        KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",
        KeyCode::KeyN => "n",
        KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",
        KeyCode::KeyQ => "q",
        KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",
        KeyCode::KeyT => "t",
        KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",
        KeyCode::KeyW => "w",
        KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",
        KeyCode::KeyZ => "z",
        _ => return None,
    })
}

pub const BINDING_RUN_PAUSE: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::Space],
    caps: &["Space"],
    action: "Play / pause",
};
pub const BINDING_STEP: KeyBinding = KeyBinding {
    chars: &["j"],
    keys: &[],
    caps: &["J"],
    action: "Step one generation",
};
pub const BINDING_RESET_TO_START: KeyBinding = KeyBinding {
    chars: &["b"],
    keys: &[],
    caps: &["B"],
    action: "Reset to start",
};
pub const BINDING_RANDOMIZE: KeyBinding = KeyBinding {
    chars: &["r"],
    keys: &[],
    caps: &["R"],
    action: "Randomize the board or view",
};
pub const BINDING_CLEAR: KeyBinding = KeyBinding {
    chars: &["x"],
    keys: &[],
    caps: &["X"],
    action: "Clear the board",
};
pub const BINDING_RESET_EPOCH: KeyBinding = KeyBinding {
    chars: &["0"],
    keys: &[],
    caps: &["0"],
    action: "Reset generation counter",
};
pub const BINDING_SPEED_UP: KeyBinding = KeyBinding {
    // `=` shares the key with `+` on US layouts.
    chars: &["+", "="],
    keys: &[],
    caps: &["+"],
    action: "Speed up",
};
pub const BINDING_SLOW_DOWN: KeyBinding = KeyBinding {
    chars: &["-"],
    keys: &[],
    caps: &["−"],
    action: "Slow down",
};

// Digits accept the physical key too, so AZERTY layouts, which need Shift
// for digits, can use the unshifted key.
pub const BINDING_TOOL_DRAW: KeyBinding = KeyBinding {
    chars: &["1"],
    keys: &[KeyCode::Digit1],
    caps: &["1"],
    action: "Draw tool",
};
pub const BINDING_TOOL_ERASE: KeyBinding = KeyBinding {
    chars: &["2"],
    keys: &[KeyCode::Digit2],
    caps: &["2"],
    action: "Erase tool",
};
pub const BINDING_TOOL_STAMP: KeyBinding = KeyBinding {
    chars: &["3"],
    keys: &[KeyCode::Digit3],
    caps: &["3"],
    action: "Stamp tool",
};
pub const BINDING_TOOL_SELECT: KeyBinding = KeyBinding {
    chars: &["4"],
    keys: &[KeyCode::Digit4],
    caps: &["4"],
    action: "Select tool",
};
/// Each tool with the shortcut that picks it.
pub const TOOL_SHORTCUTS: [(KeyBinding, EditTool); 4] = [
    (BINDING_TOOL_DRAW, EditTool::Draw),
    (BINDING_TOOL_ERASE, EditTool::Erase),
    (BINDING_TOOL_STAMP, EditTool::Stamp),
    (BINDING_TOOL_SELECT, EditTool::Select),
];

pub const BINDING_ERASE_SELECTION: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::Delete, KeyCode::Backspace],
    caps: &["Delete", "Backspace"],
    action: "Clear the selected cells",
};
pub const BINDING_DESELECT: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::Escape],
    caps: &["Esc"],
    action: "Deselect",
};

// The cell cursor: the arrow keys take it over from panning while the board
// has visible keyboard focus, reached with Tab.
pub const BINDING_MOVE_CURSOR: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
    ],
    caps: &["Arrow keys"],
    action: "Move the cell cursor (board focused with Tab)",
};
pub const BINDING_APPLY_AT_CURSOR: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::Enter, KeyCode::NumpadEnter],
    caps: &["Enter"],
    action: "Draw, erase, stamp, or select at the cursor",
};
/// Its keys are the modifier, held while an arrow key moves the cursor.
pub const BINDING_EXTEND_SELECTION: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::ShiftLeft, KeyCode::ShiftRight],
    caps: &["Shift+arrow keys"],
    action: "Extend the selection from the cursor",
};

pub const BINDING_ROTATE_STAMP: KeyBinding = KeyBinding {
    chars: &["z"],
    keys: &[],
    caps: &["Z"],
    action: "Rotate 90° clockwise",
};
pub const BINDING_FLIP_STAMP: KeyBinding = KeyBinding {
    chars: &["f"],
    keys: &[],
    caps: &["F"],
    action: "Flip horizontally",
};
pub const BINDING_CLEAR_STAMP: KeyBinding = KeyBinding {
    chars: &["c"],
    keys: &[],
    caps: &["C"],
    action: "Put the stamp away and draw",
};

pub const BINDING_TOGGLE_BOUNDARY_MODE: KeyBinding = KeyBinding {
    chars: &["m"],
    keys: &[],
    caps: &["M"],
    action: "Wrap / fixed / unbounded edges",
};
pub const BINDING_TOGGLE_PALETTE: KeyBinding = KeyBinding {
    chars: &["h"],
    keys: &[],
    caps: &["H"],
    action: "Show / hide the palette while stamping",
};
pub const BINDING_TOGGLE_DIAGNOSTICS: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::F3],
    caps: &["F3"],
    action: "Diagnostics",
};
pub const BINDING_TOGGLE_GRID: KeyBinding = KeyBinding {
    chars: &["t"],
    keys: &[],
    caps: &["T"],
    action: "Show / hide the grid",
};
pub const BINDING_TOGGLE_HELP: KeyBinding = KeyBinding {
    chars: &["/"],
    keys: &[],
    caps: &["/"],
    action: "Help",
};

pub const BINDING_PAN_LEFT: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyA, KeyCode::ArrowLeft],
    caps: &["A", "Left"],
    action: "Pan left",
};
pub const BINDING_PAN_RIGHT: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyD, KeyCode::ArrowRight],
    caps: &["D", "Right"],
    action: "Pan right",
};
pub const BINDING_PAN_UP: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyW, KeyCode::ArrowUp],
    caps: &["W", "Up"],
    action: "Pan up",
};
pub const BINDING_PAN_DOWN: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyS, KeyCode::ArrowDown],
    caps: &["S", "Down"],
    action: "Pan down",
};
pub const BINDING_PAN_CAMERA: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[
        KeyCode::KeyW,
        KeyCode::ArrowUp,
        KeyCode::KeyA,
        KeyCode::ArrowLeft,
        KeyCode::KeyS,
        KeyCode::ArrowDown,
        KeyCode::KeyD,
        KeyCode::ArrowRight,
    ],
    caps: &["WASD", "Arrow keys"],
    action: "Pan",
};
pub const BINDING_RESET_CAMERA: KeyBinding = KeyBinding {
    chars: &["v"],
    keys: &[],
    caps: &["V"],
    action: "Fit board",
};
pub const BINDING_QUIT: KeyBinding = KeyBinding {
    chars: &["q"],
    keys: &[],
    caps: &["Q"],
    action: "Quit",
};

/// Key caps of the Cmd/Ctrl chords, which have no `KeyBinding` as each is
/// matched where it is handled, shared by Help and the toolbar's tooltips.
pub const CAPS_UNDO: &[&str] = &["U", "Mod+Z"];
pub const CAPS_REDO: &[&str] = &["Mod+Shift+Z"];
pub const CAPS_COPY: &[&str] = &["Mod+C"];
pub const CAPS_SAVE: &[&str] = &["Mod+S"];
pub const CAPS_OPEN: &[&str] = &["Mod+O"];

/// A row of the Help dialog: alternative key caps and what they do. `Mod`
/// stands for Cmd on macOS and Ctrl elsewhere.
pub struct HelpEntry {
    pub caps: &'static [&'static str],
    pub action: &'static str,
}

impl From<KeyBinding> for HelpEntry {
    fn from(binding: KeyBinding) -> Self {
        Self {
            caps: binding.caps,
            action: binding.action,
        }
    }
}

pub struct HelpSection {
    pub title: &'static str,
    pub entries: Vec<HelpEntry>,
}

/// Every shortcut and pointer gesture, grouped as the Help dialog shows them.
pub fn help_sections() -> [HelpSection; 6] {
    let entry = |caps, action| HelpEntry { caps, action };
    [
        HelpSection {
            title: "Simulation",
            entries: vec![
                BINDING_RUN_PAUSE.into(),
                BINDING_STEP.into(),
                BINDING_RESET_TO_START.into(),
                BINDING_SPEED_UP.into(),
                BINDING_SLOW_DOWN.into(),
                BINDING_TOGGLE_BOUNDARY_MODE.into(),
                BINDING_RESET_EPOCH.into(),
            ],
        },
        HelpSection {
            title: "Editing",
            entries: vec![
                BINDING_TOOL_DRAW.into(),
                BINDING_TOOL_ERASE.into(),
                BINDING_TOOL_STAMP.into(),
                BINDING_TOOL_SELECT.into(),
                entry(&["Left drag"], "Draw, erase, or select with the tool"),
                entry(&["Right drag"], "Erase"),
                BINDING_MOVE_CURSOR.into(),
                BINDING_APPLY_AT_CURSOR.into(),
                BINDING_RANDOMIZE.into(),
                BINDING_CLEAR.into(),
                entry(CAPS_UNDO, "Undo and pause"),
                entry(CAPS_REDO, "Redo"),
            ],
        },
        HelpSection {
            title: "Stamps",
            entries: vec![
                entry(&["Click"], "Place the stamp"),
                BINDING_ROTATE_STAMP.into(),
                BINDING_FLIP_STAMP.into(),
                BINDING_CLEAR_STAMP.into(),
            ],
        },
        HelpSection {
            title: "Selection",
            entries: vec![
                entry(&["Mod+A"], "Select the live cells"),
                entry(CAPS_COPY, "Copy the selection as RLE"),
                entry(&["Mod+X"], "Cut the selection as RLE"),
                entry(&["Mod+V"], "Paste RLE as the stamp"),
                BINDING_ERASE_SELECTION.into(),
                BINDING_EXTEND_SELECTION.into(),
                BINDING_DESELECT.into(),
            ],
        },
        HelpSection {
            title: "Camera",
            entries: vec![
                entry(&["Wheel"], "Zoom"),
                entry(&["Middle drag"], "Pan"),
                BINDING_PAN_CAMERA.into(),
                BINDING_RESET_CAMERA.into(),
            ],
        },
        HelpSection {
            title: "Interface",
            entries: {
                let mut entries = vec![
                    BINDING_TOGGLE_GRID.into(),
                    BINDING_TOGGLE_PALETTE.into(),
                    BINDING_TOGGLE_DIAGNOSTICS.into(),
                    BINDING_TOGGLE_HELP.into(),
                ];
                if !WEB_BUILD {
                    entries.extend([
                        entry(CAPS_SAVE, "Save workspace"),
                        entry(CAPS_OPEN, "Open workspace"),
                        BINDING_QUIT.into(),
                    ]);
                }
                entries
            },
        },
    ]
}

/// The name of the command modifier on this platform, for `Mod` in key caps.
pub fn command_key() -> &'static str {
    if on_mac() { "Cmd" } else { "Ctrl" }
}

/// Whether the keyboard has a Cmd key. The web build asks the browser, as it is
/// compiled once for every platform.
fn on_mac() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        static ON_MAC: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            web_sys::window()
                .and_then(|window| window.navigator().platform().ok())
                .is_some_and(|platform| platform.starts_with("Mac"))
        });
        *ON_MAC
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        cfg!(target_os = "macos")
    }
}

/// A shortcut as a tooltip names it: its first key cap, with `Mod` named for
/// this platform.
pub fn shortcut_label(caps: &[&str]) -> String {
    caps.first()
        .map_or_else(String::new, |cap| cap.replace("Mod", command_key()))
}

/// The shortcut list in the README, generated so it cannot drift from the app.
pub fn controls_markdown() -> String {
    let mut out = String::new();
    for section in help_sections() {
        out.push_str(&format!(
            "### {}\n\n| Keys | Action |\n| --- | --- |\n",
            section.title
        ));
        for entry in section.entries {
            let caps = entry
                .caps
                .iter()
                .map(|cap| format!("`{}`", cap.replace("Mod", "Cmd/Ctrl")))
                .collect::<Vec<_>>()
                .join(" or ");
            out.push_str(&format!("| {caps} | {} |\n", entry.action));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::controls_markdown;

    #[test]
    fn readme_lists_every_shortcut_as_help_shows_it() {
        let readme = include_str!("../README.md");
        assert!(
            readme.contains(&controls_markdown()),
            "update the README's controls to:\n{}",
            controls_markdown()
        );
    }
}
