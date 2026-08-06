# The hybrid document: Word + JSON tables + calc

A design brief, not a decision. Piers, 2026-08-06:

> "Alternately we can have our format be a hybrid of Word + JSON tables + logic/calc routines
> that make emailing a document be able to render as HTML, a WG Word doc or a self contained
> spreadsheet snapshot (HTML+JS+JSON tables) — regardless of the OS that the recipient uses."

This exists so the decisions can be made awake, with the trade-offs written down.

## What already exists (more than it looks)

Nearly all the machinery is in the tree:

- **Tables already carry their data as JSON**, in `data-wg-table` on the element, with the markup
  a projection of it (`table.rs`). The model already holds text, spans, per-cell fill, column
  widths and a scoped stylesheet.
- **The document is already a standalone HTML file** that opens in any browser, with the
  stylesheet inside it.
- **The table window** — deliberately kept when in-document editing landed — is already a
  grid-shaped editor over that JSON.

So "a spreadsheet snapshot" is not a new format. It is the existing table JSON, plus a formula
column, plus a small amount of JavaScript that a recipient's browser runs.

## The one constraint I would not trade

**The visible document must be correct with JavaScript off.** Mail clients strip scripts, web
previews sandbox them, corporate gateways rewrite them. If the numbers only appear when JS runs,
then the lecturer opening the attachment on a locked-down machine sees an empty table and
concludes the document is broken.

So: **values are always written into the HTML as text.** The JSON and the script are an
*enhancement* that makes the document live where it is allowed to be. Static-correct first,
recalculating second. This also means the PDF and the docx export need no special handling — they
already see the computed values.

## The decisions that are actually yours

### 1. Where formulas evaluate

| | how | costs |
|---|---|---|
| **A. At save, in Rust** (recommended) | Word computes the values and writes them into both the HTML text and the JSON. The exported file recalculates only if the recipient has WebGen. | Simplest, safest, no script in the file at all. A recipient cannot change a number and see totals update. |
| **B. In the exported file, in JS** | A small vendored script recalculates on load and on edit, so any browser becomes a live sheet. | The file carries executable content. Some clients will strip it (see the constraint above — the document still reads correctly, it just stops recalculating). |
| **C. Both** | Values baked at save AND the script included, so it is correct everywhere and live where scripts run. | The natural end state; it is A plus B, so it is also the most work. |

My recommendation: **build A now, design the JSON so B drops in later**, ship C when there is a
second consumer asking for it.

### 2. Formula syntax and cell addressing

The temptation is `=SUM(A1:A10)`. The problem is that A1 addressing assumes a rectangular grid,
and our tables have merges — a merged cell occupies several addresses and appears once. Options:

- **Spreadsheet-style A1**, with merged cells addressed by their anchor. Familiar; needs a
  documented rule for what `B3` means when B2 spans into it.
- **Named cells** — a cell may carry a name, formulas refer to names. Robust against inserting a
  row, which is exactly when A1 references break. Less familiar.

Recommendation: **A1 for ranges, names allowed as an alias**, anchors own their addresses, and a
formula referring to a swallowed address is an error shown in the cell rather than a silent zero.

### 3. What the format contains

Proposed shape, entirely additive to what a table block is today:

```
<!-- table block -->
<style> …scoped rules… </style>
<table class="wg-t1" data-wg-table="{…model…}">…rendered values…</table>
<!-- END table block -->
```

with the model gaining, per cell, an optional `formula` string, and the table gaining an optional
`calc` version marker. A reader that knows nothing about formulas sees a table of values — which
is the whole point.

### 4. Recalculation triggers

At save; on demand from a menu item; and — if B ships — on input in the exported file. Not on
every keystroke inside Word: the document is a document, and a recalculation storm while typing
is how spreadsheets earn their reputation.

## What I would build first, given a nod

1. `formula` on the cell model plus an evaluator in Rust (arithmetic, `SUM`/`AVG`/`MIN`/`MAX`/
   `COUNT`, cell and range references, error values). Tests are the spec.
2. Recalculate-on-save, values written as text.
3. The table window gains a formula bar — it is already the grid-shaped surface.
4. Only then: the exported live snapshot (B), as an opt-in export choice beside "Save a copy as
   Word (.docx)".

Roughly a day for 1–3 in the usual increments, with the format documented as it lands so the
browser's editor can read the same files.

## The honest risk

A word processor that grows a spreadsheet inside it can become neither. The mitigation is that
the table JSON already exists and every step above is additive: if the calc layer is never
finished, what remains is exactly the table support the app has today, with a `formula` field
nobody fills in.
