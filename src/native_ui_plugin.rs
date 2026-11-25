//! Native Bevy interface for the simulation.

/// Assigns only when the value differs. UI rendering is retained, so writing an
/// unchanged value every frame would still mark the node for re-extraction.
macro_rules! set_if_neq {
    ($place:expr, $value:expr) => {{
        let value = $value;
        if $place != value {
            $place = value;
        }
    }};
}

mod accessibility;
mod actions;
mod controls;
mod dialogs;
mod icons;
mod layout;
mod minimap;
mod palette;
mod sparkline;
mod speed;
mod sync;
#[cfg(test)]
mod tests;
mod theme;
mod toolbar;
use crate::{
    app::{App as GolApp, EditTool},
    feedback::EditFeedback,
    shared::{CellAge, GridOverlay, Simulation, StartupSet, Theme, UpdateSet},
    ui_input::{BoardFocus, BoardFocusVisible, UiInputRegions, UiInputSet},
};
use actions::*;
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::ImageSampler,
    input_focus::{
        InputFocus, InputFocusVisible,
        tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
    },
    picking::hover::Hovered,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Activate, Button as WidgetButton, ScrollArea},
    window::PrimaryWindow,
};
use dialogs::DialogKind;
use icons::Icon;
pub use layout::canvas_rect;
use layout::*;
#[cfg(test)]
use palette::thumbnail_sized;
use palette::{sync_palette, thumbnail};
use sync::*;
use theme::{ThemedBackground, ThemedBorder, ThemedImage, ThemedText, Token};

/// The status bar fits on one row at every window width the app allows.
const STATUS_BAR_HEIGHT: f32 = 48.0;

/// Tab follows the layout: the toolbar's groups (negative orders), then the
/// palette, the status bar, and the board last.
const TAB_ORDER_PALETTE: i32 = 1;
const TAB_ORDER_STATUS_BAR: i32 = 2;
const TAB_ORDER_BOARD: i32 = 3;
const SIDE: f32 = 256.0;
/// Library rows are short, with a wide thumbnail, since most patterns are wider
/// than they are tall. The texture has two texels per point, for Retina displays.
const ROW_THUMBNAIL: Vec2 = Vec2::new(64.0, 32.0);
const ROW_THUMBNAIL_TEXELS: UVec2 = UVec2::new(128, 64);
const UI_FONT: Handle<Font> = bevy::asset::uuid_handle!("67dfd7f0-b829-4a23-a5ae-3e416c5aba2d");

#[derive(Resource, Default)]
pub struct NativeUi {
    pub(crate) dialog: Option<DialogKind>,
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct NativeLayout;

pub struct NativeUiPlugin;
impl Plugin for NativeUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeUi>()
            .init_resource::<EditFeedback>()
            .init_resource::<Theme>()
            .init_resource::<toolbar::WorkspaceMenu>()
            .init_resource::<minimap::MinimapArea>()
            .add_plugins((TabNavigationPlugin, bevy::picking::cursor::CursorIconPlugin))
            .add_observer(activate)
            .add_observer(toolbar::menu_action)
            .add_observer(speed::handle_key)
            .add_observer(controls::navigate)
            .add_plugins(dialogs::NativeDialogsPlugin)
            .add_plugins(accessibility::NativeAccessibilityPlugin)
            .add_systems(
                PostUpdate,
                (
                    palette::sync_scrollbar.after(bevy::ui::UiSystems::Layout),
                    controls::keep_widest.after(bevy::ui::UiSystems::Layout),
                    theme::apply.after(bevy::a11y::AccessibilitySystems::Update),
                    theme::apply_window,
                    speed::edit
                        .after(bevy::text::EditableTextSystems)
                        .before(bevy::ui::UiSystems::PostLayout),
                    speed::sync_fill.before(bevy::ui::UiSystems::Layout),
                ),
            )
            .add_systems(Startup, setup.after(StartupSet::BuildWorld))
            .add_systems(
                Update,
                (
                    dialogs::file_shortcuts,
                    dialogs::sync_switches,
                    dialogs::sync_recording,
                    dialogs::sync_density,
                    dialogs::sync_import_preview,
                )
                    .after(dialogs::update_dialogs)
                    .in_set(NativeLayout),
            )
            .add_systems(
                Update,
                (
                    dialogs::poll_file_dialog,
                    dialogs::update_dialogs,
                    sync,
                    sync_palette,
                    palette::filter,
                    palette::sync_preview,
                    speed::capture_scrub,
                    speed::sync_scrubber,
                    speed::sync_chevrons,
                    controls::sync_overlays,
                    controls::sync_footer,
                    controls::sync_status,
                    (sparkline::sync, minimap::sync).chain(),
                    toolbar::sync_layout,
                    toolbar::sync_menu,
                    controls::sync_feedback,
                    publish_input_regions,
                    release_pointer_focus,
                    release_palette_focus,
                    sync_board_focus,
                )
                    .chain()
                    .in_set(NativeLayout)
                    .before(UpdateSet::Camera)
                    .before(UiInputSet::Capture),
            );
    }
}

#[derive(Component)]
struct NativeRoot;
#[derive(Component)]
struct Sidebar;
#[derive(Component)]
struct Palette;
#[derive(Component)]
struct Footer;
#[derive(Component)]
struct FooterStatus;
#[derive(Component)]
struct BoardArea;
#[derive(Component)]
struct BoardFocusRing;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Readout {
    Status,
    Generation,
    Population,
    /// How the latest generation changed the population, or how the board settled.
    Change,
    /// The cell under the pointer.
    Pointer,
    Board,
}
/// Green while running, neutral while paused.
#[derive(Component)]
struct StatusPill;
