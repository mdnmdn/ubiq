---
id: feat-onboarding
title: Onboarding
kind: feature
status: draft
summary: The first-run wizard — a welcome with a palette choice and a highlighted tour of the main screen as one sub-flow step, a per-mode walk through the IDE — as locked dialogs and outlined callouts, its pages authored as hidden help pages, with completion remembered per flow in the interface preferences and a menu entry that restarts it.
read_when: you are adding or changing an onboarding flow or step, changing when a flow starts by itself, touching the locked dialog or the highlight callout, or deciding where a first-run message belongs
updated: 2026-10-27
verified: 2026-10-27
code_anchors: [crates/ubiq/src/state/onboarding.rs, crates/ubiq/src/app/onboarding.rs, crates/ubiq/src/ui/onboarding/mod.rs, crates/ubiq/src/ui/onboarding/flows.rs, crates/ubiq/tests/onboarding.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq/src/ui/kit/overlay.rs, help/onboarding/welcome.md, help/onboarding/tour.md, help/onboarding/done.md, help/onboarding/ide.md, help/onboarding/ide-done.md]
depends_on: [tech-ui, feat-workbench]
review_cycle: monthly
---

# Onboarding

## Purpose

Ubiq's window has many places — ten rail modes, a dock, a command field, a bell — and a person
opening it for the first time sees all of them at once. Onboarding is a short, skippable walk that
says what each place is and lets the user pick a palette on the way.

It is a wizard of **flows**. A flow is a named list of steps; a flow can chain another to follow it,
and the progress dots span the whole chain. A step can also stand for a whole other flow — a
**sub-flow** — which runs over it with its own steps and dots and hands back when it ends.

## Behaviour

**On launch**, once the interface preferences have arrived, the first window starts the first
launch flow that is not done. That is the **welcome**: four steps, four dots — a markdown welcome,
a palette picker, the "quick tour" step and a closing "You're set". Only the first window of a
process greets the user; a second window opened later does not.

**The tour is one step of the welcome.** Its dialog says what the tour is and offers **Start the
tour**; Next passes the step without running it. Started, the **tour** runs as a sub-flow: one
highlight for each of the rail, the project selector, the command field, the top-right actions,
the bell, the workspace and the status bar, with its own dots. Its last Next reads **Done**, its
Skip reads **Skip tour**, and Back on its first highlight returns to the tour step. Done and Skip
tour both return to the welcome and move past the tour step; only Done marks the tour done.

**A step is one of four things.**

- *Markdown*: a locked dialog with a title and a body.
- *Form*: the same dialog with a control in place of the body. The palette picker chooses the
  palette the window wears, and the choice applies at once.
- *Sub-flow*: the markdown dialog with a launch button under the body that starts another flow
  as a sub-run.
- *Highlight*: the dialog steps aside. The window stays interactive, the target place gets an
  accent outline over a translucent accent fill, ringed by a glow that pulses slowly, and a
  callout card beside it carries the title, the text and the same footer. A target that is not on
  screen leaves the card centred with nothing outlined.

**A markdown body fills the dialog** between the title and the footer, and scrolls inside it when
it is longer.

**The dialog has no way out but its footer.** No ×, no outside click, and Escape does nothing while
the dialog is up. The footer has **Skip** on the left, one dot per step of the whole run in the
middle (done, current, to do), and **Back** and **Next** on the right; the last step's Next reads
**Finish**. A step can hold either button inert: a button that cannot act is faded and does
nothing. During a highlight step Escape is the window's as usual.

**A flow is done when its last step is passed**, and that is remembered across launches. **Skip**
marks every flow of the run done, so none returns by itself — except inside a sub-flow, where it
skips only that sub-flow. Back never un-marks a flow.

**A mode flow starts when its mode is entered.** After every launch flow is done, the **IDE flow**
starts when the window is in IDE mode with a project loaded and its saved layout restored — at
launch if the window starts there, or on the first switch to IDE. It opens with a markdown
step, highlights the rail's modes, then walks every project mode the rail draws: each step switches
the window into that mode, and the closing step switches it back to IDE. Back replays the switch.

**By hand.** The titlebar's overflow menu has **Show onboarding**, which always restarts the
welcome, with its tour step, done or not.

## Contract

- A flow has an id, a title, a trigger (`Start`, `EnterMode(mode)` or `Manual`), an optional next
  flow and its steps. A run is a flow and everything chained after it, concatenated. A sub-run is
  a run opened from a sub-flow step, holding the run it was opened from as its parent.
- A step names its body, a gate for Back and one for Next (`On`, `Off` or live only while a project
  is open), and an optional action on arrival (enter a rail mode).
- Which flows are done is a set of flow ids in the interface preferences. No transport message
  carries it.
- At most one run is open in a window. Nothing starts before the preferences have answered, and a
  first run with nothing stored counts as an answer: every flow is not done.
- The wizard needs no project, no host and no pane; a highlight names a place by its `UiId`, so a
  place the help overlay can point at is a place a step can point at.

## Implementation

| Concern | Where |
|---|---|
| Flow, step, run and gate data; the chain, the sub-run stack and the progress arithmetic | `crates/ubiq/src/state/onboarding.rs` (`Flow`, `Step`, `StepBody`, `FormKind`, `Gate`, `StepAction`, `Trigger`, `OnboardingRun`, `enter_sub`, `leave_sub`) |
| Start, Next, Back, Skip, launching a sub-flow and the due-flow check | `crates/ubiq/src/app/onboarding.rs` (`open_onboarding`, `onboarding_start_sub`, `onboarding_next`, `onboarding_back`, `skip_onboarding`, `check_onboarding`, `AUTO_STARTED`) |
| The dialog, its palette form, the footer and the highlight layer | `crates/ubiq/src/ui/onboarding/mod.rs` (`render`, `place_callout`) |
| The flows | `crates/ubiq/src/ui/onboarding/flows.rs` (`all`) |
| The markdown | `help/onboarding/*.md`, hidden help pages, compiled in by `flows.rs` with `include_str!` |
| Completion | `WorkbenchState::onboarding_done`, mirrored to `InterfacePrefs.onboarding_done` by `AppState::remember_interface` |

**Flows are compiled into the interface.** `flows::all` is the registry, and the IDE flow is built
from the rail's project modes, so a mode added to the rail joins the walk.

**Their words are help pages that the help never shows.** Each markdown body is a page under
`help/onboarding/` with `hidden: true` in its frontmatter: the packer validates it like any page
but keeps it out of the nav, the catalogue and the bundle
([`../wip/help.md`](../wip/help.md) §2.1). `flows.rs` compiles the files in, so a step never waits
on the help bundle, and the dialog drops the frontmatter before rendering. Unlike a listed page, an
onboarding page opens with an `# h1`, which the dialog draws as its headline.

**A sub-run is a stack of one.** `OnboardingRun::enter_sub` swaps the sub-run in and keeps the run
it replaced as its `parent`; `leave_sub` swaps it back, on the step that launched it. Next past a
sub-run's last step, its Skip, and Back from its first step all leave it, and the first two then
pass the parent's step as Next would.

**The markdown body is pinned, not flexed.** The scrollable text view sizes itself as a percentage
of its parent, which a flex item does not give it: the body is an absolutely positioned box inset
to all four edges of the space between the title and the footer.

**The dialog is `kit::modal_locked`**, the modal without a ×, without an outside click and with the
onboarding layer as its dismissal rung (`Layer::Onboarding`, between the dropdowns and the help
target). The rung exists only while the dialog is visible: `AppState::top_layer` reports it for a
non-highlight step, and `cancel_dialog` swallows Escape there. A highlight step takes no rung, so
Escape reaches whatever is under it.

**Highlight geometry.** The callout reads the target's rectangle from `ui::ident`'s record of the
last complete frame, and `place_callout` puts the card right of the target, else below, else left,
else above, else over it, clamped inside the window. The outline's fill is `accent_soft`, its
border `accent`, and its glow a ring of `accent_muted` whose opacity pulses on `kit::pulse_animation`
over 2.8 s. The sizes are the `ONBOARDING_*` constants in `crates/ubiq/src/theme.rs`. The tour's two new targets are `ui_id::STATUS_BAR` and
`ui_id::WORKSPACE`, marked in `ui/status_bar.rs` and `ui/shell.rs`.

**When a check runs.** `check_onboarding` runs when either scope's preferences arrive (`apply_preferences` in
`app/projects.rs`, including a first run's nothing stored), after `enter_rail_mode`, after
`enter_project` and when a run closes, so the IDE flow follows the welcome without a second trigger. `AUTO_STARTED` is a
process-wide flag: only the first window to find a launch flow due opens it.

**Tests.** `crates/ubiq/tests/onboarding.rs` drives the run headlessly: the dialog as the top
layer, Escape, the tour as one step, the tour's sub-run (backing out, finishing, skipping it),
highlight steps, completion, Skip, a manual restart, the wait for the preferences, and the callout
placement. The style reference's specimen "A locked walk" raises the
real dialog.

## Failure

- **Preferences unanswered**: nothing starts; the check runs again when they arrive.
- **A target not on screen**: the card is centred and nothing is outlined; Next still works.
- **A mode the rail does not draw**: the arrival action is skipped and the step shows over the
  current mode.
- **Preferences that do not carry the field**: an older blob reads as nothing done, so the flows
  run once more.

## Related docs

- [`workbench.md`](./workbench.md) — the shell the flows point at, and the overflow menu entry
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — `kit::modal_locked`, the layer order and the `ONBOARDING_*` sizes
- [`../tech/decisions.md`](../tech/decisions.md) — `D223`, why flows are compiled in and completion is a preference
- [`../backlog.md`](../backlog.md) — what the shell still lacks

## Next steps

- More flows, one per mode, each entered the first time its mode is.
- Form steps beyond the palette: the first agent, the text size.
