//! Splitting an ABC file into its file header and tunes (§2.2 of the ABC 2.1 standard),
//! and each tune into classified lines.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::lex::{self, LineKind};
use crate::read::{FileHeader, ReadOptions, Reading, read_file_header};

/// One line of a tune or file header, classified, with its 1-based line number.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub number: usize,
    pub kind: LineKind,
}

/// An ABC file: an optional file header and one or more tunes.
#[derive(Debug, Clone)]
pub struct Book {
    tunes: Vec<Tune>,
}

/// One tune, from its `X:` line to the empty line or end of file that ends it.
#[derive(Debug, Clone)]
pub struct Tune {
    x: u32,
    title: Option<String>,
    /// tune's own lines.
    /// The file header, applied once and shared by every tune in the file: it sets their
    /// defaults (§2.2.2). An error in it is every tune's.
    pub(crate) header: Arc<Result<FileHeader>>,
    pub(crate) lines: Vec<(usize, String)>,
}

fn is_field(line: &str) -> bool {
    let mut chars = line.chars();
    matches!((chars.next(), chars.next()), (Some(c), Some(':')) if c.is_ascii_alphabetic() || c == '+')
}

fn is_blank(line: &str) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

impl Book {
    /// Splits a file into tunes. Each tune starts at an `X:` line and ends at the next
    /// empty line, or the end of the file. A first block of fields and directives before
    /// any `X:` is the file header: it is read once, here, and every tune starts from it.
    /// Free text between tunes is skipped. A tune's own lines are not read here: a tune that
    /// is not valid ABC fails when it is read ([`Tune::to_part`]), not when it is found, and
    /// so does every tune when the file header is the one at fault.
    pub fn parse(text: &str) -> Result<Book> {
        // §8: Unix, DOS and old Macintosh line ends.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<(usize, &str)> = text
            .split('\n')
            .enumerate()
            .map(|(i, l)| (i + 1, l.trim_end()))
            .collect();

        let mut file_header: Vec<Line> = Vec::new();
        let first_block_end = lines
            .iter()
            .position(|(_, l)| is_blank(l))
            .unwrap_or(lines.len());
        let first_block = &lines[..first_block_end];
        let header_like = first_block
            .iter()
            .all(|(_, l)| l.starts_with('%') || (is_field(l) && !l.starts_with("X:")));
        let mut start = 0;
        if header_like && !first_block.is_empty() {
            for (n, l) in first_block {
                file_header.push(Line {
                    number: *n,
                    kind: lex::classify(*n, l)?,
                });
            }
            start = first_block_end;
        }

        let header = Arc::new(read_file_header(&file_header));
        let mut tunes = Vec::new();
        let mut i = start;
        while i < lines.len() {
            let (number, line) = lines[i];
            if !line.starts_with("X:") {
                i += 1;
                continue;
            }
            let value = lex::strip_comment(&line[2..]);
            let value = value.trim();
            let x = if value.is_empty() {
                0
            } else {
                value.parse().map_err(|_| {
                    Error::parse(number, 3, format!("'X:{value}' is not a tune number"))
                })?
            };
            let mut body = Vec::new();
            while i < lines.len() && !is_blank(lines[i].1) {
                body.push((lines[i].0, lines[i].1.to_owned()));
                i += 1;
            }
            let title = body.iter().find_map(|(_, l)| {
                l.strip_prefix("T:")
                    .map(|t| lex::strip_comment(t).trim().to_owned())
            });
            tunes.push(Tune {
                x,
                title,
                header: Arc::clone(&header),
                lines: body,
            });
        }
        if tunes.is_empty() {
            return Err(Error::invalid(
                "the file holds no tune: a tune starts with an X: line",
            ));
        }
        Ok(Book { tunes })
    }

    /// Every tune, in file order.
    pub fn tunes(&self) -> &[Tune] {
        &self.tunes
    }

    /// Reads every tune, in file order. The file header's notices are listed once, with
    /// the first tune, though its lines apply to every tune.
    pub fn read_all(&self, options: &ReadOptions) -> Vec<Result<Reading>> {
        self.tunes
            .iter()
            .enumerate()
            .map(|(i, tune)| tune.read(options, i == 0))
            .collect()
    }

    /// The first tune numbered `x`.
    pub fn tune(&self, x: u32) -> Option<&Tune> {
        self.tunes.iter().find(|t| t.x == x)
    }
}

impl Tune {
    /// The tune's `X:` number, or 0 when its `X:` field is empty.
    pub fn x(&self) -> u32 {
        self.x
    }

    /// The tune's first `T:` field.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The line the tune's `X:` field is on.
    pub fn line(&self) -> usize {
        self.lines.first().map_or(0, |(n, _)| *n)
    }

    /// The tune's own lines, classified. In the tune body, a line such as `A:` is music,
    /// not a field: fields `A`-`G` and `X`-`Z` are not allowed in the body, to avoid
    /// confusion with notes and rests (§3).
    pub(crate) fn classified(&self) -> Result<Vec<Line>> {
        let mut out = Vec::new();
        let mut in_body = false;
        for (number, text) in &self.lines {
            let body_music = in_body
                && text
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c.to_ascii_uppercase(), 'A'..='G' | 'X'..='Z'));
            let kind = if body_music {
                LineKind::Music(lex::music(*number, &lex::strip_comment(text))?)
            } else {
                lex::classify(*number, text)?
            };
            if matches!(kind, LineKind::Field { letter: 'K', .. }) {
                in_body = true;
            }
            out.push(Line {
                number: *number,
                kind,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::Book;

    #[test]
    fn tunes_are_split_by_empty_lines_and_take_the_file_header() {
        let text = "%abc-2.1\nM:3/4\n\nsome free text\n\nX:1\nT:One\nK:C\nCDE|\n\nX:2\nT:Two\nK:G\n% a comment\nGAB|\n";
        let book = Book::parse(text).unwrap();
        assert_eq!(book.tunes().len(), 2);
        let two = book.tune(2).unwrap();
        assert_eq!(two.title(), Some("Two"));
        assert_eq!(two.line(), 11);
        assert!(
            std::sync::Arc::ptr_eq(&two.header, &book.tune(1).unwrap().header),
            "the file header is read once and shared"
        );
        assert_eq!(two.lines.len(), 5, "a comment line does not end a tune");
        assert!(Book::parse("just text\n").is_err());
    }
}
