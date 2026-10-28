//! The internal browser's two settings pages: the application's (search engine, the common
//! profile) and the project's (history, a private profile). Registered the way task sync is —
//! contributions to the settings container, with neither container module touched.

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{Icon, IconName};

use ubiq_proto::ids::ProjectId;

use crate::app::AppState;
use crate::ext::settings::{
    SectionCtx, SectionGate, SettingsContainer, SettingsSectionSpec, Tier, register,
};
use crate::ext::{Registry, ids};
use crate::state::MenuId;
use crate::state::browser::{Profile, SearchEngine};
use crate::ui::kit::{Picker, check_box, column, ghost_button, heading, setting_row};
use crate::ui::{handler, indexed};

pub fn settings_sections(reg: &mut Registry<SettingsSectionSpec>) {
    register(
        reg,
        SettingsSectionSpec {
            id: ids::BROWSER_APP,
            container: SettingsContainer::App,
            group: ids::SETTINGS_APP_INTERFACE,
            label: "Browser",
            icon: || Icon::new(IconName::Globe),
            gate: SectionGate::Always,
            tier: Tier::Stable,
            keywords: &[
                "browser",
                "search engine",
                "duckduckgo",
                "google",
                "cookies",
                "profile",
                "web",
            ],
            count: None,
            on_show: None,
            render: app_page,
        },
    );
    register(
        reg,
        SettingsSectionSpec {
            id: ids::PROJECT_BROWSER,
            container: SettingsContainer::Project,
            group: ids::SETTINGS_PROJECT_CONTENT,
            label: "Browser",
            icon: || Icon::new(IconName::Globe),
            gate: SectionGate::WithRecord,
            tier: Tier::Stable,
            keywords: &[
                "browser",
                "history",
                "private profile",
                "cookies",
                "clear history",
                "web",
            ],
            count: None,
            on_show: None,
            render: project_page,
        },
    );
}

fn app_page(ctx: &SectionCtx<'_>, _: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let app = ctx.app;
    let view = cx.entity();
    let engine = app.browser_engine();
    column(vec![
        heading(
            "Browser",
            "The internal browser: a tab in the dock that opens a page. Words typed into its \
             address field that are not an address are searched for.",
        ),
        setting_row(
            "Search engine",
            "Where the address field sends a search.",
            Picker::new("app-settings-browser-engine", engine.label())
                .icon(IconName::Search)
                .items(SearchEngine::ALL.map(|e| e.label()))
                .selected(
                    SearchEngine::ALL
                        .iter()
                        .position(|e| *e == engine)
                        .unwrap_or(0),
                )
                .open(app.workbench.open_menu == Some(MenuId::BrowserEngine))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_menu(MenuId::BrowserEngine, cx)
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.close_menu(cx);
                    if let Some(engine) = SearchEngine::ALL.get(index) {
                        this.set_browser_engine(*engine, cx);
                    }
                }))
                .into_any_element(),
        ),
        setting_row(
            "Persist profile",
            "Keep the common profile on disk. Off, its tabs forget cookies and storage when they \
             close. Tabs already open keep what they were opened with until they are reopened.",
            check_box(
                "app-settings-browser-persist",
                app.workbench.settings.ui.browser_persist,
                cx.listener(|this, _, _, cx| {
                    let now = this.workbench.settings.ui.browser_persist;
                    this.set_browser_persist_common(!now, cx)
                }),
            )
            .into_any_element(),
        ),
        setting_row(
            "Common profile",
            "The cookies, storage and cache every project's browser shares unless it has a \
             private profile. Tabs already open keep what they loaded until they are reopened.",
            ghost_button(
                "app-settings-browser-reset",
                None,
                "Reset profile",
                cx.listener(|this, _, _, cx| this.reset_browser_profile(Profile::Common, cx)),
            )
            .into_any_element(),
        ),
    ])
}

fn project_page(ctx: &SectionCtx<'_>, _: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(project) = ctx.project else {
        return column(vec![heading(
            "Browser",
            "Save the project first: its browser settings belong to it.",
        )]);
    };
    let private = ctx.app.browser_private(project);
    let reset = ghost_button(
        "project-settings-browser-reset",
        None,
        "Reset profile",
        cx.listener(move |this, _, _, cx| {
            if this.browser_private(project) {
                this.reset_browser_profile(Profile::Private(project), cx);
            }
        }),
    );
    column(vec![
        heading(
            "Browser",
            "How this project's internal browser tabs behave.",
        ),
        setting_row(
            "History",
            "The pages the address field offers again.",
            ghost_button(
                "project-settings-browser-history",
                None,
                "Clear history",
                cx.listener(move |this, _, _, cx| this.clear_browser_history(project, cx)),
            )
            .into_any_element(),
        ),
        setting_row(
            "Private profile",
            "Keep this project's cookies, storage and cache apart from the common profile. \
             Tabs already open keep the profile they were opened on until they are reopened.",
            check_box(
                "project-settings-browser-private",
                private,
                cx.listener(move |this, _, _, cx| {
                    let now = this.browser_private(project);
                    this.set_browser_private(project, !now, cx)
                }),
            )
            .into_any_element(),
        ),
        persist_row(ctx, private, project, cx),
        setting_row(
            "Private profile data",
            "Clears this project's private cookies, storage and cache. Only available while the \
             profile is private.",
            div()
                .when(!private, |d| d.opacity(0.4))
                .child(reset)
                .into_any_element(),
        ),
    ])
}

/// The private profile's "Persist profile" flag: drawn only while the profile is private.
fn persist_row(
    ctx: &SectionCtx<'_>,
    private: bool,
    project: ProjectId,
    cx: &mut Context<AppState>,
) -> AnyElement {
    if !private {
        return div().into_any_element();
    }
    setting_row(
        "Persist profile",
        "Keep the private profile on disk. Off, its tabs forget cookies and storage when they \
         close.",
        check_box(
            "project-settings-browser-persist",
            ctx.app.browser_persist(project),
            cx.listener(move |this, _, _, cx| {
                let now = this.browser_persist(project);
                this.set_browser_persist_private(project, !now, cx)
            }),
        )
        .into_any_element(),
    )
}
