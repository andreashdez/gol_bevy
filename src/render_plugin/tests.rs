//! Tests for the board texture, edge walls, stamp preview, and board resizing
use super::*;
use crate::coords::cell_to_world;

#[test]
fn texel_rows_are_flipped_relative_to_board_rows() {
    let (cols, rows) = (4, 3);

    // Board row 0 is the bottom row, so it maps to the last texel row.
    assert_eq!(texel_offset(0, cols, rows), (2 * cols) * 4);
    assert_eq!(texel_offset(3, cols, rows), (2 * cols + 3) * 4);
    // The top board row maps to texel row 0.
    assert_eq!(texel_offset(2 * cols, cols, rows), 0);
    assert_eq!(texel_offset(2 * cols + 1, cols, rows), 4);
    // Every cell lands on a distinct, in-bounds texel.
    let mut offsets: Vec<_> = (0..cols * rows)
        .map(|idx| texel_offset(idx, cols, rows))
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    assert_eq!(offsets.len(), cols * rows);
    assert_eq!(offsets.last(), Some(&((cols * rows - 1) * 4)));
}

#[test]
fn texel_order_matches_world_space_vertical_order() {
    let (cols, rows) = (4, 3);
    // A cell higher up in world space must sit earlier in the texture.
    for row in 1..rows {
        let lower = cell_to_world(0, row as i64 - 1, cols, rows).y;
        let upper = cell_to_world(0, row as i64, cols, rows).y;
        assert!(upper > lower);
        assert!(texel_offset(row * cols, cols, rows) < texel_offset((row - 1) * cols, cols, rows));
    }
    // Sanity check the world mapping the texture has to agree with.
    assert_eq!(
        cell_to_world(0, 0, cols, rows).y,
        -(rows as f32) * CELL_SIZE / 2.0 + CELL_SIZE * 0.5
    );
}

#[test]
fn cell_colors_round_trip_to_srgb_bytes() {
    let colors = Theme::Dark.board();
    assert_eq!(color_to_texel(colors.live), [56, 217, 143, 255]);
    assert_eq!(color_to_texel(colors.dead), [20, 23, 28, 255]);
}

/// A world with `app`'s finite board on blank tiles, ready for [`sync_cells`].
fn board_world(app: GolApp, by_age: bool) -> World {
    let mut world = World::new();
    fill_board_world(&mut world, app, by_age);
    world
}

fn fill_board_world(world: &mut World, app: GolApp, by_age: bool) {
    let (cols, rows) = app.board.size().unwrap();
    world.insert_resource(Simulation::new(app));
    world.insert_resource(RedrawRequested(true));
    world.insert_resource(Theme::Dark);
    world.insert_resource(CellAge { enabled: by_age });
    world.init_resource::<CellAges>();
    world.init_resource::<Assets<Image>>();
    let mut images = world.resource_mut::<Assets<Image>>();
    let texture = BoardTexture::new(&mut images, cols, rows, Theme::Dark.board().dead);
    world.insert_resource(texture);
}

/// The tiles put back together into the one texture a whole board used to
/// be drawn on: a texel per cell, rows from the top of the board down.
fn board_texels(world: &World) -> Vec<u8> {
    let texture = world.resource::<BoardTexture>();
    let images = world.resource::<Assets<Image>>();
    let TileLayout { cols, rows } = texture.layout;
    let mut whole = vec![0; cols * rows * 4];
    for (tile, handle) in texture.tiles.iter().enumerate() {
        let (col, row, width, height) = texture.layout.rect(tile);
        let image = images.get(handle).unwrap();
        assert_eq!(image.size(), UVec2::new(width as u32, height as u32));
        assert_eq!(image.sampler, ImageSampler::nearest());
        let data = image.data.as_ref().unwrap();
        // A tile's texel rows run top down too, from its top row of cells.
        for (y, line) in data.chunks_exact(width * 4).enumerate() {
            let board_row = row + height - 1 - y;
            let start = ((rows - 1 - board_row) * cols + col) * 4;
            whole[start..][..width * 4].copy_from_slice(line);
        }
    }
    whole
}

fn texel_at(texels: &[u8], idx: usize, cols: usize, rows: usize) -> [u8; 4] {
    let offset = texel_offset(idx, cols, rows);
    texels[offset..offset + 4].try_into().unwrap()
}

#[test]
fn restored_board_dimensions_rebuild_the_texture_and_repaint_loaded_cells() {
    use bevy::ecs::system::RunSystemOnce;
    let mut app = GolApp::new(16, 24);
    app.populate_board(15, 23);
    let mut world = board_world(GolApp::new(8, 8), false);
    world.resource_mut::<Simulation>().app = app;
    world.insert_resource(BoardSettings {
        cols: 16,
        rows: 24,
        fill_canvas: false,
    });
    let board = world
        .spawn((BoardSprite, Transform::default(), Visibility::default()))
        .id();
    let old_tile = world
        .spawn((BoardTile, Sprite::default(), ChildOf(board)))
        .id();
    world.run_system_once(apply_board_settings).unwrap();
    let texture = world.resource::<BoardTexture>();
    assert_eq!(texture.layout, TileLayout { cols: 16, rows: 24 });
    let tiles = texture.tiles.clone();
    assert!(world.get_entity(old_tile).is_err());
    let sprites: Vec<_> = world
        .query_filtered::<(&Sprite, &ChildOf), With<BoardTile>>()
        .iter(&world)
        .map(|(sprite, parent)| (sprite.image.clone(), parent.parent()))
        .collect();
    assert_eq!(sprites, [(tiles[0].clone(), board)]);

    world.run_system_once(sync_cells).unwrap();
    assert_eq!(
        texel_at(&board_texels(&world), 23 * 16 + 15, 16, 24),
        color_to_texel(Theme::Dark.board().live)
    );
}

#[test]
fn syncing_cells_leaves_the_texture_alone_on_the_unbounded_board() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = board_world(GolApp::new(4, 3), false);
    let app = &mut world.resource_mut::<Simulation>().app;
    app.set_unbounded().unwrap();
    app.board.populate(1, 2);
    world.run_system_once(sync_cells).unwrap();

    let texels = board_texels(&world);
    let dead = color_to_texel(Theme::Dark.board().dead);
    assert!(texels.as_chunks::<4>().0.iter().all(|&texel| texel == dead));
}

#[test]
fn switching_themes_repaints_every_cell_without_a_board_change() {
    use bevy::ecs::system::RunSystemOnce;
    let mut app = GolApp::new(4, 3);
    app.board.populate(1, 2);
    let mut world = board_world(app, false);
    world.run_system_once(sync_cells).unwrap();

    // Nothing on the board changes, so only the theme can cause a repaint.
    world.resource_mut::<RedrawRequested>().0 = false;
    *world.resource_mut::<Theme>() = Theme::Light;
    world.run_system_once(sync_cells).unwrap();
    let colors = Theme::Light.board();
    let texels = board_texels(&world);
    for idx in 0..12 {
        let expected = if idx == 2 * 4 + 1 {
            colors.live
        } else {
            colors.dead
        };
        assert_eq!(
            texel_at(&texels, idx, 4, 3),
            color_to_texel(expected),
            "{idx}"
        );
    }
}

#[test]
fn generations_shade_births_deaths_and_survivors_but_edits_look_settled() {
    // A horizontal blinker in the middle row.
    let mut app = GolApp::new(5, 5);
    for col in 1..=3 {
        app.board.populate(col, 2);
    }
    let mut world = board_world(app, true);
    // A registered system remembers what it has seen, unlike `run_system_once`.
    let sync = world.register_system(sync_cells);
    let texel =
        |world: &World, col: usize, row: usize| texel_at(&board_texels(world), row * 5 + col, 5, 5);
    let colors = Theme::Dark.board();

    // Cells that were drawn rather than born start out plain.
    world.run_system(sync).unwrap();
    assert_eq!(texel(&world, 1, 2), color_to_texel(colors.live));
    assert_eq!(texel(&world, 1, 1), color_to_texel(colors.dead));

    world.resource_mut::<Simulation>().app.increment_epoch();
    world.run_system(sync).unwrap();
    assert_eq!(texel(&world, 2, 1), color_to_texel(colors.newborn));
    assert_eq!(texel(&world, 1, 2), color_to_texel(colors.trail));
    let survivor = color_to_texel(live_shade(colors, MATURE_AGE + 1));
    assert_eq!(texel(&world, 2, 2), survivor);

    // Trails fade and newborns settle as generations pass.
    for _ in 0..TRAIL_LENGTH {
        world.resource_mut::<Simulation>().app.increment_epoch();
    }
    world.run_system(sync).unwrap();
    assert_eq!(texel(&world, 0, 0), color_to_texel(colors.dead));
    assert_ne!(texel(&world, 2, 2), survivor, "the centre keeps ageing");

    // A cell drawn while paused looks like any live cell, not a birth.
    world.resource_mut::<Simulation>().app.populate_board(0, 0);
    world.run_system(sync).unwrap();
    assert_eq!(texel(&world, 0, 0), color_to_texel(colors.live));

    // Switching the style off paints plain colours again.
    world.resource_mut::<CellAge>().enabled = false;
    world.resource_mut::<RedrawRequested>().0 = false;
    world.run_system(sync).unwrap();
    assert_eq!(texel(&world, 2, 2), color_to_texel(colors.live));
}

#[test]
fn a_board_of_several_tiles_paints_every_cell_at_its_own_texel() {
    // Cells are stored a chunk at a time and drawn a tile at a time, so this
    // is where a cell could land on a neighbour's texel: past columns 64 and
    // 256, past rows 64 and 256, and at the edges of the tiles cut short.
    let (cols, rows) = (300, 270);
    for by_age in [false, true] {
        let mut app = GolApp::new(cols, rows);
        let noise = crate::board::RandomizeSettings::new(30, Some(5)).unwrap();
        app.set_randomize_settings(noise);
        app.randomize_board();
        let mut world = board_world(app, by_age);
        let sync = world.register_system(sync_cells);
        let palette = CellPalette::new(Theme::Dark.board(), by_age);
        let cells = |world: &World| -> Vec<bool> {
            let board = &world.resource::<Simulation>().app.board;
            (0..cols * rows)
                .map(|idx| board.is_alive((idx % cols) as i64, (idx / cols) as i64))
                .collect()
        };

        world.run_system(sync).unwrap();
        let mut before = cells(&world);
        // What the ages should be, kept a cell at a time with nothing skipped.
        let mut ages: Vec<u8> = before.iter().map(|&alive| settled_age(alive)).collect();

        // Generations run before each frame: one, several, and none, which is a
        // frame with only a hand edit in it. The run of nines takes cells born at
        // different times past settling a few at a time, so that a run of cells
        // has some settled and some not, and the seventy settles all the rest.
        let frames = [
            1, 1, 3, 1, 0, 7, 1, 1, 20, 1, 0, 1, 9, 9, 9, 9, 9, 9, 9, 70, 1, 1, 2, 0, 1,
        ];
        for (frame, generations) in frames.into_iter().enumerate() {
            let mut changed = vec![false; cols * rows];
            let mut after = before.clone();
            for _ in 0..generations {
                world.resource_mut::<Simulation>().app.increment_epoch();
                let next = cells(&world);
                for idx in 0..cols * rows {
                    // A cell that flips and flips back has still changed.
                    changed[idx] |= next[idx] != after[idx];
                }
                after = next;
            }
            if generations == 0 {
                // A cell drawn or erased by hand, by the corner of four tiles.
                let (col, row) = (250 + frame, 255);
                let app = &mut world.resource_mut::<Simulation>().app;
                if before[row * cols + col] {
                    app.depopulate_board(col as i64, row as i64);
                } else {
                    app.populate_board(col as i64, row as i64);
                }
                after = cells(&world);
                changed[row * cols + col] = true;
            }
            world.resource_mut::<RedrawRequested>().0 = true;
            world.run_system(sync).unwrap();

            for idx in 0..cols * rows {
                if ages[idx] < SETTLED_AGE {
                    ages[idx] = ages[idx].saturating_add(generations);
                }
                if changed[idx] {
                    ages[idx] = if generations > 0 {
                        0
                    } else {
                        settled_age(after[idx])
                    };
                }
            }
            let texels = board_texels(&world);
            for idx in 0..cols * rows {
                assert_eq!(
                    texel_at(&texels, idx, cols, rows),
                    palette.texel(after[idx], if by_age { ages[idx] } else { 0 }),
                    "cell {idx} in frame {frame}, by age: {by_age}"
                );
            }
            if by_age {
                assert_eq!(world.resource::<CellAges>().ages, ages, "frame {frame}");
            }
            before = after;
        }
    }
}

#[test]
fn tile_sprites_cover_their_own_cells_and_together_the_board() {
    let (cols, rows) = (300, 270);
    let mut world = board_world(GolApp::new(cols, rows), false);
    let board = world
        .spawn((BoardSprite, Transform::default(), Visibility::default()))
        .id();
    world.resource_scope(|world, texture: Mut<BoardTexture>| {
        texture.spawn_sprites(&mut world.commands(), board);
    });
    world.flush();

    let texture = world.resource::<BoardTexture>();
    let (tiles, layout) = (texture.tiles.clone(), texture.layout);
    let sprites: Vec<_> = world
        .query_filtered::<(&Sprite, &Transform, &ChildOf), With<BoardTile>>()
        .iter(&world)
        .map(|(sprite, transform, parent)| {
            assert_eq!(parent.parent(), board);
            let rect = Rect::from_center_size(
                transform.translation.truncate(),
                sprite.custom_size.unwrap(),
            );
            (sprite.image.clone(), rect)
        })
        .collect();
    assert_eq!(sprites.len(), 4);
    let mut area = 0.0;
    for (tile, handle) in tiles.iter().enumerate() {
        let (col, row, width, height) = layout.rect(tile);
        let rects: Vec<_> = sprites
            .iter()
            .filter(|(image, _)| image == handle)
            .map(|(_, rect)| *rect)
            .collect();
        // Its corners are the outer corners of its corner cells.
        let half = Vec2::splat(CELL_SIZE / 2.0);
        let low = cell_to_world(col as i64, row as i64, cols, rows).truncate() - half;
        let (right, top) = ((col + width - 1) as i64, (row + height - 1) as i64);
        let high = cell_to_world(right, top, cols, rows).truncate() + half;
        assert_eq!(rects, [Rect::from_corners(low, high)], "tile {tile}");
        area += rects[0].size().x * rects[0].size().y;
    }
    let whole = board_sprite_size(cols, rows);
    assert_eq!(area, whole.x * whole.y);
}

#[test]
fn a_frame_sends_the_gpu_only_the_tiles_it_painted_on() {
    use bevy::{app::TaskPoolPlugin, asset::AssetPlugin, ecs::message::Messages};
    for by_age in [false, true] {
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            AssetPlugin {
                watch_for_changes_override: Some(false),
                use_asset_processor_override: Some(false),
                ..default()
            },
        ))
        .init_asset::<Image>()
        .add_systems(Update, sync_cells);
        fill_board_world(app.world_mut(), GolApp::new(1024, 1024), by_age);
        // The tiles a frame modified, after `change` to the board.
        let mut frame = |change: &dyn Fn(&mut GolApp)| {
            change(&mut app.world_mut().resource_mut::<Simulation>().app);
            app.world_mut().resource_mut::<RedrawRequested>().0 = true;
            app.update();
            let tiles = app.world().resource::<BoardTexture>().tiles.clone();
            let mut events = app
                .world_mut()
                .resource_mut::<Messages<AssetEvent<Image>>>();
            let modified: Vec<_> = events.drain().collect();
            (0..tiles.len())
                .filter(|&tile| {
                    let id = tiles[tile].id();
                    modified.iter().any(|event| event.is_modified(id))
                })
                .collect::<Vec<_>>()
        };
        let (nothing, none) = (|_: &mut GolApp| {}, [0usize; 0]);

        assert_eq!(frame(&nothing), (0..16).collect::<Vec<_>>(), "first paint");
        assert_eq!(frame(&nothing), none, "nothing changed, by age: {by_age}");
        // Column 600 and row 300 are on the third tile of the second row.
        assert_eq!(frame(&|app| app.populate_board(600, 300)), [6]);
        assert_eq!(frame(&nothing), none, "by age: {by_age}");
        // The lone cell dies, and its trail fades on its own tile.
        for generation in 0..=SETTLED_AGE {
            let tiles = frame(&|app| app.increment_epoch());
            if by_age || generation == 0 {
                assert_eq!(tiles, [6], "generation {generation}, by age: {by_age}");
            } else {
                assert_eq!(tiles, none, "generation {generation}");
            }
        }
        // Settled, so nothing is left to paint.
        assert_eq!(
            frame(&|app| app.increment_epoch()),
            none,
            "by age: {by_age}"
        );
    }
}

#[test]
fn only_cells_still_ageing_are_visited() {
    let mut app = GolApp::new(200, 150);
    // A block, which never changes, and a blinker, which never stops.
    for (col, row) in [(100, 100), (101, 100), (100, 101), (101, 101)] {
        app.board.populate(col, row);
    }
    for col in 9..12 {
        app.board.populate(col, 10);
    }
    let mut world = board_world(app, true);
    let sync = world.register_system(sync_cells);
    let frame = |world: &mut World| {
        world.resource_mut::<Simulation>().app.increment_epoch();
        world.resource_mut::<RedrawRequested>().0 = true;
        world.run_system(sync).unwrap();
        let mut ageing = world.resource::<CellAges>().ageing.clone();
        ageing.sort_unstable();
        ageing
    };

    world.run_system(sync).unwrap();
    // Drawn cells have yet to grow old: the block's two rows, and the blinker's
    // three now it has turned.
    let block = [100 * 200 + 64, 101 * 200 + 64];
    assert_eq!(
        frame(&mut world),
        [9 * 200, 10 * 200, 11 * 200, block[0], block[1]]
    );

    for _ in 0..usize::from(SETTLED_AGE) {
        frame(&mut world);
    }
    // The block has settled; the blinker's three rows change every generation.
    assert_eq!(frame(&mut world), [9 * 200, 10 * 200, 11 * 200]);
    // Of its cells, only the four that turn on and off: its centre has grown old.
    let cells = &world.resource::<CellAges>().ageing_cells;
    assert_eq!(cells.iter().map(|cells| cells.count_ones()).sum::<u32>(), 4);
}

#[test]
fn the_stamp_ghost_texture_has_one_texel_per_cell_with_the_bottom_row_last() {
    // An L, three tall: a foot two wide on the bottom row.
    let image = stamp_ghost_image(&[(0, 0), (1, 0), (0, 1), (0, 2)], 2, 3);
    assert_eq!((image.width(), image.height()), (2, 3));
    assert_eq!(image.sampler, ImageSampler::nearest());
    let texels = image.data.as_ref().unwrap().as_chunks::<4>().0;
    let (alive, clear) = ([u8::MAX; 4], [0; 4]);
    // Texel rows run top to bottom.
    assert_eq!(texels, [alive, clear, alive, clear, alive, alive]);
}

fn stamp_world(app: GolApp) -> World {
    use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfig, GizmoConfigStore};
    let mut world = World::new();
    world.insert_resource(Simulation::new(app));
    world.insert_resource(Theme::Dark);
    world.insert_resource(HoveredCell(Some((8, 12))));
    world.insert_resource(Assets::<Image>::default());
    world.init_resource::<StampGhostImage>();
    let mut gizmos = GizmoConfigStore::default();
    gizmos.insert(GizmoConfig::default(), DefaultGizmoConfigGroup);
    world.insert_resource(gizmos);
    world
}

fn ghosts(world: &mut World) -> Vec<Entity> {
    world
        .query_filtered::<Entity, With<StampGhost>>()
        .iter(world)
        .collect()
}

#[test]
fn a_large_stamp_is_one_cropped_sprite_laid_out_once() {
    use bevy::ecs::system::RunSystemOnce;
    let (width, height) = (300, 200);
    let mut rle = format!("x = {width}, y = {height}\n");
    for _ in 1..height {
        rle.push_str(&format!("{width}o$"));
    }
    rle.push_str(&format!("{width}o!"));
    let mut app = GolApp::new(16, 24);
    app.paste_rle(&rle).unwrap();
    let mut world = stamp_world(app);

    world.run_system_once(sync_pattern_preview).unwrap();
    let [ghost] = ghosts(&mut world)[..] else {
        panic!("one ghost sprite");
    };
    let handle = world.resource::<StampGhostImage>().handle.clone();
    let sprite = world.get::<Sprite>(ghost).unwrap();
    assert_eq!(sprite.image, handle);
    assert_eq!(sprite.color, Theme::Dark.board().ghost);
    // Held back to the board's corner and cropped to the whole 16 × 24 board,
    // keeping the stamp's bottom rows, which are the texture's last.
    assert_eq!(sprite.custom_size, Some(Vec2::new(16.0, 24.0) * CELL_SIZE));
    assert_eq!(sprite.rect, Some(Rect::new(0.0, 176.0, 16.0, 200.0)));
    assert_eq!(
        world.get::<Transform>(ghost).unwrap().translation,
        Vec3::new(0.0, 0.0, 2.0)
    );
    assert_eq!(world.get::<Visibility>(ghost), Some(&Visibility::Visible));

    // A frame with the same stamp in the same place neither lays it out
    // again nor touches the sprite.
    let ticks = |world: &World| {
        let entity = world.entity(ghost);
        [
            entity.get_ref::<Sprite>().unwrap().last_changed(),
            entity.get_ref::<Transform>().unwrap().last_changed(),
            entity.get_ref::<Visibility>().unwrap().last_changed(),
        ]
    };
    let before = ticks(&world);
    world.run_system_once(sync_pattern_preview).unwrap();
    assert_eq!(ticks(&world), before);
    assert_eq!(world.resource::<StampGhostImage>().handle, handle);
    assert_eq!(world.resource::<Assets<Image>>().len(), 1);
    assert_eq!(ghosts(&mut world), [ghost]);

    // Turning the stamp draws it again, on its side.
    world
        .resource_mut::<Simulation>()
        .app
        .rotate_stamp_clockwise();
    world.run_system_once(sync_pattern_preview).unwrap();
    let ghost_image = world.resource::<StampGhostImage>();
    assert_ne!(ghost_image.handle, handle);
    assert_eq!((ghost_image.width, ghost_image.height), (height, width));
    assert_eq!(ghosts(&mut world), [ghost]);

    // Off the board it hides; without a stamp it goes, texture and all.
    world.resource_mut::<HoveredCell>().0 = None;
    world.run_system_once(sync_pattern_preview).unwrap();
    assert_eq!(world.get::<Visibility>(ghost), Some(&Visibility::Hidden));
    world
        .resource_mut::<Simulation>()
        .app
        .set_tool(EditTool::Draw);
    world.run_system_once(sync_pattern_preview).unwrap();
    assert!(ghosts(&mut world).is_empty());
    assert_eq!(world.resource::<StampGhostImage>().key, None);
}
