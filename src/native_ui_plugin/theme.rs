//! Colour tokens. Controls name the role a colour plays and the theme decides the
//! value, so switching themes recolours only what it has to.
use super::*;
use crate::shared::Theme;
use bevy::text::TextCursorStyle;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Token {
    Panel,
    Surface,
    /// Text field backgrounds.
    Field,
    Line,
    Ink,
    Muted,
    Accent,
    AccentPressed,
    /// Text on an accent background.
    OnAccent,
    Hover,
    Selected,
    /// Keyboard focus outline on buttons and the slider.
    Focus,
    Error,
    /// Dims the workspace behind a dialog. Blending is linear, so it needs a
    /// high alpha to look clearly darker.
    Scrim,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
#[require(BackgroundColor)]
pub(super) struct ThemedBackground(pub Token);

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
#[require(BorderColor)]
pub(super) struct ThemedBorder(pub Token);

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
#[require(TextColor)]
pub(super) struct ThemedText(pub Token);

/// Tints an icon mask, so icons follow the theme like text.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
#[require(ImageNode)]
pub(super) struct ThemedImage(pub Token);

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
#[require(TextCursorStyle)]
pub(super) struct ThemedCursor(pub Token);

struct Colors {
    window: Color,
    panel: Color,
    surface: Color,
    field: Color,
    line: Color,
    ink: Color,
    muted: Color,
    accent: Color,
    accent_pressed: Color,
    on_accent: Color,
    hover: Color,
    selected: Color,
    focus: Color,
    error: Color,
    scrim: Color,
}

const DARK: Colors = Colors {
    window: crate::shared::BG_COLOR,
    panel: Color::srgb(0.055, 0.075, 0.095),
    surface: Color::srgb(0.095, 0.12, 0.145),
    field: Color::srgb(0.025, 0.04, 0.055),
    line: Color::srgb(0.18, 0.23, 0.27),
    ink: Color::srgb(0.88, 0.93, 0.94),
    muted: Color::srgb(0.52, 0.63, 0.68),
    accent: Color::srgb(0.22, 0.85, 0.56),
    accent_pressed: Color::srgb(0.15, 0.65, 0.43),
    on_accent: Color::srgb(0.055, 0.075, 0.095),
    hover: Color::srgb(0.16, 0.23, 0.26),
    selected: Color::srgb(0.08, 0.21, 0.17),
    focus: Color::srgb(0.88, 0.93, 0.94),
    error: Color::srgb(1.0, 0.55, 0.5),
    scrim: Color::srgba(0.0, 0.0, 0.0, 0.85),
};

const LIGHT: Colors = Colors {
    window: Color::srgb(0.86, 0.89, 0.91),
    panel: Color::srgb(0.96, 0.97, 0.975),
    surface: Color::WHITE,
    field: Color::WHITE,
    line: Color::srgb(0.74, 0.79, 0.82),
    ink: Color::srgb(0.07, 0.10, 0.12),
    muted: Color::srgb(0.32, 0.40, 0.44),
    accent: Color::srgb(0.04, 0.50, 0.31),
    accent_pressed: Color::srgb(0.03, 0.39, 0.24),
    on_accent: Color::WHITE,
    hover: Color::srgb(0.88, 0.92, 0.94),
    selected: Color::srgb(0.85, 0.95, 0.90),
    focus: Color::srgb(0.07, 0.10, 0.12),
    error: Color::srgb(0.70, 0.15, 0.12),
    scrim: Color::srgba(0.0, 0.0, 0.0, 0.55),
};

const HIGH_CONTRAST: Colors = Colors {
    window: Color::BLACK,
    panel: Color::BLACK,
    surface: Color::srgb(0.07, 0.07, 0.07),
    field: Color::BLACK,
    line: Color::srgb(0.80, 0.80, 0.80),
    ink: Color::WHITE,
    muted: Color::srgb(0.72, 0.72, 0.72),
    accent: Color::srgb(0.35, 1.0, 0.65),
    accent_pressed: Color::srgb(0.25, 0.80, 0.50),
    on_accent: Color::BLACK,
    hover: Color::srgb(0.25, 0.25, 0.25),
    selected: Color::srgb(0.0, 0.30, 0.15),
    focus: Color::srgb(1.0, 0.85, 0.0),
    error: Color::srgb(1.0, 0.60, 0.55),
    scrim: Color::srgba(0.0, 0.0, 0.0, 0.90),
};

impl Colors {
    fn of(theme: Theme) -> &'static Colors {
        match theme {
            Theme::Dark => &DARK,
            Theme::Light => &LIGHT,
            Theme::HighContrast => &HIGH_CONTRAST,
        }
    }

    fn get(&self, token: Token) -> Color {
        match token {
            Token::Panel => self.panel,
            Token::Surface => self.surface,
            Token::Field => self.field,
            Token::Line => self.line,
            Token::Ink => self.ink,
            Token::Muted => self.muted,
            Token::Accent => self.accent,
            Token::AccentPressed => self.accent_pressed,
            Token::OnAccent => self.on_accent,
            Token::Hover => self.hover,
            Token::Selected => self.selected,
            Token::Focus => self.focus,
            Token::Error => self.error,
            Token::Scrim => self.scrim,
        }
    }
}

/// Resolves tokens that are new or changed, or every token when the theme changes.
/// Runs after everything that picks tokens, including the accessibility sync.
pub(super) fn apply(
    theme: Res<Theme>,
    clear: Option<ResMut<ClearColor>>,
    mut backgrounds: Query<(Ref<ThemedBackground>, &mut BackgroundColor)>,
    mut borders: Query<(Ref<ThemedBorder>, &mut BorderColor)>,
    mut texts: Query<(Ref<ThemedText>, &mut TextColor)>,
    mut cursors: Query<(Ref<ThemedCursor>, &mut TextCursorStyle)>,
    mut images: Query<(Ref<ThemedImage>, &mut ImageNode)>,
) {
    let colors = Colors::of(*theme);
    let all = theme.is_changed();
    if all && let Some(mut clear) = clear {
        set_if_neq!(clear.0, colors.window);
    }
    for (token, mut color) in &mut backgrounds {
        if all || token.is_changed() {
            color.set_if_neq(BackgroundColor(colors.get(token.0)));
        }
    }
    for (token, mut color) in &mut borders {
        if all || token.is_changed() {
            color.set_if_neq(BorderColor::all(colors.get(token.0)));
        }
    }
    for (token, mut color) in &mut texts {
        if all || token.is_changed() {
            color.set_if_neq(TextColor(colors.get(token.0)));
        }
    }
    for (token, mut style) in &mut cursors {
        if all || token.is_changed() {
            set_if_neq!(style.color, colors.get(token.0));
        }
    }
    for (token, mut image) in &mut images {
        if all || token.is_changed() {
            set_if_neq!(image.color, colors.get(token.0));
        }
    }
}

/// Matches the OS title bar to the theme. Checked every frame because creating
/// the native window overwrites it with the OS theme, which may happen after
/// the theme last changed.
pub(super) fn apply_window(
    theme: Res<Theme>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    for mut window in &mut windows {
        set_if_neq!(window.window_theme, Some(theme.window_theme()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn switching_themes_recolours_existing_nodes() {
        let mut world = World::new();
        world.insert_resource(Theme::Dark);
        world.insert_resource(ClearColor(Color::NONE));
        let panel = world
            .spawn((ThemedBackground(Token::Panel), ThemedText(Token::Ink)))
            .id();
        world.run_system_once(apply).unwrap();
        assert_eq!(world.get::<BackgroundColor>(panel).unwrap().0, DARK.panel);
        assert_eq!(world.get::<TextColor>(panel).unwrap().0, DARK.ink);
        assert_eq!(world.resource::<ClearColor>().0, DARK.window);

        *world.resource_mut::<Theme>() = Theme::Light;
        world.run_system_once(apply).unwrap();
        assert_eq!(world.get::<BackgroundColor>(panel).unwrap().0, LIGHT.panel);
        assert_eq!(world.get::<TextColor>(panel).unwrap().0, LIGHT.ink);
        assert_eq!(world.resource::<ClearColor>().0, LIGHT.window);

        world.get_mut::<ThemedBackground>(panel).unwrap().0 = Token::Selected;
        world.run_system_once(apply).unwrap();
        assert_eq!(
            world.get::<BackgroundColor>(panel).unwrap().0,
            LIGHT.selected
        );
    }

    #[test]
    fn title_bar_follows_the_theme() {
        let mut world = World::new();
        world.insert_resource(Theme::Light);
        let window = world
            .spawn((Window::default(), bevy::window::PrimaryWindow))
            .id();
        let title_bar = |world: &World| world.get::<Window>(window).unwrap().window_theme;
        world.run_system_once(apply_window).unwrap();
        assert_eq!(title_bar(&world), Some(bevy::window::WindowTheme::Light));

        // Window creation reports the OS theme back without the theme changing.
        world.get_mut::<Window>(window).unwrap().window_theme =
            Some(bevy::window::WindowTheme::Dark);
        world.run_system_once(apply_window).unwrap();
        assert_eq!(title_bar(&world), Some(bevy::window::WindowTheme::Light));

        *world.resource_mut::<Theme>() = Theme::HighContrast;
        world.run_system_once(apply_window).unwrap();
        assert_eq!(title_bar(&world), Some(bevy::window::WindowTheme::Dark));
    }

    #[test]
    fn every_theme_keeps_text_and_cells_readable() {
        // WCAG relative luminance contrast; 4.5:1 is the minimum for body text.
        fn luminance(color: Color) -> f32 {
            let linear = color.to_linear();
            0.2126 * linear.red + 0.7152 * linear.green + 0.0722 * linear.blue
        }
        fn contrast(a: Color, b: Color) -> f32 {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        }
        for theme in Theme::ALL {
            let colors = Colors::of(theme);
            for background in [colors.panel, colors.surface, colors.field] {
                assert!(contrast(colors.ink, background) >= 7.0, "{theme:?} ink");
                assert!(contrast(colors.muted, background) >= 4.5, "{theme:?} muted");
            }
            assert!(
                contrast(colors.on_accent, colors.accent) >= 4.5,
                "{theme:?} accent button"
            );
            assert!(
                contrast(colors.accent, colors.panel) >= 4.5,
                "{theme:?} accent text"
            );
            // Cells and the pointer outline are graphics, which need 3:1.
            let board = theme.board();
            assert!(contrast(board.live, board.dead) >= 3.0, "{theme:?} cells");
            // Every age keeps cells visible, and trails stay faint enough to
            // never pass for live cells or crowd them out.
            for age in 0..=u8::MAX {
                let live = crate::render_plugin::live_shade(board, age);
                let trail = crate::render_plugin::dead_shade(board, age);
                assert!(contrast(live, board.dead) >= 3.0, "{theme:?} age {age}");
                assert!(contrast(trail, board.dead) < 1.6, "{theme:?} trail {age}");
            }
            assert!(contrast(board.hover, board.dead) >= 3.0, "{theme:?} hover");
            // A stamp preview has its own hue, so it never passes for live cells.
            let hue = |color: Color| Hsla::from(color).hue;
            let apart = (hue(board.ghost) - hue(board.live)).abs();
            assert!(
                apart.min(360.0 - apart) >= 45.0,
                "{theme:?} stamp preview hue"
            );
            assert!(
                contrast(board.ghost_outline, board.dead) >= 3.0,
                "{theme:?} stamp outline"
            );
            assert!(
                contrast(board.hover_erase, board.dead) >= 3.0,
                "{theme:?} erase hover"
            );
        }
    }
}
