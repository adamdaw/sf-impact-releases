//! Lex only enough JavaScript to recognise module specifiers. Comments, quoted
//! strings, regex bodies and template text cannot manufacture imports. Template
//! expressions are scanned as code. Nonliteral dynamic imports are diagnostics,
//! and so are TypeScript type-only imports and re-exports, which are not modelled.
#[derive(Debug)]
enum Kind {
    Word(String),
    String(String),
    Punct(u8),
    Other,
}
#[derive(Debug)]
struct Token {
    kind: Kind,
    at: usize,
}
impl Token {
    fn word(&self, word: &str) -> bool {
        matches!(&self.kind,Kind::Word(w) if w == word)
    }
    fn punct(&self, ch: u8) -> bool {
        matches!(self.kind,Kind::Punct(p) if p == ch)
    }
    fn identifier(&self) -> bool {
        matches!(self.kind, Kind::Word(_))
    }
}
#[derive(Default)]
pub struct Imports {
    pub modules: Vec<(String, usize)>,
    pub errors: Vec<usize>,
}
/// How deeply template literals may nest inside one another's `${}`. Past
/// it the file is a lexical error there and is read no further: each level
/// is a level of recursion.
const TEMPLATE_DEPTH: usize = 1_000;
struct Lexer<'a> {
    source: &'a str,
    at: usize,
    tokens: Vec<Token>,
    errors: Vec<usize>,
    /// The tokens of the `(` and the `{` that are not closed yet, innermost last.
    open_parens: Vec<usize>,
    open_braces: Vec<usize>,
    /// The token of the `(` or `{` that the last token closes, if it closes one.
    opened: Option<usize>,
    /// The template literals that the scanner is inside.
    templates: usize,
}
fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') || byte >= 128
}
impl Lexer<'_> {
    fn byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.at).copied()
    }
    fn starts(&self, text: &str) -> bool {
        self.source.as_bytes()[self.at..].starts_with(text.as_bytes())
    }
    fn push(&mut self, kind: Kind, at: usize) {
        let index = self.tokens.len();
        self.opened = match kind {
            Kind::Punct(b'(') => {
                self.open_parens.push(index);
                None
            }
            Kind::Punct(b'{') => {
                self.open_braces.push(index);
                None
            }
            Kind::Punct(b')') => self.open_parens.pop(),
            Kind::Punct(b'}') => self.open_braces.pop(),
            _ => None,
        };
        self.tokens.push(Token { kind, at });
    }
    fn string(&mut self, quote: u8) -> Option<String> {
        let start = self.at;
        self.at += 1;
        let mut result = String::new();
        while let Some(byte) = self.byte() {
            if byte == quote {
                self.at += 1;
                return Some(result);
            }
            if matches!(byte, b'\n' | b'\r') {
                break;
            }
            if byte != b'\\' {
                let ch = self.source[self.at..].chars().next().unwrap();
                result.push(ch);
                self.at += ch.len_utf8();
                continue;
            }
            self.at += 1;
            let Some(escaped) = self.byte() else {
                break;
            };
            self.at += 1;
            match escaped {
                b'\n' => {}
                b'\r' => {
                    if self.byte() == Some(b'\n') {
                        self.at += 1;
                    }
                }
                b'n' => result.push('\n'),
                b'r' => result.push('\r'),
                b't' => result.push('\t'),
                b'b' => result.push('\u{8}'),
                b'f' => result.push('\u{c}'),
                b'v' => result.push('\u{b}'),
                b'0' => result.push('\0'),
                b'x' | b'u' => {
                    let braces = escaped == b'u' && self.byte() == Some(b'{');
                    if braces {
                        self.at += 1;
                    }
                    let from = self.at;
                    let end = if braces {
                        self.source.as_bytes()[from..]
                            .iter()
                            .position(|b| *b == b'}')
                            .map(|n| from + n)
                    } else {
                        Some(from + if escaped == b'x' { 2 } else { 4 })
                    };
                    let ch = end.and_then(|end| {
                        self.source
                            .get(from..end)
                            .and_then(|s| u32::from_str_radix(s, 16).ok())
                            // JS strings can contain isolated UTF-16 surrogates.
                            // They cannot occur in a Salesforce API name; preserve
                            // the string boundary without claiming a lexical error.
                            .and_then(|value| {
                                if (0xd800..=0xdfff).contains(&value) {
                                    Some(char::REPLACEMENT_CHARACTER)
                                } else {
                                    char::from_u32(value)
                                }
                            })
                            .map(|ch| (end, ch))
                    });
                    if let Some((end, ch)) = ch {
                        result.push(ch);
                        self.at = end + usize::from(braces);
                    } else {
                        self.errors.push(start);
                        return None;
                    }
                }
                _ => {
                    let ch = self.source[self.at - 1..].chars().next().unwrap();
                    result.push(ch);
                    self.at += ch.len_utf8() - 1;
                }
            }
        }
        self.errors.push(start);
        None
    }
    fn template(&mut self) {
        let start = self.at;
        self.at += 1;
        while let Some(byte) = self.byte() {
            match byte {
                b'\\' => {
                    self.at += 1;
                    if self.byte().is_some() {
                        self.at += 1;
                    }
                }
                b'`' => {
                    self.at += 1;
                    self.push(Kind::Other, start);
                    return;
                }
                b'$' if self.starts("${") && self.templates == TEMPLATE_DEPTH => {
                    self.errors.push(self.at);
                    self.at = self.source.len();
                    return;
                }
                b'$' if self.starts("${") => {
                    self.at += 2;
                    self.templates += 1;
                    self.code(true);
                    self.templates -= 1;
                }
                _ => self.at += 1,
            }
        }
        self.errors.push(start);
    }
    fn regex_allowed(&self) -> bool {
        self.tokens.last().is_none_or(|t| match &t.kind {
            Kind::Word(w) => matches!(
                w.as_str(),
                "return"
                    | "throw"
                    | "case"
                    | "typeof"
                    | "void"
                    | "delete"
                    | "yield"
                    | "await"
                    | "else"
                    | "do"
                    | "instanceof"
                    | "in"
                    | "of"
            ),
            Kind::Punct(b')') => self.after_control_condition(),
            Kind::Punct(b'}') => self.after_statement_block(),
            Kind::Punct(b'+' | b'-') if self.tokens.len() > 1 => {
                !self.tokens[self.tokens.len() - 2].punct(match t.kind {
                    Kind::Punct(p) => p,
                    _ => unreachable!(),
                })
            }
            Kind::Punct(p) => *p != b']',
            _ => false,
        })
    }
    fn after_control_condition(&self) -> bool {
        self.opened.and_then(|i| i.checked_sub(1)).is_some_and(|i| {
            ["if", "while", "for", "with", "switch", "catch"]
                .iter()
                .any(|word| self.tokens[i].word(word))
        })
    }
    fn after_statement_block(&self) -> bool {
        self.opened.is_some_and(|i| {
            i == 0
                || self.tokens[i - 1].punct(b')')
                || self.tokens[i - 1].punct(b';')
                || ["else", "try", "finally", "do"]
                    .iter()
                    .any(|word| self.tokens[i - 1].word(word))
        })
    }
    fn regex(&mut self) {
        let start = self.at;
        self.at += 1;
        let mut class = false;
        while let Some(byte) = self.byte() {
            self.at += 1;
            match byte {
                b'\\' => {
                    if self.byte().is_some() {
                        self.at += 1;
                    }
                }
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => {
                    while self.byte().is_some_and(identifier) {
                        self.at += 1;
                    }
                    self.push(Kind::Other, start);
                    return;
                }
                b'\r' | b'\n' => break,
                _ => {}
            }
        }
        self.errors.push(start);
    }
    fn code(&mut self, interpolation: bool) {
        let mut braces = 0;
        while let Some(byte) = self.byte() {
            let start = self.at;
            if byte.is_ascii_whitespace() {
                self.at += 1;
                continue;
            }
            if self.starts("//") {
                while self.byte().is_some_and(|b| b != b'\n') {
                    self.at += 1;
                }
                continue;
            }
            if self.starts("/*") {
                match self.source[self.at + 2..].find("*/") {
                    Some(n) => self.at += n + 4,
                    None => {
                        self.errors.push(start);
                        self.at = self.source.len();
                    }
                }
                continue;
            }
            match byte {
                b'\'' | b'"' => {
                    if let Some(value) = self.string(byte) {
                        self.push(Kind::String(value), start + 1);
                    }
                }
                b'`' => self.template(),
                b'/' if self.regex_allowed() => self.regex(),
                b'{' => {
                    braces += 1;
                    self.push(Kind::Punct(byte), start);
                    self.at += 1;
                }
                b'}' if braces == 0 && interpolation => {
                    self.at += 1;
                    return;
                }
                b'}' => {
                    braces -= 1;
                    self.push(Kind::Punct(byte), start);
                    self.at += 1;
                }
                b if identifier(b) => {
                    self.at += 1;
                    while self.byte().is_some_and(identifier) {
                        self.at += 1;
                    }
                    self.push(Kind::Word(self.source[start..self.at].into()), start);
                }
                _ => {
                    self.push(Kind::Punct(byte), start);
                    self.at += 1;
                }
            }
        }
        if interpolation {
            self.errors.push(self.at);
        }
    }
}

fn bindings(tokens: &[Token], mut i: usize) -> Option<usize> {
    if tokens.get(i)?.identifier() {
        i += 1;
        if !tokens.get(i)?.punct(b',') {
            return Some(i);
        }
        i += 1;
    }
    if tokens.get(i)?.punct(b'*') {
        if !tokens.get(i + 1)?.word("as") || !tokens.get(i + 2)?.identifier() {
            return None;
        }
        return Some(i + 3);
    }
    if !tokens.get(i)?.punct(b'{') {
        return None;
    }
    i += 1;
    while !tokens.get(i)?.punct(b'}') {
        if !tokens.get(i)?.identifier() && !matches!(tokens[i].kind, Kind::String(_)) {
            return None;
        }
        i += 1;
        if tokens.get(i)?.word("as") {
            if !tokens.get(i + 1)?.identifier() {
                return None;
            }
            i += 2;
        }
        if tokens.get(i)?.punct(b',') {
            i += 1;
        } else if !tokens.get(i)?.punct(b'}') {
            return None;
        }
    }
    Some(i + 1)
}

pub fn scan(source: &str) -> Imports {
    let mut lexer = Lexer {
        source,
        at: 0,
        tokens: Vec::new(),
        errors: Vec::new(),
        open_parens: Vec::new(),
        open_braces: Vec::new(),
        opened: None,
        templates: 0,
    };
    lexer.code(false);
    let tokens = lexer.tokens;
    let mut result = Imports {
        modules: Vec::new(),
        errors: lexer.errors,
    };
    for (i, t) in tokens.iter().enumerate() {
        let import = t.word("import");
        if !import && !t.word("export") {
            continue;
        }
        if i > 0 && tokens[i - 1].punct(b'.') {
            continue;
        }
        if tokens
            .get(i + 1)
            .is_some_and(|t| t.punct(b':') || t.punct(b'.'))
        {
            continue;
        }
        // `import type {…} from` and `export type * from` name a module for
        // types only. `import type from` imports a binding named `type`.
        let type_only = tokens.get(i + 1).is_some_and(|t| t.word("type"))
            && tokens
                .get(i + 2)
                .is_some_and(|t| t.punct(b'{') || t.punct(b'*'));
        let module = if import
            && tokens
                .get(i + 1)
                .is_some_and(|t| matches!(t.kind, Kind::String(_)))
        {
            Some(i + 1)
        } else if import && tokens.get(i + 1).is_some_and(|t| t.punct(b'(')) {
            if tokens.get(i + 3).is_some_and(|t| t.punct(b')')) {
                Some(i + 2)
            } else {
                None
            }
        } else {
            let first = i + 1 + usize::from(type_only);
            let clause = if !import && tokens.get(first).is_some_and(|t| t.punct(b'*')) {
                if tokens.get(first + 1).is_some_and(|t| t.word("as")) {
                    Some(first + 3)
                } else {
                    Some(first + 1)
                }
            } else {
                bindings(&tokens, first)
            };
            if !import && !clause.is_some_and(|n| tokens.get(n).is_some_and(|t| t.word("from"))) {
                continue;
            }
            clause
                .filter(|n| tokens.get(*n).is_some_and(|t| t.word("from")))
                .map(|n| n + 1)
        };
        if !type_only
            && let Some(Token {
                kind: Kind::String(value),
                at,
            }) = module.and_then(|m| tokens.get(m))
        {
            result.modules.push((value.clone(), *at));
        } else {
            result.errors.push(t.at);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn modules(source: &str) -> Vec<String> {
        scan(source).modules.into_iter().map(|(m, _)| m).collect()
    }

    #[test]
    fn a_slash_after_a_closing_bracket_is_told_apart_by_what_the_bracket_opened() {
        // After a control condition, and after a statement block, a regex.
        let after_if = "if (f(a)) /import a from 'c\\/no'/.test(s);\nimport b from 'c/yes';";
        let after_block =
            "function f() {}\n/import a from 'c\\/no'/.test(s);\nimport b from 'c/yes';";
        // After a parenthesised value, and after an object, a division.
        let divided = "x = (f(a)) / 2 / (b);\nimport b from 'c/yes';";
        let object = "x = {a: 1} / 2 / {b: 2};\nimport b from 'c/yes';";
        // After a closer that closes nothing, a division.
        let unmatched = ") / 2 / (b);\n} / 2 / {b: 2};\nimport b from 'c/yes';";
        for source in [after_if, after_block, divided, object, unmatched] {
            assert_eq!(modules(source), ["c/yes"], "{source}");
            assert!(scan(source).errors.is_empty(), "{source}");
        }
    }

    #[test]
    fn slashes_after_closers_that_close_nothing_take_linear_time() {
        // 400 KB took 15 s when each `/` looked back over every token.
        let source = ")/".repeat(200_000);
        let started = Instant::now();

        scan(&source);

        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn templates_nested_past_the_limit_are_an_error_and_end_the_scan() {
        let deep = format!("import 'c/before';\n{}", "`${".repeat(1_000_000));

        let found = scan(&deep);

        assert_eq!(found.modules, [("c/before".to_string(), 8)]);
        assert_eq!(found.errors.iter().min(), Some(&19));
    }

    #[test]
    fn templates_nested_to_the_limit_are_read() {
        let source = format!(
            "{}import('c/inner'){}",
            "`${".repeat(TEMPLATE_DEPTH),
            "}`".repeat(TEMPLATE_DEPTH)
        );

        let found = scan(&source);

        assert!(found.errors.is_empty(), "{:?}", found.errors);
        assert_eq!(modules(&source), ["c/inner"]);
        let deeper = format!("`${{{source}}}`");
        assert!(!scan(&deeper).errors.is_empty());
    }

    #[test]
    fn the_template_limit_is_one_thousand_levels() {
        let nested = |n: usize| format!("{}1{}", "`${".repeat(n), "}`".repeat(n));

        assert!(scan(&nested(1000)).errors.is_empty());
        assert!(!scan(&nested(1001)).errors.is_empty());
    }
}
