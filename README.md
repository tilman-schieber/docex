# docex

Work on the *content* of Word documents (text, headings, lists, tables, images, tracked changes, comments) from the terminal, in Markdown. It is for people who live in Markdown, Typst and AI agents but still have to swap .docx files with people who don't.

docex is not a Word clone. It never converts your colleague's document to Markdown and back. The original .docx stays the source of truth, and docex only patches the paragraphs you actually changed. Styles, spacing, headers, footers, fields, text boxes and everything else docex doesn't model survive byte for byte.

```sh
docex cat report.docx                 # Markdown, with changes and comments as CriticMarkup
docex edit report.docx                # opens $EDITOR; saves your edits back into the .docx
docex apply report.docx edited.md     # same, from a file (or - for stdin), e.g. written by an agent
docex track report.docx on|off        # Word's "Track Changes" switch
docex changes report.docx             # numbered list of tracked changes
docex accept report.docx 3 5          # or --all, or --by "Alice"
docex reject report.docx --by Bob
docex comments report.docx
docex comment report.docx --on "quoted text" "Please check this"
docex comment report.docx --reply 4 "Done"   # also --resolve / --reopen / --delete ID
docex new notes.md -o notes.docx [--template corporate.docx]
docex diff a.docx b.docx              # content diff; --into writes B onto A as changes
```

## Tracked or clean edits

As in Word, every document has a Track Changes switch (`docex track FILE on|off`, stored in the file, so Word shows the same state). `edit`, `apply` and `diff --into` follow it. `--track` or `--untracked` overrides it for one command.

With tracking on, your edits become word-level insertions and deletions under your name (`--author`, else `$DOCEX_AUTHOR`, else `git config user.name`). Formatting-only edits (bolding a word) become format changes, and turning a paragraph into a heading becomes a paragraph-format change. If you delete text you inserted yourself, it simply disappears; other people's insertions get a tracked deletion instead.

Files are changed in place with a `FILE.docx~` backup. Use `-o OUT.docx` to write elsewhere.

## The Markdown view

| In Word | In docex |
|---|---|
| Heading 1–6 / Title | `#` … `######` |
| Bulleted / numbered lists (nested) | `-` / `1.` |
| Quote, code styles | `>` / fenced code |
| Bold, italic, strikethrough, code | `**b**`, `*i*`, `~~s~~`, `` `c` `` |
| Hyperlinks | `[text](url)` |
| Simple tables | pipe tables (`<br>` separates paragraphs in a cell) |
| Tables with merged cells | read-only `docex-table` block |
| Images | `![alt](media/image1.png)` (`cat --media DIR` extracts them) |
| Captions (figures, tables) | `: Figure 1: text` |
| Footnotes, endnotes | `[^1]` with `[^1]: text` at the end (endnotes `[^e1]`) |
| Text boxes | `<!-- docex:textbox tb1 -->` … paragraphs … `<!-- /docex:textbox -->` after the paragraph they're anchored in |
| Tracked insertion / deletion | `{++new++}` / `{--old--}` |
| Comment | `{==anchored text==}{>>Author: comment<<}` |
| TOC, content controls, shapes, equations, page breaks | `<!-- docex:… -->` placeholders, kept as-is |

You can write CriticMarkup yourself: `{++text++}`, `{--text--}` and `{~~old~>new~~}` are always recorded as tracked changes, even with tracking off. A new `{==text==}{>>remark<<}` (or a bare `{>>remark<<}`) becomes a Word comment. New images referenced by path (PNG, JPEG, GIF) are embedded and sized to the page width.

Footnote text, text box text and captions are edited like any other paragraph, with tracked changes. A new `[^any-label]` reference plus its `[^any-label]: text` definition creates a Word footnote, which Word numbers itself. A new `: Figure 3: text` caption gets a real auto-numbering field (`SEQ Figure`), so Word renumbers it. Tracked changes in footnotes show up in `docex changes` like any other; edits inside a text box are written to both copies Word keeps of it.

Tables are deliberately kept simple: you can edit cell text and add or remove rows. Column changes and tables with merged cells (shown read-only) are left to Word, or to the planned GUI.

Things deliberately left out: spacing, fonts, colours, page layout. New text takes on the formatting of the text around it, and new paragraphs copy the nearest paragraph of the same kind, so additions look native in Word.

## Building

```sh
cargo build --release   # binary: target/release/docex
cargo test
```

See `AGENTS.md` for how an AI agent should drive docex.
