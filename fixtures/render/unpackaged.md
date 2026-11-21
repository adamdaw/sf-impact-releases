<!-- sf-impact -->
## Salesforce impact

| Head commit | Confidence | Selected tests | Untested symbols | Possible tests | Changed symbols |
| --- | --- | ---: | ---: | ---: | ---: |
| `a111111111111111111111111111111111111111` | partial | 1 | 0 | 0 | 1 |

### Partial confidence (1 reasons)

Grouped by kind, input and file. Full list: impact.json artifact.

- `scope_error` (refs), `not-there`: 1 reasons; first lines: unknown

### Selected tests (1)

- `ExtraTests` (`unpackaged/classes/ExtraTests.cls:1`): `direct`; seed `BillingPlugin`

### Untested changed symbols (0)

None.

### Changed symbols (1)

- `BillingPlugin` (`plugins/Billing/classes/BillingPlugin.cls:1`): 4 parsed, 6 textual

<details>
<summary>Possible tests (0) — not selected</summary>

None.

</details>

<details>
<summary>Direct referrers (parsed first)</summary>

- `BillingPlugin` ← parsed `method` `ExtraTests.billingRuns()` (`unpackaged/classes/ExtraTests.cls`)
- `BillingPlugin` ← parsed `permission_set` `Billing` (`force-app/main/default/permissionsets/Billing.permissionset-meta.xml`)
- `BillingPlugin` ← parsed `profile` `Admin` (`plugins/Billing/declarative/profiles/Admin.profile-meta.xml`)
- `BillingPlugin` ← textual `class` `BillingPlugin` (`plugins/Billing/classes/BillingPlugin.cls`)
- `BillingPlugin` ← textual `method` `BillingPlugin.firstSealArea()` (`plugins/Billing/classes/BillingPlugin.cls`)
- `BillingPlugin` ← textual `method` `BillingPlugin.sealArea()` (`plugins/Billing/classes/BillingPlugin.cls`)
- `BillingPlugin` ← textual `method` `ExtraTests.billingRuns()` (`unpackaged/classes/ExtraTests.cls`)
- `BillingPlugin` ← textual `permission_set` `Billing` (`force-app/main/default/permissionsets/Billing.permissionset-meta.xml`)
- `BillingPlugin` ← textual `profile` `Admin` (`plugins/Billing/declarative/profiles/Admin.profile-meta.xml`)

</details>

Full evidence: impact.json artifact.
