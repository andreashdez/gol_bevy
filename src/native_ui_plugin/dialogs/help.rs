//! The shortcut reference in the Help dialog.
use super::*;
use crate::keybindings::{HelpSection, help_sections};

/// Width of the key caps beside each action, so the actions line up.
const HELP_KEY_COLUMN: f32 = 132.0;

/// Shortcut groups in two columns, which stack when the window is narrow:
/// Simulation and Editing, then Stamps, Selection, Camera, and Interface.
pub(super) fn help_columns(parent: &mut ChildSpawnerCommands) {
    let mut sections = help_sections().into_iter();
    parent
        .spawn(Node {
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(32),
            row_gap: px(20),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            for count in [2, 4] {
                p.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(20),
                    flex_grow: 1.0,
                    flex_basis: px(320),
                    min_width: px(0),
                    ..default()
                })
                .with_children(|p| {
                    for section in sections.by_ref().take(count) {
                        help_section(p, section);
                    }
                });
            }
        });
}

fn help_section(parent: &mut ChildSpawnerCommands, section: HelpSection) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(7),
            ..default()
        })
        .with_children(|p| {
            p.spawn(label(section.title, 16.0, Token::Ink));
            for entry in section.entries {
                p.spawn(Node {
                    align_items: AlignItems::Center,
                    column_gap: px(12),
                    ..default()
                })
                .with_children(|p| {
                    p.spawn(Node {
                        width: px(HELP_KEY_COLUMN),
                        flex_shrink: 0.0,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: px(4),
                        row_gap: px(4),
                        ..default()
                    })
                    .with_children(|p| {
                        for cap in entry.caps {
                            key_cap(p, &cap.replace("Mod", command_key()));
                        }
                    });
                    p.spawn(label(entry.action, 14.0, Token::Ink));
                });
            }
        });
}

/// A key or gesture drawn as a small raised key.
fn key_cap(parent: &mut ChildSpawnerCommands, text: &str) {
    parent
        .spawn((
            Node {
                min_width: px(24),
                padding: UiRect::axes(px(7), px(2)),
                justify_content: JustifyContent::Center,
                border: UiRect {
                    bottom: px(2),
                    ..UiRect::all(px(1))
                },
                border_radius: BorderRadius::all(px(5)),
                ..default()
            },
            ThemedBackground(Token::Surface),
            ThemedBorder(Token::Line),
            Pickable::IGNORE,
        ))
        .with_child(label(text, 12.0, Token::Ink));
}
