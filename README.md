# docex

Work on the *content* of Word documents (text, headings, lists, tables, images, tracked changes, comments) from the terminal, in Markdown. It is for people who live in Markdown, Typst and AI agents but still have to swap .docx files with people who don't.

docex is not a Word clone. It never converts your colleague's document to Markdown and back. The original .docx stays the source of truth, and docex only patches the paragraphs you actually changed. Styles, spacing, headers, footers, fields, text boxes and everything else docex doesn't model survive byte for byte.

```sh
docex cat report.docx                 # Markdown, with changes and comments as CriticMarkup
docex edit report.docx                # opens $EDITOR; saves your edits back into the .docx
docex apply report.docx edited.md     # same, from a file (or - for stdin), e.g. written by an agent
docex track report.docx on|off        # Word's "Track Changes" switch
docex review report.docx              # every change and comment thread: stable id, section, context
docex review report.docx --json --by Alice --in "Results"
docex resolve report.docx --accept c4,c9 --reject c7   # all decisions at once, ids never shift
docex accept report.docx --by "Alice" # or --all; reject works the same way
docex cat report.docx --ids           # the document with ⟦c12⟧ / ⟦m3⟧ after each change and comment
docex comments report.docx
docex comment report.docx --on "quoted text" "Please check this"   # refuses ambiguous quotes; --nth N, --in SECTION
docex comment report.docx --reply m4 "Done"  # also --resolve / --reopen / --delete m4
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
| Equations (Office Math) | `$E=mc^2$` inline, `$$\int_0^1 x\,dx$$` on its own line |
| Footnotes, endnotes | `[^1]` with `[^1]: text` at the end (endnotes `[^e1]`) |
| Text boxes | `<!-- docex:textbox tb1 -->` … paragraphs … `<!-- /docex:textbox -->` after the paragraph they're anchored in |
| Tracked insertion / deletion | `{++new++}` / `{--old--}` |
| Comment | `{==anchored text==}{>>Author: comment<<}` |
| TOC, content controls, shapes, equations, page breaks | `<!-- docex:… -->` placeholders, kept as-is |

You can write CriticMarkup yourself: `{++text++}`, `{--text--}` and `{~~old~>new~~}` are always recorded as tracked changes, even with tracking off. A new `{==text==}{>>remark<<}` (or a bare `{>>remark<<}`) becomes a Word comment. New images referenced by path (PNG, JPEG, GIF) are embedded and sized to the page width.

Equations are Word's own (Office Math), shown and written as LaTeX. Existing equations convert to LaTeX: fractions, scripts, roots, sums/integrals/products with limits, `\left…\right` delimiters, matrices, functions and limits, accents, over/underlines, `\text`, `\mathbf`/`\mathbb`/`\mathcal`, Greek letters and the usual operators. A new `$…$` becomes a real Word equation you can keep editing in Word. An equation using something outside that set shows as an `[equation]` placeholder and is kept as it is. In tables, write `|` inside a formula as `\|`.

Footnote text, text box text and captions are edited like any other paragraph, with tracked changes. A new `[^any-label]` reference plus its `[^any-label]: text` definition creates a Word footnote, which Word numbers itself. A new `: Figure 3: text` caption gets a real auto-numbering field (`SEQ Figure`), so Word renumbers it. Tracked changes in footnotes show up in `docex changes` like any other; edits inside a text box are written to both copies Word keeps of it.

Tables are deliberately kept simple: you can edit cell text and add or remove rows. Column changes and tables with merged cells (shown read-only) are left to Word, or to the planned GUI.

Things deliberately left out: spacing, fonts, colours, page layout. New text takes on the formatting of the text around it, and new paragraphs copy the nearest paragraph of the same kind, so additions look native in Word.

## GUI

`gui/` is a desktop editor on the same core (Tauri + ProseMirror). It shows the document formatted:
- tracked changes in color, with Accept/Reject on each one and in the sidebar
- comments, which you can add, reply to, resolve or delete
- tables (add and remove rows), images, footnotes, text boxes and captions

It's built for reviewing, not just editing:
- **Outline:** the heading tree with per-section counts of changes and comments, plus an activity bar colored by author. It follows where you are and collapses; click to jump.
- **Review feed:** changes and comment threads grouped by section, each with a few words of context. A deletion followed by an insertion reads as one `old → new` replacement. Comments expand inline to reply or resolve.
- **People:** a one-line summary ("143 changes by 7 · 15 threads (15 open) by 4") and a chip per person. Hover a chip for their breakdown; click to filter to them (shift+click adds people). Their edits stay highlighted in the text and everyone else's fade.
- **Bulk actions follow the filter:** accept or reject everything shown, by one person or in one section, at once.
- **Activity rail:** a minimap beside the page marks every change (in its author's color) and comment; click to jump.
- **Formulas:** `$…$` or `$$…$$` while typing (or the Σ button) inserts an equation, typeset with KaTeX. Click one to edit it with a live preview. It's saved as a native Word equation.
- **Triage (Alt+T):** go through what the review list shows, one item at a time, in large type with context. `a` accept, `r` reject, `s`/→ skip, `x` resolve a thread, `c` comment or reply, `u` undo, ← back. Decisions are collected and applied together when you finish.
- **Noise and repeated edits:** formatting-only, whitespace and punctuation changes are flagged as noise; hide them, or accept them all at once. The same edit made in several places ("KI" → "AI" ×37) is listed once under "Repeated edits" and can be accepted or rejected everywhere at once.
- **Since you last saw it (Alt+S):** docex remembers the text each time you open or save a document. When it comes back, one click highlights everything that's different (new words, removed text, new and removed paragraphs), whether or not the others tracked their changes. A renamed attachment is matched to its earlier version by content.
- **Command palette (Ctrl+P):** fuzzy search over every command, every heading (jump), every person (show / triage / accept / reject their changes), repeated edits, styles and recent files.
- **AI (Ctrl+J):** rewrite the selection (tighten, fix grammar, more formal, simpler, translate, or your own instruction); only the words that change come back, as tracked changes in your name. Also: ask questions about the document, summarize the review state, and draft replies to comments (you edit before sending). It defaults to the Anthropic API (`claude-opus-5-5`; Claude Max includes monthly API credits, claimed on claude.ai under Settings → Billing); any OpenAI-compatible endpoint (OpenAI, OpenRouter, Ollama, …) can be set in AI settings. Keys live in `~/.config/docex/ai.json` (mode 600) or `ANTHROPIC_API_KEY`.
- **Markdown while you type:** `## ` makes a heading, `- `/`1. ` lists (two leading spaces per level), `> ` a quote, ```` ``` ```` code, `: ` a caption. `**bold**`, `*italic*`, `` `code` ``, `~~strike~~` and `[text](url)` format as you close them. Backspace right after a shortcut undoes it, and pasting Markdown inserts it formatted.
- **Keys:** Alt+↑/↓ next/previous item, Alt+A accept, Alt+R reject, Ctrl+1/Ctrl+2 toggle the panels.
- **Omarchy:** the app follows the active theme's palette and monospace font, and recolors live on `omarchy theme set`.

With Track Changes on (the switch in the toolbar, stored in the document like Word's), your typing shows up as insertions and deleted text gets struck through. On save, the core records the real tracked changes against the original file. As with the CLI, everything you didn't touch stays byte-for-byte.

```sh
cd gui
npm install
npx tauri dev -- -- path/to/file.docx   # run from source
npx tauri build --no-bundle             # binary: target/release/docex-gui
npm test                                # GUI conversion + live tracking against the core
```

## Building

```sh
cargo build --release   # binary: target/release/docex
cargo test
```

See `AGENTS.md` for how an AI agent should drive docex.
