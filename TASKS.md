# webgen-word — open tasks

Written 2026-08-01 before a reboot, so nothing lives only in a session's head. Update it in place;
it is the resume point for whoever picks this up, including a later me.

## Where things stand

| | |
|---|---|
| version | **0.8.0** (`0f855dc`), pushed to `piersrowan/webgen-word` `main` |
| OS pin | `wanted: 0f855dc` set in `webgen-distro/INTEGRATION.md`. **Not my task** — Piers has the OS session shipping on its own cadence, and they update the documentation after |
| installed locally | `webgen-word` 0.8.0-1 (`/usr/bin/webgen-word`, sha matches the release build) |
| tests | 103, `cargo test`; clean-clone build verified |

## Mine, and outstanding

1. **Convert the document style sidebar to `StyleRows`.** `src/stylerows.rs` is the shared
   property-row widget the table window's CSS panel uses. `src/sidebar.rs` still has its own copy of
   the same rows, written before the extraction. They agree today only because they were the same
   code an hour before; they will drift, and the drift will show up as "this worked in the table but
   not in the sidebar". Mechanical change. Verify afterwards by re-running the two driving scripts
   that already exist for the sidebar and for the a1–a5 undo sequence.

## Waiting on a decision from Piers

2. **In-app pagination.** The single investment that would unlock, all at once: headers, footers,
   page numbers, a true page count, a real multi-page view on screen (right now the sheet just grows
   instead of becoming page 2), and links-in-PDF *if* UAT-0047 comes back negative.
   Measured groundwork, so this is not speculation:
   - `webkit6::PrintOperation` exposes `page_setup`, `print_settings`, `print`, `run_dialog` and two
     signals. **No `draw-page`, no header/footer** — WebKit owns the rendering and we cannot draw on
     the printed page. `gtk::PrintOperation` *does* have `connect_draw_page`, but WebKit builds its
     own internally and never hands it over.
   - CSS `@page` margin boxes are not implemented by WebKit, consistent with the already-measured
     fact that `@page` is ignored on this print path entirely.

   The route is: measure the content, slice it into page-sized sections, emit header/footer/number
   per section, then print. Real work, fully under our control.

## Unverified, and honestly so

3. **Every `gtk::FileDialog` path** — Open, Save As, Insert picture, *Save a copy as one file*,
   *Save a copy as .wgz*. The xdg-desktop-portal file chooser does not work under the headless
   Xvfb rig, so the choosers themselves cannot be clicked through here. The code behind them is
   covered by tests and by two end-to-end runs (a referenced picture landing in `cats_files/` on
   save, and Ctrl+K producing a link in the saved file), but the dialogs need a screen.
   → **UAT-0045, UAT-0046** in `webgen-distro/UAT.tsv`.

4. **UAT-0043** print geometry end to end, **UAT-0044** picture portability, **UAT-0047** whether
   WebKit emits PDF link annotations. 0047 is the one that matters beyond itself: it decides whether
   links in a PDF are ever possible without item 2.

## Asks on other sessions (all recorded in `webgen-distro/INTEGRATION.md`)

5. **Files + Settings sessions** — add `Program { command: "webgen-word", name: "Word", icon:
   "com.webgen.Word" }` to `assoc::PROGRAMS`. It is a hardcoded const duplicated verbatim in both
   repos, so both must change together. Until then Word cannot be offered in Files' "Open With…" for
   an `.html`, no matter what its `.desktop` says — that dialog does not read `MimeType=` lines.
6. **Settings session** — `webgen-settings` `acbc27e` adds the `colour` manifest row type that
   Word's panel needs. Pushed deliberately **unversioned**; the bump and the pin are theirs.
7. **Browser session** — its `docstyle.rs` keeps only the selectors and properties it recognises and
   re-emits the block, so a document carrying `.wg-i1` instance rules, `div`/`span` rules, or
   `font-weight`/`text-decoration`/`width` **loses them silently** if opened in its editor and
   re-applied. Word reads the browser's older blocks unchanged, so the loss runs one way. Fix is
   either "preserve what you do not understand" or adopt the v2 format. CONTRACT §4c.

## Things worth not rediscovering

- **`@page` does nothing** on WebKit's GTK print path. Geometry comes from `gtk::PageSetup`.
- **Sanitise before `load_html`.** A `<script>` handed to WebKit has already run by the time any
  DOM-level cleanup could reach it.
- **A `GestureClick` on a `WebView` never fires**, in either propagation phase. Put it on the window
  and translate the coordinates — and use `pressed`, not `released`, because WebKit claims the
  sequence in the target phase and cancels the gesture before any release arrives.
- **`AdwMessageDialog` stacks responses in reverse order of addition.**
- **A bare number is not a CSS length.** `padding: 20` does nothing and looks exactly like the
  setting being ignored — hence the unit pickers.
- The undo fingerprint must ignore `wg-iN` handles **and** the empty `class=""` they leave behind,
  or minting a handle reads as a text edit and undo takes back the wrong thing.
- Local deb install applies to **every repo a turn pushed to**, not just the app I own; and
  `which -a` before claiming an install took effect (a stale `/usr/local/bin` copy once shadowed
  the packaged `webgen-settings` — removed 2026-08-01).
