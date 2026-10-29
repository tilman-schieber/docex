# Working on Word documents with docex (for agents)

docex lets you read, review and edit a .docx through the command line. The .docx stays the source of truth. Your edits and comments go into it as tracked changes and Word comments under the user's name (`--author`, else `$DOCEX_AUTHOR`, else `git config user.name`), so a human reviews them in Word as usual.

## Reviewing other people's changes

1. **Survey:** `docex review FILE --json`. One call returns every tracked change and comment thread:
   - a **stable id**: `c12` for a change (`f…`/`e…` in footnotes/endnotes), `m3` for a comment
   - `kind` (insert, delete, replace, format, comment), `author` and `date`
   - the `section` heading path
   - `before`/`after` context, and `old`/`new` text (a deletion right next to an insertion is one `replace`)
   - for comments: `text`, `anchor`, `replies` and `resolved`

   The summary counts changes and comments per person. Filters: `--by NAME`, `--kind replace|insert|delete|format|comment|change`, `--in SECTION`, `--open` (unresolved threads only). Without `--json` you get a readable list grouped by section.
2. **Read more context if needed:** `docex cat FILE --ids` shows the whole document with `⟦c12⟧` after every change and `⟦m3⟧` after every comment.
3. **Decide in one call:** `docex resolve FILE --accept c4,c9,c12 --reject c7`. Ids are Word's own, so they never shift: decide everything, then resolve once. Any Word id inside an item selects the whole item (both halves of a replacement). Unknown ids are refused before anything changes. Per person: `docex accept FILE --by "Name"`.
4. **Answer comments:** `docex comment FILE --reply m3 "text"`, `--resolve m3`, `--reopen m3`, `--delete m3`.

## Adding comments

- **Many at once, exactly placed:** in the output of `docex cat FILE` (or `--ids`), wrap the passage and add your note: `{==exact passage==}{>>your remark<<}`. Then run `docex apply FILE edited.md`. Leave everything else as it is.
- **One at a time:** `docex comment FILE --on "exact quote" "remark"`. If the quote occurs more than once, docex refuses and lists the matches with their sections. Pick one with `--nth N` or narrow with `--in "Section"`.

## Editing text

1. `docex cat FILE > FILE.md`.
2. Edit FILE.md. Change only what needs changing; leave everything else exactly as rendered.
3. `docex apply FILE FILE.md --track`. Read the summary on stderr and any `warning:` lines.
4. Check with `docex review FILE --by "<you>"`.

Rules:
- **Don't reformat.** Re-wrapping lines, changing list markers or "tidying" escapes all count as no-ops, but changing words creates word-level changes. Keep `\*`, `\_`, `\$` and similar escapes as they are.
- **Leave `<!-- docex:… -->` placeholders alone.** They stand for content docex doesn't edit (TOC, shapes, page breaks, equations it can't convert). Deleting one deletes that content.
- **Existing `{++…++}` / `{--…--}` belong to other people.** Leave them in place. Accept or reject them with `resolve`; don't delete them by hand.
- **To suggest an edit explicitly**, write `{~~old~>new~~}`, `{++added++}` or `{--removed--}`. These are always tracked.
- **Structure:** `#` makes a heading, `- ` a bullet, `1. ` a numbered item, `> ` a quote, `: ` a caption.
- **Formulas:** `$…$` inline, `$$…$$` on its own line. They become Word equations.
- **Footnotes:** edit the `[^n]: text` definitions at the end. To add one, put `[^new]` in the text and `[^new]: text` at the end.
- **Tables:** edit cells in place; adding or removing rows works, adding columns does not. Blocks marked `docex-table` are read-only. Inside a cell, write `|` as `\|`.
- **Text boxes:** edit the paragraphs between `<!-- docex:textbox tbN -->` and `<!-- /docex:textbox -->`. Leave the marker lines alone.

## Machine-readable output

- `docex review FILE --json`: summary plus items, as above.
- `docex cat FILE --json`: blocks with id, type, style and text.
- `docex changes FILE --json`: raw tracked changes with their Word ids (`wids`) and part.
- `docex comments FILE --json`: id, author, date, text, anchor, parent, done.
