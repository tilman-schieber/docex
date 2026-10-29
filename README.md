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
| Footnotes | `[^1]` plus the note text at the end (read-only) |
| Tracked insertion / deletion | `{++new++}` / `{--old--}` |
| Comment | `{==anchored text==}{>>Author: comment<<}` |
| TOC, content controls, text boxes, equations, page breaks | `<!-- docex:… -->` placeholders, kept as-is |

You can write CriticMarkup yourself: `{++text++}`, `{--text--}` and `{~~old~>new~~}` are always recorded as tracked changes, even with tracking off. A new `{==text==}{>>remark<<}` (or a bare `{>>remark<<}`) becomes a Word comment. New images referenced by path (PNG, JPEG, GIF) are embedded and sized to the page width.

Things deliberately left out: spacing, fonts, colours, page layout. New text takes on the formatting of the text around it, and new paragraphs copy the nearest paragraph of the same kind, so additions look native in Word.

## Building

```sh
cargo build --release   # binary: target/release/docex
cargo test
```

See `AGENTS.md` for how an AI agent should drive docex.
