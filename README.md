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
- **This document's own** — `<style id="webgen-doc-custom">`, edited under *Menu → This document's
  style*, written in a fixed layout so it reads back exactly as it was written, and carried **in the
  file** so it survives being emailed.

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
  what CSS can say about a picture are reachable per-document under *This document's style*.
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

- `cargo test` — 53 tests: the sanitiser (including a script containing markup, nested drops,
  idempotence, and that a clean document comes back byte for byte), the page-setup round trip
  through the document meta, the style round trip through the fixed layout, and document
  preparation.
- Driven under Xvfb + XTEST and screenshotted: opens a file, edits it, the window titles after the
  file name, Ctrl+S writes, the close-confirm names the document with Cancel as the default.
- **The doctype is written.** The 0.2.0 bug — `outerHTML` omits it, so every document Word saved
  opened in quirks mode in every browser — is fixed, covered by a test, and confirmed by a
  save-and-read-back run.
- A two-page CV prints correctly through this WebKit: forced page break honoured, `break-inside:
  avoid` keeps job blocks whole, headings do not strand at a page foot.
- `PageSetup` produces a true A4 MediaBox (210 × 297 mm), overriding GTK's Letter default.

Not verified: on-screen editing on real WebGen hardware under labwc, the print dialog's "Print to
File" flow end to end, and System Settings rendering the new `colour` rows. All need a screen — see
the UAT queue.

## Next tranche: the element style sidebar

Piers, 2026-08-01. Clicking an object — a picture, a `div`, an `li`, a `ul` — opens a CSS editor in
a **sidebar** rather than a modal. It is scoped to the element you clicked, and it navigates the
document tree:

- the sidebar's title shows the resolved element, e.g. **`<LI>`**
- an **up** arrow moves focus to the parent, so `<LI>` → `<UL>`
- a **down** arrow moves focus to the first child, so `<LI>` → `<SPAN>`

Given `<ul><li><span>Test</span></li></ul>`, clicking the text resolves to `<li>`; up lands on
`<ul>`, down on `<span>`.

One decision to settle before it is built: the per-document block is addressed **by tag**, so
styling *one* element wants either a generated class per styled element or a move to inline styles.
That is a format change affecting the browser too (CONTRACT.md §4c), so it deserves its own call
rather than being settled by whoever writes the sidebar first.

## Not done yet

- Tables, find/replace, spell check, styles beyond the four.
- Any autosave. Save is explicit and there is no recovery file yet.
- Word is not yet offered in Files' "Open With…". That list is `assoc::PROGRAMS`, a hardcoded const
  duplicated verbatim across `webgen-files` and `webgen-settings`, neither of which this session
  owns — raised in INTEGRATION.md for both.
