---
id: inbox-capture
title: Proposal — capture, and the image editor it opens into
kind: proposal
status: proposal
summary: A capture control in the titlebar that photographs the whole window and opens the result as an untitled tab, and the image editor that tab is — crop, arrows, boxes, freehand and text over the picture, undo and redo, saved by the path an untitled text file already takes — built on the Excalidraw scene model Ubiq already parses and paints, with the clipboard as a second source through a new-file keystroke that asks what it is holding — designed Windows-first, because that is what forces the one capture route that crosses all three platforms.
read_when: you are deciding what a capture becomes, how an image is edited or cropped, what an untitled binary buffer means, or how the scene painter grows an editing mode
updated: 2026-09-07
depends_on: [feat-workbench, tech-ui, tech-architecture, tech-transport-contract, tech-diagram-format, wip-windows-build]
---

# Proposal — capture, and the image editor it opens into

**Ubiq is the one application whose screenshots are the product.** A pane is a real terminal with a
harness driving it; when it goes wrong, the wrongness is on the screen and nowhere else — not in the
log ring, not in the transcript, not in anything `just verify` can be pointed at. Today the only way
to get that picture out is the operating system's screenshot key, which knows nothing about where a
pane ends and the dock begins, produces a file the user then has to find, and offers no way to circle
the thing that went wrong.

This proposes two features that only make sense together. **A capture control** in the titlebar that
photographs the window. And **the image editor it opens into** — because a capture is not finished
when the pixels exist, and the place to crop it, mark it up and decide where it goes is a tab, not a
dialog.

The second is the larger of the two, and almost all of it is already in the tree.

## 1. Where it stands

**Nothing captures anything.** No `screenshot`, no `capture`, no render-to-image path in
`crates/ubiq/`, and no message family for it in `crates/ubiq-proto/src/messages.rs`.

**The titlebar's right cluster is a flat row that has been growing.**
`crates/ubiq/src/ui/titlebar.rs` draws three region switches, a rule, then search, a bell,
remote-connect, web-export, settings and the theme toggle — eight `icon_button`s, one of which
(`"bell"`, `|_, _, _| {}`) does nothing at all. Every capability that wanted a home in the top right
took one by appending a button.

**An untitled buffer is already a first-class thing, and the whole round trip is built.**
`⌘N` is `NewFile` in the `"Workbench"` context (`app/mod.rs:790`) and lands in `new_untitled_file`
(`app/editor.rs:730-763`), which names the tab `untitled-{n}`, pushes `OpenFile::untitled`
(`state/editor.rs:377`) and then pushes a *synthetic* `FileArrival` with empty bytes — because
building a buffer needs a `Window` and an action does not have one. A save on such a tab is
intercepted before anything is sent (`app/editor.rs:349-353` and `:696-700`), raises
`FileDialog::SaveAs`, and `save_untitled_as` (`:779-810`) retargets the tab optimistically and writes
with `expected: None`, which already means *create, refuse if something is there*. `retarget`
(`state/editor.rs:388`) re-derives the viewer from the new extension and clears the flag without
touching the buffer, and `ProjectFileWritten` (`app/wire.rs:496-524`) rebases the baseline. No host
change anywhere in that path.

**And the save-as dialog is Ubiq's own, inside the project** — a `prompt_modal` titled *Save as*
asking for a project-relative path (`ui/file_dialog.rs:63-77`), not a native panel.
`prompt_for_new_path` is called nowhere in the tree, and this proposal does not need it.

**`FileBody` has an image arm, and it has no buffer.** `FileBody::Bytes(Vec<u8>)`
(`:269-273`) is *bytes a viewer draws rather than a buffer edits*, and `viewer/image.rs` draws them —
seventy lines, no zoom, no pan, no toolbar. That comment is the shape this proposal changes.

**Three things about an image tab today that phase 3 has to move.** `savable()`
(`state/editor.rs:494-503`) admits only `FileBody::Text`, so no image tab can be written.
`ViewerKind::Image` returns `false` from `shows_buffer()` (`:143-150`), so an image tab **never takes
the keyboard** — which an undo stack and a toolbar both need. And `image::format` decides what to
decode **from the path's extension alone** (`ui/viewer/image.rs:20-37`), with no magic-byte sniffing,
so a picture with no filename cannot be drawn at all. §5 and §7 are where each of those lands.

**The annotation model exists, in full, and is not this feature's to invent.**
`crates/ubiq/src/state/scene.rs` parses an Excalidraw scene into `Element` (`:214`) — `x`, `y`,
`width`, `height`, `angle`, `stroke`, `fill`, `stroke_width`, `stroke_style`, `opacity` — over
`ElementKind` (`:152`): `Rectangle`, `Ellipse`, `Diamond`, `Frame`, `Line`, `Arrow`, `Text`,
`FreeDraw`, `Image`. `crates/ubiq/src/ui/viewer/scene.rs` paints all of it with `canvas()` and
`PathBuilder`. `_tools/excalidraw.py` renders the same subset for the wireframes, and
[`../tech/diagram-format.md`](../tech/diagram-format.md) documents exactly what it reproduces.

**So does the camera.** `crates/ubiq/src/ui/viewer/viewport.rs` is the surface a picture sits on —
fits on open, wheel zooms about the pointer, drag pans, double-click restores the fit, pinch zooms.
Mermaid and Excalidraw already share it. An image editor needs precisely this and nothing more.

**Two gaps, and one of them has expired.** `ElementKind::Image` paints a placeholder box
(`ui/viewer/scene.rs:276`), and the module's own doc gives the reason: decoding PNG *"would mean a
new dependency in the interface crate for a case `_docs/design/` does not contain."* That reason no
longer holds — `image` with the `png` feature is at `crates/ubiq/Cargo.toml:108`, and
`ui/ribbon.rs:76-97` already encodes with it. The other gap is stated twice in that same file: **the
scene viewer is read-only, and editing "is not proposed and is not built."** §5 is about how far this
reverses that, and it is less far than it sounds.

**The clipboard already carries images, and Ubiq has simply never asked.** GPUI's `ClipboardEntry`
is *"either a `ClipboardString` or a `ClipboardImage`"*, there is a `ClipboardItem::new_image`
constructor, `ClipboardImage` carries an `ImageFormat` whose variants include `Png`, and the macOS
backend handles `NSPasteboardTypePNG` and `NSPasteboardTypeTIFF` beside `NSPasteboardTypeString` in
both `read_from_clipboard` and `write_to_clipboard`. Ubiq uses `ClipboardItem::new_string` at nine
call sites and nothing else. §7 needs both directions and gets them.

**Rendering an element tree to an image is the one thing GPUI does not offer.** There is no
`screenshot`, no `render_to`, no offscreen surface, no pixel readback in its public surface; the only
headless renderer in the crate sits beside `VisualTestPlatform` and `image_tests` and is its own
visual-test machinery. What GPUI does have is **operating-system screen capture** —
`is_screen_capture_supported()`, `screen_capture_sources()` returning sources with a
`SourceMetadata { id, label, is_main, resolution }`, and a `ScreenCaptureStream` yielding a
`ScreenCaptureFrame` with `to_image_data(format)`. It is provided by `zed-scap`, an **optional
dependency behind gpui's `scap_screen_capture` feature**: the crate is resolved in `Cargo.lock` and
**is not compiled in this build**, because nothing turns that feature on. §4 is what follows from
that.

**There is no camera icon.** `IconName` is generated from the SVGs `gpui-component-assets` ships,
`assets/` holds only logos, and the closest glyphs are `Frame`, `Maximize` and `Copy`.

## 2. What this decides

- What a capture **becomes** — §3, and it is the decision the rest falls out of;
- what the titlebar control is, and how much of it phase 1 needs — §4;
- what editing an image means, in a model Ubiq already has — §5;
- what a save writes, and the trap in the alternative — §6;
- what the clipboard is allowed to start — §7;
- and what the setting removes — §8.

## 3. A capture becomes an untitled tab

**The pixels open as a new tab in the editor, untitled and dirty, exactly as a new text file does.**
Not a file written somewhere the user has to go and find, not a clipboard item that is gone the next
time anything is copied, and not a dialog asking where to put something the user has not looked at
yet.

Everything else in this proposal is downstream of that sentence, and it is worth naming what it
deletes: **a destination menu, a sticky default, a project screenshots folder, an offer to add that
folder to `.gitignore`, and a bus message carrying PNG bytes to the host at the moment of capture.**
An earlier draft of this document had all six. They existed to answer *where does it go*, and the
answer is that the user decides later, in the same gesture they already use for a file they have just
written — which is `⌘S`, a save-as, and the `WriteProjectFile` path that exists.

Three things follow immediately.

**A capture is never lost and never silently written.** It is a dirty tab. Closing it asks, the way
closing any dirty tab asks. Nothing reached the disk without the user naming a path.

**The picture is looked at before it is kept.** Which is the whole reason the editing half of this
proposal is worth building: the interval between *capture* and *save* is where a crop and an arrow
belong, and today there is no interval at all.

**The capture tool stops being special.** It is one source of an image buffer. The clipboard is
another (§7), and opening a PNG from the explorer is a third. All three arrive at the same tab, and
only the first one needs anything in the titlebar.

## 4. The control

**Phase 1 captures the whole window, and that is the entire control**: one `icon_button` in the right
cluster, between web-export and settings, wearing `IconName::Frame` — an existing glyph rather than
the first custom icon asset in the repository, for one button.

### What the capture costs, now that it is known

An earlier draft hoped Ubiq could photograph itself for free by rendering its own element tree, and
treated the operating system's capture as the fallback. **That hope is gone**: GPUI has no
render-to-image, so there is exactly one route and it is the operating system's.

Three consequences, and they are the substance of this section.

**On macOS a capture costs a screen-recording permission from the first one.** Not at phase 5, not
only for the desktop subjects — for photographing Ubiq's own window. It is a system prompt, granted
once and revocable, asked the first time the button is pressed and never at startup. It is also
**not a universal cost**, which is what the next subsection is about.

**The feature has to be turned on.** `scap_screen_capture` is off in this build and `zed-scap` is
compiled nowhere, so phase 2 begins by enabling a feature on a git dependency.
`is_screen_capture_supported()` is the runtime gate that decides whether the button is offered at all
on a given platform.

**The API is a stream, because it was built for screen sharing.** A source, a stream, and frames
arriving until it is stopped. A screenshot is *start, take one frame, stop* — which works, and is
worth naming as a shape that will read oddly to whoever implements it.

**And it makes §4's own argument stronger.** Whether a source can be a single window or only a
display is unresolved — `SourceMetadata` carries `is_main` and a `resolution`, which reads like a
display — and it does not much matter, because **if the capture is of a display, then capturing "the
window" is already a crop**, and Ubiq knows its own window's bounds. The crop-in-the-editor model was
argued below on its own merits; it turns out to also be the mechanism.

### Windows first

`_docs/wip/windows-build.md` records that macOS is the primary platform and Windows is the one the
build rarely runs on. **This feature should be designed the other way round**, and the reason is not
even-handedness — it is that designing it macOS-first produces the wrong technical choice.

**Because the macOS-only route is the tempting one.** ScreenCaptureKit through the `objc2` stack is
already linked (`crates/ubiq/Cargo.toml:99` is a `cfg(target_os = "macos")` block, and the dock-icon
code lives in it), it is less code for a one-shot, and it would work perfectly on the machine this is
built on. It also cannot cross, and the second platform would then need a second implementation
against a second API with a different frame type and a different consent model — which is the point
at which features stop being ported. An earlier draft of this section called that choice "a spike
rather than a paragraph". Windows-first settles it: **`scap_screen_capture`, and nothing else.**

**And that route already covers all three platforms.** `zed-scap`'s own dependencies say so:
`screencapturekit` and `screencapturekit-sys` for macOS, **`windows-capture` 1.5 — the Windows
Graphics Capture API — for Windows**, and `x11` and `xcb` for Linux. One feature flag, one
`ScreenCaptureFrame`, one `to_image_data`, three platforms.

**Windows is the better platform to design against, not merely the harder one**, because it is
stricter in the two places that matter and looser in the one that does not:

| | Windows | macOS | Linux |
|---|---|---|---|
| Backend | Windows Graphics Capture | ScreenCaptureKit | X11 / XCB |
| Consent prompt | None for capturing your own window | Screen-recording permission, revocable | None on X11 |
| A single window as a source | Native — a capture item is made from a window handle | Native, if gpui's wrapper exposes it | Not really; X11 has no equivalent worth relying on |
| Version floor | Windows 10 1803 for capture, 1903 for a window without chrome | Ventura-era API | — |
| Visible tell | A capture border is drawn around the window; removing it needs a recent Windows 11 | None | None |

Four things follow, and each is a design decision rather than a port note.

**The permission story is macOS's, not the feature's.** §8's setting and §9's failure row are written
as though a prompt is universal, and it is not: on Windows and on X11 the button simply works. So the
first press must not *assume* a prompt — it asks the platform, and where there is nothing to ask, it
captures. A confirmation dialog explaining a permission that is not being requested is worse than no
dialog.

**Windows can hand back the window itself, which makes the crop a refinement rather than the
mechanism.** §4 argues the window capture may be a crop of a display, and on X11 it certainly is. On
Windows a window is a first-class capture item, and on macOS ScreenCaptureKit has the same notion. So
the honest shape is: **ask for the window; fall back to a display plus a crop to the window's
bounds.** Both paths end in the same untitled tab, and whether gpui's `screen_capture_sources()`
exposes windows at all — `SourceMetadata`'s `is_main` and `resolution` read like displays — is the
one thing the spike must answer, because it decides whether the fallback is the common case or the
rare one.

**A Windows capture may photograph its own capture border.** WGC draws one around the captured
window, and suppressing it needs a recent enough Windows 11. Ubiq cannot control that, and a
screenshot with a stray yellow rectangle in it is a bug report waiting to happen — so the crop tool
matters more here than anywhere, and phase 3 stops being a nicety on the platform that goes first.

**The keystroke is two keystrokes**, as everything else in this tree already is: `NewFile` is bound to
`cmd-n` and `ctrl-n` side by side (`app/mod.rs:790-791`), and the capture binds `cmd-shift-2` and
`ctrl-shift-2` the same way.

**Linux is the platform this leaves behind, and it should be said plainly.** `zed-scap` reaches X11
through `x11` and `xcb` and has no Wayland backend, while `crates/ubiq/Cargo.toml:14-19` enables both
`wayland` and `x11` on `gpui_platform`. A Wayland session captures through
`xdg-desktop-portal`'s ScreenCast interface or not at all, and nothing in this dependency graph does
that. So on Wayland `is_screen_capture_supported()` is expected to say no, the button is not offered,
and **the clipboard and file routes of §7 are the whole feature there** — which is a real answer
rather than a broken button, and it is another reason the editor half is worth building
independently of the capture half.

**No chevron yet, and no clip-before-capture.** Both were in an earlier draft and both are wrong at
this stage, for the same reason: **cropping is an edit.** A clip tool that runs before the capture
needs a full-screen transparent overlay window that takes the whole display's input — a window kind
Ubiq does not have — and it produces a rectangle the user cannot revise. A crop in the editor is one
more entry on an undo stack, needs no new window, can be redone against the full picture, and is the
same tool whether the picture came from the capture button, the clipboard or a file. One
implementation, strictly more capable, and it is not in the titlebar.

So the narrower subjects — the active panel, the focused pane, a region — are **not phase 1 and may
never be needed**. A window capture plus a crop reaches all three, and the only thing lost is exact
node bounds, which the crop tool can offer later as snapping if anyone misses it. That is a much
better trade than four menu rows and a subject model.

**The chevron arrives when there is a second thing to put behind it** — the desktop subjects of §11's
last phase, which do need a picker and do need that overlay window. Until then a split control is
furniture around a single action.

**One keystroke: `⌘⇧2`**, doing what the button does. Whether it is free of the component library's
own bindings is unverified and is a backlog row, in the same class as the `⌘⇧F` question in
[`find-in-file-proposal.md`](./find-in-file-proposal.md) §5.

**Adding to that cluster costs two things.** The dead bell gets a meaning or goes — a control that has
never done anything is not a placeholder. And every control in the cluster gets a tooltip; four of
eight have one today, and one more unlabelled glyph makes the row worse. Neither is required for
capture to work. Both are the honest price of the ninth button.

## 5. The image editor

### The buffer is a scene

**`FileBody::Bytes` gains a sibling that has a buffer**, holding a `state::scene::Scene` whose first
element is `ElementKind::Image` over the captured PNG in the scene's own `files` map — which is
precisely what that map is for. Every annotation is another `Element` appended to it. Crop is the
scene's bounds.

**The tab has to be able to hear a keystroke**, which today it cannot: `ViewerKind::Image` declines
both `shows_buffer()` and `has_preview()`, so an image tab draws no header and never takes focus.
Phase 3 gives the image viewer a focus handle and the 32-pixel `viewer::header` strip that Markdown
and Mermaid already get — the toolbar's home — and that is the smallest change that makes undo, redo
and a tool selection reachable at all.

**The vocabulary is not invented, chosen or argued.** It is `ElementKind`, and the tools are a subset
of it: **rectangle, ellipse, arrow, freehand and text**, with a stroke colour, a fill, a width and an
opacity, all of them fields `Element` already carries and `viewer/scene.rs` already paints. Diamond
and Frame stay in the model, unoffered by the toolbar, because removing them would mean forking a
parser that reads real files.

**Undo and redo are a stack of scene edits**, not snapshots of a pixel buffer. This is the reason to
build the editor this way rather than painting into the bitmap: a freehand stroke over a
four-megabyte capture, undone by keeping the previous four megabytes, costs a hundred megabytes after
twenty strokes. A `Vec<Element>` costs the points. It also means an arrow stays an arrow — movable,
recolourable and deletable an hour after it was drawn — which is the difference between an annotation
tool and a paint program, and the user always wants the first one.

**The base image is never edited.** Every tool adds, moves or removes an element above it. A crop
changes what is shown, not what is held. This is what makes every operation reversible with no bitmap
bookkeeping at all — and it is also the trap in §6.

### The tools

The viewer grows a toolbar — the first one any viewer has — over the `viewport.rs` surface it already
sits on.

| Tool | Does | Is |
|---|---|---|
| Select | Pick an element, move it, change its colour, delete it | Hit-testing on `Element.id`, which the model already keeps for it |
| Crop | Drag a rectangle; the scene's bounds become it | A change to `Scene::bounds`, undoable, re-croppable against the whole |
| Rectangle · Ellipse · Arrow | Drag one out | One `Element` appended |
| Freehand | Draw | `ElementKind::FreeDraw`, points collected on drag |
| Text | Click and type | `ElementKind::Text` |
| Colour · width · fill | The style the next element takes, and the selected one's | `stroke`, `fill`, `stroke_width`, `opacity` |
| Copy region | Drag a rectangle; the flattened pixels under it go to the clipboard | §6's flatten, over a sub-rectangle |
| Undo · redo | The edit stack | — |

**"Selection" means two different gestures and both are wanted, so they get two names.** *Select*
picks an object. *Copy region* takes a rectangle of the flattened picture. Conflating them into one
marquee is how a user ends up copying a rectangle when they meant to move an arrow.

**Zoom, pan and fit are free.** They are `viewer/viewport.rs`, unchanged, and every tool works in
scene coordinates the camera maps for it.

### How far this reverses "read-only"

`ui/viewer/scene.rs` says, twice, that the scene viewer is read-only and that editing is not proposed.
This reverses that **for a scene Ubiq authored in this session, and not for a file it read.** Opening
an `.excalidraw` document from the explorer still draws and does not edit.

The line is: **the editor edits a capture, not a document.** It is a real line — a captured scene has
no file on disk to be consistent with, no `appState` Ubiq did not write, and no round-trip fidelity to
preserve through the parser's stated losses (roughness renders clean, hachure renders solid, text is
never rotated). A document has all three. Whether the line should later move — whether
`.excalidraw` files become editable, now that the machinery is there — is a real question and a
backlog row, not something this proposal should decide on its way past.

**The first task is not the toolbar.** It is making `ElementKind::Image` paint its bytes rather than a
placeholder box, because the whole model rests on the base image being drawn. It is a small change
with a dependency that is already present, and it independently fixes every `.excalidraw` file with
an embedded image — including in `_docs/design/`.

## 6. Saving

**A save flattens the scene to a PNG and takes the untitled path that exists.** `⌘S` on an untitled
tab asks where to put it; `retarget` repoints the tab; the bytes go over `WriteProjectFile` like any
other write, with the version discipline that message already carries. Nothing new crosses the bus,
and no path enters interface code — the flatten happens here, the file is the host's, and that stays
true when the host is not local.

**Two small edits make that true rather than nearly true.** `savable()` admits only
`FileBody::Text`, and `save_untitled_as` reads the buffer's *text* to get its bytes
(`app/editor.rs:779-810`); both branch on the body instead, and the image arm hands back the flatten.
Neither is a new path — they are the existing one learning that a buffer is not always a string.

**And the save-as dialog needs the name seeded, which is a bug it already has.** `ask_save_as`
(`app/editor.rs:768-771`) sets the dialog without seeding or focusing the shared `file_name` field,
unlike New and Rename which go through `open_file_dialog`. A text buffer survives that; a capture
does not, because the suggested name is the only thing standing between the user and typing
`capture-1.png` by hand every time. It is a two-line fix in the path this feature depends on.

**The flatten is the same painting, into an image rather than onto the screen.** Elements walked in
`paint_rank` order at scene scale, cropped to the scene's bounds, encoded by the `image` crate that
is already linked.

**Saving the scene instead is tempting and has a trap in it.** The edit list *is* an Excalidraw
scene — Ubiq could write `.excalidraw`, read it back, and keep every annotation editable forever,
which is genuinely elegant and solves the one real cost of §5's model (close the tab and the arrows
become pixels). But **the base image stays whole in `Scene::files`**. So a rectangle drawn over a
token to hide it is a redaction in the flattened PNG and **is not one in the `.excalidraw` file** —
the original pixels are in there, one unzip away, under a shape.

That is exactly the class of thing that ships quietly and is found later. So:

> **Redaction is only real in the flattened PNG.** If saving the scene is ever offered, it flattens
> and re-embeds anything a filled opaque element covers, or it is not offered.

Phase-wise this is simple: **PNG only.** The scene format is a backlog row with that rule attached to
it, and the honest consequence of not having it is stated rather than hidden — **closing an image tab
ends its annotations' editability**, and the tab is dirty until saved, so the user is asked first.

## 7. The clipboard, and `⌘N`

**A capture is not the only picture worth editing.** Anything on the clipboard — from a browser, from
another application, from Ubiq's own copy-region — should be able to start a tab.

So: **`⌘N` asks what the clipboard is holding, but only when it is holding an image.** With text on
the clipboard, or nothing, `⌘N` opens an untitled text buffer exactly as it does now. With an image, a
small modal offers two rows — *Paste the image on the clipboard* and *New text file* — and Escape
takes the second, because that is what the keystroke has always meant.

It is a ninth `FileDialog` variant and a hand-rolled `modal(...)` rather than `confirm_modal`, which
is confirm-and-cancel: this is two choices, the shape `CloseWindow`'s dialog already uses. `⌘N` is
bound in the `"Workbench"` context only and deliberately not in `"Input"`, so none of this fires while
a text field has the keyboard — which is correct and needs no thought.

**An untitled picture must be named `capture-1.png`, not `untitled-1`.** `image::format` reads the
extension and nothing else, so a tab called `untitled-1` draws *"Not an image this build decodes"*
over a perfectly good capture. Naming it with its extension costs nothing, makes the save-as
suggestion right, and makes `ViewerKind::of` select the image arm without a special case.

That condition is the whole design. A modal in front of the common case would be a tax on every new
file, paid so that a rare gesture can be discovered; a modal that only appears when there is
genuinely a choice to make costs nothing and needs no second shortcut to remember.

**This makes the clipboard load-bearing in both directions, and both are available.** Reading an
image starts a tab; writing one is *copy region*. `ClipboardEntry` has an image arm,
`ClipboardItem::new_image` exists, `ImageFormat::Png` is one of its formats, and the macOS backend
already speaks `NSPasteboardTypePNG` and `NSPasteboardTypeTIFF` in both directions. Nothing here
needs a fork, a feature or an upstream change — only that Ubiq stop assuming the clipboard is a
string. **What arrives on the clipboard is per-platform, and it decides a line in `Cargo.toml`.**
`image` is compiled here with `default-features = false, features = ["png"]`
(`crates/ubiq/Cargo.toml:108`), which is enough for a PNG and nothing else: a macOS screenshot lands
on the pasteboard as TIFF as often as PNG, and Windows offers a device-independent bitmap beside
whatever registered format the copying application used. Either GPUI normalises all of that into
`ImageFormat` before Ubiq sees it — which the macOS backend's handling of `NSPasteboardTypePNG` and
`NSPasteboardTypeTIFF` suggests it does — or the feature list here grows to match. It is a small
question with a definite answer, and it is Windows and macOS that ask it, not Linux.

## 8. The setting

**`capture_enabled: bool`, defaulting to on, in `UiSettings`, under Appearance.** It is chrome, and
Appearance owns the titlebar's other switches. Off means the button, the tooltip and the keystroke are
absent — not hidden; a setting that leaves the shortcut live does not do what its label says.

**`SCHEMA` does not move.** `#[serde(default = "default_true")]` is what the existing booleans use and
what it is for: a blob written by an older build reads as on. Bumping the schema would discard every
user's settings to add one field.

**The setting gates the capture control, not the editor.** An image tab is reachable by opening a PNG
and by `⌘N`, and neither is what an environment that would rather Ubiq could not photograph itself is
asking about. It also does a different amount of work per platform, which is worth knowing: on macOS
the operating system asks its own question and the user can revoke the answer, so the setting is a
second lock on a door that already has one. **On Windows and on X11 nothing else asks**, and this
switch is the only thing standing between the application and a screenshot — which is the strongest
argument for it defaulting to on but existing at all. That the tool exists at all is the thing worth a switch: Ubiq holds accounts, connector
fields and panes in which a harness may have printed a token, and §5 gives it a filled rectangle to
cover them with, which is a mitigation and not a guarantee.

## 9. Failure

| When | What happens |
|---|---|
| The setting is off | Nothing is drawn and the keystroke is not registered |
| The platform denies a screen-recording permission (macOS) | The capture says so and offers the system settings pane |
| The platform has no capture backend (a Wayland session) | The button is not offered; `⌘N`, paste and opening a file still reach the editor |
| Windows draws its capture border into the frame | Nothing automatic; the crop tool is the answer, which is why phase 3 matters most on Windows |
| The window is minimised or fully occluded | The capture is refused rather than saved as whatever the compositor had |
| The capture succeeds | A dirty untitled tab, focused, fitted to its panel |
| That tab is closed unsaved | The dirty-tab prompt, as for any buffer |
| A save is refused by the host | The tab stays dirty and untitled, and the failure names the folder |
| `⌘N` with text or nothing on the clipboard | An untitled text buffer, unchanged from today |
| `⌘N` with an image on the clipboard | The two-row modal; Escape takes the text file |
| The clipboard holds an image format the decoder does not know | The modal does not appear; `⌘N` is a text file |
| A crop is dragged to nothing | Ignored; the scene keeps its bounds |
| An element is dragged outside the crop | It is kept and clipped, and undo brings the view back |
| The scene painter meets an element kind it does not know | It is skipped and the rest draws, as it does today |

## 10. Rules this adds

**A capture becomes a buffer, not a file.** Nothing reaches the disk until the user names a path.

**The annotation vocabulary is `state::scene::ElementKind`.** No second shape model enters the
interface, and a tool that cannot be expressed as an `Element` is not added without changing that
model where it lives.

**An image edit is an element on a stack, never a write into the base pixels.** Undo is popping the
stack, and the picture underneath is untouched for the life of the buffer.

**The scene editor edits a capture, not a document.** A scene read from a file stays read-only until
someone decides otherwise, deliberately.

**Redaction is only real in the flattened PNG.** Any format that keeps the base image whole must
flatten what an opaque element covers, or must not be offered.

**A modal in front of a keystroke appears only when there is genuinely a choice.**

**A control in the titlebar's right cluster carries a tooltip.**

## 11. Phases

1. **The base image draws.** `ElementKind::Image` paints its bytes instead of a placeholder box.
   One change, dependency already present, and it fixes `_docs/design/` on its own.
2. **Capture to a tab.** `scap_screen_capture` turned on and spiked **on Windows first**; then the
   titlebar button behind `is_screen_capture_supported()`, `capture_enabled`, the permission asked at
   first press where the platform has one, the window capture — as a window source if there is one,
   otherwise a display cropped to the window's bounds — and the untitled image tab — read-only, named with its extension, wrapped in the existing viewport for zoom and
   pan, and saved by the existing untitled path once `savable()` admits it. Useful the day it lands,
   with no editor at all.
3. **The editor.** The image tab takes focus and grows a header; then the toolbar, the scene buffer,
   select, crop, rectangle, ellipse, arrow, freehand, text, the colour and width controls, undo and
   redo, the flatten, and `savable()` and `save_untitled_as` learning about a body that is not text.
4. **The clipboard.** `⌘N`'s modal, paste-to-tab, and copy-region, over `ClipboardEntry`'s image arm
   and `ClipboardItem::new_image`. Nothing here is gated on anything.
5. **The desktop.** A region of the screen and another application's window: a full-screen overlay
   window Ubiq does not have today, a source picker, and the chevron the titlebar control does not
   need before then.

Phase 2 stands alone. Phase 1 is worth doing whatever happens to the rest.

**Phase 5 is closer to phase 2 than an earlier draft assumed**, and the reason is §4: the
screen-recording permission is paid by phase 2, so the desktop phase no longer introduces the consent
model — it only adds the overlay window and the picker. That does not make it small, and it does not
promote it, but the argument for keeping it far away was partly that it brought a new class of cost,
and it does not.

## 12. What this asks to be decided

- A capture is taken with the operating system's screen capture, because GPUI offers no other route,
  and therefore costs a screen-recording permission asked at first use — for Ubiq's own window, not
  only for the desktop.
- That capture arrives by turning on gpui's `scap_screen_capture` feature, and **not** by calling
  ScreenCaptureKit through the `objc2` stack already linked — the second is less code and cannot
  cross, and Windows-first is what makes that choice obvious rather than arguable.
- The feature is designed and spiked Windows first, against Windows Graphics Capture, even though
  macOS is the tree's primary platform.
- The consent prompt is macOS's alone; the first press asks the platform rather than assuming one.
- Wayland has no backend in this dependency graph, so the capture button is absent there and the
  editor is reached by the clipboard and by opening a file — which is a stated answer, not a gap.
- A capture opens as an untitled, dirty tab and is saved by the path an untitled text file already
  takes — which deletes the destination menu, the project folder and the bus message an earlier draft
  needed.
- Cropping is an edit in that tab, not a mode before the capture, which removes the full-screen
  overlay window from everything before the desktop phase.
- Phase 1's capture is the whole window; the panel, pane and region subjects are reached by cropping
  and may never need to exist.
- The annotation model is the Excalidraw scene Ubiq already parses and paints; the tools are a subset
  of `ElementKind` and nothing new is invented.
- Editing is enabled for a scene Ubiq authored, and `.excalidraw` files stay read-only.
- A save writes a flattened PNG only; saving the scene waits on the redaction rule.
- `⌘N` asks its question only when the clipboard holds an image.
- The control wears `IconName::Frame`, and adding it costs the dead bell and tooltips across the
  cluster.

Backlog rows this leaves open: **whether a screen-capture source can be a single window or only a
display**, which decides whether the window capture is a source or a crop of one, and which
`SourceMetadata`'s `is_main` and `resolution` only hint at; what turning on `scap_screen_capture` costs in build time on a
git dependency, and whether gpui's Windows backend wires that feature up at all — `gpui_windows` does
not list `zed-scap` among its dependencies while `gpui_linux` does, which is the first thing the
spike should check; whether a Wayland backend through `xdg-desktop-portal` is worth proposing
upstream, or whether Wayland stays a platform where Ubiq does not capture; whether Windows' capture
border can be suppressed on the versions Ubiq intends to support, and what the picture looks like
where it cannot; whether an image read from the clipboard as TIFF can be decoded, given that `image`
is compiled with `png` alone here; whether `⌘⇧2` is free of the component library's
bindings, and GPUI's precedence rules for a binding registered after `gpui_component::init`; whether
`image::format` should sniff magic bytes rather than trust an extension, which this feature works
around rather than fixes; whether
`.excalidraw` files should become editable now that the machinery exists; whether an annotated capture
should be savable as a scene, and the flatten-what-is-covered rule that would have to come with it;
whether a capture can be sent to a conversation — the obvious reason to photograph a pane is to show
an agent what it did, and no message in the transport carries an image today, which is a
`ubiq-proto` question and a `crates/agent-manager` one before it is a capture question; what a capture
is on Windows and Linux, where neither the `objc2` path nor the permission model transfers; blur and
pixelate as redaction tools, which are pixel operations and do not fit the element model; and whether
the crop tool should snap to panel and pane bounds, which is the only thing §4 gives up.

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the titlebar, the editor, its tabs and the save path
- [`../tech/diagram-format.md`](../tech/diagram-format.md) — the scene vocabulary this borrows, and the limits it inherits
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the kit, the icon-button conventions and the tooltip rule §4 leans on
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — `WriteProjectFile`, which a save uses unchanged
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — why a pane's picture is pixels and not text
- [`completed/file-viewers-proposal.md`](./completed/file-viewers-proposal.md) — where the scene viewer, the image viewer and the shared camera came from, and the read-only decision §5 bounds
- [`find-in-file-proposal.md`](./find-in-file-proposal.md) — the same open question about who wins a keybinding
