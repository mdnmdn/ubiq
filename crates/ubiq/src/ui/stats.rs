//! The Control screen: what this Ubiq is doing, and what its agents have spent.
//!
//! **It answers whether or not a project is open**, like the sink beside it in the rail, and for a
//! different reason: the host is one process behind every window, so its memory, its uptime and
//! the projects it holds open are facts about the application rather than about a folder.
//!
//! Two pages, and the split is the honest one. General is five readings the host takes when it is
//! asked — every one of them true today. AI is the usage meter, and **nothing writes to it yet**:
//! the tables exist, the agents engine does not report what it spends, so the page says so instead
//! of drawing a plausible number. The table under that page is written and simply not reached: the day
//! rows arrive it draws them, and until then a fixture would be a lie on a real screen.
//!
//! Everything here is drawn from [`crate::state::stats::StatsState`], which the window refreshes on
//! a timer while this screen is up — see `AppState::poll_stats`. Nothing on this screen asks the
//! host for anything itself.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::IconName;
use ubiq_proto::stats::UsageRow;

use crate::app::AppState;
use crate::state::stats::StatsTab;
use crate::theme;
use crate::ui::kit::panel::tab_strip;
use crate::ui::kit::{Tab, badge, mono, section_label, slab, state_chip};
use crate::ui::{empty, indexed};

/// What a figure reads as before the first answer has arrived. An em dash rather than a zero: the
/// host has not said, and a host that has not said is not a host using nothing.
const UNKNOWN: &str = "\u{2014}";

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let tab = app.stats.tab;
    let active = StatsTab::all()
        .iter()
        .position(|candidate| *candidate == tab)
        .unwrap_or(0);

    let tabs: Vec<Tab> = StatsTab::all()
        .iter()
        .map(|tab| Tab::new(tab.label()))
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(tab_strip(
            "stats-tabs",
            tabs,
            active,
            indexed(&view, |this, index, _, cx| {
                let tab = StatsTab::all()
                    .get(index)
                    .copied()
                    .unwrap_or(StatsTab::General);
                this.set_stats_tab(tab, cx);
            }),
            None,
            Some(
                mono(tab.note(), theme::text_faint())
                    .text_size(px(11.))
                    .into_any_element(),
            ),
        ))
        .child(match tab {
            StatsTab::General => general(app),
            StatsTab::Ai => ai(app),
        })
        .into_any_element()
}

// ── General ─────────────────────────────────────────────────────────

/// The five readings, as label-and-value rows on one surface.
///
/// **No meter anywhere on this page.** A bar needs a denominator and nobody can name a memory
/// ceiling for a host that spawns pseudo-terminals on demand — a fraction of an invented total is
/// an invented fact, drawn with more confidence than the number behind it deserves. A figure that
/// only a number can honestly carry gets a number.
fn general(app: &AppState) -> AnyElement {
    let host = app.stats.host.as_ref();

    let memory = match host {
        // `None` inside a reading is the platform declining to say, which is a different answer
        // from not having asked yet — and neither of them is zero.
        Some(stats) => stats
            .rss_bytes
            .map(humanise)
            .unwrap_or_else(|| "unavailable".to_string()),
        None => UNKNOWN.to_string(),
    };
    let uptime = host
        .map(|stats| uptime(stats.uptime_secs))
        .unwrap_or_else(|| UNKNOWN.to_string());
    let projects = count(host.map(|stats| stats.open_projects));
    let this_run = count(host.map(|stats| stats.agents_this_run));

    let live: AnyElement = match host {
        Some(stats) => badge(&stats.agents_live.to_string(), theme::accent()).into_any_element(),
        None => mono(UNKNOWN, theme::text_faint()).into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .p_4()
        .child(
            slab(theme::border())
                .p_3()
                .gap_2()
                .child(reading(
                    "Memory",
                    mono(memory, theme::text()).into_any_element(),
                ))
                .child(reading(
                    "Uptime",
                    mono(uptime, theme::text()).into_any_element(),
                ))
                .child(reading(
                    "Open projects",
                    mono(projects, theme::text()).into_any_element(),
                ))
                .child(reading("Agents live", live))
                .child(reading(
                    "Agents this run",
                    mono(this_run, theme::text()).into_any_element(),
                )),
        )
        .into_any_element()
}

/// One reading: what it is on the left, what it says on the right.
fn reading(label: &str, value: AnyElement) -> impl IntoElement {
    div()
        .h(px(24.))
        .flex()
        .items_center()
        .gap_3()
        .child(section_label(label))
        .child(div().flex_1().min_w(px(0.)))
        .child(value)
}

/// A count the host has answered with, or the em dash for one it has not.
fn count(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| UNKNOWN.to_string())
}

/// A byte count as a human reads one. Decimal units, because that is what a process monitor shows
/// and this figure is read beside one.
fn humanise(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "kB")];
    for (scale, unit) in UNITS {
        if bytes >= scale {
            let whole = bytes / scale;
            // One decimal below ten, where the difference between 1.2 and 1.9 is the whole story;
            // none above it, where it is noise.
            return match whole < 10 {
                true => format!("{:.1} {unit}", bytes as f64 / scale as f64),
                false => format!("{whole} {unit}"),
            };
        }
    }
    format!("{bytes} B")
}

/// How long the coordinator has been up, at the coarsest scale that still says something: hours
/// and minutes once there is an hour, minutes and seconds before that.
fn uptime(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    match hours > 0 {
        true => format!("{hours}h {minutes:02}m"),
        false => format!("{minutes:02}m {seconds:02}s"),
    }
}

// ── AI ──────────────────────────────────────────────────────────────

/// What the agents have spent, hour by hour — once anything is recorded.
///
/// The empty page is the honest state and it is today's every time: the database and its tables
/// exist, and the agents engine does not yet report what a turn cost, so the history the host
/// answers with is empty on every reply. Nothing is seeded to make the table look inhabited.
fn ai(app: &AppState) -> AnyElement {
    let rows = app
        .stats
        .host
        .as_ref()
        .map(|stats| stats.history.as_slice())
        .unwrap_or_default();

    if rows.is_empty() {
        return empty::empty_page(
            "Token usage",
            "Nothing recorded. The database and its tables exist; the agents engine does not \
             report what it spends yet, so there is nothing to aggregate.",
            IconName::Cpu,
            Some(empty::not_built()),
        )
        .into_any_element();
    }

    table(rows)
}

/// The columns, in the order they are read: when, who, then what it cost.
///
/// Widths are fixed rather than proportional, because a table of figures is read down a column and
/// a column that resizes with its longest cell stops lining up. What does not fit scrolls
/// sideways, so a narrow window makes the table reachable rather than making the page wide.
const COLUMNS: [(&str, f32); 11] = [
    ("Hour", 72.),
    ("Harness", 90.),
    ("Model", 130.),
    ("In", 80.),
    ("Out", 80.),
    ("Think", 80.),
    ("Other", 80.),
    ("Total", 88.),
    ("Msgs in", 72.),
    ("Msgs out", 72.),
    ("Tools", 64.),
];

fn table(rows: &[UsageRow]) -> AnyElement {
    let header = div()
        .h(px(24.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .children(
            COLUMNS
                .iter()
                .map(|(label, width)| div().w(px(*width)).child(section_label(label))),
        );

    let body: Vec<AnyElement> = rows.iter().map(row).collect();

    div()
        .id("stats-usage")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .p_4()
        .gap_1()
        .overflow_x_scroll()
        .overflow_y_scroll()
        .child(header)
        .children(body)
        .into_any_element()
}

/// One bucket. The harness is a badge and the model a chip, because those two are what the eye
/// picks a row out by; everything else is a figure and reads as one.
fn row(usage: &UsageRow) -> AnyElement {
    let cell = |value: u64, width: f32| {
        div()
            .w(px(width))
            .child(mono(value.to_string(), theme::text()))
    };

    slab(theme::border())
        .h(px(28.))
        .px_3()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(COLUMNS[0].1))
                .child(mono(at_hour(usage.bucket), theme::text_muted())),
        )
        .child(
            div()
                .w(px(COLUMNS[1].1))
                .child(badge(&usage.harness, theme::accent())),
        )
        .child(div().w(px(COLUMNS[2].1)).child(state_chip(
            SharedString::from(match usage.model.is_empty() {
                // The harness did not say which model answered, and an empty chip would read as a
                // model with no name rather than as an answer nobody gave.
                true => "unknown".to_string(),
                false => usage.model.clone(),
            }),
            theme::text_faint(),
            1.0,
        )))
        .child(cell(usage.tokens_in, COLUMNS[3].1))
        .child(cell(usage.tokens_out, COLUMNS[4].1))
        .child(cell(usage.tokens_think, COLUMNS[5].1))
        .child(cell(usage.tokens_other, COLUMNS[6].1))
        .child(cell(usage.tokens_total(), COLUMNS[7].1))
        .child(cell(usage.msgs_in, COLUMNS[8].1))
        .child(cell(usage.msgs_out, COLUMNS[9].1))
        .child(cell(usage.tool_calls, COLUMNS[10].1))
        .into_any_element()
}

/// The bucket's hour, in the reader's own zone. The host stores UTC and has no opinion about how
/// an hour is written; this is that opinion, in the one place that draws it.
fn at_hour(bucket: i64) -> String {
    chrono::DateTime::from_timestamp(bucket, 0)
        .map(|at| {
            chrono::DateTime::<chrono::Local>::from(at)
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_else(|| UNKNOWN.to_string())
}
