//! Search, library grouping, and a preview of the actual stamp orientation.
use super::*;
use bevy::text::{EditableText, TextCursorStyle};

#[derive(Component)]
pub(super) struct PatternSearch;
#[derive(Component)]
pub(super) struct PatternGroup(pub bool);
#[derive(Component)]
pub(super) struct SearchSummary;
#[derive(Component)]
pub(super) struct StampPreview;
#[derive(Component)]
pub(super) struct StampImage;
#[derive(Component)]
pub(super) struct StampCaption;

pub(super) fn search(parent: &mut ChildSpawnerCommands) {
    parent.spawn(label("Search patterns", 12.0, MUTED));
    let mut input = EditableText::new("");
    input.visible_lines = Some(1.0);
    parent.spawn((
        PatternSearch,
        input,
        TabIndex(0),
        accessibility::named_node(accesskit::Role::TextInput, "Search patterns"),
        Node {
            width: percent(100),
            min_height: px(38),
            padding: UiRect::all(px(9)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            flex_shrink: 0.0,
            ..default()
        },
        TextFont {
            font: UI_FONT.into(),
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(INK),
        TextCursorStyle {
            color: ACCENT,
            ..default()
        },
        BorderColor::all(LINE),
        BackgroundColor(Color::srgb(0.025, 0.04, 0.055)),
    ));
    parent.spawn((
        SearchSummary,
        label("", 12.0, MUTED),
        accessibility::feedback_node(),
    ));
}

pub(super) fn preview(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            StampPreview,
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                padding: UiRect::all(px(10)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(SURFACE),
            BorderColor::all(LINE),
        ))
        .with_children(|p| {
            p.spawn(label("STAMP PREVIEW", 11.0, MUTED));
            p.spawn((
                StampImage,
                ImageNode::default(),
                Node {
                    width: px(88),
                    height: px(88),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            p.spawn((
                StampCaption,
                label("", 13.0, INK),
                accessibility::named_node(accesskit::Role::Label, ""),
            ));
            p.spawn(row()).with_children(|p| {
                button(p, "Rotate", Action::Rotate);
                button(p, "Flip", Action::Flip);
            });
        });
}

fn matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.trim().to_lowercase())
}

pub(super) fn filter(
    sim: Res<Simulation>,
    search: Query<&EditableText, With<PatternSearch>>,
    mut entries: Query<(&mut Node, Option<&Action>, Option<&PatternGroup>)>,
    mut summary: Query<&mut Text, With<SearchSummary>>,
    mut previous: Local<Option<(String, usize)>>,
) {
    let Ok(search) = search.single() else {
        return;
    };
    let query = search.value().into_iter().collect::<String>();
    let count = sim.app.pattern_count();
    if previous
        .as_ref()
        .is_some_and(|p| p.0 == query && p.1 == count)
    {
        return;
    }
    let builtin_count = crate::patterns::Pattern::builtin_count();
    let visible: Vec<bool> = (0..count)
        .map(|i| matches(sim.app.pattern_name_at(i).unwrap_or(""), &query))
        .collect();
    for (mut node, action, group) in &mut entries {
        let show = if let Some(Action::Pattern(index)) = action {
            visible.get(*index).copied().unwrap_or(false)
        } else if let Some(PatternGroup(imported)) = group {
            visible
                .iter()
                .enumerate()
                .any(|(i, found)| *found && (i >= builtin_count) == *imported)
        } else {
            continue;
        };
        node.display = if show { Display::Flex } else { Display::None };
    }
    let found = visible.iter().filter(|v| **v).count();
    if let Ok(mut text) = summary.single_mut() {
        text.0 = if found == 0 {
            "No matches. Try another name or clear the search.".into()
        } else if query.trim().is_empty() {
            format!("{count} patterns · select to stamp")
        } else {
            format!("{found} of {count} patterns")
        };
    }
    *previous = Some((query, count));
}

pub(super) fn sync_preview(
    sim: Res<Simulation>,
    mut panel: Query<&mut Node, With<StampPreview>>,
    mut image: Query<&mut ImageNode, With<StampImage>>,
    mut caption: Query<&mut Text, With<StampCaption>>,
    mut images: ResMut<Assets<Image>>,
    mut previous: Local<Option<(Option<usize>, String, bool)>>,
) {
    let key = (
        sim.app.selected_pattern_index(),
        sim.app.selected_pattern_status_label(),
        sim.app.has_selected_pattern(),
    );
    if previous.as_ref() == Some(&key) {
        return;
    }
    *previous = Some(key);
    let Ok(mut panel) = panel.single_mut() else {
        return;
    };
    let Some((cells, width, height)) = sim.app.selected_pattern_layout() else {
        panel.display = Display::None;
        return;
    };
    panel.display = Display::Flex;
    if let Ok(mut image) = image.single_mut() {
        let bitmap = thumbnail(&cells, width, height);
        if image.image == ImageNode::default().image {
            image.image = images.add(bitmap);
        } else if let Some(mut existing) = images.get_mut(&image.image) {
            *existing = bitmap;
        } else {
            image.image = images.add(bitmap);
        }
    }
    if let Ok(mut text) = caption.single_mut() {
        let (rotation, flipped) = sim.app.stamp_orientation();
        text.0 = format!(
            "{}\n{} × {} · {}° · {}\nClick the board to place",
            sim.app.selected_pattern_name().unwrap_or("Pattern"),
            width,
            height,
            rotation as u32 * 90,
            if flipped { "flipped" } else { "unflipped" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test]
    fn stamp_preview_owns_its_texture_and_tracks_orientation() {
        let mut world = World::new();
        let mut app = GolApp::new(30, 30);
        app.select_pattern_index(0);
        world.insert_resource(Simulation::new(app));
        world.init_resource::<Assets<Image>>();
        let shared = ImageNode::default().image;
        let placeholder = thumbnail(&[], 1, 1);
        let original = placeholder.data.clone();
        world
            .resource_mut::<Assets<Image>>()
            .insert(shared.id(), placeholder)
            .unwrap();
        world.spawn((StampPreview, Node::default()));
        let image = world.spawn((StampImage, ImageNode::default())).id();
        let caption = world.spawn((StampCaption, Text::new(""))).id();
        world.run_system_once(sync_preview).unwrap();
        let handle = world.get::<ImageNode>(image).unwrap().image.clone();
        assert_ne!(handle, shared);
        assert_eq!(
            world.resource::<Assets<Image>>().get(&shared).unwrap().data,
            original
        );
        let before = world
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .clone();
        world
            .resource_mut::<Simulation>()
            .app
            .rotate_stamp_clockwise();
        world.resource_mut::<Simulation>().app.toggle_stamp_flip();
        world.run_system_once(sync_preview).unwrap();
        assert!(
            world
                .get::<Text>(caption)
                .unwrap()
                .0
                .contains("90° · flipped")
        );
        assert_ne!(
            world.resource::<Assets<Image>>().get(&handle).unwrap().data,
            before
        );
    }

    #[test]
    fn search_is_case_insensitive_and_preserves_library_indices() {
        assert!(matches("My Glider", " GLIDER "));
        assert!(!matches("Blinker", "glider"));
        let mut app = GolApp::new(30, 30);
        app.import_rle_pattern("Glider imported", "x = 1, y = 1\no!")
            .unwrap();
        let mut world = World::new();
        world.insert_resource(Simulation::new(app));
        world.spawn((PatternSearch, EditableText::new("imported")));
        let builtin = world.spawn((Node::default(), Action::Pattern(0))).id();
        // Imports follow the built-ins, however many files there are.
        let first_import = crate::patterns::Pattern::builtin_count();
        let imported = world
            .spawn((Node::default(), Action::Pattern(first_import)))
            .id();
        let heading = world.spawn((Node::default(), PatternGroup(false))).id();
        world.run_system_once(filter).unwrap();
        assert_eq!(world.get::<Node>(builtin).unwrap().display, Display::None);
        assert_eq!(world.get::<Node>(heading).unwrap().display, Display::None);
        assert_eq!(world.get::<Node>(imported).unwrap().display, Display::Flex);
        assert_eq!(
            world.resource::<Simulation>().app.selected_pattern_index(),
            Some(first_import)
        );
    }
}
