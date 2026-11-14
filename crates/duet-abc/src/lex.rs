//! The tokenizer: splits a tune's lines into fields, directives and music code, and
//! music code into tokens, each with the line and column it starts at.
//!
//! Written from the ABC 2.1 standard: line kinds from §2.2 and §3, music code from §4.

use crate::error::{Error, Result};
use crate::ratio::Ratio;

/// Where a token starts: 1-based line and column, counted in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

/// One input line, classified (§2.2.1).
#[derive(Debug, Clone)]
pub(crate) enum LineKind {
    /// `X:value`, with any `%` comment removed and the value trimmed.
    Field { letter: char, value: String },
    /// `%%directive`, as text after the `%%`, comment-free and trimmed.
    Directive(String),
    /// Music code.
    Music(Vec<(Span, Tok)>),
    /// A comment line, or a line that is empty once its comment is removed.
    Nothing,
}

/// An accidental written before a note (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Acc {
    DoubleFlat,
    Flat,
    Natural,
    Sharp,
    DoubleSharp,
}

/// A note: accidental, letter, octave and length multiplier (§4.1 to §4.3).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NoteTok {
    pub acc: Option<Acc>,
    /// 0 to 6 for C to B.
    pub letter: u8,
    /// Octaves above the uppercase octave: `C` is 0, `c` is 1, `c'` is 2, `C,` is -1.
    pub octave: i32,
    pub len: Ratio,
    /// The note as written, for messages.
    pub text: String,
}

/// A bar line as written (§4.8), liberally read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BarTok {
    pub text: String,
    /// Colons before the first `|`: one for `:|`.
    pub ends: usize,
    /// Colons after the last `|`: one for `|:`.
    pub starts: usize,
    /// A thick or double bar (`||`, `|]`, `[|`), which also ends a variant ending.
    pub closing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Note(NoteTok),
    /// `z` (or invisible `x`) with its length.
    Rest {
        len: Ratio,
    },
    /// `Z` or `X`, with its number of bars.
    BarRest {
        bars: u32,
    },
    /// `y`, with any length written after it.
    Spacer,
    ChordStart,
    /// `]` ending a chord, with the length written after it.
    ChordEnd {
        len: Ratio,
    },
    Tie,
    /// `>` (`left_long`) or `<`, written `count` times.
    Broken {
        left_long: bool,
        count: u8,
    },
    /// `(p:q:r`.
    Tuplet {
        p: u32,
        q: Option<u32>,
        r: Option<u32>,
    },
    SlurStart,
    SlurEnd,
    Bar(BarTok),
    /// `[1`, or the `1` of `|1` or `:|2`: the passes it plays on.
    Ending(Vec<u32>),
    /// `[K:...]` and the like.
    InlineField {
        letter: char,
        value: String,
    },
    /// `!trill!` (the name between the marks), or a one-character shortcut such as `.`
    /// or `~`.
    Decoration(String),
    /// `"^text"` and its kin.
    Annotation(String),
    /// `"Am7"`.
    ChordSymbol(String),
    /// `{...}`, as written.
    Grace(String),
    /// `&`.
    Overlay,
    /// `+...+`, as written.
    OldChord(String),
    /// A run of spaces or tabs between music.
    Space,
    /// `\` at the end of a music line.
    Continuation,
}

/// Removes a `%` comment from the end of a line (§2.2.5). `\%` is a literal percent and
/// is kept as `%`.
pub(crate) fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'%') => {
                out.push('%');
                chars.next();
            }
            '%' => break,
            _ => out.push(c),
        }
    }
    out
}

/// Classifies one line of a tune. `line` is its 1-based number in the file.
pub(crate) fn classify(line: usize, text: &str) -> Result<LineKind> {
    let trimmed_start = text.trim_start();
    if let Some(directive) = text.strip_prefix("%%") {
        return Ok(LineKind::Directive(
            strip_comment(directive).trim().to_owned(),
        ));
    }
    if trimmed_start.starts_with('%') {
        return Ok(LineKind::Nothing);
    }
    let mut chars = text.chars();
    if let (Some(letter), Some(':')) = (chars.next(), chars.next())
        && (letter.is_ascii_alphabetic() || letter == '+')
    {
        return Ok(LineKind::Field {
            letter,
            value: strip_comment(chars.as_str()).trim().to_owned(),
        });
    }
    let code = strip_comment(text);
    if code.trim().is_empty() {
        return Ok(LineKind::Nothing);
    }
    Ok(LineKind::Music(music(line, &code)?))
}

struct Cursor<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    _text: &'a str,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        self.pos += 1;
        c
    }

    fn span(&self) -> Span {
        Span {
            line: self.line,
            col: self.pos + 1,
        }
    }

    fn err(&self, span: Span, msg: impl Into<String>) -> Error {
        Error::parse(span.line, span.col, msg)
    }

    fn number(&mut self) -> Option<u64> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    /// A length multiplier after a note, rest or chord (§4.3): `2`, `3/2`, `/`, `//`,
    /// `/4`. Missing means 1.
    fn length(&mut self) -> Result<Ratio> {
        let span = self.span();
        let num = self.number();
        if num == Some(0) {
            return Err(self.err(span, "a length of 0 is not a length"));
        }
        let mut len = Ratio::new(num.unwrap_or(1), 1);
        while self.peek() == Some('/') {
            self.bump();
            let den = self.number().unwrap_or(2);
            if den == 0 {
                return Err(self.err(span, "a length divided by 0 is not a length"));
            }
            len = len * Ratio::new(1, den);
        }
        Ok(len)
    }

    /// The text from `start` to here.
    fn since(&self, start: usize) -> String {
        self.chars[start..self.pos].iter().collect()
    }

    /// Reads up to the closing `close`, and returns what lies between.
    fn delimited(&mut self, open_span: Span, close: char, what: &str) -> Result<String> {
        let start = self.pos;
        while let Some(c) = self.bump() {
            if c == close {
                return Ok(self.chars[start..self.pos - 1].iter().collect());
            }
        }
        Err(self.err(open_span, format!("{what} is never closed on this line")))
    }
}

fn letter_index(c: char) -> Option<u8> {
    Some(match c.to_ascii_uppercase() {
        'C' => 0,
        'D' => 1,
        'E' => 2,
        'F' => 3,
        'G' => 4,
        'A' => 5,
        'B' => 6,
        _ => return None,
    })
}

/// The decoration shortcuts of §4.14 and §4.16's default `U:` table, plus the other
/// redefinable letters, which are user symbols when not defined.
fn is_shortcut(c: char) -> bool {
    matches!(c, '~' | 'H'..='W' | 'h'..='w')
}

/// Splits music code into tokens (§4).
pub(crate) fn music(line: usize, code: &str) -> Result<Vec<(Span, Tok)>> {
    let mut cur = Cursor {
        chars: code.chars().collect(),
        pos: 0,
        line,
        _text: code,
    };
    let mut out: Vec<(Span, Tok)> = Vec::new();
    while let Some(c) = cur.peek() {
        let span = cur.span();
        let start = cur.pos;
        match c {
            ' ' | '\t' => {
                while matches!(cur.peek(), Some(' ' | '\t')) {
                    cur.bump();
                }
                // A space before a continuation, or at the end, separates nothing.
                if cur.peek().is_some() && cur.peek() != Some('\\') {
                    out.push((span, Tok::Space));
                }
            }
            '`' => {
                // §4.7: back quotes between beamed notes are ignored.
                cur.bump();
            }
            '\\' => {
                cur.bump();
                if cur.chars[cur.pos..].iter().all(|c| c.is_whitespace()) {
                    cur.pos = cur.chars.len();
                    out.push((span, Tok::Continuation));
                } else {
                    return Err(cur.err(span, "a backslash continues a line only at its end"));
                }
            }
            '^' | '_' | '=' | 'A'..='G' | 'a'..='g' => {
                let acc = match c {
                    '^' => {
                        cur.bump();
                        if cur.peek() == Some('^') {
                            cur.bump();
                            Some(Acc::DoubleSharp)
                        } else {
                            Some(Acc::Sharp)
                        }
                    }
                    '_' => {
                        cur.bump();
                        if cur.peek() == Some('_') {
                            cur.bump();
                            Some(Acc::DoubleFlat)
                        } else {
                            Some(Acc::Flat)
                        }
                    }
                    '=' => {
                        cur.bump();
                        Some(Acc::Natural)
                    }
                    _ => None,
                };
                let Some(letter_char) = cur.peek().filter(|c| letter_index(*c).is_some()) else {
                    return Err(cur.err(span, "an accidental must be followed by a note"));
                };
                cur.bump();
                let mut octave = i32::from(letter_char.is_ascii_lowercase());
                while let Some(mark @ (',' | '\'')) = cur.peek() {
                    octave += if mark == '\'' { 1 } else { -1 };
                    cur.bump();
                }
                let len = cur.length()?;
                out.push((
                    span,
                    Tok::Note(NoteTok {
                        acc,
                        letter: letter_index(letter_char).unwrap_or(0),
                        octave,
                        len,
                        text: cur.since(start),
                    }),
                ));
            }
            'z' | 'x' => {
                cur.bump();
                let len = cur.length()?;
                out.push((span, Tok::Rest { len }));
            }
            'Z' | 'X' => {
                cur.bump();
                let bars = cur.number().unwrap_or(1);
                let bars = u32::try_from(bars)
                    .ok()
                    .filter(|b| *b > 0)
                    .ok_or_else(|| cur.err(span, "a multi-bar rest needs 1 or more bars"))?;
                out.push((span, Tok::BarRest { bars }));
            }
            'y' => {
                cur.bump();
                cur.length()?;
                out.push((span, Tok::Spacer));
            }
            '-' => {
                cur.bump();
                out.push((span, Tok::Tie));
            }
            '>' | '<' => {
                let mut count = 0u8;
                while cur.peek() == Some(c) {
                    cur.bump();
                    count += 1;
                }
                if count > 3 {
                    return Err(cur.err(span, "broken rhythm goes up to three marks"));
                }
                out.push((
                    span,
                    Tok::Broken {
                        left_long: c == '>',
                        count,
                    },
                ));
            }
            '(' => {
                cur.bump();
                if cur.peek().is_some_and(|c| c.is_ascii_digit()) {
                    let p = cur.number().unwrap_or(0);
                    let mut q = None;
                    let mut r = None;
                    if cur.peek() == Some(':') {
                        cur.bump();
                        q = cur.number();
                        if cur.peek() == Some(':') {
                            cur.bump();
                            r = cur.number();
                        }
                    }
                    let fit = |n: u64| u32::try_from(n).ok();
                    out.push((
                        span,
                        Tok::Tuplet {
                            p: fit(p).ok_or_else(|| cur.err(span, "tuplet too large"))?,
                            q: q.and_then(fit),
                            r: r.and_then(fit),
                        },
                    ));
                } else {
                    out.push((span, Tok::SlurStart));
                }
            }
            ')' => {
                cur.bump();
                out.push((span, Tok::SlurEnd));
            }
            '.' => {
                cur.bump();
                match cur.peek() {
                    // A dotted slur, tie or bar line is drawn dotted, and played the same.
                    Some('(' | '-' | '|' | ')') => {}
                    _ => out.push((span, Tok::Decoration(".".to_owned()))),
                }
            }
            '!' => {
                cur.bump();
                let name = cur.delimited(span, '!', "a !decoration!")?;
                out.push((span, Tok::Decoration(name)));
            }
            '"' => {
                cur.bump();
                let text = cur.delimited(span, '"', "a quoted string")?;
                let tok = match text.chars().next() {
                    Some('^' | '_' | '<' | '>' | '@') => Tok::Annotation(text),
                    _ => Tok::ChordSymbol(text),
                };
                out.push((span, tok));
            }
            '{' => {
                cur.bump();
                let text = cur.delimited(span, '}', "a grace note group")?;
                out.push((span, Tok::Grace(format!("{{{text}}}"))));
            }
            '+' => {
                cur.bump();
                let text = cur.delimited(span, '+', "a +...+ group")?;
                // Like a chord, it may carry a length.
                cur.length()?;
                out.push((span, Tok::OldChord(format!("+{text}+"))));
            }
            '&' => {
                cur.bump();
                out.push((span, Tok::Overlay));
            }
            '[' => match cur.peek_at(1) {
                Some('|') => bar(&mut cur, &mut out, span)?,
                Some(d) if d.is_ascii_digit() => {
                    cur.bump();
                    let endings = ending_list(&mut cur, span)?;
                    out.push((span, Tok::Ending(endings)));
                }
                Some(l) if l.is_ascii_alphabetic() && cur.peek_at(2) == Some(':') => {
                    cur.bump();
                    cur.bump();
                    cur.bump();
                    let value = cur.delimited(span, ']', "an inline field")?;
                    out.push((
                        span,
                        Tok::InlineField {
                            letter: l,
                            value: value.trim().to_owned(),
                        },
                    ));
                }
                _ => {
                    cur.bump();
                    out.push((span, Tok::ChordStart));
                }
            },
            ']' => {
                cur.bump();
                let len = cur.length()?;
                out.push((span, Tok::ChordEnd { len }));
            }
            '|' | ':' => bar(&mut cur, &mut out, span)?,
            c if is_shortcut(c) => {
                cur.bump();
                out.push((span, Tok::Decoration(c.to_string())));
            }
            other => {
                return Err(cur.err(span, format!("'{other}' is not ABC music code")));
            }
        }
    }
    Ok(out)
}

/// A bar line, starting at `|`, `:` or `[|`, with any ending number after it.
fn bar(cur: &mut Cursor<'_>, out: &mut Vec<(Span, Tok)>, span: Span) -> Result<()> {
    let start = cur.pos;
    loop {
        match cur.peek() {
            Some('|' | ':') => {
                cur.bump();
            }
            Some(']') if cur.pos > start => {
                cur.bump();
            }
            Some('[') if cur.peek_at(1) == Some('|') => {
                cur.bump();
            }
            _ => break,
        }
    }
    let text = cur.since(start);
    if !text.contains('|') && text != "::" {
        return Err(cur.err(span, format!("'{text}' is not a bar line")));
    }
    let ends = text.chars().take_while(|c| *c == ':').count();
    let starts = text.chars().rev().take_while(|c| *c == ':').count();
    let (ends, starts) = if text.chars().all(|c| c == ':') {
        // `::` is `:|` then `|:`.
        (1, 1)
    } else {
        (ends, starts)
    };
    let closing = text.contains("||") || text.contains(']') || text.contains('[');
    out.push((
        span,
        Tok::Bar(BarTok {
            text,
            ends,
            starts,
            closing,
        }),
    ));
    if cur.peek().is_some_and(|c| c.is_ascii_digit()) {
        let ending_span = cur.span();
        let endings = ending_list(cur, ending_span)?;
        out.push((ending_span, Tok::Ending(endings)));
    }
    Ok(())
}

/// `1`, `1,3`, `1-3` or `1,3,5-7` (§4.10).
fn ending_list(cur: &mut Cursor<'_>, span: Span) -> Result<Vec<u32>> {
    let mut passes = Vec::new();
    loop {
        let first = cur
            .number()
            .ok_or_else(|| cur.err(span, "an ending needs a number"))?;
        let last = if cur.peek() == Some('-') && cur.peek_at(1).is_some_and(|c| c.is_ascii_digit())
        {
            cur.bump();
            cur.number().unwrap_or(first)
        } else {
            first
        };
        for pass in first..=last {
            passes.push(
                u32::try_from(pass).map_err(|_| cur.err(span, "an ending number is too large"))?,
            );
        }
        if cur.peek() == Some(',') && cur.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            cur.bump();
        } else {
            break;
        }
    }
    Ok(passes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(code: &str) -> Vec<Tok> {
        music(1, code)
            .unwrap()
            .into_iter()
            .map(|(_, t)| t)
            .collect()
    }

    #[test]
    fn notes_lengths_and_octaves() {
        let t = toks("^c'3/2 _B,/ =e//");
        let Tok::Note(n) = &t[0] else { panic!() };
        assert_eq!(
            (n.acc, n.letter, n.octave, n.len),
            (Some(Acc::Sharp), 0, 2, Ratio::new(3, 2))
        );
        let Tok::Note(n) = &t[2] else { panic!() };
        assert_eq!(
            (n.acc, n.letter, n.octave, n.len),
            (Some(Acc::Flat), 6, -1, Ratio::new(1, 2))
        );
        let Tok::Note(n) = &t[4] else { panic!() };
        assert_eq!(n.len, Ratio::new(1, 4));
    }

    #[test]
    fn bars_and_endings() {
        let t = toks("|: A :|2 B [1 c || d :: e |]");
        let bars: Vec<(usize, usize, bool)> = t
            .iter()
            .filter_map(|t| match t {
                Tok::Bar(b) => Some((b.ends, b.starts, b.closing)),
                _ => None,
            })
            .collect();
        assert_eq!(
            bars,
            [
                (0, 1, false),
                (1, 0, false),
                (0, 0, true),
                (1, 1, false),
                (0, 0, true)
            ]
        );
        assert!(t.contains(&Tok::Ending(vec![2])));
        assert!(t.contains(&Tok::Ending(vec![1])));
        assert_eq!(toks("[1,3-4 A")[0], Tok::Ending(vec![1, 3, 4]));
    }

    #[test]
    fn spans_count_characters() {
        let spans: Vec<usize> = music(4, "A \"Am\"B !p!c")
            .unwrap()
            .iter()
            .map(|(s, _)| s.col)
            .collect();
        assert_eq!(spans, [1, 2, 3, 7, 8, 9, 12]);
    }

    #[test]
    fn comments_end_any_line() {
        assert_eq!(strip_comment("Q:1/4=96 % estimated"), "Q:1/4=96 ");
        assert_eq!(strip_comment("T:100\\% sure"), "T:100% sure");
    }
}
