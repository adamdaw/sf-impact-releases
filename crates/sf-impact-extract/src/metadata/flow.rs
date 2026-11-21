use super::{Named, Parsed, Target, xml};
use sf_impact_common::rows::{RefKind, UnsupportedReason, fold};
use std::collections::HashMap;

// Recognised structure gives position-dependent leaves their meaning and avoids
// diagnostics on ordinary control metadata. It never gates reference/merge scans.
fn container(name: &str) -> bool {
    matches!(
        name,
        "Flow"
            | "start"
            | "recordLookups"
            | "recordCreates"
            | "recordUpdates"
            | "recordDeletes"
            | "variables"
            | "constants"
            | "formulas"
            | "textTemplates"
            | "dynamicChoiceSets"
            | "choices"
            | "filters"
            | "inputAssignments"
            | "outputAssignments"
            | "assignments"
            | "assignmentItems"
            | "value"
            | "rightValue"
            | "defaultValue"
            | "conditions"
            | "decisions"
            | "rules"
            | "actions"
            | "actionCalls"
            | "apexPluginCalls"
            | "subflows"
            | "screens"
            | "fields"
            | "inputParameters"
            | "outputParameters"
            | "visibilityRule"
            | "validationRule"
            | "dataTypeMappings"
            | "loops"
            | "collectionProcessors"
            | "sortOptions"
            | "processMetadataValues"
            | "scheduledPaths"
            | "waits"
            | "waitEvents"
            | "entryConditions"
            | "exitCriteria"
            | "entryCriteria"
            | "steppedStages"
            | "steps"
            | "orchestratedStages"
            | "stageSteps"
            | "entryActionInputParameters"
            | "exitActionInputParameters"
            | "assignees"
            | "assignee"
            | "mapItems"
            | "exitConditions"
            | "customErrors"
            | "customErrorMessages"
            | "transforms"
            | "transformValues"
            | "transformValueActions"
            | "connector"
            | "defaultConnector"
            | "nextValueConnector"
            | "faultConnector"
            | "noMoreValuesConnector"
            | "styleProperties"
            // The two styleProperties children Flow Builder writes. Any
            // other child of styleProperties is reported as not read.
            | "verticalAlignment"
            | "width"
            | "schedule"
            | "isDisabled"
            | "stages"
            | "customProperties"
            | "triggers"
            | "recordRollbacks"
    )
}

// Leaves that Flow Builder writes under a recognised element and that name no
// dependency: labels, layout, flags, enumerated types, literal values and the
// names of elements in the same Flow. Taken from every Flow in the 25 pinned
// corpora and the fixtures, then checked one by one against the Flow metadata
// reference: a leaf documented as able to name something (an event naming a
// form or external service, an action's generic type variable) is not here.
// Any other leaf under a recognised element is reported as not read, unless a
// reference or merge scan interprets it.
fn control_leaf(parent: &str, name: &str) -> bool {
    match name {
        // An element's own name names nothing. The name of a parameter or an
        // assignment is a member of what the element configures: an Apex
        // action's invocable variable, a component property, a subflow variable.
        "name" => !matches!(
            parent,
            "inputParameters"
                | "outputParameters"
                | "inputAssignments"
                | "outputAssignments"
                | "entryActionInputParameters"
                | "exitActionInputParameters"
        ),
        // A literal names nothing, except a step assignee: a user, group or queue.
        "stringValue" => parent != "assignee",
        _ => matches!(
            name,
            "actionType"
                | "allowBack"
                | "allowFinish"
                | "allowPause"
                | "apiVersion"
                | "areMetricsLoggedToDataCloud"
                | "assignNextValueToReference"
                | "assignNullValuesIfNoRecordsFound"
                | "assigneeType"
                | "backButtonLabel"
                | "booleanValue"
                | "choiceReferences"
                | "choiceText"
                | "collectionProcessorType"
                | "conditionLogic"
                | "dataType"
                | "dateTimeValue"
                | "dateValue"
                | "defaultConnectorLabel"
                | "defaultSelectedChoiceReference"
                | "description"
                | "doesPutEmptyStringAndNullFirst"
                | "doesRequireRecordChangedToMeetCriteria"
                | "elementSubtype"
                | "entryActionType"
                | "entryConditionLogic"
                | "environments"
                | "errorMessage"
                | "eventSource"
                | "eventType"
                | "exitActionType"
                | "exitConditionLogic"
                | "fieldText"
                | "fieldType"
                | "filterLogic"
                | "flowRunAsUser"
                | "flowTransactionModel"
                | "frequency"
                | "getFirstRecordOnly"
                | "helpText"
                | "inputConfiguratorMode"
                | "inputsOnNextNavToAssocScrn"
                | "interviewLabel"
                | "isActive"
                | "isCollection"
                | "isGoTo"
                | "isInput"
                | "isOutput"
                | "isRequired"
                | "isTemplate"
                | "isViewedAsPlainText"
                | "iterationOrder"
                | "label"
                | "limit"
                | "locationX"
                | "locationY"
                | "nextOrFinishButtonLabel"
                | "numberValue"
                | "offset"
                | "operator"
                | "outputReference"
                | "pathType"
                | "pauseButtonLabel"
                | "pausedText"
                | "processType"
                | "recordTriggerType"
                | "regionContainerType"
                | "requiresAsyncProcessing"
                | "runInMode"
                | "scale"
                | "showFooter"
                | "showHeader"
                | "sortOrder"
                | "stageOrder"
                | "startDate"
                | "startElementReference"
                | "startTime"
                | "status"
                | "storeOutputAutomatically"
                | "targetReference"
                | "text"
                | "triggerType"
                | "versionSegment"
        ),
    }
}

/// A screen component of the platform: no reference is read for it.
fn platform_component(extension: &str) -> bool {
    extension.starts_with("flowruntime:") || extension.starts_with("forceContent:")
}

/// A wait event of the platform: no reference is read for it.
fn platform_event(event: &str) -> bool {
    event == "AlarmEvent"
}

/// For a leaf that names a member of its element's target (a parameter, an
/// assignment, a type variable), the leaves that can be that target.
fn member_of(parent: &str, name: &str) -> Option<&'static [&'static str]> {
    match (parent, name) {
        ("entryActionInputParameters", "name") => Some(&["entryActionName"]),
        ("exitActionInputParameters", "name") => Some(&["exitActionName"]),
        (
            "inputParameters" | "outputParameters" | "inputAssignments" | "outputAssignments",
            "name",
        )
        | ("dataTypeMappings", "typeName") => Some(&[
            "actionName",
            "nameSegment",
            "flowName",
            "apexClass",
            "extensionName",
            "eventType",
        ]),
        _ => None,
    }
}

// A member's name cannot be known better than its target, so it needs no row
// of its own when the target is a reference row, whatever its resolution, or
// is omitted as the platform's. Without either it is reported.
fn member_is_covered(
    doc: &xml::Document,
    emitted: &[bool],
    member: usize,
    targets: &[&str],
) -> bool {
    let Some(element) = doc.nodes[member].parent.and_then(|p| doc.nodes[p].parent) else {
        return false;
    };
    let target_is_a_row = doc.nodes[element]
        .children
        .iter()
        .any(|&c| emitted[c] && targets.contains(&doc.nodes[c].name.as_str()));
    // A region's width is the one input of a platform layout component;
    // no other member of a region is proven the platform's.
    let region_width = doc.nodes[element].name == "fields"
        && doc.value(element, "fieldType") == Some("Region")
        && doc.nodes[member].parent.map(|p| doc.nodes[p].name.as_str()) == Some("inputParameters")
        && doc.nodes[member].name == "name"
        && doc.nodes[member].value() == "width";
    let platform = doc
        .value(element, "extensionName")
        .is_some_and(platform_component)
        || (doc.nodes[element].name == "waitEvents"
            && doc.value(element, "eventType").is_some_and(platform_event))
        || region_width;
    target_is_a_row || platform
}

type Bindings<'a> = HashMap<String, Option<&'a str>>;

fn record_operation(name: &str) -> bool {
    matches!(
        name,
        "recordLookups" | "recordCreates" | "recordUpdates" | "recordDeletes"
    )
}

fn bind<'a>(bindings: &mut Bindings<'a>, name: &str, object: &'a str) {
    bindings
        .entry(fold(name))
        .and_modify(|old| {
            if old.is_some_and(|old| !old.eq_ignore_ascii_case(object)) {
                *old = None;
            }
        })
        .or_insert(Some(object));
}

fn lookup<'a>(bindings: &Bindings<'a>, name: &str) -> Option<&'a str> {
    bindings.get(&fold(name)).copied().flatten()
}

// Build record identities before reading uses, including aliases declared later.
fn bindings(doc: &xml::Document) -> Bindings<'_> {
    let mut bindings = HashMap::new();
    for (id, node) in doc.nodes.iter().enumerate() {
        if node.name == "start" {
            if let Some(object) = doc.value(id, "object") {
                bind(&mut bindings, "$Record", object);
                bind(&mut bindings, "$Record__Prior", object);
            }
        } else if node.name == "variables" {
            if let (Some(name), Some(object)) = (doc.value(id, "name"), doc.value(id, "objectType"))
            {
                bind(&mut bindings, name, object);
            }
        } else if record_operation(&node.name)
            && let Some(object) = doc.value(id, "object")
        {
            for name in [doc.value(id, "name"), doc.value(id, "outputReference")]
                .into_iter()
                .flatten()
            {
                bind(&mut bindings, name, object);
            }
        }
    }
    // Loop/processor aliases can be chained; a bounded fixed point is order independent.
    for _ in 0..doc.nodes.len() {
        let before = bindings.clone();
        for (id, node) in doc.nodes.iter().enumerate() {
            if matches!(node.name.as_str(), "loops" | "collectionProcessors")
                && let (Some(collection), Some(alias)) = (
                    doc.value(id, "collectionReference"),
                    doc.value(id, "assignNextValueToReference")
                        .or(doc.value(id, "name")),
                )
                && let Some(object) = lookup(&before, collection)
            {
                bind(&mut bindings, alias, object);
            }
        }
        if bindings == before {
            break;
        }
    }
    bindings
}

// Fields belong to the nearest operation/choice/transform object. Relationship
// paths are kept intact here; the declarative catalog follows their lookups.
fn field_context<'a>(
    doc: &'a xml::Document,
    node: &xml::Node,
    bindings: &Bindings<'a>,
) -> Option<&'a str> {
    if node.name == "fieldSelection" {
        return lookup(bindings, "$Record");
    }
    let mut current = node.parent;
    while let Some(p) = current {
        if !container(&doc.nodes[p].name) {
            current = doc.nodes[p].parent;
            continue;
        }
        let object = if node.name == "picklistField" {
            doc.value(p, "picklistObject")
        } else {
            None
        }
        .or_else(|| doc.value(p, "object"))
        .or_else(|| doc.value(p, "objectType"))
        .or_else(|| doc.value(p, "outputSObjectType"))
        .or_else(|| {
            doc.value(p, "inputReference")
                .and_then(|v| lookup(bindings, v))
        });
        if object.is_some() {
            return object;
        }
        current = doc.nodes[p].parent;
    }
    None
}

fn field(result: &mut Parsed, name: String, offset: usize, unknown: bool) {
    result.references.push(Named {
        name,
        kind: RefKind::FlowField,
        target: Target::Field,
        offset,
        unknown,
    });
}

// Structural leaves are interpreted only in recognised parent contexts. Names
// such as object/field in a future element do not prove Salesforce identities.
fn structural_reference(
    doc: &xml::Document,
    node: &xml::Node,
    bindings: &Bindings<'_>,
    result: &mut Parsed,
) -> bool {
    let Some(parent) = node.parent.filter(|p| container(&doc.nodes[*p].name)) else {
        return false;
    };
    let value = node.value();
    let at = node.offset();
    match node.name.as_str() {
        "object" | "objectType" | "outputSObjectType" | "picklistObject" => {
            result.add(value, RefKind::FlowObject, Target::Object, at)
        }
        "apexClass" => result.add(value, RefKind::FlowAction, Target::Class, at),
        "flowName" | "overriddenFlow" => result.add(value, RefKind::FlowSubflow, Target::Flow, at),
        "actionName" | "nameSegment" | "entryActionName" | "exitActionName" => {
            // entry/exitActionName have corresponding entry/exitActionType siblings.
            let type_key = if node.name == "nameSegment" {
                "actionType".into()
            } else {
                node.name.replace("Name", "Type")
            };
            match doc.value(parent, &type_key) {
                Some("apex") => result.add(value, RefKind::FlowAction, Target::Class, at),
                Some(
                    "flow" | "evaluationFlow" | "stepBackground" | "stepInteractive"
                    | "createWorkItem",
                ) => result.add(value, RefKind::FlowSubflow, Target::Flow, at),
                Some("component") => result.add(
                    value.strip_prefix("c:").unwrap_or(value),
                    RefKind::FlowComponent,
                    Target::Lwc,
                    at,
                ),
                _ => result.add(value, RefKind::FlowAction, Target::Action, at),
            }
        }
        "extensionName" => {
            if !platform_component(value) {
                result.add(
                    value.strip_prefix("c:").unwrap_or(value),
                    RefKind::FlowComponent,
                    Target::Lwc,
                    at + if value.starts_with("c:") { 2 } else { 0 },
                );
            }
        }
        "objectFieldReference" => {
            if let Some((variable, rest)) = value.split_once('.') {
                let object = lookup(bindings, variable);
                field(
                    result,
                    object.map_or_else(|| value.into(), |o| format!("{o}.{rest}")),
                    at,
                    object.is_none(),
                );
            } else if !value.is_empty() {
                field(result, value.into(), at, true);
            }
        }
        "field"
        | "queriedFields"
        | "triggerFields"
        | "displayField"
        | "valueField"
        | "picklistField"
        | "sortField"
        | "recordField"
        | "fieldSelection"
        | "outputFieldApiName"
        | "assignToFieldReference" => {
            if !value.is_empty() {
                let object = field_context(doc, node, bindings);
                field(
                    result,
                    object.map_or_else(|| value.into(), |o| format!("{o}.{value}")),
                    at,
                    object.is_none(),
                );
            }
        }
        "inputReference" if record_operation(&doc.nodes[parent].name) => {
            if value.is_empty() {
                return true;
            }
            let object = lookup(bindings, value);
            result.references.push(Named {
                name: object.unwrap_or("SObject").into(),
                kind: RefKind::FlowObject,
                target: Target::Object,
                offset: at,
                unknown: object.is_none(),
            });
        }
        "typeValue" if doc.nodes[parent].name == "dataTypeMappings" => {
            result.add(value, RefKind::FlowObject, Target::Object, at)
        }
        "stringValue" => {
            if let Some(group) = doc.nodes[parent].parent
                && doc.nodes[group].name == "processMetadataValues"
                && doc.value(group, "name") == Some("ObjectType")
            {
                result.add(value, RefKind::FlowObject, Target::Object, at);
            } else {
                return false;
            }
        }
        "eventType" if doc.nodes[parent].name == "waitEvents" => {
            if !platform_event(value) {
                result.add(value, RefKind::FlowObject, Target::Object, at);
            }
        }
        _ => return false,
    }
    true
}

fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn expression_token(token: &str, at: usize, bindings: &Bindings<'_>, result: &mut Parsed) {
    let Some((variable, rest)) = token.split_once('.') else {
        return;
    };
    if variable.bytes().all(|b| b.is_ascii_digit()) {
        return;
    }
    match fold(variable).as_str() {
        "$flow" | "$api" | "$organization" | "$system" => {}
        "$user" => field(result, format!("User.{rest}"), at, false),
        "$label" => result.add(
            &format!("Label.{rest}"),
            RefKind::FlowLabel,
            Target::Label,
            at,
        ),
        "$permission" => result.add(
            &format!("CustomPermission.{rest}"),
            RefKind::FlowCustomPermission,
            Target::Permission,
            at,
        ),
        "$setup" => {
            let object = rest.split('.').next().unwrap_or(rest);
            result.add(object, RefKind::FlowObject, Target::Object, at);
            if rest.contains('.') {
                field(result, rest.into(), at, false);
            }
        }
        "$custommetadata" => {
            let mut parts = rest.splitn(3, '.');
            let object = parts.next().unwrap_or(rest);
            result.add(object, RefKind::FlowObject, Target::Object, at);
            if let Some(record) = parts.next() {
                result.add(
                    &format!("{object}.{record}"),
                    RefKind::FlowCustomMetadata,
                    Target::MetadataRecord,
                    at,
                );
            }
            if let Some(name) = parts.next() {
                field(result, format!("{object}.{name}"), at, false);
            }
        }
        _ => {
            let object = lookup(bindings, variable);
            field(
                result,
                object.map_or_else(|| token.into(), |o| format!("{o}.{rest}")),
                at,
                object.is_none(),
            );
        }
    }
}

// Explicit reference/formula leaves can contain bare references. All other text
// is scanned only inside merge tokens, regardless of its ancestors or leaf name.
fn expression_references(node: &xml::Node, bindings: &Bindings<'_>, result: &mut Parsed) -> bool {
    let bare = matches!(
        node.name.as_str(),
        "elementReference"
            | "leftValueReference"
            | "assignToReference"
            | "inputReference"
            | "collectionReference"
            | "assignRecordIdToReference"
            | "expression"
            | "formulaExpression"
            | "filterFormula"
            | "formula"
    );
    let bytes = node.text.as_bytes();
    let mut i = 0;
    let mut interpreted = bare;
    let mut merge = false;
    let mut quote = None;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"{!") {
            merge = true;
            interpreted = true;
            i += 2;
            continue;
        }
        if bytes[i] == b'}' {
            merge = false;
        }
        // Bare formula string literals are text; explicit merge tokens still
        // name references even when surrounded by a quoted string.
        if bare && !merge {
            if let Some(q) = quote {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            if matches!(bytes[i], b'\'' | b'"') {
                quote = Some(bytes[i]);
                i += 1;
                continue;
            }
        }
        if !(bare || merge) || !identifier(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (identifier(bytes[i]) || bytes[i] == b'.') {
            i += 1;
        }
        expression_token(
            node.text[start..i].trim_end_matches('.'),
            node.at(start),
            bindings,
            result,
        );
    }
    interpreted
}

pub fn parse(source: &str) -> Parsed {
    let doc = xml::parse(source, "Flow");
    let bindings = bindings(&doc);
    let mut result = Parsed::default();
    let mut interpreted = vec![false; doc.nodes.len()];
    // The leaves that are a reference row of their own.
    let mut emitted = vec![false; doc.nodes.len()];
    let mut members = Vec::new();
    for (id, node) in doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.children.is_empty())
    {
        let before = result.references.len();
        let structural = structural_reference(&doc, node, &bindings, &mut result);
        emitted[id] = result.references.len() > before;
        interpreted[id] = expression_references(node, &bindings, &mut result) || structural;
        if interpreted[id] && node.value().is_empty() {
            result
                .unsupported
                .push((path(&doc, id), node.start, UnsupportedReason::NoName));
        } else if !interpreted[id]
            && let Some(parent) = node.parent.map(|p| doc.nodes[p].name.as_str())
            && container(parent)
            && !control_leaf(parent, &node.name)
        {
            // A target can stand after its members, so they wait for all leaves.
            match member_of(parent, &node.name) {
                Some(targets) => members.push((id, targets)),
                None => result.unsupported.push((
                    path(&doc, id),
                    node.start,
                    UnsupportedReason::NotInterpreted,
                )),
            }
        }
    }
    for (id, targets) in members {
        if !member_is_covered(&doc, &emitted, id, targets) {
            result.unsupported.push((
                path(&doc, id),
                doc.nodes[id].start,
                UnsupportedReason::NotInterpreted,
            ));
        }
    }
    // An unknown wrapper containing only interpretable references needs no
    // diagnostic. If even one leaf remains uninterpreted, the outer unknown
    // structure is reported as an element that was not read. That is not a
    // syntax error: the file is well-formed, and `errors` is for what is not.
    for (id, node) in doc
        .nodes
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, n)| !n.children.is_empty())
    {
        interpreted[id] = node.children.iter().all(|c| interpreted[*c]);
        if !container(&node.name)
            && !interpreted[id]
            && node.parent.is_none_or(|p| container(&doc.nodes[p].name))
        {
            result.unsupported.push((
                path(&doc, id),
                node.start,
                UnsupportedReason::NotInterpreted,
            ));
        }
    }
    // In the order they stand in the file.
    result.unsupported.sort_by_key(|(_, offset, _)| *offset);
    result.errors.extend(doc.errors);
    result
}

/// The path of a node below the root element, with `/` between names:
/// `recordLookups/futureFilter`.
fn path(doc: &xml::Document, node: usize) -> String {
    let mut names = vec![doc.nodes[node].name.as_str()];
    let mut current = doc.nodes[node].parent;
    while let Some(parent) = current {
        current = doc.nodes[parent].parent;
        // The root itself is not part of the path.
        if current.is_some() {
            names.push(doc.nodes[parent].name.as_str());
        }
    }
    names.reverse();
    names.join("/")
}
