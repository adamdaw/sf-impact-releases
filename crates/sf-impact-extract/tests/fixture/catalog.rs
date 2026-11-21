use super::*;

fn resolved(owner: &'static str, kind: RefKind, name: &str) {
    let rows: Vec<_> = parsed_from(owner)
        .filter(|r| r.kind == kind && r.referenced == name)
        .collect();
    assert!(!rows.is_empty(), "{owner}: missing {kind:?} {name}");
    for row in rows {
        assert_eq!(row.resolution, Some(Resolution::Declaration), "{row:?}");
        assert_eq!(row.detail, None, "{row:?}");
        assert!(row.candidates.is_empty(), "{row:?}");
    }
}

#[test]
fn flow_names_resolve_to_the_declarative_catalog() {
    for (kind, name) in [
        (RefKind::FlowObject, "Shipment__c"),
        (RefKind::FlowObject, "Parcel__c"),
        (RefKind::FlowField, "Shipment__c.Weight__c"),
        (RefKind::FlowField, "Shipment__c.Delivered__c"),
        (RefKind::FlowField, "Shipment__c.Tracking_Id__c"),
        (RefKind::FlowField, "Parcel__c.Weight__c"),
        (RefKind::FlowField, "Parcel__c.Fragile__c"),
        (RefKind::FlowLabel, "Label.Shipment_Title"),
        (RefKind::FlowCustomPermission, "Waive_Shipping_Fee"),
        (RefKind::FlowObject, "Shipping_Settings__c"),
        (RefKind::FlowField, "Shipping_Settings__c.Default_Region__c"),
        (RefKind::FlowObject, "Shipping_Plugin__mdt"),
        (RefKind::FlowCustomMetadata, "Shipping_Plugin.Standard"),
        (RefKind::FlowField, "Shipping_Plugin__mdt.Enabled__c"),
    ] {
        resolved("Shipment_Catalog", kind, name);
    }
}

#[test]
fn lwc_imports_and_metadata_objects_resolve_to_the_declarative_catalog() {
    for (kind, name) in [
        (RefKind::LwcSchema, "Shipment__c"),
        (RefKind::LwcSchema, "Shipment__c.Weight__c"),
        (RefKind::LwcLabel, "Label.Shipment_Title"),
        (RefKind::LwcCustomPermission, "Waive_Shipping_Fee"),
        (RefKind::LwcSchema, "Shipping_Settings__c"),
        (RefKind::LwcSchema, "Shipping_Settings__c.Default_Region__c"),
        (RefKind::LwcSchema, "Shipping_Plugin__mdt"),
        (RefKind::LwcSchema, "Shipping_Plugin__mdt.Enabled__c"),
        (RefKind::LwcObject, "Shipment__c"),
        (RefKind::LwcObject, "Parcel__c"),
    ] {
        resolved("shipmentCatalog", kind, name);
    }
}

#[test]
fn message_channel_import_resolves_to_its_source_format_declaration() {
    resolved(
        "shipmentCatalog",
        RefKind::LwcMessageChannel,
        "MessageChannel.Shipment_Updated__c",
    );
    let channel = declaration("MessageChannel.Shipment_Updated__c");
    assert_eq!(
        serde_json::to_value(channel.kind).unwrap(),
        "message_channel"
    );
}

#[test]
fn identical_relationships_resolve_once_at_each_step_for_flow_and_lwc() {
    for (owner, kind, line) in [
        ("Shipment_Catalog", RefKind::FlowField, 12),
        ("shipmentCatalog", RefKind::LwcSchema, 9),
    ] {
        let rows: Vec<_> = parsed_from(owner)
            .filter(|r| r.kind == kind && r.line == line)
            .collect();
        assert_eq!(
            rows.iter()
                .map(|r| r.referenced.as_str())
                .collect::<Vec<_>>(),
            ["Carrier__c.Region__c", "Shipment__c.Carrier__c"]
        );
        assert_eq!(rows[0].column, rows[1].column);
        for row in rows {
            assert_eq!(row.resolution, Some(Resolution::Declaration));
            assert_eq!(row.detail, None);
        }
    }
}

#[test]
fn divergent_relationships_keep_both_candidates_for_flow_and_lwc() {
    for (owner, kind, line) in [
        ("Shipment_Catalog", RefKind::FlowField, 13),
        ("shipmentCatalog", RefKind::LwcSchema, 10),
    ] {
        let rows: Vec<_> = parsed_from(owner)
            .filter(|r| r.kind == kind && r.line == line)
            .collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let ambiguous = rows.iter().find(|r| r.referenced == "Region__c").unwrap();
        assert_eq!(ambiguous.resolution, Some(Resolution::Declaration));
        assert_eq!(ambiguous.detail, Some(Detail::AmbiguousReceiver));
        assert_eq!(
            ambiguous.candidates,
            ["Account.Region__c", "Contact.Region__c"]
        );
        assert!(rows.iter().any(|r| r.referenced == "Shipment__c.Payer__c"));
    }
}

#[test]
fn relationship_ends_without_metadata_and_standard_fields_stay_unresolved() {
    for (owner, kind, line) in [
        ("Shipment_Catalog", RefKind::FlowField, 14),
        ("shipmentCatalog", RefKind::LwcSchema, 11),
    ] {
        let rows: Vec<_> = parsed_from(owner)
            .filter(|r| r.kind == kind && r.line == line)
            .collect();
        assert_eq!(
            rows.iter()
                .map(|r| r.referenced.as_str())
                .collect::<Vec<_>>(),
            ["Account.No__c", "Contact.No__c", "Shipment__c.Payer__c"]
        );
        assert_eq!(rows[0].resolution, Some(Resolution::Declaration));
        assert_eq!(rows[1].resolution, Some(Resolution::Unresolved));
        for name in ["Account.Name", "Contact.Name", "Shipment__c.Name"] {
            let row = parsed_from(owner)
                .find(|r| r.referenced.eq_ignore_ascii_case(name))
                .unwrap();
            assert_eq!(row.resolution, Some(Resolution::Unresolved), "{row:?}");
        }
        let object = parsed_from(owner)
            .find(|r| r.referenced == "Account")
            .unwrap();
        assert_eq!(object.resolution, Some(Resolution::Unresolved));
    }
    let unknown = parsed("Shipment_Catalog", RefKind::FlowField, "unknown.");
    assert_eq!(unknown.resolution, Some(Resolution::Unresolved));
    assert_eq!(unknown.detail, Some(Detail::UnknownType));
}
