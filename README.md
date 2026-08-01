# webgen-word

A light word processor whose document format is **plain HTML**.

A CV, a letter or a report is a *document*, and HTML+CSS already describes documents perfectly well.
So the file on disk is a standalone `.html` that any browser opens, and "export to PDF" is just
printing. No proprietary container, nothing to convert, nothing to lose if the app goes away.

## A word processor, not an HTML editor

That distinction decides most of the design. A document here holds **text, structure, pictures and
style — and nothing else.**

| Thing | What happens |
|---|---|
| `<script>`, `on*=` handlers, `javascript:` URLs | removed |
| `<iframe> <object> <embed> <canvas> <svg> <audio> <video>` | removed with their contents |
| `<form> <input> <button> <select> <textarea>` | tag removed, **text kept** — a document is not a form, but its words are still words |
| `<base>`, `<meta http-equiv=refresh>` | removed — both re-point the document out from under itself |
| pictures, local or relative | **embedded** as `data:` URIs, so the file stays one file |
| pictures, remote | removed, and counted |
| `url(…)` in CSS, local | embedded |
| `url(…)` in CSS, remote | removed **unless** it is a font or a stylesheet |
| `<link rel=stylesheet>`, font links, `@import` | kept — the one allowed outward reference |
| hyperlinks | kept: a link is a reference, not an imported asset |

**The ordering matters.** The sanitiser is a text pass in Rust that runs on the source **before**
`load_html`. A `<script>` in a document handed to WebKit has already run by the time any DOM-level
cleanup could reach it, so cleaning afterwards would be cleaning up after the thing you were trying
to prevent. It runs a second time on save, over the DOM, which is what catches paste.

Nothing is silent: everything removed is counted and named in a banner. `enable-javascript-markup`
is off as well — belt and braces, and `enable_javascript` is deliberately *not* off, because the
app's own `evaluate_javascript` is how saving, styling and page setup work.

## Style: base plus per-document

Two stylesheets, and the split is the point.

- **Base** — `<style id="webgen-doc-style">`, generated from the shared registry and edited in
  **System Settings → Word**, rendered generically from `packaging/settings/com.webgen.Word.toml`
  (CONTRACT.md §2/§3). Word writes no GTK for that panel.
- **This document's own** — `<style id="webgen-doc-custom">`, edited in the **style sidebar**,
  written in a fixed layout so it reads back exactly as it was written, and carried **in the file**
  so it survives being emailed. It holds both `img { … }` rules (every picture) and `.wg-i1 { … }`
  rules (one picture) — see below.

The format — element ids, `wg-*` picture classes, styleable tags, property order — is **shared with
the browser's editor** (CONTRACT.md §4c). A document styled in one opens correctly styled in the
other.

## What it does

- Rich-text editing via `WebView::set_editable` — WebKit supplies the caret, selection, undo,
  clipboard and IME behaviour; the toolbar drives it through `execute_editing_command`.
- Paragraph styles (Body / Title / Heading / Subheading), bold, italic, underline, bulleted and
  numbered lists, indent/outdent (which is how you make a **sub-list**, also Tab/Shift+Tab), four
  alignments, remove-formatting, undo/redo.
- **Pictures**: insert (embedded immediately), then right-click → *Picture* for align left / centre /
  right, wrap text left / right, and scale to 25 / 50 / 75 / 100%. Float, alignment and the rest of
  what CSS can say about a picture are reachable in the style sidebar, for every picture or for
  one.
- **Insert page break** — the one thing a CV needs that HTML has no key for.
- **Page setup**: paper size and all four margins, carried in the document and optionally saved as
  the default for new ones.
- Open, Save, **Save As**, Close; print, which is also how you get a PDF (Print → Print to File).

Every control is icon-only with a tooltip, and every icon name was checked against the shipped
Adwaita theme before use.

## ⚠ The thing that is not obvious: CSS `@page` does nothing

Measured before this app was written, with WebKit's own print path:

| Asked for (CSS `@page`) | Got |
|---|---|
| A4 | **US Letter** (215.9 × 279.4 mm) |
| 45 mm top / 40 mm left margins | **10 mm / 7 mm** — GTK's defaults |

WebKit's GTK print backend takes page geometry from `gtk::PageSetup` and **ignores the stylesheet
entirely**. Setting a `PageSetup` explicitly was then verified to produce a true A4 page.

That single fact is why `page.rs` exists, why Page setup is a menu item rather than a CSS comment,
and why the default is A4/20 mm rather than whatever GTK would have picked. A document's `@page`
rule is still written into the saved file — it is correct for *other* renderers (a browser's own
print, `weasyprint`) — but it has no effect here and the code says so.

## Identity (CONTRACT.md §1)

| | |
|---|---|
| appId / `StartupWMClass` | `com.webgen.Word` |
| binary | `webgen-word` |
| display name | `Word` |
| icon | `com.webgen.Word` — deep blue `#1c4f9c`, page glyph |
| file arguments | `webgen-word FILE` and "Open With" both work (`HANDLES_OPEN`) |
| settings | registry namespace `com.webgen.Word`; manifest at `/usr/share/webgen/settings/com.webgen.Word.toml` |

## Build

```sh
cargo build --release
cargo test
```

Needs `libwebkitgtk-6.0-dev`. Same `gtk4 0.9` / `libadwaita 0.7` / `webkit6 0.4` line as the
browser, so both link one gtk4-sys and the OS builds one WebKit. `webgen-registry` and
`webgen-swatch` are pinned to the same revs the browser uses.

## Verified

- `cargo test` — 63 tests: the sanitiser (including a script containing markup, nested drops,
  idempotence, and that a clean document comes back byte for byte), the page-setup round trip
  through the document meta, the style round trip through the fixed layout including instance
  rules, selector validation, and document preparation.
- Driven under Xvfb + XTEST and screenshotted: opens a file, edits it, the window titles after the
  file name, Ctrl+S writes, the close-confirm names the document with Cancel as the default.
- **The sidebar's whole scoping model, driven end to end.** Two pictures: border set to red in *All*
  scope → both red, `img { border: 1px solid red; }` and no classes on either. Toggle to *This one*,
  border green → one green and one red on screen, `.wg-i1 { border: 1px solid green; }` added and
  `class="wg-i1"` on that picture alone. Toggle back to *All*, Apply → the `.wg-i1` rule is gone from
  the file and the class is gone from the element. Reopening that document and clicking the
  overridden picture opens on *This one* showing green; clicking the other opens on *All* showing red.
- Tree navigation: `<SPAN>` → up → `<LI>` → up → `<UL>` → down → `<LI>` → down → `<SPAN>`, with the
  arrows greying out at the ends.
- **The doctype is written.** The 0.2.0 bug — `outerHTML` omits it, so every document Word saved
  opened in quirks mode in every browser — is fixed, covered by a test, and confirmed by a
  save-and-read-back run.
- A two-page CV prints correctly through this WebKit: forced page break honoured, `break-inside:
  avoid` keeps job blocks whole, headings do not strand at a page foot.
- `PageSetup` produces a true A4 MediaBox (210 × 297 mm), overriding GTK's Letter default.

Found by that driving, and fixed: saving used to strip the sidebar's cursor class from the **live**
DOM, so after one Ctrl+S the sidebar had no selected element and silently stopped responding. The
markers now come off, the document is serialised, and they go straight back on.

Not verified: on-screen editing on real WebGen hardware under labwc, the print dialog's "Print to
File" flow end to end, and System Settings rendering the `colour` rows. All need a screen — see the
UAT queue.

## The element style sidebar

Click anything in the document and the sidebar shows that element's style. The title is the resolved
element — **`<LI>`** — and the arrows walk the tree: up to the parent, down to the first element
child. Given `<ul><li><span>Test</span></li></ul>`, clicking the text resolves to `<span>`; up lands
on `<li>`, up again on `<ul>`, down back to `<span>`.

### All instances vs this instance

Every edit lands on one of two selectors, chosen with a toggle:

| Scope | Writes | Means |
|---|---|---|
| **All `<img>`** (default) | `img { border: 1px solid red; }` | every picture in the document |
| **This one** | `.wg-i1 { border: 1px solid green; }` | that picture alone |

So: click a picture, give it a red border — **all** pictures are red. Toggle to *This one*, change
the border to green — that picture has a **one-line override** and the rest stay red. An instance
rule beats the tag rule by CSS specificity, so the override is exactly the properties it names; the
green picture keeps everything else the red rule gave it.

The element gets a minted `wg-iN` class only if it has no `id` of its own to use as a handle.

Two rules follow, and both are implemented:

- **An element the document already styles specifically opens on "This one"** — a minted handle, or
  an id or class of its own that some stylesheet actually targets.
- **Toggling back to "All" shows the page-wide values, and Apply then deletes the element-specific
  rule**, leaving the page-wide one to apply. The minted class comes off the element too, so dead
  handles do not accumulate. An `id` is left alone — it is the document's own and may mean something
  to somebody; only its rule in our block goes.

**No inline styles.** Nothing writes a `style=""` attribute. Every edit is a rule in the document's
`webgen-doc-custom` block, which is what keeps a document restyleable in one place.

### Two measured facts about clicks on a WebView

Both look like broken code until you know them, and both are in the comments:

- A `GestureClick` added to the **WebView** never fires, in either propagation phase — WebKit's own
  event handling owns those events outright. The gesture goes on the **window**, and the coordinates
  are translated back into the WebView's space, which is what `elementFromPoint` wants anyway.
- It must be `pressed`, not `released`. WebKit claims the event sequence in the target phase, which
  **cancels** the gesture before any release arrives.

## Not done yet

- Tables, find/replace, spell check, styles beyond the four.
- Any autosave. Save is explicit and there is no recovery file yet.
- Word is not yet offered in Files' "Open With…". That list is `assoc::PROGRAMS`, a hardcoded const
  duplicated verbatim across `webgen-files` and `webgen-settings`, neither of which this session
  owns — raised in INTEGRATION.md for both.
