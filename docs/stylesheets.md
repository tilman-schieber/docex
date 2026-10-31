# Stylesheets

docex gives a document its look from a small stylesheet written in CSS. Everything goes into Word's own built-in styles (Normal, Heading 1–6, Title, Quote, Caption, …) and the page setup. The document stays an ordinary Word file: the navigation pane and table of contents work, and Word in any language knows the styles ("Überschrift 1"). The text and its tracked changes are not touched.

```sh
docex style --list                       # the built-in themes
docex style report.docx --theme classic  # give the document a look
docex style report.docx --theme my.css   # your own stylesheet
docex style report.docx --theme other.docx   # look like another document
docex style report.docx > my.css         # the document's look as CSS, to edit
docex new notes.md -o notes.docx --theme thesis
```

In the app: the palette button in the toolbar, or "Document look" in the command palette. Pick a theme to see it on the page, then apply it; Undo takes it back. The editor always shows the document in its own fonts, sizes and spacing; "Print colours" shows a white page with the document's colours.

Word doesn't track changes to styles, so a new look is not a tracked change.

## Built-in themes

| Theme | Look |
|---|---|
| `report` | Calibri, dark blue headings with a rule under the first level. Reports, memos, proposals. |
| `classic` | Georgia, justified, quiet headings in small capitals. Longer texts. |
| `thesis` | Cambria 12 pt, 1.5 line spacing, justified, numbered headings, chapters on a new page, margins 2.5/2.5/2/3 cm. |
| `plain` | Arial, black, no colour. Safe for any recipient. |

The theme files are in [`crates/docex-core/themes`](../crates/docex-core/themes): a good start for your own.

## What a stylesheet can say

| Selector | Word style |
|---|---|
| `body` (or `p`) | Normal, and the document defaults |
| `h1` … `h6` | Heading 1 … 6 |
| `.title`, `.subtitle` | Title, Subtitle |
| `blockquote` | Quote |
| `figcaption` | Caption |
| `pre` | Source Code |
| `a` | Hyperlink (colour and underline) |
| `@page` | page size and margins of every section |

| Property | Notes |
|---|---|
| `font-family` | Word uses the first font; the rest are for the editor's preview (Carlito stands in for Calibri, …). |
| `font-size`, `font-weight`, `font-style`, `font-variant: small-caps`, `text-transform: uppercase` | |
| `font: italic bold 11pt/1.3 Georgia` | the shorthand |
| `color`, `background` | `#rrggbb`, `#rgb` or a few names |
| `line-height` | a number (1.15) is a multiple of single spacing; a length is "at least" |
| `margin-top`, `margin-bottom`, `margin-left`, `margin`, `text-indent` | space before and after, indent, first-line indent (negative: hanging) |
| `text-align` | left, center, right, justify |
| `letter-spacing` | |
| `border-bottom: 0.75pt solid #8EAADB` | a rule under the paragraph |
| `break-before: page` | start on a new page |
| `counter-increment: chapter` | on `h1`–`h6`: numbered headings (1, 1.1, 1.1.1); `none` removes the numbering |
| `@page { size: A4; margin: 2.5cm 2cm }` | A4, A5, A3, letter, legal, `landscape`, or two lengths |

Lengths: `pt`, `px`, `cm`, `mm`, `in`, `em`. As in CSS, headings and the rest take font, size, colour, line height and alignment from `body` unless they say otherwise, and weight and style are normal unless they say so. Anything else is reported and ignored.

Styles the stylesheet doesn't mention keep their look, and so does formatting applied by hand to the text.
