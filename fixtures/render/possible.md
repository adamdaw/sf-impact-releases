<!-- sf-impact -->
## Salesforce impact

| Head commit | Confidence | Selected tests | Untested symbols | Possible tests | Changed symbols |
| --- | --- | ---: | ---: | ---: | ---: |
| `a111111111111111111111111111111111111111` | partial | 0 | 0 | 1 | 1 |

### Partial confidence (1 reasons)

Grouped by kind, input and file. Full list: impact.json artifact.

- `scope_error` (refs), `not-there`: 1 reasons; first lines: unknown

### Selected tests (0)

None.
No tests selected; do not pass an empty tests value to RunSpecifiedTests.

### Untested changed symbols (0)

None.

### Changed symbols (1)

- `ShapeService.totalArea(list<shape>)` (`force-app/main/default/classes/ShapeService.cls:15`): 1 parsed, 2 textual

<details>
<summary>Possible tests (1) — not selected</summary>

- `ExtraTests` (`unpackaged/classes/ExtraTests.cls:1`): `member_name_only`; seed `BillingPlugin`

</details>

<details>
<summary>Direct referrers (parsed first)</summary>

- `ShapeService.totalArea(list<shape>)` ← parsed `method` `ShapeServiceTest.totalsAreas()` (`force-app/main/default/classes/ShapeServiceTest.cls`)
- `ShapeService.totalArea(list<shape>)` ← textual `method` `ShapeService.totalArea(list<shape>)` (`force-app/main/default/classes/ShapeService.cls`)
- `ShapeService.totalArea(list<shape>)` ← textual `method` `ShapeServiceTest.totalsAreas()` (`force-app/main/default/classes/ShapeServiceTest.cls`)

</details>

Full evidence: impact.json artifact.
