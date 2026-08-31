//! Window creation, theme install and key bindings. Everything else lives in the library.

use gpui::{App, KeyBinding, actions};
use gpui_platform::application;

use ubiq::app;
use ubiq::log;
use ubiq::theme;

actions!(ubiq, [Quit]);

fn main() {
    // Before the window, before the coordinator: anything either says on the way up belongs in
    // the console with everything else.
    log::install();

    application()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            // Sets both palettes at once: Ubiq's tokens and the component library's theme.
            theme::set_mode(app::boot_theme(), cx);

            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);

            app::open_project_window(0, cx);

            // A closed window leaves the registry, and the application ends with its last one —
            // not with any particular one.
            cx.on_window_closed(|cx, window_id| {
                app::window_closed(window_id, cx);
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            cx.activate(true);
        });
}
