//! Word's Compare: another version's text written into this document as new
//! edits, while the changes already open here stay as they are.
//!
//! The other version is compared with this one *as if its open changes were
//! accepted* (that is what a version sent back usually contains). The
//! differences are then made in this document's Review Markdown, around the
//! CriticMarkup of the open changes, and applied like any other edit.

use std::time::Duration;

/// The accepted text of a Review rendering, and where each of its characters
/// sits in that rendering.
struct Accepted {
    chars: Vec<char>,
    /// Byte offset in the rendering of each character.
    at: Vec<usize>,
    /// Byte offset to insert new text at, before each character: in front of
    /// the markup when the character opens an insertion or a comment anchor.
    before: Vec<usize>,
}

/// Leave out deletions and comments, keep insertions and anchored text.
fn accept(review: &str) -> Accepted {
    let mut out = Accepted { chars: Vec::new(), at: Vec::new(), before: Vec::new() };
    let (mut ins, mut hl) = (0usize, 0usize);
    let mut open: Option<usize> = None;
    let mut i = 0;
    while i < review.len() {
        let rest = &review[i..];
        let skip_to = |close: &str| rest.find(close).map_or(review.len(), |k| i + k + close.len());
        if rest.starts_with("{--") {
            i = skip_to("--}");
        } else if rest.starts_with("{>>") {
            i = skip_to("<<}");
        } else if rest.starts_with("{++") || rest.starts_with("{==") {
            if rest.starts_with("{++") {
                ins += 1;
            } else {
                hl += 1;
            }
            open.get_or_insert(i);
            i += 3;
        } else if ins > 0 && rest.starts_with("++}") {
            ins -= 1;
            i += 3;
        } else if hl > 0 && rest.starts_with("==}") {
            hl -= 1;
            i += 3;
        } else {
            let c = rest.chars().next().unwrap();
            out.chars.push(c);
            out.at.push(i);
            out.before.push(open.take().unwrap_or(i));
            i += c.len_utf8();
        }
    }
    out
}

/// The text of a Review rendering with every open change accepted and the
/// comments left out.
pub fn accepted_text(review: &str) -> String {
    accept(review).chars.into_iter().collect()
}

/// `review` (this document's Review Markdown) edited so that its accepted
/// text becomes `theirs`, keeping the open changes' markup.
pub fn merge(review: &str, theirs: &str) -> String {
    let a = accept(review);
    let mine: String = a.chars.iter().collect();
    let diff = similar::TextDiff::configure().timeout(Duration::from_secs(10)).diff_words(&mine, theirs);
    let mut removed = vec![false; review.len()];
    let mut inserts: Vec<(usize, String)> = Vec::new();
    let mut k = 0; // character index in `mine`
    for ch in diff.iter_all_changes() {
        let n = ch.value().chars().count();
        match ch.tag() {
            similar::ChangeTag::Equal => k += n,
            similar::ChangeTag::Delete => {
                for j in k..k + n {
                    removed[a.at[j]..a.at[j] + a.chars[j].len_utf8()].fill(true);
                }
                k += n;
            }
            similar::ChangeTag::Insert => {
                let at = a.before.get(k).copied().unwrap_or(review.len());
                match inserts.last_mut() {
                    Some((p, s)) if *p == at => s.push_str(ch.value()),
                    _ => inserts.push((at, ch.value().to_string())),
                }
            }
        }
    }
    let mut out = String::with_capacity(review.len() + theirs.len() / 8);
    let mut next = inserts.into_iter().peekable();
    for (i, c) in review.char_indices() {
        while let Some((_, s)) = next.next_if(|(p, _)| *p <= i) {
            out.push_str(&s);
        }
        if !removed[i] {
            out.push(c);
        }
    }
    for (_, s) in next {
        out.push_str(&s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_text_drops_deletions_and_comments() {
        let r = "The {--old--}{++new++} plan {==is late==}{>>A: why?<<}.\n";
        assert_eq!(accepted_text(r), "The new plan is late.\n");
    }

    #[test]
    fn merge_keeps_open_changes() {
        let r = "The {--old--}{++new++} plan costs 12 euros.\n";
        // Theirs accepted the open change and changed the figure.
        let m = merge(r, "The new plan costs 15 euros.\n");
        assert_eq!(m, "The {--old--}{++new++} plan costs 15 euros.\n");
        // Unchanged: nothing to do.
        assert_eq!(merge(r, &accepted_text(r)), r);
    }

    #[test]
    fn merge_inserts_in_front_of_markup() {
        let r = "A {++b++} c\n";
        assert_eq!(merge(r, "A x b c\n"), "A x {++b++} c\n");
        assert_eq!(accepted_text(&merge(r, "A x b c\n")), "A x b c\n");
    }
}
