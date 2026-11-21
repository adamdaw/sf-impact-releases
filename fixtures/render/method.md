<!-- sf-impact -->
## Salesforce impact

| Head commit | Confidence | Selected tests | Untested symbols | Possible tests | Changed symbols |
| --- | --- | ---: | ---: | ---: | ---: |
| `a111111111111111111111111111111111111111` | partial | 1 | 0 | 0 | 1 |

### Partial confidence (1 reasons)

Grouped by kind, input and file. Full list: impact.json artifact.

- `scope_error` (refs), `not-there`: 1 reasons; first lines: unknown

### Selected tests (1)

- `ShapeServiceTest` (`force-app/main/default/classes/ShapeServiceTest.cls:1`): `convention`, `direct`; seed `ShapeService.totalArea(list<shape>)`

### Untested changed symbols (0)

None.

### Changed symbols (1)

- `ShapeService.totalArea(list<shape>)` (`force-app/main/default/classes/ShapeService.cls:15`): 1 parsed, 2 textual

<details>
<summary>Possible tests (0) — not selected</summary>

None.

</details>

<details>
<summary>Direct referrers (parsed first)</summary>

- `ShapeService.totalArea(list<shape>)` ← parsed `method` `ShapeServiceTest.totalsAreas()` (`force-app/main/default/classes/ShapeServiceTest.cls`)
- `ShapeService.totalArea(list<shape>)` ← textual `method` `ShapeService.totalArea(list<shape>)` (`force-app/main/default/classes/ShapeService.cls`)
- `ShapeService.totalArea(list<shape>)` ← textual `method` `ShapeServiceTest.totalsAreas()` (`force-app/main/default/classes/ShapeServiceTest.cls`)

</details>

Full evidence: impact.json artifact.
