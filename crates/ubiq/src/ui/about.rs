//! The About modal: the mark and the version, every release channel, and the open-source packages
//! this build is made of — three tabs in one fixed-size panel, raised from the titlebar's mark.
//!
//! Nothing here acts on its own: the update buttons are the Updates section's messages, the release
//! list is the host's answer to `QueryReleases`, and the package list is the manifest
//! `state::about` bakes in.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Image, ImageFormat, ImageSource, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, Styled, Window, div, img, px, uniform_list,
};
use ubiq_proto::update::{ApplyMode, ChannelRelease, UpdateChannel, UpdateStatus};

use crate::app::AppState;
use crate::state::Layer;
use crate::state::about::{AboutTab, ThirdParty, third_party};
use crate::theme::{self, Family, Role};
use crate::ui::kit::{
    Tab, choice_pill, elided, ghost_button, modal_sized, mono, primary_button, tab_strip,
};
use crate::version;

const ABOUT_WIDTH: f32 = 600.0;
/// Fixed, so switching tabs never resizes the panel.
const ABOUT_HEIGHT: f32 = 520.0;
const LOGO_SIZE: f32 = 96.0;
/// One package row in the licence list — a uniform list needs every row the same height.
const PACKAGE_ROW: f32 = 44.0;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(about) = app.workbench.about.as_ref() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let active = AboutTab::ALL
        .iter()
        .position(|tab| *tab == about.tab)
        .unwrap_or(0);

    let strip = tab_strip(
        "about-tabs",
        AboutTab::ALL
            .iter()
            .map(|tab| Tab::new(tab.label()))
            .collect(),
        active,
        {
            let view = view.clone();
            move |ix, _, cx| {
                if let Some(tab) = AboutTab::ALL.get(ix).copied() {
                    view.update(cx, |this, cx| this.set_about_tab(tab, cx));
                }
            }
        },
        None,
        None,
    );

    let page = match about.tab {
        AboutTab::About => about_page(app, cx),
        AboutTab::Releases => releases_page(app, about.releases.as_deref(), cx),
        AboutTab::Licences => licences_page(),
    };

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(div().flex().flex_none().child(strip))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .pt_3()
                .child(page),
        )
        .into_any_element();

    modal_sized(
        "about-modal",
        theme::accent(),
        ABOUT_WIDTH,
        Some(ABOUT_HEIGHT),
        "About Ubiq",
        None,
        body,
        div().into_any_element(),
        crate::ui::dismiss(&view, Layer::About, |this, _, cx| this.close_about(cx)),
        window,
    )
}

/// A line of muted chrome text.
fn note(text: impl Into<gpui::SharedString>, colour: gpui::Rgba) -> impl IntoElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(colour)
        .child(text.into())
}

fn link(id: &'static str, label: &str, url: &'static str) -> AnyElement {
    ghost_button(id, None, label.to_string(), move |_, _, cx| {
        cx.open_url(url)
    })
    .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(url).build(window, cx))
    .into_any_element()
}

/// The status line's colour — the same reading the Updates section gives it.
fn status_colour(status: &UpdateStatus) -> gpui::Rgba {
    match status {
        UpdateStatus::Failed { .. } => theme::danger(),
        UpdateStatus::Disabled { .. } => theme::warning(),
        UpdateStatus::Available { .. } | UpdateStatus::Ready { .. } => theme::accent(),
        UpdateStatus::UpToDate { .. } => theme::success(),
        _ => theme::text_faint(),
    }
}

fn about_page(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let state = &app.workbench.updates;
    let running = if state.current_version.is_empty() {
        version::FULL.to_string()
    } else {
        state.current_version.clone()
    };
    let logo = Arc::new(Image::from_bytes(
        ImageFormat::Png,
        (if theme::mark_dark(theme::surface_raised()) {
            crate::ui::rail::LOGO_WHITE
        } else {
            crate::ui::rail::LOGO_BLUE
        })
        .to_vec(),
    ));
    let busy = matches!(
        state.status,
        UpdateStatus::Checking | UpdateStatus::Downloading { .. } | UpdateStatus::Applying { .. }
    );
    let live = state.enabled() && !busy;

    let check = ghost_button(
        "about-check",
        None,
        "Check for updates",
        cx.listener(|this, _, _, cx| {
            if this.workbench.updates.enabled() {
                this.check_for_updates(cx);
            }
        }),
    );
    let check = if live { check } else { check.opacity(0.5) };

    let update_label = match &state.status {
        UpdateStatus::Available {
            apply: ApplyMode::Manual,
            ..
        } => Some("Open download page"),
        UpdateStatus::Available { .. } => Some("Update"),
        UpdateStatus::Ready { .. } => Some("Restart and update"),
        _ => None,
    };

    let mut buttons = div().flex().items_center().gap_2().child(check);
    if let Some(label) = update_label {
        buttons = buttons.child(primary_button(
            "about-update",
            None,
            label,
            cx.listener(|this, _, _, cx| this.take_update(cx)),
        ));
    }

    // What the update is, when there is one: its version, its date and its notes.
    let offer = state.info().map(|info| {
        let url = info.notes_url.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(note(
                format!(
                    "{} {} is available \u{2014} published {}",
                    info.channel.label(),
                    info.version,
                    info.published
                ),
                theme::accent(),
            ))
            .children((!url.is_empty()).then(|| {
                ghost_button("about-notes", None, "Release notes", move |_, _, cx| {
                    cx.open_url(&url)
                })
            }))
    });

    div()
        .id("about-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .items_center()
        .gap_3()
        .pt_4()
        .child(img(ImageSource::Image(logo)).size(px(LOGO_SIZE)))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Title))
                .text_color(theme::text())
                .child("Ubiq"),
        )
        .child(note("The agentic workspace", theme::text_muted()))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(note("Version", theme::text_muted()))
                .child(mono(running, theme::text()))
                .child(note(
                    format!("\u{00b7} {}", state.settings.channel.label()),
                    theme::text_faint(),
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(link("about-site", "Website", version::SITE))
                .child(link("about-repo", "Source code", version::REPO)),
        )
        .child(note(state.line(), status_colour(&state.status)))
        .children(offer)
        .child(buttons)
        .into_any_element()
}

fn releases_page(
    app: &AppState,
    releases: Option<&[ChannelRelease]>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let state = &app.workbench.updates;
    let live = state.enabled();
    let following = state.settings.channel;
    let built = UpdateChannel::build();

    let rows: Vec<AnyElement> = match releases {
        None => {
            vec![note("Asking the release feed\u{2026}", theme::text_faint()).into_any_element()]
        }
        Some(channels) => channels
            .iter()
            .map(|entry| {
                channel_row(
                    entry,
                    entry.channel == following,
                    entry.channel == built,
                    live,
                    cx,
                )
            })
            .collect(),
    };

    div()
        .id("about-releases")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .gap_3()
        .child(note(
            "Each channel's newest release for this platform. Following a channel offers its \
             releases and those of every steadier one.",
            theme::text_muted(),
        ))
        .children(rows)
        .child(div().flex().child(ghost_button(
            "about-releases-refresh",
            None,
            "Refresh",
            cx.listener(|this, _, _, cx| this.refresh_releases(cx)),
        )))
        .into_any_element()
}

fn channel_row(
    entry: &ChannelRelease,
    following: bool,
    built: bool,
    live: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let channel = entry.channel;
    let edge = if following {
        theme::accent()
    } else {
        theme::border()
    };

    let headline = match (&entry.release, &entry.error) {
        (_, Some(error)) => note(error.clone(), theme::danger()).into_any_element(),
        (Some(info), None) => div()
            .flex()
            .items_center()
            .gap_2()
            .child(mono(info.version.clone(), theme::text()))
            .child(note(
                format!("published {}", info.published),
                theme::text_faint(),
            ))
            .into_any_element(),
        (None, None) => {
            note("Nothing published for this platform", theme::text_faint()).into_any_element()
        }
    };

    let notes = entry
        .release
        .as_ref()
        .filter(|info| !info.notes_url.is_empty())
        .map(|info| {
            let url = info.notes_url.clone();
            ghost_button(
                gpui::ElementId::Name(format!("about-notes-{}", channel.slug()).into()),
                None,
                "Notes",
                move |_, _, cx| cx.open_url(&url),
            )
        });

    let follow = choice_pill(
        gpui::ElementId::Name(format!("about-follow-{}", channel.slug()).into()),
        if following { "Following" } else { "Follow" },
        following,
        cx.listener(move |this, _, _, cx| {
            if this.workbench.updates.enabled() {
                this.set_update_channel(channel, cx);
            }
        }),
    );
    let follow = if live { follow } else { follow.opacity(0.5) };

    div()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .bg(theme::pane_bg())
        .border_l(px(theme::accent_edge()))
        .border_color(edge)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(theme::font(Family::Chrome, Role::Body))
                                .text_color(theme::text())
                                .child(channel.label()),
                        )
                        .children(built.then(|| note("this build's channel", theme::text_faint()))),
                )
                .child(headline),
        )
        .children(notes)
        .child(follow)
        .into_any_element()
}

fn licences_page() -> AnyElement {
    let packages = third_party();
    let count = packages.len();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_2()
        .child(note(
            format!(
                "Ubiq is built on {count} open-source packages. Each is used under its own \
                 licence \u{2014} click one to read it, or a name to visit its project."
            ),
            theme::text_muted(),
        ))
        .child(
            uniform_list("about-licences", count, move |range, _, _| {
                range
                    .filter_map(|ix| third_party().get(ix).map(|p| package_row(ix, p)))
                    .collect()
            })
            .flex_1()
            .min_h(px(0.)),
        )
        .into_any_element()
}

fn package_row(ix: usize, package: &ThirdParty) -> AnyElement {
    let size = theme::font(Family::Chrome, Role::Label);
    let small = theme::font(Family::Chrome, Role::Meta);

    let mut name = div()
        .id(("about-pkg-name", ix))
        .flex_none()
        .text_size(size)
        .text_color(theme::text())
        .child(format!("{} {}", package.name, package.version));
    if let Some(url) = package.project_link().map(str::to_string) {
        name = name
            .cursor_pointer()
            .hover(|this| this.text_color(theme::accent()))
            .on_click(move |_, _, cx| cx.open_url(&url));
    }

    let mut licence = div()
        .id(("about-pkg-licence", ix))
        .flex_none()
        .text_size(small)
        .text_color(theme::accent())
        .child(package.license_label().to_string());
    if let Some(url) = package.license_link().map(str::to_string) {
        licence = licence
            .cursor_pointer()
            .hover(|this| this.text_color(theme::text()))
            .on_click(move |_, _, cx| cx.open_url(&url));
    }

    let attribution = package.attribution();
    div()
        .h(px(PACKAGE_ROW))
        .flex()
        .flex_col()
        .justify_center()
        .gap_0p5()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(name)
                .child(div().flex_1())
                .child(licence),
        )
        .child(div().flex().min_w(px(0.)).child(elided(
            ("about-pkg-credit", ix),
            if attribution.is_empty() {
                "\u{2014}".to_string()
            } else {
                attribution
            },
            theme::text_faint(),
            small,
        )))
        .into_any_element()
}
