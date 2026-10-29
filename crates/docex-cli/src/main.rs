use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use docex_core::render::View;
use docex_core::{now_iso, ApplyOpts, Docx, Report};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Work on the content of .docx files through Markdown. Your edits become
/// tracked changes; everything you don't touch stays exactly as it was.
#[derive(Parser)]
#[command(name = "docex", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the document as Markdown (tracked changes and comments as CriticMarkup).
    Cat {
        file: PathBuf,
        /// Show the text as if all changes were accepted.
        #[arg(long, conflicts_with = "original")]
        accepted: bool,
        /// Show the text as if all changes were rejected.
        #[arg(long)]
        original: bool,
        /// Also extract images into this directory.
        #[arg(long)]
        media: Option<PathBuf>,
        /// Blocks as JSON (id, kind, text).
        #[arg(long)]
        json: bool,
        /// Mark every change and comment with its review id (⟦c12⟧, ⟦m3⟧).
        /// The markers are ignored if the text is applied.
        #[arg(long)]
        ids: bool,
    },
    /// Every tracked change and comment thread with a stable id, its
    /// section and surrounding text: the list to review from.
    Review {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Only this person's changes and comments.
        #[arg(long)]
        by: Option<String>,
        /// Only items of this kind: insert, delete, replace, format, comment, change.
        #[arg(long)]
        kind: Option<String>,
        /// Only items in sections whose heading path contains this text.
        #[arg(long = "in", value_name = "SECTION")]
        within: Option<String>,
        /// Only unresolved comment threads.
        #[arg(long)]
        open: bool,
    },
    /// Accept and reject changes by review id in one go (ids don't shift).
    Resolve {
        file: PathBuf,
        /// Review ids to accept (from `docex review`), comma or space separated.
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        accept: Vec<String>,
        /// Review ids to reject.
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        reject: Vec<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Edit the document in $EDITOR; changes are saved as tracked changes.
    Edit {
        file: PathBuf,
        #[command(flatten)]
        w: WriteArgs,
    },
    /// Apply an edited Markdown file (`-` for stdin) to the document.
    Apply {
        file: PathBuf,
        markdown: PathBuf,
        #[command(flatten)]
        w: WriteArgs,
    },
    /// List tracked changes.
    Changes {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Accept tracked changes (all, by review id or number, or by author).
    Accept {
        file: PathBuf,
        /// Review ids (c12, from `docex review`) or numbers from `docex changes`.
        ids: Vec<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        by: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Reject tracked changes (all, by review id or number, or by author).
    Reject {
        file: PathBuf,
        ids: Vec<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        by: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// List comments.
    Comments {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Add, reply to, resolve or delete a comment.
    Comment {
        file: PathBuf,
        /// Comment text.
        text: Option<String>,
        /// Anchor the new comment on this exact text. Fails if it occurs more
        /// than once, unless --nth or --in narrows it to one.
        #[arg(long, value_name = "QUOTE")]
        on: Option<String>,
        /// Which occurrence of the quote (1-based).
        #[arg(long)]
        nth: Option<usize>,
        /// Only look in sections whose heading path contains this text.
        #[arg(long = "in", value_name = "SECTION")]
        within: Option<String>,
        /// Reply to comment ID.
        #[arg(long, value_name = "ID")]
        reply: Option<String>,
        /// Mark comment ID as resolved.
        #[arg(long, value_name = "ID")]
        resolve: Option<String>,
        /// Mark comment ID as open again.
        #[arg(long, value_name = "ID")]
        reopen: Option<String>,
        /// Delete comment ID and its replies.
        #[arg(long, value_name = "ID")]
        delete: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Create a .docx from Markdown.
    New {
        markdown: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Take styles, page setup, headers and footers from this .docx.
        #[arg(long)]
        template: Option<PathBuf>,
        #[arg(long)]
        author: Option<String>,
    },
    /// Show or switch the document's Track Changes setting (as in Word).
    /// `edit`, `apply` and `diff --track` follow it unless --track/--untracked is given.
    Track {
        file: PathBuf,
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Show a content diff between two documents, or write B onto A as tracked changes.
    Diff {
        a: PathBuf,
        b: PathBuf,
        /// Write B's content into A (to -o, or in place); tracked per A's setting or --track/--untracked.
        #[arg(long)]
        into: bool,
        #[command(flatten)]
        w: WriteArgs,
    },
}

#[derive(clap::Args)]
struct OutArgs {
    /// Write to this file instead of modifying in place.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Don't keep a FILE.docx~ backup when writing in place.
    #[arg(long)]
    no_backup: bool,
}

#[derive(clap::Args)]
struct WriteArgs {
    /// Record edits as tracked changes, whatever the document's setting.
    #[arg(long, conflicts_with = "untracked")]
    track: bool,
    /// Edit the text directly (no tracked changes), whatever the document's setting.
    #[arg(long)]
    untracked: bool,
    /// Author name for changes (default: $DOCEX_AUTHOR, then git user.name).
    #[arg(long)]
    author: Option<String>,
    #[command(flatten)]
    out: OutArgs,
}

fn author(explicit: Option<&str>) -> String {
    if let Some(a) = explicit {
        return a.to_string();
    }
    if let Ok(a) = std::env::var("DOCEX_AUTHOR") {
        if !a.trim().is_empty() {
            return a;
        }
    }
    if let Ok(o) = Command::new("git").args(["config", "user.name"]).output() {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    std::env::var("USER").unwrap_or_else(|_| "docex".into())
}

fn save(doc: &mut Docx, src: &Path, out: &OutArgs) -> Result<()> {
    let bytes = doc.to_bytes()?;
    let dest = out.output.as_deref().unwrap_or(src);
    if out.output.is_none() && !out.no_backup {
        let mut b = src.as_os_str().to_owned();
        b.push("~");
        std::fs::copy(src, &b).with_context(|| "writing backup")?;
    }
    let tmp = dest.with_extension("docx.docex-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn print_report(r: &Report, tracked: bool) {
    for w in &r.warnings {
        eprintln!("warning: {w}");
    }
    let s = &r.stats;
    if !r.changed() {
        eprintln!("no changes");
        return;
    }
    let mut parts = Vec::new();
    if s.insertions > 0 {
        parts.push(format!("{} chars inserted", s.insertions));
    }
    if s.deletions > 0 {
        parts.push(format!("{} chars deleted", s.deletions));
    }
    if s.formats > 0 {
        parts.push(format!("{} chars reformatted", s.formats));
    }
    if s.paragraphs_added > 0 {
        parts.push(format!("{} paragraphs added", s.paragraphs_added));
    }
    if s.paragraphs_removed > 0 {
        parts.push(format!("{} paragraphs removed", s.paragraphs_removed));
    }
    if s.kind_changes > 0 {
        parts.push(format!("{} paragraph styles changed", s.kind_changes));
    }
    if r.comments_added > 0 {
        parts.push(format!("{} comments added", r.comments_added));
    }
    eprintln!("{}{}", parts.join(", "), if tracked { " (tracked)" } else { "" });
}

/// Directory relative image paths in a Markdown file are resolved against.
fn base_of(md: &Path) -> PathBuf {
    if md.as_os_str() == "-" {
        return PathBuf::from(".");
    }
    md.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn read_md(p: &Path) -> Result<String> {
    if p.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
}

fn apply_and_save(doc: &mut Docx, file: &Path, md: &str, base: &Path, w: &WriteArgs) -> Result<()> {
    let tracked = if w.track {
        true
    } else if w.untracked {
        false
    } else {
        doc.track_changes()?
    };
    let opts = ApplyOpts { author: author(w.author.as_deref()), date: now_iso(), tracked, base_dir: Some(base.to_path_buf()) };
    let report = doc.apply(md, &opts)?;
    print_report(&report, opts.tracked);
    if doc.is_dirty() {
        save(doc, file, &w.out)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    // Exit quietly when output is piped into `head` and the like.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Cat { file, accepted, original, media, json, ids } => {
            let doc = Docx::open(&file)?;
            let view = if accepted { View::Accepted } else if original { View::Original } else { View::Review };
            if let Some(dir) = media {
                let n = doc.extract_media(&dir)?;
                eprintln!("{n} images extracted to {}", dir.display());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&blocks_json(&doc, view))?);
            } else if ids {
                print!("{}", doc.render_with_ids());
            } else {
                print!("{}", doc.render(view));
            }
        }
        Cmd::Review { file, json, by, kind, within, open } => {
            let doc = Docx::open(&file)?;
            let mut rv = doc.review();
            let total = rv.items.len();
            rv.items.retain(|i| {
                by.as_deref().is_none_or(|b| i.author.to_lowercase().contains(&b.to_lowercase()))
                    && kind.as_deref().is_none_or(|k| i.kind == k || (k == "change" && i.kind != "comment"))
                    && within.as_deref().is_none_or(|w| i.section.join(" › ").to_lowercase().contains(&w.to_lowercase()))
                    && (!open || (i.kind == "comment" && !i.resolved))
            });
            if json {
                println!("{}", serde_json::to_string_pretty(&rv)?);
            } else {
                print_review(&rv);
                if rv.items.len() != total {
                    println!("\n(showing {} of {total} items)", rv.items.len());
                }
            }
        }
        Cmd::Resolve { file, accept, reject, out } => {
            if accept.is_empty() && reject.is_empty() {
                bail!("give --accept and/or --reject with ids from `docex review`");
            }
            let mut doc = Docx::open(&file)?;
            let (a, r) = doc.resolve_ids(&accept, &reject)?;
            eprintln!("{a} accepted, {r} rejected");
            if a + r > 0 {
                save(&mut doc, &file, &out)?;
            }
        }
        Cmd::Edit { file, w } => {
            let mut doc = Docx::open(&file)?;
            let tracking = if w.track { true } else if w.untracked { false } else { doc.track_changes()? };
            eprintln!("track changes {} for this edit", if tracking { "on" } else { "off" });
            let dir = std::env::temp_dir().join(format!("docex-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            doc.extract_media(&dir)?;
            let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("doc".into());
            let md_path = dir.join(format!("{stem}.md"));
            let before = doc.render(View::Review);
            std::fs::write(&md_path, &before)?;
            let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
            let status = Command::new("sh")
                .arg("-c")
                .arg(format!("{editor} \"$1\""))
                .arg("sh")
                .arg(&md_path)
                .status()
                .with_context(|| format!("starting editor {editor}"))?;
            if !status.success() {
                bail!("editor exited with {status}; document unchanged (your text is in {})", md_path.display());
            }
            let after = std::fs::read_to_string(&md_path)?;
            if after == before {
                eprintln!("no changes");
            } else if let Err(e) = apply_and_save(&mut doc, &file, &after, &dir, &w) {
                bail!("{e:#}\nyour edited text is kept in {}", md_path.display());
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        Cmd::Apply { file, markdown, w } => {
            let mut doc = Docx::open(&file)?;
            let md = read_md(&markdown)?;
            apply_and_save(&mut doc, &file, &md, &base_of(&markdown), &w)?;
        }
        Cmd::Changes { file, json } => {
            let doc = Docx::open(&file)?;
            let ch = doc.changes();
            if json {
                println!("{}", serde_json::to_string_pretty(&ch)?);
            } else if ch.is_empty() {
                eprintln!("no tracked changes");
            } else {
                for c in ch {
                    let kind = serde_json::to_value(c.kind)?.as_str().unwrap_or("").to_string();
                    println!("{:>4}  {:<16} {:<20} {:<20} {}", c.n, kind, trunc(&c.author, 20), c.date, c.text.replace('\n', " "));
                }
            }
        }
        Cmd::Accept { file, ids, all, by, out } | Cmd::Reject { file, ids, all, by, out }
            if ids.is_empty() && !all && by.is_none() =>
        {
            let _ = (file, out);
            bail!("say which changes: numbers from `docex changes`, --all, or --by AUTHOR");
        }
        Cmd::Accept { file, ids, by, out, .. } => resolve(&file, &ids, by.as_deref(), true, &out)?,
        Cmd::Reject { file, ids, by, out, .. } => resolve(&file, &ids, by.as_deref(), false, &out)?,
        Cmd::Comments { file, json } => {
            let doc = Docx::open(&file)?;
            let list = doc.comment_list();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                eprintln!("no comments");
            } else {
                for c in &list {
                    let indent = if c.parent.is_some() { "    ↳ " } else { "" };
                    let state = if c.done { " [resolved]" } else { "" };
                    println!("{indent}#{} {} ({}){state}", c.id, c.author, c.date);
                    if c.parent.is_none() && !c.anchor.is_empty() {
                        println!("{indent}   on: \"{}\"", trunc(&c.anchor, 70));
                    }
                    for line in c.text.lines() {
                        println!("{indent}   {line}");
                    }
                }
            }
        }
        Cmd::Comment { file, text, on, nth, within, reply, resolve, reopen, delete, author: a, out } => {
            let mut doc = Docx::open(&file)?;
            let cid = |s: String| s.trim_start_matches('m').to_string();
            let (resolve, reopen, delete, reply) = (resolve.map(cid), reopen.map(cid), delete.map(cid), reply.map(cid));
            let who = author(a.as_deref());
            let date = now_iso();
            if let Some(id) = resolve {
                doc.set_comment_done(&id, true)?;
                eprintln!("comment {id} resolved");
            } else if let Some(id) = reopen {
                doc.set_comment_done(&id, false)?;
                eprintln!("comment {id} reopened");
            } else if let Some(id) = delete {
                doc.delete_comment(&id)?;
                eprintln!("comment {id} deleted");
            } else if let Some(parent) = reply {
                let Some(text) = text else { bail!("reply text missing") };
                let id = doc.reply_comment(&parent, &text, &who, &date)?;
                eprintln!("reply {id} added to comment {parent}");
            } else if let Some(quote) = on {
                let Some(text) = text else { bail!("comment text missing") };
                let id = doc.add_comment_at(&quote, nth, within.as_deref(), &text, &who, &date)?;
                eprintln!("comment m{id} added");
            } else {
                bail!("use --on QUOTE TEXT, --reply ID TEXT, --resolve ID, --reopen ID or --delete ID");
            }
            save(&mut doc, &file, &out)?;
        }
        Cmd::New { markdown, output, template, author: a } => {
            let md = read_md(&markdown)?;
            let (mut doc, report) = Docx::new_from_markdown(&md, template.as_deref(), &author(a.as_deref()), &base_of(&markdown))?;
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
            std::fs::write(&output, doc.to_bytes()?)?;
            eprintln!("wrote {}", output.display());
        }
        Cmd::Track { file, state, out } => {
            let mut doc = Docx::open(&file)?;
            match state.as_deref() {
                None => println!("track changes: {}", if doc.track_changes()? { "on" } else { "off" }),
                Some(s) => {
                    let on = s == "on";
                    if doc.track_changes()? == on {
                        eprintln!("track changes already {s}");
                    } else {
                        doc.set_track_changes(on)?;
                        save(&mut doc, &file, &out)?;
                        eprintln!("track changes {s}");
                    }
                }
            }
        }
        Cmd::Diff { a, b, into, w } => {
            let da = Docx::open(&a)?;
            let db = Docx::open(&b)?;
            if into {
                let mut da = da;
                apply_and_save(&mut da, &a, &db.render(View::Accepted), Path::new("."), &w)?;
            } else {
                let ta = da.render(View::Accepted);
                let tb = db.render(View::Accepted);
                let diff = similar::TextDiff::from_lines(&ta, &tb);
                print!("{}", diff.unified_diff().context_radius(2).header(&a.display().to_string(), &b.display().to_string()));
            }
        }
    }
    Ok(())
}

fn resolve(file: &Path, ids: &[String], by: Option<&str>, accept: bool, out: &OutArgs) -> Result<()> {
    let mut doc = Docx::open(file)?;
    let (nums, rids): (Vec<&String>, Vec<&String>) = ids.iter().partition(|i| i.chars().all(|c| c.is_ascii_digit()));
    if !nums.is_empty() && !rids.is_empty() {
        bail!("mix of change numbers and review ids; use one kind");
    }
    let n = if !rids.is_empty() {
        let rids: Vec<String> = rids.into_iter().cloned().collect();
        let (a, r) = if accept { doc.resolve_ids(&rids, &[])? } else { doc.resolve_ids(&[], &rids)? };
        a + r
    } else {
        let nums: Vec<usize> = nums.iter().map(|n| n.parse().unwrap()).collect();
        doc.resolve_changes(&nums, by, accept)?
    };
    eprintln!("{n} changes {}", if accept { "accepted" } else { "rejected" });
    if n > 0 {
        save(&mut doc, file, out)?;
    }
    Ok(())
}

fn print_review(rv: &docex_core::review::Review) {
    let s = &rv.summary;
    println!("{} changes · {} comment threads ({} open)", s.changes, s.threads, s.open_threads);
    for p in &s.people {
        let mut parts = Vec::new();
        for (n, one, many) in [
            (p.inserts, "insertion", "insertions"),
            (p.deletes, "deletion", "deletions"),
            (p.replaces, "replacement", "replacements"),
            (p.other, "other change", "other changes"),
            (p.comments, "comment", "comments"),
            (p.replies, "reply", "replies"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        }
        println!("  {:<24} {}", trunc(&p.name, 24), parts.join(", "));
    }
    let mut section: Option<Vec<String>> = None;
    for it in &rv.items {
        if section.as_ref() != Some(&it.section) {
            println!("\n§ {}", if it.section.is_empty() { "(start)".into() } else { it.section.join(" › ") });
            section = Some(it.section.clone());
        }
        let date = it.date.get(..10).unwrap_or("");
        let what = it.label.clone().unwrap_or_else(|| it.kind.clone());
        let state = if it.kind == "comment" { if it.resolved { " (resolved)" } else { " (open)" } } else { "" };
        println!("  {:<6} {} · {} · {}{}", it.id, it.author, date, what, state);
        if it.kind == "comment" {
            if let Some(a) = &it.anchor {
                println!("         on \"{}\"", trunc(&a.replace('\n', " "), 80));
            }
            println!("         {}", it.text.as_deref().unwrap_or("").replace('\n', "\n         "));
            for r in &it.replies {
                println!("         ↳ {} ({}): {}", r.author, r.id, r.text.replace('\n', " "));
            }
        } else if it.old.is_some() || it.new.is_some() {
            let mut line = it.before.clone();
            if let Some(o) = &it.old {
                line.push_str(&format!("[-{o}-]"));
            }
            if let Some(n) = &it.new {
                line.push_str(&format!("{{+{n}+}}"));
            }
            line.push_str(&it.after);
            println!("         {}", line.replace('\n', " "));
        } else if let Some(t) = &it.text {
            println!("         {}", trunc(t, 100));
        }
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

fn blocks_json(doc: &Docx, view: View) -> serde_json::Value {
    use docex_core::model::Block;
    use docex_core::render::view_text;
    let blocks = doc.blocks();
    let mut out = Vec::new();
    for b in &blocks {
        match b {
            Block::Para(p) => {
                let text = view_text(p, view);
                if text.is_empty() {
                    continue;
                }
                out.push(serde_json::json!({ "id": format!("b{}", p.path[0]), "type": "paragraph", "style": p.kind, "text": text }));
            }
            Block::Table(t) => {
                let rows: Vec<Vec<String>> = t
                    .rows
                    .iter()
                    .map(|r| r.cells.iter().map(|c| c.paras.iter().map(|p| view_text(p, view)).collect::<Vec<_>>().join("\n")).collect())
                    .collect();
                out.push(serde_json::json!({ "id": format!("b{}", t.idx), "type": "table", "editable": t.simple, "rows": rows }));
            }
            Block::Opaque { idx, label } => {
                out.push(serde_json::json!({ "id": format!("b{idx}"), "type": "opaque", "label": label }));
            }
        }
    }
    serde_json::Value::Array(out)
}
