//! The names in a Salesforce formula: field paths (`Account__r.Name__c`) and
//! globals (`$Setup.Settings__c.Limit__c`). This is a tokenizer, not a
//! grammar: a formula is read only as far as telling a name from a function,
//! a literal, a string and a comment. A name inside a string or a comment is
//! not a reference; the textual tier still sees it.

/// Each name in `formula` with its byte offset, in the order written. A
/// name is a run of identifiers joined by `.`; `Owner:User.Name` keeps its
/// `:`. Function names and the literals `true`, `false` and `null` are not
/// names.
pub fn names(formula: &str) -> Vec<(usize, &str)> {
    let bytes = formula.as_bytes();
    let starts_name = |byte: u8| byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$';
    let continues_name = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut names = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at = match formula[at + 2..].find("*/") {
                    Some(end) => at + 2 + end + 2,
                    None => bytes.len(),
                };
            }
            quote @ (b'"' | b'\'') => {
                at += 1;
                while at < bytes.len() && bytes[at] != quote {
                    // A backslash takes the next character with it.
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                at += 1;
            }
            // A number: `1.5` is not a path.
            b'0'..=b'9' => {
                while at < bytes.len() && (continues_name(bytes[at]) || bytes[at] == b'.') {
                    at += 1;
                }
            }
            byte if starts_name(byte) => {
                let start = at;
                at += 1;
                while at < bytes.len() {
                    let joins = matches!(bytes[at], b'.' | b':')
                        && bytes.get(at + 1).is_some_and(|next| starts_name(*next));
                    if !continues_name(bytes[at]) && !joins {
                        break;
                    }
                    at += 1;
                }
                let name = &formula[start..at];
                let is_call = formula[at..].trim_start().starts_with('(');
                let is_literal = ["true", "false", "null"]
                    .iter()
                    .any(|literal| name.eq_ignore_ascii_case(literal));
                let is_one_word = !name.contains(['.', ':']);
                if !(is_one_word && (is_call || is_literal)) {
                    names.push((start, name));
                }
            }
            _ => at += 1,
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(formula: &str) -> Vec<&str> {
        names(formula).into_iter().map(|(_, name)| name).collect()
    }

    #[test]
    fn a_field_is_a_name_and_a_function_is_not() {
        let found = named("IF(ISBLANK(Total__c), 0, Total__c * Rate__c)");

        assert_eq!(found, ["Total__c", "Total__c", "Rate__c"]);
    }

    #[test]
    fn a_relationship_path_is_one_name() {
        let found = named("Account__r.Owner.Name & Parent.Name");

        assert_eq!(found, ["Account__r.Owner.Name", "Parent.Name"]);
    }

    #[test]
    fn a_global_keeps_its_dollar_sign() {
        let found = named("$Setup.Limits__c.Max__c > $User.Id");

        assert_eq!(found, ["$Setup.Limits__c.Max__c", "$User.Id"]);
    }

    #[test]
    fn a_polymorphic_relationship_keeps_the_object_it_is_read_as() {
        assert_eq!(named("Owner:User.Email"), ["Owner:User.Email"]);
    }

    #[test]
    fn names_in_strings_and_comments_are_not_names() {
        let found = named(
            "/* Hidden__c */ Shown__c & 'In_Single__c' & \"In_Double__c\" & 'it\\'s Escaped__c'",
        );

        assert_eq!(found, ["Shown__c"]);
    }

    #[test]
    fn literals_and_numbers_are_not_names() {
        let found = named("AND(TRUE, false, Null, 1.5, 2e3, Count__c = 10)");

        assert_eq!(found, ["Count__c"]);
    }

    #[test]
    fn a_function_name_may_stand_apart_from_its_parenthesis() {
        assert_eq!(named("TEXT (Stage__c)"), ["Stage__c"]);
    }

    #[test]
    fn a_name_is_located_by_its_byte_offset() {
        let found = names("A__c +\n  B__c");

        assert_eq!(found, [(0, "A__c"), (9, "B__c")]);
    }

    #[test]
    fn an_unterminated_string_or_comment_ends_the_formula() {
        assert_eq!(named("A__c & 'never closed B__c"), ["A__c"]);
        assert_eq!(named("A__c /* never closed B__c"), ["A__c"]);
    }

    #[test]
    fn text_that_is_not_ascii_does_not_shift_the_offsets_after_it() {
        let formula = "'caf\u{e9} \u{2014}' & Total__c";

        let found = names(formula);

        assert_eq!(found, [(formula.find("Total__c").unwrap(), "Total__c")]);
    }
}
