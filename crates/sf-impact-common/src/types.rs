//! Canonical Apex type text shared by symbol creation and type comparison.

use crate::rows::fold;

/// A type with type arguments, as its name and their text:
/// `Map<Id,List<Foo>>` is `("map", ["Id", "List<Foo>"])`. An array is a
/// list, and the name is canonical: `System.List<Foo>` is `("list", ["Foo"])`.
pub fn generic(text: &str) -> Option<(String, Vec<&str>)> {
    if let Some(element) = text.strip_suffix("[]") {
        return Some(("list".to_string(), vec![element]));
    }
    let (name, rest) = text.split_once('<')?;
    let inner = rest.strip_suffix('>')?;
    Some((canonical(name), arguments(inner)))
}

/// A type name case-folded, and without the `System.` that a primitive or
/// a collection may be written with: `System.List` is `list`. `System` is
/// the default namespace, so the two spellings are one type.
pub fn canonical(name: &str) -> String {
    let name = fold(name);
    match name.strip_prefix("system.") {
        Some(plain)
            if is_closed(plain)
                || matches!(plain, "iterable" | "iterator" | "object" | "sobject") =>
        {
            plain.to_string()
        }
        _ => name,
    }
}

/// Type text for comparison: canonical names throughout, so that
/// `System.List<Foo>`, `list<foo>` and `Foo[]` are all `list<foo>`.
pub fn normal(text: &str) -> String {
    match generic(text) {
        Some((name, arguments)) => {
            let arguments: Vec<String> = arguments.into_iter().map(normal).collect();
            format!("{name}<{}>", arguments.join(","))
        }
        None => canonical(text),
    }
}

/// Primitive and collection types: two of these are either the same type,
/// convertible by a known rule, or certainly incompatible.
pub fn is_closed(head: &str) -> bool {
    matches!(
        head,
        "string"
            | "integer"
            | "long"
            | "double"
            | "decimal"
            | "boolean"
            | "date"
            | "datetime"
            | "time"
            | "id"
            | "blob"
            | "list"
            | "set"
            | "map"
    )
}

/// A callable query with canonical parameter types, preserving owner and member spelling.
/// Names without a complete parameter list are left as supplied.
pub fn normal_symbol(symbol: &str) -> String {
    let Some((name, parameters)) = symbol.split_once('(') else {
        return symbol.to_string();
    };
    let Some(parameters) = parameters.strip_suffix(')') else {
        return symbol.to_string();
    };
    let parameters: Vec<_> = arguments(parameters).into_iter().map(normal).collect();
    format!("{name}({})", parameters.join(","))
}

fn arguments(text: &str) -> Vec<&str> {
    let (mut arguments, mut depth, mut start) = (Vec::new(), 0, 0);
    for (at, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                arguments.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    arguments.push(&text[start..]);
    arguments
}

#[cfg(test)]
mod tests {
    use super::normal_symbol;

    #[test]
    fn nested_parameter_lists_preserve_owner_and_member_spelling() {
        assert_eq!(
            normal_symbol("Owner.Member(System.Map<Id,System.List<Foo[]>>,Id[])"),
            "Owner.Member(map<id,list<list<foo>>>,list<id>)"
        );
        for name in ["Owner.Member", "Owner.Member()", "Owner.Member(Id[]"] {
            assert_eq!(normal_symbol(name), name);
        }
    }
}
