# UI Architecture Decisions
Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing egui/eframe, layouts, response viewer, theme/palette/fonts, forms, and dialogs.
## Table of Contents
- [Architecture & Thread Model](#architecture-thread-model)
- [Design System, Typography & Themes](#design-system-typography-themes)
- [Layout, Panes & Global Navigation](#layout-panes-global-navigation)
- [Request Bar & Editors](#request-bar-editors)
- [Response Viewer](#response-viewer)
- [State, Saving & Dialogs](#state-saving-dialogs)
- [Screens (Variables, Run, History, Settings)](#screens-variables-run-history-settings)

## Decisions

### Architecture & Thread Model

- **GUI: egui + eframe (glow backend); Iced not needed.** Why: the most mature tables, trees and
  panels, custom text layout for `{{var}}` highlighting, and a UI test harness (`egui_kittest`,
  headless, no GPU). Slint was rejected for limited text-editing control and license
  conditions; gpui for an unstable API. Accepted trade-off: custom dark and light themes (accent
  `#ff6c37`), not a native look. Confirmed in stage 0: release binary 5.6 MiB (LTO, stripped,
  macOS arm64); first frame about 225 ms after process start (24 ms after app creation).

- **UI architecture: widgets emit actions, background work returns messages.** Widgets edit the
  editor's form state in place and push `Action`s; the app applies them after the frame and runs
  database and network work through the bridge; results come back as `Message`s drained at the
  start of the next frame. Every navigation bumps a generation and results carry the generation
  they started under, so a late result never lands on another request. Why: immediate-mode
  drawing stays free of IO, and the workflow (guard, saves, close) is testable as state.

- **The UI thread never blocks.** `imposter-ui/clippy.toml` disallows blocking database calls,
  file IO, sleeps, `block_on` and synchronous file dialogs. Only test and example crates allow
  them, at their top, and `window_state`, which runs before the window exists. Why: a frozen
  window is the most visible failure a desktop app has.

- **The layout lint gates every screen** (`imposter-ui/tests/layout.rs`).
  - **Coverage:** at the minimum and a wide window, in both themes and both densities, it
    walks the screens, a sent response, a History run and the dialogs.
  - **Widths:** no visible control, and no outlined frame (panes, groups, cards, read
    from the painted shapes), may run past the window's sides; a control inside a pane
    card may not run past the card's sides, which clip it unseen (the Auth tab's eye at
    720 points, Phase 6 review Q27).
  - **Overlap:** no two buttons may overlap, and no text label may run into a button
    (a long "500 Internal Server Error" into the Runs menu). This is checked only without a dialog open,
    since a dialog is a layer above.
  - **Clipping:** egui does not tell AccessKit about clipping, so controls crossing the
    header's bottom or the status bar's top count as scrolled out, as does anything below
    the status bar's top other than its own controls.
  - **Upkeep:** new screens and states are added to its walk.

  Why: horizontal overflow and crowded rows only show at some sizes; the lint found
  History's toolbar overlapping at 720 points in Comfortable, and Settings groups 56
  points past the window (`set_max_width` widens a narrower `Ui`).

- **UI tests drive the real app through AccessKit** (`egui_kittest`): every widget a test or a
  screen reader needs has an accessible name, set with `widgets::name` when no visible label
  exists. Only characters present in egui's bundled fonts are used; kittest fails on a missing
  glyph, which would otherwise render as a box. `cargo run -p imposter-ui --example screenshot --
  <dir>` renders the app offscreen (wgpu) for looking at layouts; `--example tour` renders
  every primary task step by step and the states a design review needs (first start,
  loading, errors, dialogs) at the minimum and a wide size. Design reviews follow the
  `design-ux` procedure: a reviewer that did not build the screens scores the tour.


### Design System, Typography & Themes

- **Two themes from one set of named roles** (`theme::Palette`: background, surface, input,
  field_border, selected_bar, text, tones, syntax…, as `DARK` and `LIGHT`).
  - **Setting:** Settings → Appearance → Dark / Light / System; it applies at once and is
    stored in UI state (`appearance`), not in the application settings, which govern sends.
  - **Reading colors:** widgets read colors through `theme::palette()`, a thread-local that
    `theme::begin_frame` sets from the frame's theme. egui draws a frame on one thread, and a
    context-free lookup keeps helpers like `tone_text` simple.
  - **Contrast, checked by `theme.rs` tests for both palettes:**
    - text, tones, accent, syntax and method colors are ≥4.5:1 on every background they
      appear on;
    - control edges (`field_border`, used for fields and buttons) and the selection marker
      are ≥3:1, per WCAG 1.4.11;
    - tinted fills (status pills at 12%, the red tint of unknown variables and failed
      checks at 12%) keep their text ≥4.5:1, even with the blend rounded a level either
      way as rendering does; 18% red fell to 4.08:1 in light, and the light red is
      #bf2424 because #c62828 measured 4.47:1 on its tint;
    - every syntax color stays clearly apart from the accent (RGB distance ≥80), because
      `{{variables}}` inside a body are marked in the accent; light strings are green.
  - **Light accent:** the light theme's accent is a darker orange (#b83b0a), because
    #ff6c37 on white is below 3:1.

- **Bundled fonts: Inter (Regular, SemiBold) for the interface, JetBrains Mono for data,
  Phosphor Regular for icons.**
  - **Files:** in `crates/imposter-ui/assets/fonts`, with their SIL OFL licenses (Inter and
    JetBrains Mono) and MIT license (Phosphor), shipped in every package as resources,
    and pinned hashes. Phosphor is the only font in the dedicated `icons` family;
    proportional text uses Inter followed by egui's fallbacks. Inter maps some of the
    same Private Use Area codepoints (including Settings' U+E272), so putting Phosphor
    after Inter rendered icons as letters. Inline icons use separate `RichText` or
    layout sections in the icon family.
  - **Type scale:** `theme::page_title` is 18 pt SemiBold, `theme::section` is 14 pt
    SemiBold, `theme::body` is 13 pt and `theme::meta` is 12 pt. Dialog titles and Settings
    section headings use these helpers; existing 15 pt screen titles and breadcrumbs migrate
    when their screens are restyled. `theme::semibold` remains available for selected labels.
    Single-line variable fields (URL, headers, paths, values) are monospace. Why: a small,
    explicit scale gives headings hierarchy without competing sizes across screens.
  - **Size:** Inter and JetBrains Mono add about 1.1 MiB to the release binary; Inter
    Medium was left out for that reason. Phosphor's TTF is 488,636 bytes.
    Replacing the painted toolbar icons added 495,360 bytes (483.75 KiB) on macOS arm64:
    25,359,920 → 25,855,280 bytes, measured before and after with
    `CARGO_TARGET_DIR=target cargo build --release -p imposter` (2026-11-20, LTO, stripped).
  - **Coverage:** `theme.rs` tests check that the fonts cover every non-ASCII character in
    the UI, domain and service string literals, that all icon glyphs resolve through the
    dedicated family and that every `Icon` codepoint exists in Phosphor's cmap.
    `tests/icons.rs` checks the laid-out family for every icon in both labels and buttons,
    including the Settings codepoint that both Inter weights also map: glyph existence
    alone cannot prove which font draws it.
    CPU atlas tests compare the pixels, advances and offsets of every icon against a
    Phosphor-only reference after warming both Inter weights' overlapping glyphs, and
    check the real app's Settings, More, Save, Filter and Copy glyphs. An Inter-first
    negative control reproduces the wrong letters (Copy has no Inter mapping), so the
    comparison detects the original bug. These tests need no GPU; a family-name check
    alone does not verify rasterized output.

  Why: stock egui fonts gave every text the same look and size, and lacked glyphs such as
  "●". Phosphor is vendored from `@phosphor-icons/web` 2.1.2, with codepoints from its
  regular CSS: `egui-phosphor` 0.14 requires egui ^0.36, incompatible with our egui 0.37.

- **Density is a setting: Compact (default) or Comfortable** (Settings → Appearance, stored
  with the theme).
  - **Tokens:** `theme::Spacing` keeps list rows at 26 / 32 pt and controls at 24 / 28 pt.
    Named gaps share a 4/8 pt grid: xs=4, sm=8, md=12, lg=16, xl=24; stacked gaps are 4 / 8
    and density padding is 8 / 12. Why: shared increments keep new controls aligned while
    preserving the tested row and control heights.
  - **Users:** the tree, History, tabs, icon buttons and the request bar read
    `theme::spacing()` instead of fixed sizes.
  - **Target size:** pointer targets never go below 24×24 points (WCAG 2.5.8), which is
    what Compact uses.
  - **Checked by** `imposter-ui/tests/appearance.rs`: the row pitch in both densities, and
    at least 10 tree rows at the minimum window in Compact (7 before).

- **Orange means primary, selected or focused; nothing else.**
  - **Hover and press** outline a control in the muted text color; a hovered icon turns
    the text color.
  - **Selection** (`selection.stroke`) is the accent: selected items' text, a focused field's
    1.5-point ring, and a 2-point bar on the left of selected tree, History and palette rows
    (`widgets::selection_bar`). Selected rows and menu options are filled, never outlined
    (`widgets::option`): egui outlines a selected button with the control edge, which
    read as a hover left on.
  - **Tabs** are frameless: muted text, or semibold accent text with an underline. Hover
    previews the underline in the muted color rather than drawing a button frame. They
    keep the button padding (a transparent frame; `frame(false)` drops the padding and ran
    the labels together, 1.3 points apart).

  Why: an orange outline on every hover diluted the accent until it meant nothing (the
  review's #60). Checked in `theme.rs` tests.

- **One controls kit defines action hierarchy** (`widgets::button`, `Variant`): Primary
  uses accent fill and `on_accent` text; Secondary uses input fill and a `field_border`
  edge; Ghost has no fill or edge at rest; Danger is neutral with red text. Filled red
  remains reserved for a dialog's main destructive action. Disabled actions are neutral
  and readable. Buttons use the density's control height, 6 pt radius, 10 pt horizontal
  padding and an optional 16 pt Phosphor icon with a 6 pt gap. Labels are sentence case
  and are also their accessible names. Hover outlines are muted. The focus ring uses
  the accent on neutral fills and `on_accent` on accent fills.
  Why: the owner's review found mixed button treatments unfinished. Screen migrations
  adopt the kit incrementally rather than changing their hierarchy in the foundation step.

- **Dropdowns, checkboxes and collapsing controls use drawn Phosphor marks.** Every
  picker uses `widgets::combo`, with a muted 14 pt caret at the right. `widgets::checkbox`
  draws a 14 pt square with 4 pt corners and a control-edge border; checked uses accent
  fill and an `on_accent` check glyph. Its target remains at least 24 pt, with checkbox
  role and toggled state. Sidebar folders and collapsing headers share
  `widgets::collapsing_icon`, switching between right and down carets. The folder toggle
  also reports whether the folder is expanded. Icons are always
  laid out through the isolated icon family. Global stock controls have 6 pt corners,
  input fill at rest and neutral hover/press colors. Why: stock triangles and checks made
  the app look dated in the owner's review; common marks make controls feel related.

- **Short choices have a shared segmented-control API** (`widgets::segmented`): one
  rounded input track with a control-edge border, equal-width segments at control height,
  selected fill with accent SemiBold text, muted regular text otherwise. Each segment
  is a named button with selected state, operable by keyboard and screen readers. Existing
  screen pickers migrate in later steps. Why: short alternatives should be visible together
  and share geometry instead of looking like unrelated buttons.

- **Contextual help uses a tooltip** (`widgets::help`, `label_with_help`): a frameless,
  muted 13 pt info glyph in a 24 pt focusable target, named "Help", with the explanation
  as its accessible description. The tooltip opens on hover or keyboard focus. A caption
  can expose the same explanation on hover.
  Existing persistent help migrates with its screen; errors and empty states stay visible.
  Why: the owner's review found persistent explanations noisy; hover and accessible
  descriptions retain guidance without another line in every form.

  The controls kit's paint geometry, state, glyph families and help semantics are checked
  by `tests/widgets.rs`, which also rejects stock picker and checkbox constructors at
  screen call sites; `theme.rs` pins tokens and contrast in both palettes.

- **Highlighting: small hand-written tokenizers, not syntect.** Why: JSON highlighting of
  256 KiB takes about 2 ms by hand and about 100 ms with syntect, which is also a large
  dependency.

- **Common actions in a toolbar or row are icons; menus are words.** Copy, Copy as curl,
  Refresh, Newer and Older (chevrons), the find bar's Next and Previous, Close (×),
  Browse (a folder) and Save are Phosphor Regular icon buttons (`icons.rs`),
  each named for screen readers and tests and with its words in the hover text. Menu items
  stay text, and so do actions that start a flow (Restore as draft, Retry, Add check,
  "+ Environment"), whose words say more than a glyph could. Why: the same action was an
  icon in one place and a word in another ("Copy as curl" in the request bar and in
  History, the inspector's "Copy value"; the user's note), and text buttons crowded the
  toolbars. The hand-painted set did not scale; gaps forced text buttons. Phosphor provides
  a consistent set of glyphs centred at 16 pt in the existing square targets, keeping the
  muted, hover, selected/focused, ready and disabled colors. `glyph_text` also supports
  icons beside labels. Checked by `support::assert_icon_button` in the History, find and
  inspector tests, the icon interaction tests and the font coverage tests.


### Layout, Panes & Global Navigation

- **The header keeps Requests, Variables and History together.** Mocks, Load and Run live
  in a named Tools menu; application Settings is a gear at the far right, beside Tools and
  the environment picker. Why: variables belong to everyday request work, while the two
  supporting tools need less prominence and settings is application configuration.
  Tools is selected while any tool is open, and each screen has its own title.
  Unsaved mock edits mark Tools even when its menu is closed; the menu names running,
  stopping and unsaved states. Settings keeps its own unsaved marker. Running tools are
  clickable in the status bar, including while starting or stopping, with room reserved
  before the latest result (which truncates with its full text on hover). Load remains
  available from a request's context menu and the palette, keeping its connection to the
  chosen request. No speculative future features determine this menu's shape. Long
  environment names truncate with their full name on hover, so the picker cannot overlap
  navigation. Checked by `imposter-ui/tests/header.rs`, the load and mock UI tests, and
  the layout lint.

- **The layout is a switch, not a mode:** side by side or stacked, changed only by the status
  bar's two layout buttons (the current one selected: one button showing the other
  layout's icon read as the current state), never by the window width. The window can shrink to 720×480 (`MIN_WINDOW`;
  1000×650 took more than half a laptop screen), so narrow panes adapt instead
  of wrapping: single-line fields and button labels never wrap (a long label is cut, with
  the whole text on hover).
  Why: an automatic choice next to a manual one made the button seem broken. The sidebar
  toggle beside them is an action named by what it does (Hide or Show Collections), never
  drawn selected, with its own glyph and a separator: selected like the layouts, it read
  as a third layout (Q4).

- **Split panes keep a minimum in points** (`layout::split`): request and response 220 wide
  or 160 tall, the History list 110 and its details 160; when both minimums do not fit,
  they share in proportion. A drag starts from the size as drawn, not the stored ratio, so
  dragging back from a held minimum responds at once. Each split has a grip in the
  control-edge color (`widgets::split_grip`, ≥3:1), History's included. Why: a ratio alone left a pane with
  only its tab row at 720×480 (the review's #10, #14). Checked by `imposter-ui/tests/fit.rs`.

- **Rows of tabs, tools and status stay one row high; what does not fit goes into a menu.**
  - **Tabs** (`widgets::tab_strip`): tabs that do not fit go under "More", a chevron no
    wider than a target needs; the selected tab always stays, in place of the last one that
    fitted. Tabs come before tools.
  - **Badges** say what a tab holds without opening it (`Editor::badge`): Query, Headers,
    Cookies, Tests and Captures count rows in use (enabled and named); Auth, Body and Docs show a
    small green dot raised to the top of the line when set (a muted "•" at mid-height read
    as a separator, Q2). The request's badges describe the request, never its last
    response; the response's Headers are counted, and its Tests show "✗1" in red when
    checks failed, else their count (a red total read as that many failures, Q5). Screen readers hear the count as the
    tab's description, so its name stays the label. "More" shows how many tabs it hides
    ("+2", so it does not read as the tools' "⋯" beside it; dropped only when even the
    selected tab leaves no room) and the dot when one of them holds something, and names
    those tabs in its description. A screen with unsaved edits (Variables, Settings,
    Mocks) has a dot in the amber of the word "Unsaved", described as "Unsaved
    changes": one muted "•" for both meant two things a few centimetres apart (Q16). It is
    painted over the tab's corner (`widgets::corner_dot`), not written into its text, so the
    tabs after it do not move (the review's #54).
  - **Viewer tools:** those that do not fit go under "More tools" (⋯); fold buttons go
    first, then Wrap, the filter, Find and Copy. In the menu, Wrap and the filter are
    check boxes: a filled row for "on" read as the row under the pointer (Q11).
  - **The response's status** is one line whose parts show in order of importance while
    they fit what the row has left (measured as drawn: fitting them all up front
    overestimated and dropped the checks with room to spare): the
    status as a pill ("200 OK"), the checks as "✓3 ✗1 ?1" (a click opens their results;
    screen readers hear "Checks: 3 passed, 1 failed"), "12 ms", "103 B", the content type.
    The pill's hover text has them all; earlier runs are under a "Runs" menu at its end.
    A transport failure has no pill, only its time: the card below names it, and a pill
    made "Couldn't connect" appear three times with the status bar (Q8). Check results
    list failures first, on a red tint.

  Why: at 720×480 the request tabs took three rows and the response header four before the
  body began (the review's #12, #13). Checked by `imposter-ui/tests/fit.rs`.

- **The status bar tells the latest result, only while it applies** (`app::StatusLine`).
  - **Request messages:** a message about the open request (a send's result, "Not sent", a
    restored draft) lasts while that request and screen stay shown.
  - **Other messages** fade after `STATUS_SECONDS`.
  - **Wording:** results read plainly ("200 OK · 12 ms", "Couldn't connect · 3 ms", "Send
    cancelled", `viewer::outcome_summary`), never internal type names.
  - **During a send** the response pane shows "Sending… n s" with a spinner, and the bar
    does only while that pane is out of sight (another screen): both at once said it twice.
    The pane never invites a send while one runs.
  - **Merging:** a failed check makes the result read "· n checks failed", and a capture
    written right after a send is added to that result (" · set token") rather than
    replacing it.
  - **The right end** shows the mock server's port while it serves; the environment is
    left to the picker and Send.

  Why: the review found it reporting an earlier request's failure after a blocked send, and
  "Ready" during a send. A status that outlives its context misleads more than no status.

- **Each pane says what to do once, as buttons.** With no request open, one pane over the
  whole work area (no request bar, no response pane: empty, they read as a failed render,
  Q7) offers the next step: New Collection, Import Collection… and Paste curl… on first run;
  New Request in an empty collection; otherwise "Select a request". The buttons are
  centred under the text, the first filled as the next step (left-aligned equal outlines
  looked unfinished, Q6). The sidebar,
  breadcrumb and response pane stay quiet meanwhile (they repeated it two or three times,
  the review's #16, #17). Disabled menu items say why ("Create a collection first"),
  under them rather than only on hover; the curl entry is "Paste curl…" everywhere; the
  import review says what it creates ("Creates Pet Store: 8 requests in 2 folders",
  counted at every depth).

- **Shortcuts are one table, `shortcuts::ALL`, handled in one place before widgets draw**
  (`shortcuts::handle`) and doing nothing while a dialog is open. The same table drives the
  shortcut sheet (Cmd/Ctrl+Shift+/: grouped as Navigate, Send, Edit and View, keys drawn as
  caps with "or" between alternatives, which side by side read as a chord), and a test
  checks that the README table equals it. Keys are one cap each, written as keyboards
  print them (`shortcuts::keys`: "Ctrl", "/", "Esc"; "⇧", "⌘" on a Mac), and the palette
  draws the same caps: egui's "Ctrl+Shift+Slash" in one cap had to be translated (Q12).
  The sheet's keys column fits its widest row, so every description starts in line.
  Cmd/Ctrl+R sends too ("Run" in many tools); F2 renames, with Ctrl+Shift+R because F2 needs Fn
  on Mac keyboards (Yaak's choice). egui ignores extra Shift/Alt when matching, so the
  handler consumes the most specific combinations first. Find, Alt+Enter and Esc are listed
  but stay with the widget they act on (Find with the viewer under the pointer). Why: one
  handler means a body editor cannot swallow Send and a dialog cannot be bypassed; one table
  means the help and the README cannot drift from the keys.

- **The command palette (Cmd/Ctrl+K) searches actions, the requests of every collection and
  environments** (`palette.rs`, from Yaak). Requests come from `collections::request_index`,
  read when the palette opens through the generated columns (no template is decoded); actions
  and environments are captured at open. Matching is an in-order fuzzy subsequence that
  rewards word starts, the very start and runs, over the best of all placements
  (`palette::fuzzy_match`; taking the first letters found ranked "Keyboard Shortcuts" above
  "Order" for "ord"). Of equal placements the last wins, so the request's name is marked
  rather than a folder before it, and a match averaging under 2 points a letter
  (scattered letters, no word start or run) is dropped as noise (Q13). Environment rows
  are named alone under their heading. Matched letters are semibold, and rows fill the palette's width
  with their shortcut at the right; while typing, the group with the best match comes first so a
  strong request match is not buried under a weak action match. Choosing returns an ordinary
  `Action`, so the unsaved-changes guard applies; request rows put the method in its color in
  a column of one width (OPTIONS) with a gap before the name, so DELETE never runs into it; a request in another collection opens
  through `Navigation::OpenIn` (switch collection, then `restore`). The palette is pinned to
  the top: centred, it moved as its list shrank while typing, and clicks landed on the
  backdrop.

- **Recent requests: the last 20 opened saved requests, newest first, in `ui_state`
  (`recent_requests`)**, pruned when the request index shows one was deleted; an unreadable
  list starts empty instead of failing startup. Cmd/Ctrl+P is the palette with requests only,
  recent first. Ctrl+Tab (the real Ctrl key on every platform) is hold-and-release, as in
  editors: Tab steps while Ctrl is held, releasing Ctrl opens the selection, so a quick
  Ctrl+Tab goes back to the previous request; Shift starts from the oldest; Esc cancels. A
  release that comes before the index loads opens as soon as it arrives. Why: the
  back-and-forth between two requests is the most common jump.

- **The sidebar filter is a small query language** (`imposter_app::filter`, after Yaak's): words combine
  with AND, plus OR, NOT/`-word`, `"phrases"`, parentheses, and `method:` (whole method),
  `url:` and `name:`. Plain words match the request's folder path, name and URL, so typing a
  folder name shows its requests and a negation never brings a whole folder back; folders
  left without matches are hidden. Text that does not parse is searched as typed. Filtered
  folders are drawn open under their own collapse ids, so clearing the filter restores the
  folders the user closed. The tree's names, methods and URLs come from memory. Cmd/Ctrl+F
  over the sidebar focuses the filter,
  as Find elsewhere applies to what is under the pointer. The clear button sits inside the
  field's right padding, so typing never changes the field's or the sidebar's width (the
  review's #52), and "5 of 9 requests" shows under it while filtering.

- **The tree is refreshed after a save that changed a request's method or name**, so it never
  shows a stale name.

- **The tree is reordered by dragging, by Move up and Move down, or by Alt+↑/↓.**
  - **Drop zones** (`workspace::drop_zone`): on a folder, the top and bottom quarters drop
    before and after it and the middle into it (last); under an open folder's header the
    drop goes first inside it. A request row has halves. A 2-point accent line between rows
    shows before and after, and an outline shows into.
  - **Store:** one command (`collections::place`) moves the node and numbers its new
    siblings 0, 1, 2… in the order the tree shows, folders and requests mixed, so no two
    share a position. Move… and a drop on the collection root put it last. The unused
    `reorder`, which numbered folders before requests, is gone.
  - **While filtering** only drops into folders work, and Move up and down are off:
    hidden siblings make "before X" ambiguous, and a step past a hidden one looked like
    nothing happened.
  - **Keyboard:** Alt+↑/↓ move the selected node, except while a text field has focus,
    where they move the cursor (a field is recognised by its stored edit state, since
    egui reports any focused widget as wanting the keyboard).
  - **Long trees** scroll while a node is held within a row of the tree's top or bottom.

  Why: drags only went into folders, so the order inside one could not be changed, and
  dragging cannot be done from a keyboard or screen reader. Checked by
  `imposter-ui/tests/editing.rs` and the store's `placing_*` tests.

- **Request names stay on one line with ellipsis and full text on hover; breadcrumbs reveal their full path on hover.** Why: long names must not expand a sidebar row or hide their identity. Accessible names remain complete. Enforced by `imposter-ui/tests/editing.rs`. `{{variables}}` in a name are drawn in the accent, as in the editor, and the sidebar's edge has a grip like the splits', since it can be dragged wider (200–400 points, remembered) and nothing said so (the review's #59).


### Request Bar & Editors

- **The request bar is one control:** method, URL and Send share one edge
  (`field_border`) on the input fill, with a divider after the method and Send filling the
  right end; the fields inside draw no frame of their own (`workspace::frameless`), and the
  focus ring goes around the whole bar. Single-line variable fields centre their text
  vertically. A custom method being typed takes the method picker's place, so the URL
  keeps its room at 720 points. Save's icon is in the text color while there is something
  to save (`icons::ready_button`), muted at rest and faded when disabled. Why: the bar was
  a frame in a frame in a card, its URL text sat 6 points above centre, and enabled and
  disabled Save looked alike (the review's #22–#24). Checked by
  `imposter-ui/tests/request_bar.rs` and the layout lint's custom-method state.

- **Request tabs start on Body and run Body, Auth, Query, Headers, Cookies, Settings, Docs, Tests, Captures, Scripts.** Docs edits the existing plain-text description through the normal save guard and exports; Tests edits the request's checks and Captures the variables it sets from a response, each tab counting its own rows ("Set variable" from a JSON row opens Captures). Why: body editing is the primary workflow; descriptions and checks belong with the request rather than in a separate document; checks and captures in one tab under two headings made captures hard to find (user's note). The response keeps one Tests tab for check results and capture writes, both results of the run. Enforced by `imposter-ui/tests/editing.rs`.

- **The body editor fills its pane**, above a one-line footer ("Valid JSON" or the parse
  error with its line, then Format), and is never shorter than about four lines. It
  scrolls its own text in a scroll area sized from the pane's content rect, which moves
  with scrolling, never from the clip rect, which does not: sized from the clip rect it grew
  by every point scrolled, so a long body never reached its end (user's note; checked by
  `scrolling_a_long_body_reaches_its_end`). A
  `{{placeholder}}` outside a string stands for a value, so `{"n": {{n}}}` is valid
  (`viewer::check_json_template`). Format (the button, Cmd/Ctrl+Shift+F or the palette)
  re-indents with two spaces, keeping every token and placeholder as typed
  (`viewer::format_json_body`), and a JSON document pasted into an empty body is formatted
  at once. Typing is never reformatted, since re-indenting under the cursor fights it, but
  it is helped locally as code editors do (`json_typing`, JSON bodies up to 64 KiB):
  `{`, `[` and `"` close themselves where there is room and outside strings (so `{{`
  becomes `{{}}` and the name suggestions still open), wrap a selection, and a typed
  closer steps over the one there; Backspace removes an empty pair; Enter keeps the
  indent, one level more after an opener, with the closer on its own line; a closer on a
  line of indent goes one level out. The help is computed from the text before the key
  and applied only when egui made exactly the plain edit, so a paste, IME or an accepted
  suggestion is left as egui made it, and undo never stops at a half pair. Why: a pasted
  minified body stayed one line, and the footer called every templated body invalid; JSON
  typed by hand needed every closer and indent typed too (the user's notes). Why: a fixed
  ten rows left most of a large pane empty (the review's #20).

- **Body editor: egui `TextEdit`; `{{var}}`/JSON highlighting only up to 64 KiB.** Why: per
  interactive frame the editor costs about 4 ms at 64 KiB, 24 ms at 256 KiB and 130 ms at 1 MiB
  (plain), and highlighting adds about 30%. Larger bodies stay editable but slow; a custom
  virtualized editor would be needed to fix that and is out of scope.

- **Variables in the editor:** fields mark `{{name}}` tokens with the highlight pattern
  `\{\{([^{}\n]+)\}\}` (stricter than the resolution pattern, so half-typed text is not marked): defined names and
  generated values in the accent color, unknown ones in the error color on a red tint
  (the two colors are equally bright, so hue alone failed colour-blind users). Typing `{{` suggests names with their scope and
  source, never values; the list opens only from typing in an unfinished token (clicking into
  a token only inspects it) and can be chosen by keyboard or pointer. Values appear only in the inspector, opened explicitly (click on a token
  or Alt+Enter); from a password field they stay masked until "Reveal". The inspector and
  execution share `executor::inputs`, so it explains exactly what a send would use.
  It opens under the token, with a pointer up to it (frame and pointer edged in the
  control-edge color, ≥3:1, Q21), and says where each value comes from
  in words ("Defined in the Dev environment (overrides Global)", then "Overridden in
  Global"; `vars::describe`). It closes with × at its top right or Esc, and "Copy value"
  copies what a send would use, masked or not, since copying is explicit. Why: opened at
  the field's edge it pointed at nothing, and "Effective · Environment · Dev" read as
  internal labels (the review's #36).

- **The Auth tab is a two-column grid** (`widgets::auth_form`): one label column, fields to
  the pane's edge (sized from the pane, since a grid cell sees only itself), and an eye
  button after each secret (password, token, key value) that shows or hides it for the session; the
  choice is never stored. Why: labels were indented differently, the field had a fixed width,
  and a typed secret could not be checked (the review's #25).

- **The request's Settings tab is one row per setting** (`widgets::options_form`), under a
  muted heading per group: the label in a 150-point column, then the control. A text
  setting shows the application default in its empty field ("Default: 30 seconds");
  typing makes it the request's own, and a reset icon after it ("Use application default
  for …") goes back. An on/off setting is a choice of "Default (On)", On and Off. In a
  pane too narrow for the column each control goes under its label. Why: each setting took
  three or four rows (label, a "Use application default" box, the field greyed out, a
  hint), about 35 rows for ten settings, and the useful value sat in the small grey hint
  (the user's note).

- **A curl command pasted over the whole URL field replaces the open request** (after Yaak):
  method, URL, headers, cookies, auth, body and options come from the command; identity,
  name, description and place stay; the result is an ordinary unsaved edit (autosave or Save
  applies) and ignored options are listed afterwards. Only a paste that would replace the
  whole field (empty, or all of it selected, as after Cmd/Ctrl+L) and that parses as curl
  is taken (`widgets::take_whole_paste`, before the field draws); any other paste is text.
  Why: updating a request from a browser's "Copy as cURL" is one paste, while pasting into
  part of a URL can never replace a request by surprise. + → Paste curl… still opens a draft.

- **The Tests and Captures tabs edit checks and captures in a fixed grid** (`checks.rs`):
  every row has
  the same cells, and a source without a path (status, body, duration, size) or an
  operator without a value leaves its cell empty, so the columns stay put. By the pane's
  width a row takes one line, two (what to read, then the comparison), or one line per
  part, as at 720 points with the sidebar shown. The last field of a line takes what the
  line has left when drawn, since a picker grows past its width for a long label ("does not
  contain"), and the check box is as wide as a control in the density. Each capture shows
  the scope it writes to ("→ Dev"), computed as the
  executor does: the defining scope, else the active environment, else Global. Why:
  operators jumped between rows, a narrow pane stacked five lines per check with fields cut
  at its edge, and nothing said where a capture would write (the review's #31, #32).

- **Duplicate creates a fresh saved sibling, including current editor content when open, through the navigation guard.** Name allocation and insertion are one transaction; history stays with the original. Why: copying must neither lose edits nor overwrite an existing request. Enforced by app operation tests and `imposter-ui/tests/editing.rs`.


### Response Viewer

- **Response viewer: virtualized rows, never one big text widget.** Only visible rows are laid
  out (`ScrollArea::show_rows`, monospace, wrap by column count). Why: a 5 MiB read-only
  `TextEdit` costs about 730 ms per frame; virtualized rows cost about 0.1 ms, highlighted or
  not. Measured by `cargo run --release -p imposter-ui --example text_perf` (M-series Mac).

- **The preview stays at 256 KiB; the whole body is one click away.** When the preview is
  shorter than the body, the pane says so precisely ("Preview shows 256 KB of 584 KB", or
  "Body cut at … when captured") and offers Save body… (written atomically on the bridge,
  `transfer::write_file`) and Copy full body, in Requests and History. Why: the limit keeps
  the viewer fast; the old "truncated or preview limited" left no way to the rest. The
  notice is one line, cut if need be, with the two actions as icons at its end: wrapped
  words and buttons took three rows from the body at 720 points (Q14). A cut JSON body is
  still re-indented as far as it goes (`viewer::pretty_json_start`, when the parser only
  ran out of input), without folding, which needs the whole document: one minified line
  wrapped mid-token (#26).
  Every response body can be saved as received: Save body… ends the Body tab's tools
  (beside the notice instead when the preview is cut), named for its content type
  (`viewer::body_file_name`: `.png`, `.pdf`, `.bin` for unknown bytes). A binary body
  (`viewer::is_binary`: an image, audio, video, font, PDF, archive or
  `application/octet-stream` type, or a NUL byte in the first 8 KiB without a type that
  says text) is described ("Binary body", its size and type) instead of shown as Latin-1
  noise, with no Find, Wrap or Copy, since bytes copied as text are damaged. Text that is
  not UTF-8 is still text. Why: small binary bodies could not be saved at all, and showed
  as garbage (the user's note). Not done: showing images.

- **Response headers are a table by default** (`widgets::headers_table`): names muted,
  values selectable and wrapping at words, a copy button on the row under the pointer.
  Where values would get under 160 points, each goes under its name: narrow columns broke
  "content-length" and "application/json" mid-word (Q9).
  "Raw" switches to the text viewer with Find and Wrap. Why: a raw block broke values
  mid-word in a narrow pane (the review's #13, #27). Raw is a framed toggle at the right
  end of the tab row, and the tabs leave room for it: drawn after the tabs in their style
  it read as a fourth tab, and without room the copy icon covered it (Q1, Q15).

- **JSON navigation keeps the virtualized rows.** Folding hides row ranges computed from an
  outline of the formatted text (`viewer::Outline`: path, depth and closing line for every
  line), so nothing is laid out that is not visible. A row with a fold marker is exactly as
  tall as a plain row, because the scroll area places rows by a fixed height. Paths are
  RFC 9535 JSONPath, written one way everywhere (`select::member`: `.name` when the shorthand
  allows it, `['name']` otherwise), so a copied path works in a filter, and later in checks
  and extraction. JSONPath comes from `serde_json_path` (pure Rust); a hand-written subset
  would differ from what users read in the RFC.

- **JSON rows offer their actions without a right-click.** The line under the pointer is
  highlighted with a "⋯" at the viewer's right that opens the row menu (Add check, Set
  variable, Copy path, Copy value), headed by the row's path, under the line so it stays
  in view; both stay while the menu is open. Right-click opens the same menu. Wrapped
  lines stop short of the "⋯", which sits on the row's fill where a line still reaches
  it (Q10, #30). Why: the actions were found only by accident (the
  review's #30).

- **The response filter runs over the whole captured body in the background**, parsed once
  per response and kept for later queries; one run is in flight at a time and the newest query
  follows. A late result for a replaced response is dropped. Results are re-serialized, so
  numbers keep their value but not their spelling (`1.50` becomes `1.5`); the formatted body,
  "Copy value" and the preview keep tokens exactly as received. A truncated body is refused
  rather than reported as invalid JSON.

- **Every problem in the response pane says what happened and what to do next.**
  - **Transport failure:** a plain headline per error kind and a hint (`viewer::failure_title`,
    `failure_hint`); the raw library text sits under a collapsed Details; Retry appears in the
    open request's pane, not in History.
  - **A send that never went out** (`app::NotSent`): the reason replaces any earlier response,
    which would read as this send's. Undefined variables come as names
    (`ExecuteError::Unresolved`), so the pane offers "Define in <active environment or
    Global>" under the headline that names `{{x}}` (its accessible name is the whole
    "Define {{x}} in …", since a long name cut the destination off). That opens Variables
    with a new unsaved row; nothing is written until Save.

  Why: the review found raw `os error 61` text clipped at the pane edge, and an empty-state
  invitation shown right under "Request failed".


### State, Saving & Dialogs

- **The saver is plain UI-side state** (`imposter_app::saver`): one write in flight at a time,
  the newest snapshot per request, failures kept for retry or explicit discard. The UI runs
  the writes through the bridge.

- **Every navigation that replaces the editor goes through one guard**: clean → proceed; saved
  request with autosave on → save first (failure asks Retry/Discard/Cancel); otherwise ask
  Save/Discard/Cancel. For an unsaved draft the guard asks "Keep this draft?" and names the collection
  Save adds it to (Save as new request / Discard / Stay), because "Save changes to …" read as
  overwriting the request the draft came from. Closing uses the same guard after background work drained, then writes
  UI state (layout, last selection, window) and closes; any failure keeps the window open.

- **Every screen saves the same way:** a disk icon at the top right, with its state in a word
  when the icon cannot show it
  (`widgets::save_controls`); Variables adds a revert icon. The header logo is the app icon
  drawing, like the window and Dock icon.

- **Save states are one word** (`widgets::SaveWord`: "Saving…", "Save failed", "Invalid
  fields", "Loading…", "Not loaded") beside the Save icon, colored by tone, in the request
  bar, Variables, Settings and Mocks. Saved, Unsaved and Draft are not written: the icon is
  in the text color while there is something to save, and a draft has its banner. Every
  state is the icon's accessible description, with the explanation in its hover text. A
  longer reason (a duplicate name, a failed write) goes to the status bar, never into the
  word. In the request bar Copy as curl sits between the icon and the word
  (`widgets::save_controls_with`), so a word that comes and goes, as "Saving…" does on every
  autosave, never moves it (the user's note). Save is an icon button named by its action. Section headings and explanatory
  subtitles are left out where tabs and controls already say it. Why: user review found the
  longer texts noisy, and later that "Unsaved" beside a highlighted Save said it twice.

- **Every action that removes saved data asks first, in a dialog that names what goes**
  (`Dialog::delete_collection`, `delete_node`, `delete_environment`, `clear_history`,
  `clear_variables`, `revert_variables`): "Delete \"API\" with 8 requests in 2 folders",
  "Delete 312 executions from history?", and the red button says what it does ("Delete
  collection", "Discard changes"). Menu items that open one end with "…". Revert on
  Variables asks too (Mocks' goes through its unsaved-changes guard). Edits inside a form
  (a row's ×, a table's "Clear all") keep their two-click or inline question, since Save
  has not written them yet. Why: a misclick must never lose data (user's rule); a
  question that does not say how much goes is answered without reading. Checked by
  `imposter-ui/tests/destructive.rs`.

- **Removing a row takes two clicks** (`widgets::remove_button`): the first arms the × (red,
  "Click again to remove …"), the second removes; it disarms after a few seconds or when the
  pointer goes elsewhere. Why: one stray click must not lose a header or variable; this works
  from the keyboard and screen readers, unlike a double-click.

- **File dialogs go through one seam** (`ImposterApp::pick`); tests answer them with
  `script_file_dialogs`, since native dialogs cannot be driven headless.

- **Every dialog ends with the same button row** (`dialogs::buttons`), except a sheet with
  nothing to choose (the shortcut sheet): it says "Esc to close" beside its title, since Esc
  and a click outside close every modal, and a lone Close button only took a row. The
  find and filter bars keep Close: Esc reaches them only while their field has focus.
  - **Order:** the choices and Cancel come first; the main action comes last, filled with
    the accent (or red when destructive) and labelled with a verb (Create, Rename, Save,
    Delete, Import).
  - **Fill:** only the main action is filled. A destructive choice before it (the guard's
    Discard) is a neutral button with red text, because filled red beside filled orange
    differ only in hue (1.02:1); a disabled button is neutral, so its label stays readable.
  - **Keys:** Enter presses it unless the dialog edits multi-line text (curl); a name field
    reads Enter before drawing, because a single-line field consumes it. Escape cancels.
  - **Captions** have no trailing colons.
  - **Height:** dialogs are capped at the window, and long ones scroll inside, with their
    text wrapped clear of the scroll bar.
  - **Layout:** the row is laid out left to right, because a right-to-left row in an
    auto-sized modal put its buttons outside the modal, where clicks missed.

  Checked by `imposter-ui/tests/dialogs.rs`.

- **Each dialog kind has its own modal id.** Why: a dialog replacing another (the import review
  followed by the unsaved-changes guard) inherited the old one's area, so a click on its buttons
  counted as a click outside and cancelled it.


### Screens (Variables, Run, History, Settings)

- **The Variables screen edits one scope at a time** (Global, an environment, a collection),
  independently of the active execution environment, with its own unsaved-changes guard for
  scope changes, reloads, environment management, "Open definition" and closing. Closing
  resolves the request guard first, then the variables guard, then writes UI state.
  The scope picker is captioned "Editing"; its list marks the active environment
  ("(active)"). Under it, one sentence says what the scope is for ("Used by every
  request.", "Dev is the active environment: requests use these now.") and one line where
  it stands in precedence (`variables_screen::explanation`); the name is not repeated,
  since the picker shows it. The toolbar has a labelled "+ Environment"; editing the name
  and color, deleting and "Clear all variables…" are in its "⋯" menu, and clearing asks
  in a dialog with the count. In environment lists, names without a color keep the dot's room,
  so all start in line. Why: the screen did not say which scope wins, nor whether the
  environment shown is the one sends use, and "Clear All" sat beside "Add Variable" (the
  review's #37, #39; Phase 5 review Q22, Q25).

- **Settings save only on Save Settings**, against a baseline: undoing an edit is not a change. Settings the screen does not
  show are kept. Closing asks about unsaved settings after the request and variables guards.
  Saving applies history retention at once, not at the next send, so the
  limits the History screen shows are already true. The screen is one form: section headings
  instead of framed groups, one label column shared by every section (`LABEL_COLUMN`), at most
  720 points wide, with a line of help under fields that need it. Every path field has
  Browse… (through `ImposterApp::pick`) and says "File not found" when the path names no file;
  the check runs on the bridge (`transfer::is_file`) and is redone when the screen is opened.
  "Verify TLS certificates", not SSL. Why: label columns differed per group, paths had to be
  typed, and a wrong path showed only on the next send (the review's #50, #51).

- **The Run screen runs a collection or a folder in the app** (`run_screen.rs`, Tools →
  Run, "Run…" in the collection's ⋯ menu and a folder's context menu, and the palette's
  "Run Collection" and "Go to Run"), through the same runner as `imposter run`. Its target
  is chosen by id (`profile::select_under`), so a renamed folder cannot make it run
  another. Opened with nothing chosen it takes the open collection.
  - **Environment:** the active one by default, changeable for the run (user's choice);
    with a red one Start asks first ("Run against Prod?"), as Load does, instead of the
    command line's refusal: the person at the screen is the one deciding.
  - **Captured values stay in the run**, never written back (user's choice), as on the
    command line. Each request sent is recorded in History as the app's (`Via::App`).
  - **The open request's unsaved edits are not sent**: the run reads every request as
    stored when it starts, and one line says so when the open request is inside the
    target. Sending the editor's copy for one request of a run would make the run depend
    on what happens to be open.
  - **Progress:** the runner's `on_step` pushes each step into a shared list the screen
    reads every frame, as Load reads its counters; the summary uses `run_report::outcome`.
    The status bar shows "Run running" ("Run stopping…" after Stop), and Stop cancels the
    request in flight and skips the rest.
  - **Results:** each row is the outcome word in its tone and the path; selecting one
    shows its response, checks and request as sent in the response viewer, with its
    JSONPath filter (`PaneKind::Run`). The last ten runs of the session are kept in
    memory, as Load's are. The list and the details are laid out in fixed rectangles: in
    a horizontal row the response tools measured another width and covered the tabs at
    720 points (the layout lint).
  Pinned by `imposter-ui/tests/run.rs` and the layout lint's Run states.

- **History opens a headless result or report** through “Open result file…” or the
  palette. It shows the command, target, environment, run ID, revision and outcome.
  Reports choose a step to view the same request, response and verdicts; a skipped
  step shows its reason. The file is read on the bridge with the import size limit.
  Restore and Copy as curl stay unavailable because the file holds redacted evidence.
  Why: someone can explain a failure without reconstructing a run or reading logs.
  The History result tests open files written by the real CLI.

