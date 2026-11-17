pub mod app;
mod atomic_file;
pub mod board;
pub mod canvas_view;
pub mod input_plugin;
pub mod native_ui_plugin;
pub mod patterns;
pub mod plane_view;
pub mod render_plugin;
pub mod settings_plugin;
pub mod settle;
pub mod shared;
pub mod simulation_plugin;
pub mod sparse_board;
#[cfg(test)]
mod test_support;
pub mod ui_input;
pub mod universe;
pub mod workspace;

#[cfg(test)]
mod docs_tests {
    /// Modules declared as `mod name;` in `source`, leaving out test-only ones.
    fn modules(source: &str) -> Vec<&str> {
        let mut names = Vec::new();
        let mut test_only = false;
        for line in source.lines().map(str::trim) {
            let declared = line.strip_prefix("pub ").unwrap_or(line);
            if let Some(name) = declared
                .strip_prefix("mod ")
                .and_then(|rest| rest.strip_suffix(';'))
                && !test_only
            {
                names.push(name);
            }
            test_only = line == "#[cfg(test)]";
        }
        names
    }

    #[test]
    fn architecture_page_describes_every_module() {
        let page = include_str!("../docs/architecture.qd");
        let top_level = modules(include_str!("lib.rs"));
        let interface = modules(include_str!("native_ui_plugin.rs"));
        assert!(top_level.contains(&"board") && !top_level.contains(&"test_support"));
        assert!(interface.contains(&"dialogs"));

        for name in top_level.into_iter().chain(interface) {
            assert!(
                page.contains(&format!("- {name}.rs\n")),
                "add {name}.rs to the file tree in docs/architecture.qd"
            );
            assert!(
                page.contains(&format!("| `{name}` ")),
                "add `{name}` to a module table in docs/architecture.qd"
            );
        }
    }

    #[test]
    fn architecture_docs_name_every_plugin() {
        let plugins: Vec<&str> = include_str!("main.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("use gol_bevy::")?.strip_suffix("Plugin;"))
            .filter_map(|path| path.rsplit("::").next())
            .collect();
        assert!(plugins.contains(&"Simulation"));

        for (file, text) in [
            (
                "docs/architecture.qd",
                include_str!("../docs/architecture.qd"),
            ),
            ("README.md", include_str!("../README.md")),
        ] {
            for plugin in &plugins {
                assert!(
                    text.contains(&format!("{plugin}Plugin")),
                    "add {plugin}Plugin to the architecture in {file}"
                );
            }
        }
    }
}
