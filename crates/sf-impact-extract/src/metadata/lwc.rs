use super::{Parsed, Target, js, xml};
use sf_impact_common::rows::RefKind;

pub fn javascript(source: &str) -> Parsed {
    let imports = js::scan(source);
    let mut result = Parsed {
        errors: imports.errors,
        ..Parsed::default()
    };
    for (module, at) in imports.modules {
        let (name, kind, target) = if let Some(name) = module
            .strip_prefix("@salesforce/apex/")
            .or_else(|| module.strip_prefix("@salesforce/apexContinuation/"))
        {
            if !name.contains('.') {
                result.errors.push(at);
                continue;
            }
            (name.to_string(), RefKind::LwcApex, Target::Method)
        } else if let Some(name) = module.strip_prefix("@salesforce/schema/") {
            (
                name.to_string(),
                RefKind::LwcSchema,
                if name.contains('.') {
                    Target::Field
                } else {
                    Target::Object
                },
            )
        } else if let Some(name) = module.strip_prefix("@salesforce/label/") {
            (
                format!("Label.{}", name.strip_prefix("c.").unwrap_or(name)),
                RefKind::LwcLabel,
                Target::Label,
            )
        } else if let Some(name) = module.strip_prefix("@salesforce/customPermission/") {
            (
                format!("CustomPermission.{name}"),
                RefKind::LwcCustomPermission,
                Target::Permission,
            )
        } else if let Some(name) = module.strip_prefix("@salesforce/messageChannel/") {
            (
                format!("MessageChannel.{name}"),
                RefKind::LwcMessageChannel,
                Target::Channel,
            )
        } else if let Some(name) = module.strip_prefix("c/") {
            (name.to_string(), RefKind::LwcComponent, Target::Lwc)
        } else {
            continue;
        };
        if name.is_empty()
            || name.ends_with('.')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$'))
        {
            result.errors.push(at);
            continue;
        }
        result.add(&name, kind, target, at);
    }
    result
}

/// HTML is not XML: boolean attributes, void elements and unquoted LWC
/// expressions are valid. Scan tags while respecting comments and attributes.
pub fn template(source: &str) -> Parsed {
    let mut result = Parsed::default();
    let bytes = source.as_bytes();
    let mut stack = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let start = i;
        if bytes[i..].starts_with(b"<!--") {
            if let Some(end) = source[i + 4..].find("-->") {
                i += end + 7;
                continue;
            }
            result.errors.push(i);
            break;
        }
        i += 1;
        let closing = bytes.get(i) == Some(&b'/');
        if closing {
            i += 1;
        }
        let from = i;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
        {
            i += 1;
        }
        let name = &source[from..i];
        let mut quote = None;
        let mut braces = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if matches!(b, b'\'' | b'"') {
                quote = Some(b);
            } else if b == b'{' {
                braces += 1;
            } else if b == b'}' {
                braces -= 1;
            } else if b == b'>' && braces == 0 {
                break;
            }
            i += 1;
        }
        if i == bytes.len() {
            result.errors.push(start);
            break;
        }
        let self_closing = i > 0 && bytes[i - 1] == b'/';
        let void = matches!(
            name,
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        );
        if closing {
            if stack.pop().is_none_or(|(open, _)| open != name) {
                result.errors.push(start);
            }
        } else if !self_closing && !void && !name.is_empty() {
            stack.push((name, start));
        }
        i += 1;
        if !closing && let Some(component) = component_name(name) {
            result.add(&component, RefKind::LwcComponent, Target::Lwc, from);
        }
    }
    result
        .errors
        .extend(stack.into_iter().map(|(_, offset)| offset));
    result
}

// Both template tags and configurationEditor use namespace-kebab names.
fn component_name(tag: &str) -> Option<String> {
    let (namespace, component) = tag.split_once('-')?;
    if namespace == "lightning" {
        return None;
    }
    let mut camel = String::new();
    let mut upper = false;
    for ch in component.chars() {
        if ch == '-' {
            upper = true;
        } else {
            camel.push(if upper { ch.to_ascii_uppercase() } else { ch });
            upper = false;
        }
    }
    Some(if namespace == "c" {
        camel
    } else {
        format!("{namespace}:{camel}")
    })
}

/// CSS strings/comments may contain import-looking text. Only an @import rule
/// names a stylesheet; ordinary URLs and declaration values are not module names.
pub fn stylesheet(source: &str) -> Parsed {
    let mut result = Parsed::default();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            if let Some(end) = source[i + 2..].find("*/") {
                i += end + 4;
            } else {
                result.errors.push(i);
                break;
            }
        } else if matches!(bytes[i], b'\'' | b'"') {
            i = css_string_end(bytes, i, &mut result);
        } else if bytes[i..].starts_with(b"@import")
            && bytes.get(i + 7).is_some_and(u8::is_ascii_whitespace)
        {
            i += 7;
            loop {
                while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                if !bytes[i..].starts_with(b"/*") {
                    break;
                }
                if let Some(end) = source[i + 2..].find("*/") {
                    i += end + 4;
                } else {
                    result.errors.push(i);
                    i = bytes.len();
                    break;
                }
            }
            if bytes.get(i).is_some_and(|b| matches!(b, b'\'' | b'"')) {
                let start = i + 1;
                let errors_before = result.errors.len();
                i = css_string_end(bytes, i, &mut result);
                if result.errors.len() == errors_before
                    && let Some(name) = source[start..i - 1].strip_prefix("c/")
                {
                    result.add(name, RefKind::LwcComponent, Target::Lwc, start);
                }
            }
        } else {
            i += 1;
        }
    }
    result
}

fn css_string_end(bytes: &[u8], start: usize, result: &mut Parsed) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            return i + 1;
        }
        if bytes[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    result.errors.push(start);
    bytes.len()
}

pub fn meta(source: &str) -> Parsed {
    let doc = xml::parse(source, "LightningComponentBundle");
    let mut result = Parsed::default();
    for node in &doc.nodes {
        match node.name.as_str() {
            "target" => result.add(
                node.value(),
                RefKind::LwcTarget,
                Target::Exposure,
                node.offset(),
            ),
            "object" => result.add(
                node.value(),
                RefKind::LwcObject,
                Target::Object,
                node.offset(),
            ),
            _ => {}
        }
        for (key, value, offset) in &node.attributes {
            if key == "configurationEditor"
                && let Some(component) = component_name(value)
            {
                result.add(&component, RefKind::LwcComponent, Target::Lwc, *offset);
            }
            if key == "type"
                && let Some(object) = value.strip_prefix("@salesforce/schema/")
            {
                result.add(
                    object.strip_suffix("[]").unwrap_or(object),
                    RefKind::LwcObject,
                    Target::Object,
                    *offset,
                );
            }
            if key == "datasource"
                && let Some(class) = value.strip_prefix("apex://")
            {
                result.add(class, RefKind::LwcApex, Target::Class, *offset + 7);
            }
            if key == "targets" {
                for target in value.split(',') {
                    result.add(target.trim(), RefKind::LwcTarget, Target::Exposure, *offset);
                }
            }
            if key == "type"
                && let Some(class) = value.strip_prefix("apex://")
            {
                result.add(class, RefKind::LwcApex, Target::Class, *offset + 7);
            }
        }
    }
    result.errors = doc.errors;
    result
}
