//! Typing aids for the JSON body editor, as code editors do: brackets and quotes close
//! themselves, a typed closer steps over the one already there, Backspace removes an empty
//! pair, and Enter keeps the indent (one level more after an opener). Pure functions over
//! the text before the key and its selection; the editor applies the result only when egui
//! did exactly the plain edit ([`plain`]), so anything unusual is left as egui made it.

use std::ops::Range;

/// One indent level, as Format writes it.
const INDENT: &str = "  ";

/// A key that may get help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Char(char),
    Enter,
    Backspace,
}

/// The text after a key, and the selection in it (byte offsets; empty for a cursor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub selection: Range<usize>,
}

impl Edit {
    fn at(text: String, cursor: usize) -> Self {
        Self {
            text,
            selection: cursor..cursor,
        }
    }
}

fn closer(opener: char) -> Option<char> {
    match opener {
        '{' => Some('}'),
        '[' => Some(']'),
        '"' => Some('"'),
        _ => None,
    }
}

/// Whether `before` ends inside a JSON string, and right after a backslash there: quotes
/// open and close strings, and a backslash inside one escapes the next character.
fn string_state(before: &str) -> (bool, bool) {
    let mut inside = false;
    let mut escaped = false;
    for c in before.chars() {
        match (inside, escaped, c) {
            (true, true, _) => escaped = false,
            (true, false, '\\') => escaped = true,
            (_, _, '"') => inside = !inside,
            _ => {}
        }
    }
    (inside, escaped)
}

/// What egui does with `input` on its own: the edit help is compared against.
pub fn plain(text: &str, selection: Range<usize>, input: Input) -> Edit {
    let (start, end) = (selection.start, selection.end);
    let mut out = String::with_capacity(text.len() + 1);
    match input {
        Input::Char(c) => {
            out.push_str(&text[..start]);
            out.push(c);
            out.push_str(&text[end..]);
            Edit::at(out, start + c.len_utf8())
        }
        Input::Enter => {
            out.push_str(&text[..start]);
            out.push('\n');
            out.push_str(&text[end..]);
            Edit::at(out, start + 1)
        }
        Input::Backspace if start < end => {
            out.push_str(&text[..start]);
            out.push_str(&text[end..]);
            Edit::at(out, start)
        }
        Input::Backspace => {
            let from = text[..start]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            out.push_str(&text[..from]);
            out.push_str(&text[start..]);
            Edit::at(out, from)
        }
    }
}

/// The helped edit for `input`, or `None` where the plain one is right.
pub fn assisted(text: &str, selection: Range<usize>, input: Input) -> Option<Edit> {
    let (start, end) = (selection.start, selection.end);
    let (before, after) = (&text[..start], &text[end..]);
    let next = after.chars().next();
    let (string, escaped) = string_state(before);
    match input {
        // A selection is wrapped, and stays selected inside the pair.
        Input::Char(c) if start < end => {
            let close = closer(c)?;
            (!string || c == '"').then_some(())?;
            let text = format!("{before}{c}{}{close}{after}", &text[start..end]);
            let inner = start + c.len_utf8();
            Some(Edit {
                text,
                selection: inner..inner + (end - start),
            })
        }
        // Step over a quote that ends the string.
        Input::Char('"') if string => {
            (next == Some('"') && !escaped).then(|| Edit::at(text.to_owned(), start + 1))
        }
        Input::Char(c @ ('{' | '[' | '"')) if !string => {
            let room = next.is_none_or(|n| n.is_whitespace() || matches!(n, ',' | '}' | ']'));
            let close = closer(c)?;
            room.then(|| Edit::at(format!("{before}{c}{close}{after}"), start + 1))
        }
        Input::Char(c @ ('}' | ']')) if !string => {
            if next == Some(c) {
                return Some(Edit::at(text.to_owned(), start + 1));
            }
            // On a line of nothing but indent, the closer goes one level out.
            let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
            let dedent = line.len().min(INDENT.len());
            (!line.is_empty() && line.chars().all(|ch| ch == ' ')).then(|| {
                let kept = &before[..start - dedent];
                Edit::at(format!("{kept}{c}{after}"), start - dedent + 1)
            })
        }
        Input::Enter if start == end && !string => {
            let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let opener = before.trim_end_matches([' ', '\t']).chars().next_back();
            let Some(close) = opener.filter(|o| matches!(o, '{' | '[')).and_then(closer) else {
                return (!indent.is_empty()).then(|| {
                    Edit::at(
                        format!("{before}\n{indent}{after}"),
                        start + 1 + indent.len(),
                    )
                });
            };
            let inner = format!("{indent}{INDENT}");
            let cursor = start + 1 + inner.len();
            let rest = after.trim_start_matches([' ', '\t']);
            Some(if rest.starts_with(close) {
                Edit::at(format!("{before}\n{inner}\n{indent}{rest}"), cursor)
            } else {
                Edit::at(format!("{before}\n{inner}{after}"), cursor)
            })
        }
        Input::Backspace if start == end => {
            let previous = before.chars().next_back()?;
            (closer(previous) == next && next.is_some()).then(|| {
                let from = start - previous.len_utf8();
                Edit::at(format!("{}{}", &before[..from], &after[1..]), from)
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `input` on `text`, where `|` marks the cursor or `[…]` a selection; the result
    /// is written the same way.
    fn typed(text: &str, input: Input) -> String {
        let (text, selection) = parse(text);
        let edit = assisted(&text, selection.clone(), input)
            .unwrap_or_else(|| plain(&text, selection, input));
        show(&edit)
    }

    fn parse(marked: &str) -> (String, Range<usize>) {
        if let Some(at) = marked.find('|') {
            return (marked.replacen('|', "", 1), at..at);
        }
        let start = marked.find('«').expect("a cursor or selection");
        let end = marked.find('»').expect("a closed selection") - '«'.len_utf8();
        (marked.replacen('«', "", 1).replacen('»', "", 1), start..end)
    }

    fn show(edit: &Edit) -> String {
        let Range { start, end } = edit.selection.clone();
        if start == end {
            format!("{}|{}", &edit.text[..start], &edit.text[start..])
        } else {
            format!(
                "{}«{}»{}",
                &edit.text[..start],
                &edit.text[start..end],
                &edit.text[end..]
            )
        }
    }

    #[test]
    fn openers_close_themselves_where_there_is_room() {
        assert_eq!(typed("|", Input::Char('{')), "{|}");
        assert_eq!(typed("{\"a\": |}", Input::Char('[')), "{\"a\": [|]}");
        assert_eq!(typed("[1, |]", Input::Char('"')), "[1, \"|\"]");
        assert_eq!(typed("|x", Input::Char('{')), "{|x", "not before a word");
        assert_eq!(
            typed("\"a |\"", Input::Char('{')),
            "\"a {|\"",
            "not in a string"
        );
    }

    #[test]
    fn placeholders_close_too() {
        let once = typed("{\"n\": |}", Input::Char('{'));
        assert_eq!(once, "{\"n\": {|}}");
        assert_eq!(typed(&once, Input::Char('{')), "{\"n\": {{|}}}");
        let typed_name = "{\"n\": {{id|}}}";
        let step = typed(typed_name, Input::Char('}'));
        assert_eq!(step, "{\"n\": {{id}|}}");
        assert_eq!(typed(&step, Input::Char('}')), "{\"n\": {{id}}|}");
    }

    #[test]
    fn closers_and_quotes_step_over() {
        assert_eq!(typed("{|}", Input::Char('}')), "{}|");
        assert_eq!(typed("[1|]", Input::Char(']')), "[1]|");
        assert_eq!(typed("\"ab|\"", Input::Char('"')), "\"ab\"|");
        assert_eq!(typed("\"a\\|\"", Input::Char('"')), "\"a\\\"|\"", "escaped");
        assert_eq!(typed("{|", Input::Char('}')), "{}|", "plain without one");
    }

    #[test]
    fn a_closer_on_an_indented_line_goes_one_level_out() {
        assert_eq!(
            typed("{\n  \"a\": 1\n    |", Input::Char('}')),
            "{\n  \"a\": 1\n  }|"
        );
        assert_eq!(typed("{\n  |", Input::Char('}')), "{\n}|");
        assert_eq!(typed("[1,\n |", Input::Char(']')), "[1,\n]|");
    }

    #[test]
    fn selections_are_wrapped() {
        assert_eq!(typed("«name»", Input::Char('"')), "\"«name»\"");
        assert_eq!(typed("«1, 2»", Input::Char('[')), "[«1, 2»]");
        assert_eq!(
            typed("\"«ab»\"", Input::Char('{')),
            "\"{|\"",
            "not in a string"
        );
        assert_eq!(typed("«ab»", Input::Char('x')), "x|");
    }

    #[test]
    fn enter_indents() {
        assert_eq!(typed("{|}", Input::Enter), "{\n  |\n}");
        assert_eq!(typed("  [|]", Input::Enter), "  [\n    |\n  ]");
        assert_eq!(typed("{ |  }", Input::Enter), "{ \n  |\n}");
        assert_eq!(typed("{|", Input::Enter), "{\n  |");
        assert_eq!(typed("  \"a\": 1,|", Input::Enter), "  \"a\": 1,\n  |");
        assert_eq!(typed("1|", Input::Enter), "1\n|", "no indent to keep");
        assert_eq!(typed("\"{|\"", Input::Enter), "\"{\n|\"", "not in a string");
    }

    #[test]
    fn backspace_removes_an_empty_pair() {
        assert_eq!(typed("{|}", Input::Backspace), "|");
        assert_eq!(typed("[\"|\"]", Input::Backspace), "[|]");
        assert_eq!(typed("{a|}", Input::Backspace), "{|}");
        assert_eq!(typed("«ab»", Input::Backspace), "|");
        assert_eq!(typed("é|", Input::Backspace), "|", "a whole character");
        assert_eq!(typed("|", Input::Backspace), "|");
    }

    fn in_string(before: &str) -> bool {
        string_state(before).0
    }

    #[test]
    fn strings_are_tracked_across_escapes() {
        assert!(in_string("{\"a"));
        assert!(!in_string("{\"a\""));
        assert!(in_string("\"a\\\""));
        assert!(!in_string("\"a\\\\\""));
    }
}
