//! Project settings, composed as the dialog a real project will raise.
//!
//! **It is drawn on the page, not over it.** The kit's modal is one question at `MODAL_WIDTH`;
//! this layout is a form with a nav, and looking at it means seeing the whole of it. The shape
//! is the same shape: square, `surface_raised`, a coloured left edge. Cancel puts the fixture
//! back; nothing is written, because the sink has no project behind it.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Bounds, Context, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div,
    fill, point, px, relative, size,
};
use gpui_component::IconName;
use gpui_component::input::{Input, Textarea};

use crate::app::AppState;
use crate::state::sink::{
    PROJECT_ABOUT, PROJECT_ABOUT_LIMIT, PROJECT_BRANCH, PROJECT_COLOUR, PROJECT_MARK, PROJECT_NAME,
    PROJECT_PATH, ProjectNav, hex_string, hsv_to_rgb,
};
use crate::theme;
use crate::ui::kit::{elided, ghost_button, icon_button, mono, primary_button};
use crate::ui::sink::settings::{heading, nav_item, setting_row};
use crate::ui::sink::style::{framed_active, input_on, textarea_on};

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id("sink-project")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .bg(theme::app_bg())
        .p_6()
        .child(dialog(app, window, cx))
        .into_any_element()
}

fn dialog(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let colour = current_rgba(app);

    div()
        .id("sink-project-dialog")
        .w(px(820.))
        .max_h(relative(1.))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(colour)
        .shadow_lg()
        .child(header(colour))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(nav(app, cx))
                .child(body(app, window, cx)),
        )
        .child(footer(app, cx))
        .into_any_element()
}

fn header(colour: gpui::Rgba) -> AnyElement {
    div()
        .h(px(52.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(28.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .bg(colour)
                .child(
                    mono(PROJECT_MARK, theme::on_accent())
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child("Project settings"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            elided("sink-project-path", PROJECT_PATH, theme::text_faint(), 11.0)
                                .flex_none(),
                        )
                        .child(mono("·", theme::text_faint()).text_size(px(11.)))
                        .child(mono(PROJECT_BRANCH, theme::text_faint()).text_size(px(11.))),
                ),
        )
        .child(icon_button(
            "sink-project-close",
            IconName::Close,
            false,
            |_, _, _| {},
        ))
        .into_any_element()
}

fn nav(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.sink.project.nav;
    let items: Vec<AnyElement> = ProjectNav::all()
        .iter()
        .copied()
        .map(|item| {
            nav_item(
                ElementId::Name(format!("sink-project-nav-{}", item.label()).into()),
                project_icon(item),
                item.label(),
                item.count().map(|n| n as usize),
                item == current,
                cx.listener(move |this, _, _, cx| this.set_sink_project_nav(item, cx)),
            )
        })
        .collect();

    div()
        .id("sink-project-nav")
        .w(px(200.))
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .px_2()
        .py_3()
        .border_r_1()
        .border_color(theme::border())
        .children(items)
        .into_any_element()
}

fn project_icon(item: ProjectNav) -> IconName {
    match item {
        ProjectNav::General => IconName::Settings,
        ProjectNav::Documentation => IconName::BookOpen,
        ProjectNav::Integrations => IconName::Network,
    }
}

fn body(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.sink.project.nav {
        ProjectNav::General => general(app, window, cx),
        ProjectNav::Documentation => documentation(),
        ProjectNav::Integrations => integrations(),
    };

    div()
        .id("sink-project-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_5()
        .py_4()
        .child(content)
        .into_any_element()
}

fn general(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let colour = app.sink.project.colour;
    let custom = app.sink.project.custom;
    let about = app.sink_project_about.read(cx).value();
    let used = about.chars().count();
    let current = current_rgba(app);

    let swatches: Vec<AnyElement> = (0..theme::project_colour_count())
        .map(|index| {
            let mut swatch = div()
                .id(ElementId::Name(
                    format!("sink-project-swatch-{index}").into(),
                ))
                .size(px(22.))
                .flex_none()
                .cursor_pointer()
                .bg(theme::project_colour(index));
            if custom.is_none() && index == colour {
                swatch = swatch.border_2().border_color(theme::text());
            }
            swatch
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_sink_project_colour(index, window, cx)
                }))
                .into_any_element()
        })
        .collect();

    let label = match custom {
        Some(rgb) => hex_string(rgb),
        None => colour_name(colour).to_string(),
    };

    let mut colour_block = div()
        .flex()
        .flex_col()
        .gap_1p5()
        .py_3()
        .border_b_1()
        .border_color(theme::border())
        .child(label_line(
            "Color",
            "Tints the title chip, the active rails and this project's icon, so windows \
             stay distinguishable when several projects are open.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(swatches)
                .child(icon_button(
                    "sink-project-custom-colour",
                    IconName::Palette,
                    app.sink.project.picker_open || custom.is_some(),
                    cx.listener(|this, _, window, cx| this.toggle_sink_colour_picker(window, cx)),
                ))
                .child(
                    div()
                        .size(px(22.))
                        .flex_none()
                        .bg(current)
                        .border_1()
                        .border_color(theme::border()),
                )
                .child(mono(label, theme::text_muted()).text_size(px(11.))),
        );
    if app.sink.project.picker_open {
        colour_block = colour_block.child(colour_picker(app, window, cx));
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(label_line(
                    "Project name",
                    "Shown in the title chip, the project switcher and every agent prompt.",
                ))
                .child(
                    framed_active(
                        theme::border(),
                        input_on(&app.sink_project_name, window, cx),
                    )
                    .h(px(30.))
                    .items_center()
                    .child(Input::new(&app.sink_project_name).appearance(false)),
                ),
        )
        .child(colour_block)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(label_line(
                            "Description",
                            "Given to every agent as project context. Two lines about what this \
                             codebase is and what matters in it beat a paragraph of history.",
                        ))
                        .child(
                            mono(format!("{used}/{PROJECT_ABOUT_LIMIT}"), theme::text_faint())
                                .text_size(px(11.)),
                        ),
                )
                .child(
                    framed_active(
                        theme::border(),
                        textarea_on(&app.sink_project_about, window, cx),
                    )
                    .p_2()
                    .child(
                        Textarea::new(&app.sink_project_about)
                            .appearance(false)
                            .bordered(false)
                            .w_full()
                            .text_size(px(13.)),
                    ),
                ),
        )
        .child(setting_row(
            "Repository path",
            "Set when the project was opened. Move it from the project switcher.",
            mono(PROJECT_PATH, theme::text())
                .text_size(px(12.5))
                .into_any_element(),
        ))
        .into_any_element()
}

const SV_COLS: usize = 16;
const SV_ROWS: usize = 10;
const HUE_STEPS: usize = 24;
const CELL: f32 = 12.0;

fn colour_picker(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let hue = app.sink.project.hue;
    let sat = app.sink.project.sat;
    let val = app.sink.project.val;
    let current = current_rgba(app);

    let rows: Vec<AnyElement> = (0..SV_ROWS)
        .map(|row| {
            let cells: Vec<AnyElement> = (0..SV_COLS)
                .map(|col| {
                    let s = col as f32 / (SV_COLS - 1) as f32;
                    let v = 1.0 - row as f32 / (SV_ROWS - 1) as f32;
                    div()
                        .id(ElementId::Name(format!("sink-sv-{col}-{row}").into()))
                        .size(px(CELL))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let hue = this.sink.project.hue;
                            this.set_sink_project_hsv(hue, s, v, window, cx)
                        }))
                        .into_any_element()
                })
                .collect();
            div().flex().flex_none().children(cells).into_any_element()
        })
        .collect();

    let hues: Vec<AnyElement> = (0..HUE_STEPS)
        .map(|step| {
            let h = step as f32 / (HUE_STEPS - 1) as f32;
            div()
                .id(ElementId::Name(format!("sink-hue-{step}").into()))
                .w(px(CELL))
                .h(px(14.))
                .flex_none()
                .cursor_pointer()
                .bg(rgba_of(hsv_to_rgb(h, 1.0, 1.0)))
                .when((h - hue).abs() < 0.5 / HUE_STEPS as f32, |this| {
                    this.border_1().border_color(theme::text())
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    let sat = this.sink.project.sat;
                    let val = this.sink.project.val;
                    this.set_sink_project_hsv(h, sat, val, window, cx)
                }))
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_1()
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    div()
                        .relative()
                        .w(px(CELL * SV_COLS as f32))
                        .h(px(CELL * SV_ROWS as f32))
                        .child(sv_plane(hue))
                        .child(div().absolute().inset_0().flex().flex_col().children(rows))
                        .child(sv_mark(sat, val)),
                )
                .child(
                    div()
                        .w(px(36.))
                        .h(px(CELL * SV_ROWS as f32))
                        .flex_none()
                        .bg(current)
                        .border_1()
                        .border_color(theme::border()),
                ),
        )
        .child(div().flex().children(hues))
        .child(
            framed_active(theme::border(), input_on(&app.sink_project_hex, window, cx))
                .w(px(140.))
                .h(px(30.))
                .items_center()
                .child(Input::new(&app.sink_project_hex).appearance(false)),
        )
        .into_any_element()
}

fn sv_plane(hue: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            let w = f32::from(bounds.size.width).max(1.0);
            let h = f32::from(bounds.size.height).max(1.0);
            let step = 4.0;
            let mut y = 0.0;
            while y < h {
                let mut x = 0.0;
                while x < w {
                    let sat = (x / w).clamp(0.0, 1.0);
                    let val = 1.0 - (y / h).clamp(0.0, 1.0);
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin + point(px(x), px(y)),
                            size(px(step), px(step)),
                        ),
                        rgba_of(hsv_to_rgb(hue, sat, val)),
                    ));
                    x += step;
                }
                y += step;
            }
        },
    )
    .size_full()
}

fn sv_mark(sat: f32, val: f32) -> impl IntoElement {
    div()
        .absolute()
        .left(px((sat * (SV_COLS as f32 - 1.0) * CELL).round()))
        .top(px(((1.0 - val) * (SV_ROWS as f32 - 1.0) * CELL).round()))
        .size(px(CELL))
        .border_1()
        .border_color(theme::text())
}

fn current_rgba(app: &AppState) -> Rgba {
    match app.sink.project.custom {
        Some(rgb) => rgba_of(rgb),
        None => theme::project_colour(app.sink.project.colour),
    }
}

fn rgba_of(rgb: u32) -> Rgba {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

fn documentation() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(
            "Documentation",
            "The files a new agent is handed as the project's own briefing.",
        ))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child("Four documents in the fixture. A real project lists what it indexed."),
        )
        .into_any_element()
}

fn integrations() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(
            "Integrations",
            "MCP servers and accounts this project adds on top of the application defaults.",
        ))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child("One integration in the fixture. Wiring it is the host's."),
        )
        .into_any_element()
}

fn footer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let name = app.sink_project_name.read(cx).value();
    let about = app.sink_project_about.read(cx).value();
    let dirty = name.as_ref() != PROJECT_NAME
        || about.as_ref() != PROJECT_ABOUT
        || app.sink.project.colour != PROJECT_COLOUR
        || app.sink.project.custom.is_some();
    let status = if dirty {
        "Unsaved changes"
    } else {
        "No unsaved changes"
    };

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::text_faint())
                .child(SharedString::from(status)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "sink-project-cancel",
                    None,
                    "Cancel",
                    cx.listener(|this, _, window, cx| this.reset_sink_project(window, cx)),
                ))
                .child(primary_button(
                    "sink-project-save",
                    None,
                    "Save changes",
                    |_, _, _| {},
                )),
        )
        .into_any_element()
}

fn label_line(label: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.5))
                        .text_color(theme::text())
                        .child(SharedString::from(label.to_string())),
                )
                .when(label == "Description", |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::text_faint())
                            .child("optional"),
                    )
                }),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

fn colour_name(index: usize) -> &'static str {
    match index {
        0 => "blue",
        1 => "violet",
        2 => "teal",
        3 => "gold",
        4 => "rose",
        5 => "green",
        _ => "custom",
    }
}
