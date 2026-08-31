use gpui::{
    App, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions, prelude::*, px,
    size,
};
use gpui_component::{Root, Theme as GpuiComponentTheme, ThemeMode};
use gpui_platform::application;

mod app;
mod orchestrator;
mod agent;
mod messages;
mod mcp_server;
mod pty;
mod state;
mod theme;
mod ui;

use app::AppState;
use theme::Theme;

actions!(ubiq, [Quit]);

fn main() {
    application()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            GpuiComponentTheme::change(ThemeMode::Dark, None, cx);
            Theme::set(theme::dark());

            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);

            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Ubiq - Agent Harness Multiplexer".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| AppState::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx).bg(theme::app_bg()))
                },
            )
            .unwrap();

            cx.on_window_closed(|cx, _window_id| {
                cx.quit();
            })
            .detach();

            cx.activate(true);
        });
}
