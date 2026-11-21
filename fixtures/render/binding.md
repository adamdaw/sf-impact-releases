<!-- sf-impact -->
## Salesforce impact

| Head commit | Confidence | Selected tests | Untested symbols | Possible tests | Changed symbols |
| --- | --- | ---: | ---: | ---: | ---: |
| `a111111111111111111111111111111111111111` | partial | 0 | 2 | 0 | 5 |

### Partial confidence (2 reasons)

Grouped by kind, input and file. Full list: impact.json artifact.

- `dynamic_binding` (refs), `force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`: 2 reasons; first lines: 18, 19

### Selected tests (0)

None.
No tests selected; do not pass an empty tests value to RunSpecifiedTests.

### Untested changed symbols (2)

- `ShipmentAudit` (`force-app/declarative/classes/ShipmentAudit.cls:1`)
- `ShipmentTracker` (`force-app/declarative/classes/ShipmentTracker.cls:1`)

### Changed symbols (5)

- `ShipmentAudit` (`force-app/declarative/classes/ShipmentAudit.cls:1`): 1 parsed, 2 textual
- `ShipmentTracker` (`force-app/declarative/classes/ShipmentTracker.cls:1`): 0 parsed, 1 textual
- `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml:1`): 0 parsed, 1 textual
- `Shipping_Plugin__mdt` (`force-app/declarative/objects/Shipping_Plugin__mdt/Shipping_Plugin__mdt.object-meta.xml:1`): 1 parsed, 0 textual
- `Shipping_Plugin__mdt.Handler__c` (`force-app/declarative/objects/Shipping_Plugin__mdt/fields/Handler__c.field-meta.xml:1`): 1 parsed, 2 textual

<details>
<summary>Possible tests (0) — not selected</summary>

None.

</details>

<details>
<summary>Direct referrers (parsed first)</summary>

- `ShipmentAudit` ← parsed `metadata_record` `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`)
- `Shipping_Plugin__mdt` ← parsed `metadata_record` `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`)
- `Shipping_Plugin__mdt.Handler__c` ← parsed `metadata_record` `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`)
- `ShipmentAudit` ← textual `class` `ShipmentAudit` (`force-app/declarative/classes/ShipmentAudit.cls`)
- `ShipmentAudit` ← textual `metadata_record` `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`)
- `ShipmentTracker` ← textual `class` `ShipmentTracker` (`force-app/declarative/classes/ShipmentTracker.cls`)
- `Shipping_Plugin.Tracking` ← textual `metadata_record` `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`)
- `Shipping_Plugin__mdt.Handler__c` ← textual `metadata_record` `Shipping_Plugin.Tracking` (`force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml`)
- `Shipping_Plugin__mdt.Handler__c` ← textual `object_field` `Shipping_Plugin__mdt.Handler__c` (`force-app/declarative/objects/Shipping_Plugin__mdt/fields/Handler__c.field-meta.xml`)

</details>

Full evidence: impact.json artifact.
