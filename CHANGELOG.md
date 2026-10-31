# Changelog

All notable changes to docex. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Stylesheets.** A document's look as a few lines of CSS, written into Word's own built-in styles (Normal, Heading 1–6, Title, Quote, Caption, …) and the page setup, so the result is an ordinary Word file. Four themes come with docex (report, classic, thesis, plain); your own `.css` works, and so does another .docx whose look to copy. `docex style FILE --theme NAME`, `docex style FILE` (the look as CSS), `docex style --list`, `docex new --theme`. The text and its tracked changes stay as they are. See docs/stylesheets.md.
- **The page as Word will show it.** The editor shows the document in its own fonts, sizes, spacing and page size. The palette button in the toolbar previews a theme on the page before applying it (Undo takes it back); "Print colours" shows a white page with the document's colours; "Save this document's look as CSS…" starts a theme of your own.
- **Pictures in the app.** Paste a screenshot, or drop picture files where they should go. Click a picture for its menu: replace it (the new one takes the old one's width and alt text, as a tracked replacement), edit its alt text, save it as a file, delete it; drag its corner to resize. Pictures show at their size on the page. WebP, BMP and SVG files are converted to PNG on the way in; pictures the editor can't show (EMF, WMF) appear as a labelled box instead of a broken image.
- **Alt text by the AI.** "Write alt text for pictures without one" in the command palette sends each picture with the text around it to the AI, then lists the suggestions to check and edit before they go in. "Describe with AI" in a picture's menu does it for one.
- **Open a document by dropping it** on the window.
- **`docex images FILE`** lists the pictures with their size, alt text and section (`--json`); `--extract DIR` saves them, numbered in reading order. "Save all pictures to a folder…" does the same in the app.
- In the Markdown, a picture's alt text can now be edited, and `{width=8cm}` (or `50%`) after it resizes it. Putting another file's path in a picture's place replaces it at the same width, keeping its wrapping and alt text.
- **Distraction-free mode** (F11, or "Distraction-free mode" in the command palette): only the page, full screen. Show it with the changes marked, or as if every change were accepted, to read and edit the version everyone would end up with. Alt+V switches between the two, Esc or F11 leaves.

### Fixed

- Editing a picture's alt text in the Markdown was silently ignored.
- A picture moved to another place came back at a default size.

## [0.2.0] - 2026-10-30

### Added

- docex is now under the MIT license.
- **Compare with another version.** When a document comes back without tracked changes, docex writes that version into yours as tracked changes by whoever saved it ("Compare with another version…" in the app, `docex diff mine.docx theirs.docx --into` on the command line). Changes already open in your document stay as they are; new pictures come along.
- **Triage with AI suggestions.** The AI goes through other people's changes and suggests accept, reject or look closer for each, with a short reason. Triage starts with those decisions preset, and you can change any of them before applying.
- The app says when a newer release is available (one check a day; "Stop checking for updates" in the command palette turns it off).

### Changed

- Focusing on one person now also fades everyone else's formatting changes.
- Many reviewers in one document no longer share colours. People get their colour in order of appearance (you first), so the first ones are always different. Beyond the theme's own colours, each one also comes more and less saturated at the same brightness, then lighter and darker: 11 to 16 distinct colours for most Omarchy themes, all taken from the active theme and kept clear of the green and red used for insertions and deletions.

### Fixed

- `docex diff --into` no longer counts the document's open changes as edits when the other version has them accepted, and takes the other version's new pictures from that version (before, it looked for them in the current directory).
- The review panel's buttons wrap instead of running off a narrow panel.

## [0.1.0] - 2026-10-30

First release: the `docex` command line and the desktop app, for Linux, macOS and Windows.

[Unreleased]: https://github.com/tilman-schieber/docex/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/tilman-schieber/docex/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tilman-schieber/docex/releases/tag/v0.1.0
