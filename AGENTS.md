# Editing Word documents with docex (for agents)

docex lets you edit a .docx through Markdown. The .docx itself stays the source of truth, and your edits go back into it as tracked changes, so a human reviews them in Word.

## Workflow

1. `docex cat FILE.docx > FILE.md`: read the document. Use `--accepted` if you only need the current text.
2. Edit `FILE.md`. Change only what needs changing, and leave everything else byte-for-byte as rendered.
3. `docex apply FILE.docx FILE.md --track --author "<name>"`: write it back. Read the summary on stderr and any `warning:` lines.
4. Check the result with `docex cat FILE.docx` or `docex changes FILE.docx`.

For a one-shot edit, pipe it in: `docex cat F.docx | sed … | docex apply F.docx - --track`.

## Rules

- **Don't reformat.** Re-wrapping lines, changing list markers or "tidying" escapes all count as no-ops. Changing words, though, makes word-level changes. Keep `\*`, `\_` and similar escapes as they are.
- **Leave `<!-- docex:… -->` placeholders alone.** They stand for content docex doesn't edit (TOC, shapes, equations, page breaks); a text box's placeholder sits in its anchor paragraph. Deleting one deletes that content.
- **Existing changes** (`{++…++}`, `{--…--}`) belong to other people. Leave them in place. To accept or reject them, use `docex accept|reject FILE N…` with numbers from `docex changes FILE --json`.
- **To suggest an edit explicitly**, write `{~~old~>new~~}`, `{++added++}` or `{--removed--}`. These are always tracked.
- **To comment**, write `{==text you refer to==}{>>your remark<<}`, or run `docex comment FILE --on "exact text" "remark"`. Existing comments render as `{>>Author: text<<}`; don't edit them. To reply, use `docex comment FILE --reply ID "text"`.
- **Tables:** edit cells in place. Adding or removing rows works. Adding columns does not. Blocks marked `docex-table` (merged cells) are read-only.
- **Footnotes:** edit the `[^n]: text` definitions at the end of the document. To add a footnote, put `[^new]` in the text and add `[^new]: text` at the end. Don't reuse an existing number.
- **Text boxes:** edit the paragraphs between `<!-- docex:textbox tbN -->` and `<!-- /docex:textbox -->`. Leave the two marker lines alone; you can't create new text boxes.
- **Captions:** lines starting with `: ` are captions (`: Figure 2: text`). Keep the number in a new caption; Word renumbers it.
- **Structure:** `#` makes a heading, `- ` a bullet, `1. ` a numbered item, `> ` a quote. Changing one of these on an existing paragraph changes its style, and the change is tracked.

## Machine-readable output

- `docex cat FILE --json`: blocks with id, type, style and text.
- `docex changes FILE --json`: n, kind, author, date, text.
- `docex comments FILE --json`: id, author, date, text, anchor, parent, done.
