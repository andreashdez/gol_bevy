//! The thumbnail and summary of the RLE being imported.
use super::*;

#[derive(Component)]
pub(in crate::native_ui_plugin) struct ImportPreviewText;

/// Stable, so the preview can be spawned before its image exists.
const IMPORT_PREVIEW: Handle<Image> =
    bevy::asset::uuid_handle!("2f6d3b1e-8a4c-4e0b-9b7d-5c1a7e9f3d20");

/// A thumbnail and summary of the RLE in the source field, updated as it changes.
pub(super) fn import_preview(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            align_items: AlignItems::Center,
            column_gap: px(12),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            p.spawn((
                ImageNode::new(IMPORT_PREVIEW),
                Node {
                    width: px(64),
                    height: px(64),
                    flex_shrink: 0.0,
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            p.spawn((ImportPreviewText, label("", 14.0, Token::Muted)));
        });
}

/// What the preview says about `source`, and whether that is an error.
pub(super) fn import_summary(source: &str) -> (Option<crate::patterns::Pattern>, String, bool) {
    if source.trim().is_empty() {
        return (None, "Paste RLE above to see a preview.".into(), false);
    }
    match crate::patterns::Pattern::from_named_rle("", source) {
        Ok(pattern) => {
            let mut summary = format!(
                "{} × {} · {} cells",
                pattern.width,
                pattern.height,
                pattern.cells.len()
            );
            if !pattern.name.is_empty() {
                summary = format!("{} · {summary}", pattern.name);
            }
            if let Some(rule) = pattern.rule {
                summary.push_str(&format!(" · rule {rule}"));
            }
            (Some(pattern), summary, false)
        }
        Err(error) => (None, format!("Not valid RLE: {error}"), true),
    }
}

/// Redraws the preview when the source or theme changes, or the dialog reopens.
pub(in crate::native_ui_plugin) fn sync_import_preview(
    drafts: Res<Drafts>,
    theme: Res<Theme>,
    mut images: ResMut<Assets<Image>>,
    mut texts: Query<(Ref<ImportPreviewText>, &mut Text, &mut ThemedText)>,
    mut last: Local<Option<(u64, Theme)>>,
) {
    use std::hash::{Hash, Hasher};
    let Some((_, _, _)) = texts.iter().next() else {
        *last = None;
        return;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    drafts.source.hash(&mut hasher);
    let key = (hasher.finish(), *theme);
    let reopened = texts.iter().any(|(marker, _, _)| marker.is_added());
    if *last == Some(key) && !reopened {
        return;
    }
    *last = Some(key);
    let (pattern, summary, error) = import_summary(&drafts.source);
    let image = match &pattern {
        Some(pattern) => {
            super::thumbnail(&pattern.cells, pattern.width, pattern.height, theme.board())
        }
        None => super::thumbnail(&[], 1, 1, theme.board()),
    };
    let _ = images.insert(IMPORT_PREVIEW.id(), image);
    for (_, mut text, mut color) in &mut texts {
        if text.0 != summary {
            text.0 = summary.clone();
        }
        color.set_if_neq(ThemedText(if error { Token::Error } else { Token::Muted }));
    }
}
