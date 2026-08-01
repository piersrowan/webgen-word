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

- `cargo test` — 84 tests, 16 of them on the table grid alone (merge, split, insert and delete
  through a span, refusing a merge that would half-swallow a cell, and the JSON round trip): the sanitiser (including a script containing markup, nested drops,
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
- **The whole table process, driven end to end.** Cursor in a paragraph → grid button → 3×3 →
  Table Window → type into three cells → merge right (the caption reads "merged 1×2" and the cell
  holds "one two") → Save. The block lands **between** the two paragraphs with both intact, the
  markup is the shape above, and the merge is `colspan="2"`. Clicking inside that table and pressing
  the button again reopens it **with the merge preserved** — proving the JSON round trip through the
  document. Three clicks on Border width in the CSS panel put `border: 3px solid #000000` on
  `table.wg-t1` and nowhere else. Delete removes block, style and table, leaving both paragraphs.
- **Piers' a1–a5 undo sequence, driven end to end.** Type into a four-paragraph document, make all
  paragraphs bold, underline one of them, then Ctrl+Z twice: both style rules are gone from the file
  and **the typed text is still there**. Ctrl+Y twice puts both rules back exactly. The sidebar's
  rows follow — Underline reads "—" after the undo and "underline" after the redo, so the next Apply
  cannot put back what was just undone.
- **The doctype is written.** The 0.2.0 bug — `outerHTML` omits it, so every document Word saved
  opened in quirks mode in every browser — is fixed, covered by a test, and confirmed by a
  save-and-read-back run.
- A two-page CV prints correctly through this WebKit: forced page break honoured, `break-inside:
  avoid` keeps job blocks whole, headings do not strand at a page foot.
- `PageSetup` produces a true A4 MediaBox (210 × 297 mm), overriding GTK's Letter default.

Found by that driving, and fixed: (1) saving used to strip the sidebar's cursor class from the
**live** DOM, so after one Ctrl+S the sidebar had no selected element and silently stopped
responding — the markers now come off, the document is serialised, and they go straight back on;
(2) the content fingerprint moved when a handle was minted, because removing the class token left
`class=""` behind on an element that had no class attribute before, so undo read it as a text edit
and took back the wrong thing; (3) a save that referenced an unbound variable failed outright, which
only the 0.3.0 "could not read the document back" banner made visible.

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

### Border: three pickers, and 0 removes it

Width, style and colour, rather than a string to get wrong. **Width 0 writes no `border`
declaration at all** — not `border: none` — so the rule is left clean, and a rule that had nothing
else in it disappears entirely.

A border written by hand or by another editor is read back into the three pickers by shape rather
than by position, so `DOTTED 2px #abcdef` and `1px solid red` both land correctly; common CSS colour
names are resolved to hex.

### Properties

Font, **weight**, **italic**, **underline**, text colour, background, border (width/style/colour),
corner radius, drop shadow, padding, margin, float and text alignment.

Weight, italic and underline are three-state — *say nothing*, `normal`, `bold` — rather than
switches, because "say nothing" and "explicitly normal" are different answers: an explicit `normal`
is how you take the bold off an element that inherits it.

### Two measured facts about clicks on a WebView

Both look like broken code until you know them, and both are in the comments:

- A `GestureClick` added to the **WebView** never fires, in either propagation phase — WebKit's own
  event handling owns those events outright. The gesture goes on the **window**, and the coordinates
  are translated back into the WebView's space, which is what `elementFromPoint` wants anyway.
- It must be `pressed`, not `released`. WebKit claims the event sequence in the target phase, which
  **cancels** the gesture before any release arrives.

## Tables

Tables are painful to get right, and the reason is that a table is the one genuinely
two-dimensional thing in a document. A merged cell means the row below it has fewer cells than the
table has columns, so "insert a column" is not "push a cell into every row" and "what is at row 3,
column 2" is not `rows[3][2]`. `src/table.rs` exists to keep that straight and its tests are where
it is actually kept straight.

### The process

Put the cursor where the table goes and press the grid button → say how many rows and columns →
the **Table Window** opens. Everything about a table happens there: cell text, add and remove rows
and columns, merge and split, bold/italic/underline and alignment per cell, and the table's own CSS
on the right. **Save** regenerates the block in the document; **Delete** takes it out. Put the
cursor inside a table and press the same button to reopen it.

Nothing edits a table in the document itself. That is deliberate — every editor that has tried to be
both a stream and a grid at once has ended up with cells you cannot select and merges you cannot
undo.

### What a table is on disk

```html
<!-- table block -->
<style>
table.wg-t1 { border-collapse: collapse; }
.wg-t1 th { font-weight: bold; border: 1px solid #999999; padding: 4px 8px; }
</style>
<table class="wg-t1" data-wg-table="{…json…}">
  <thead><tr><th>Heading 1</th></tr></thead>
  <tbody><tr><td colspan="2">one two</td></tr></tbody>
</table>
<!-- END table block -->
```

Three things there are load-bearing:

- **The comments delimit what gets replaced.** Saving regenerates everything between them rather
  than patching markup in place.
- **The CSS is scoped to `.wg-tN`.** The style block sits in the document, so an unscoped
  `table { … }` would restyle every table in the file. The editor talks in terms of `table`,
  `thead tr`, `td`, `tbody tr:nth-child(odd)`; the prefix is added on the way out.
- **The JSON is the truth**, and it lives in `data-wg-table`. Reopening a table parses the
  attribute, not the markup, so the editor never has to guess what a `rowspan` meant. It is not in
  the comment because a cell containing `-->` would end the block early, and not in a
  `<script type="application/json">` because a word processor that strips script should not make
  exceptions to that.

`border-collapse: collapse` is emitted for every table and is not a knob — a document table with
separated borders looks like a mistake, and there is no reason to offer the mistake.

## Undo covers the last action, whatever kind it was

WebKit's undo stack knows about typing, pasting and the toolbar's editing commands. It knows nothing
about a style change, because a style change is not an edit to the text — it rewrites the `<style>`
block. So this sequence used to go wrong at the end:

> a1 type "hello world" · a2 copy and paste it 3 times · a3 sidebar: all paragraphs bold ·
> a4 pick one paragraph and underline it · **a5 undo × 2**

Undo × 2 would have thrown away two of the *pastes* and left both style changes applied. It now
takes back a4 and then a3, leaving the text exactly as it was typed.

**How the two stacks stay in order.** There is no notification when WebKit records an undo step, so
they cannot simply be merged. Each style step remembers a **fingerprint of the document's content**
at the moment it was applied — a hash of the body with editing markers and `wg-iN` handles stripped
out, so minting a handle does not read as an edit and only real content moves it. Undo then asks one
question: has the content changed since the newest style step? No → take the style step back. Yes →
something was typed since, so hand over to WebKit. Redo is the mirror image and invalidates itself
for free: type after undoing a style change and the fingerprint no longer matches, so redo goes to
WebKit instead.

Ctrl+Z and Ctrl+Shift+Z (and Ctrl+Y) are intercepted rather than left to WebKit, or the keys and the
toolbar buttons would do different things.

**The one corner it does not cover:** text, then a style change, then more text, then undo held
down. The text undoes newest-first as it should, but WebKit's stack runs on past the style boundary,
so older text comes back before the style step does. The end state after undoing everything is the
same; only the middle differs. When WebKit reports it has nothing left, style steps are taken
regardless, so none is ever stranded.

## Known debt

`src/stylerows.rs` is the shared style-row widget the table window's CSS panel uses. The document
style sidebar still has its own copy of the same rows, written before the extraction. They agree
today because they were the same code an hour ago; they will drift. The sidebar should adopt
`StyleRows` — it is a mechanical change, and the two driving scripts that verify the sidebar make it
cheap to check.

## Not done yet

- Tables, find/replace, spell check, styles beyond the four.
- Any autosave. Save is explicit and there is no recovery file yet.
- Word is not yet offered in Files' "Open With…". That list is `assoc::PROGRAMS`, a hardcoded const
  duplicated verbatim across `webgen-files` and `webgen-settings`, neither of which this session
  owns — raised in INTEGRATION.md for both.
