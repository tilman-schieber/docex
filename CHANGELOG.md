# Changelog

All notable changes to docex. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/tilman-schieber/docex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tilman-schieber/docex/releases/tag/v0.1.0
