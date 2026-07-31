# webgen-word

A light word processor whose document format is **plain HTML**.

A CV, a letter or a report is a *document*, and HTML+CSS already describes documents perfectly well.
So the file on disk is a standalone `.html` that any browser opens, and "export to PDF" is just
printing. No proprietary container, nothing to convert, nothing to lose if the app goes away.

## What it does

- Rich-text editing via `WebView::set_editable` — WebKit supplies the caret, selection, undo,
  clipboard and IME behaviour; the toolbar drives it through `execute_editing_command`.
- Paragraph styles (Body / Title / Heading / Subheading), bold, italic, underline, bulleted and
  numbered lists, remove-formatting, undo/redo.
- **Insert page break** — the one thing a CV needs that HTML has no key for.
- **Page setup**: paper size and all four margins.
- Open and save `.html`; print, which is also how you get a PDF (Print → Print to File).

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

That single fact is why `page.rs` exists, why Page setup is a toolbar button rather than a CSS
comment, and why the default is A4/20 mm rather than whatever GTK would have picked. A document's
`@page` rule is still written into the saved file — it is correct for *other* renderers (a browser's
own print, `weasyprint`) — but it has no effect here and the code says so.

## Identity (CONTRACT.md §1)

| | |
|---|---|
| appId / `StartupWMClass` | `com.webgen.Word` |
| binary | `webgen-word` |
| display name | `Word` |
| icon | `com.webgen.Word` — deep blue `#1c4f9c`, page glyph |
| file arguments | `webgen-word FILE` and "Open With" both work (`HANDLES_OPEN`) |

## Build

```sh
cargo build --release
```

Needs `libwebkitgtk-6.0-dev`. Same `gtk4 0.9` / `libadwaita 0.7` / `webkit6 0.4` line as the
browser, so both link one gtk4-sys and the OS builds one WebKit.

## Verified

- Builds warning-free; the window renders with the full toolbar (screenshotted under Xvfb).
- A two-page CV prints correctly through this WebKit: forced page break honoured, `break-inside:
  avoid` keeps job blocks whole, headings do not strand at a page foot.
- `PageSetup` produces a true A4 MediaBox (210 × 297 mm), overriding GTK's Letter default.

Not yet verified: on-screen editing on real WebGen hardware under labwc, and the print dialog's
"Print to File" flow end to end. Both need a screen.

## Not done yet (v0.1)

- Images. Inserting one is easy; making it *portable* is not — a document that references
  `/home/you/photo.jpg` breaks the moment it is emailed. That wants either data-URI embedding or a
  bundle format, which is the `.htmlx` idea and deserves its own decision rather than a quick hack.
- Tables, find/replace, spell check, styles beyond the four.
- Any autosave. Save is explicit and there is no recovery file yet.
