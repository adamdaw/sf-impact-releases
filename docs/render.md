# sf-impact-render

Reads only `impact.json`, verifies it, and renders Markdown or GitHub Actions
step outputs. No source tree or git is needed.

```sh
sf-impact-render --impact impact.json --format md --out comment.md
sf-impact-render --impact impact.json --format gh-step >> "$GITHUB_OUTPUT"
cat impact.json | sf-impact-render --impact - --format md
```

`--out` defaults to stdout; `-` and `stdout` mean the same thing. Bad arguments
exit 2; invalid input exits 1 with one line. Verification finishes before the
output file is opened, so invalid input preserves any existing destination.
Size alone never refuses a valid report.

The document must have schema-1 `provenance`: the writer's tool/version and
`inputs.refs`, plus `inputs.changed` (null for symbol queries). The renderer
checks [the report contract](../schema/impact.schema.json) with typed deserialization
and the shared reader, including required nullable fields and unknown fields.
The provenance chain uses the same checks as preceding consumers: known schema,
valid fields, and matching head/tree, project root,
scope and config. Unverified refs must retain their confidence reasons. It also
checks confidence consistency, reference IDs, direct counts, and disjoint
selected/possible identities, and refuses reports containing errors. Older
reports without provenance must be regenerated. Provenance checks compatibility,
not authenticity. JSON Schema validation is a test dependency only.

Markdown starts with a summary table: the full head commit, confidence and
counts. For symbol queries the commit comes from refs; if none is known, the
comment explicitly says so. Partial-confidence reasons follow, grouped by kind,
input and file with each group's reason count and first five lines (`unknown`
for a null line). Every reason is counted; the complete list remains reachable
in the uploaded `impact.json` artifact. Selected tests follow with reasons and
witness seeds, then untested symbols, changed symbols, collapsed possible tests,
and collapsed direct referrers (parsed first). Symbol queries use the changed
section for their seeds. The hidden line `<!-- sf-impact -->` identifies the
comment for updates. User-controlled text is in code spans with fences longer
than any backtick run: GitHub renders mentions, issue references, HTML and emoji
syntax literally there. Line breaks and directional controls are replaced with
`�`. See [a rendered example](render-example.md).

The budget is **60,000 UTF-8 bytes**, including markup and the marker,
conservatively below GitHub's 65,536-character limit. Secondary sections have
independent caps including their headings and notices: **4,000 bytes for
untested**, and **6,000 each for changed symbols, possible tests and referrers**.
An overflowing section does not prevent later sections from appearing. Whole
rows are kept in order; a visible **N more not shown** paragraph outside the list
counts omitted records (original reasons for grouped confidence rows), pointing
to the artifact. Confidence groups and selected tests normally appear in full.
If even those exceed the remaining budget, reasons receive up to half that
budget, and the sections after the selected tests give up their rows, the last
section first; each keeps its heading and its count of what is not shown. A
selected test is left out only when the list does not fit even then, with the
same explicit omission count. No name or reason is cut mid-text, and every
report still produces a fresh, head-stamped comment. Always retain the complete
JSON artifact.

`gh-step` emits these outputs using GitHub's [environment-file format](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands#setting-an-output-parameter):

| Output | Meaning |
| --- | --- |
| `tests` | Sorted, distinct selected class names, comma-separated for `RunSpecifiedTests`. |
| `confidence` | `full` or `partial`. |
| `selected_count` | Distinct selected class names, matching `tests`. |
| `possible_count` | Distinct possible class names. |
| `untested_count` | Untested symbol identities (colliding declarations stay distinct). |
| `selection` | `selected` or **`empty`**. |

Selected names must follow [Apex class identifier rules](https://developer.salesforce.com/docs/platform/api-metadata/references/meta-types/metadata-apexclass.html): an ASCII letter followed
by letters, digits or single underscores, with no trailing underscore. Empty,
dotted, comma-containing and multiline names are refused in one line naming
the bad value; they cannot become Salesforce CLI arguments. The output writer
supports a collision-free heredoc delimiter for multiline values, although the
current validated outputs are single-line. With no selection, `tests=` is
accompanied by `selected_count=0` and `selection=empty`. **Check `selection`
before running Salesforce**; an empty selection is not evidence that no tests
are needed. The tool does not choose a deployment policy.

## Example: GitHub Actions

This is an example for pull requests and a project whose source
is under `force-app`. It installs all five commands from one pinned release.
Use a published `SF_IMPACT_VERSION` (the `v0.1.0` example pin is unreleased), supply
your pinned Salesforce CLI on the runner, configure `SF_INSTANCE_URL` and the
`SF_ACCESS_TOKEN` secret, and adapt the source directory.
The [validate command](https://github.com/salesforcecli/plugin-deploy-retrieve#sf-project-deploy-validate)
uses `RunSpecifiedTests` with the selected names. Nothing here merges or deploys.

The example runs selected tests for **both full and partial confidence**; partial
confidence is flagged in the comment and check log for review. An empty selection
with a changed file in scope fails and prints the untested list (including changed
Apex). With no Apex/metadata changes in scope it explicitly skips validation.
Fork PRs run the job and fail its first step with a clear message; a maintainer
must validate them separately. They are never silently accepted by a skipped job.
The workflow uses `pull_request`, never privileged execution of fork code via
`pull_request_target`. Failed rendering replaces any old comment with a short
head-stamped “render failed: see artifact” message, and fails the check. A
cancelled run posts nothing: a newer push has superseded it, and its comment
would be of the old head.

```yaml
name: Salesforce impact example
on: pull_request
permissions:
  contents: read
  pull-requests: write
concurrency:
  group: sf-impact-${{ github.event.pull_request.number }}
  cancel-in-progress: true
jobs:
  impact:
    runs-on: ubuntu-latest
    env:
      SF_IMPACT_VERSION: 'v0.1.0' # Example pin: use a published release when available.
    steps:
      - name: Reject fork pull requests visibly
        env:
          HEAD_REPO: ${{ github.event.pull_request.head.repo.full_name }}
          BASE_REPO: ${{ github.repository }}
        run: |
          if [[ "$HEAD_REPO" != "$BASE_REPO" ]]; then
            echo '::error::Fork PR requires maintainer validation; this check cannot validate or post with fork credentials.'
            exit 1
          fi
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          ref: ${{ github.event.pull_request.head.sha }}
          fetch-depth: 0
      - name: Install pinned sf-impact commands
        run: |
          curl -fsSL --proto '=https' --proto-redir '=https' "https://raw.githubusercontent.com/adamdaw/sf-impact-releases/$SF_IMPACT_VERSION/install.sh" \
            -o "$RUNNER_TEMP/install-sf-impact.sh"
          sh "$RUNNER_TEMP/install-sf-impact.sh" --version "$SF_IMPACT_VERSION"
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - name: Extract the checked-out head
        run: sf-impact-extract --root . --out "$RUNNER_TEMP/refs.pack"
      - name: Delta from the merge base, changed symbols, impact
        env:
          BASE_SHA: ${{ github.event.pull_request.base.sha }}
        run: |
          sf-impact-delta --root . --merge-base "$BASE_SHA" --head HEAD --out "$RUNNER_TEMP/delta.jsonl"
          sf-impact-changed --delta "$RUNNER_TEMP/delta.jsonl" --refs "$RUNNER_TEMP/refs.pack" --out "$RUNNER_TEMP/changed.jsonl"
          sf-impact-tests --refs "$RUNNER_TEMP/refs.pack" --changed "$RUNNER_TEMP/changed.jsonl" --out "$RUNNER_TEMP/impact.json"
      - name: Render
        id: render
        env:
          HEAD_SHA: ${{ github.event.pull_request.head.sha }}
        run: |
          if ! sf-impact-render --impact "$RUNNER_TEMP/impact.json" --format md --out "$RUNNER_TEMP/comment.md" ||
             ! sf-impact-render --impact "$RUNNER_TEMP/impact.json" --format gh-step > "$RUNNER_TEMP/outputs"; then
            # shellcheck disable=SC2016 # Markdown backticks are literal.
            printf '<!-- sf-impact -->\n## Salesforce impact\n\nHead `%s`: render failed: see artifact.\n' "$HEAD_SHA" > "$RUNNER_TEMP/comment.md"
            exit 1
          fi
          cat "$RUNNER_TEMP/outputs" >> "$GITHUB_OUTPUT"
      - name: Keep the complete evidence even if rendering fails
        if: always()
        uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4
        with:
          name: impact-json
          path: ${{ runner.temp }}/impact.json
          if-no-files-found: warn
      - name: Post or update the impact comment
        if: ${{ !cancelled() && github.event.pull_request.head.repo.full_name == github.repository }}
        uses: actions/github-script@f28e40c7f34bde8b3046d885e986cb6290c5673b # v7
        with:
          script: |
            const fs = require('fs');
            const file = `${process.env.RUNNER_TEMP}/comment.md`;
            const head = context.payload.pull_request.head.sha;
            const body = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') :
              `<!-- sf-impact -->\n## Salesforce impact\n\nHead ${head}: render failed: see artifact.`;
            const issue = { ...context.repo, issue_number: context.issue.number };
            const comments = await github.paginate(github.rest.issues.listComments, issue);
            const previous = comments.find(c => c.user?.login === 'github-actions[bot]' &&
              c.body?.startsWith('<!-- sf-impact -->'));
            if (previous) {
              await github.rest.issues.updateComment({ ...context.repo, comment_id: previous.id, body });
            } else {
              await github.rest.issues.createComment({ ...issue, body });
            }
      - name: Apply this example's gate and validate the selected tests
        env:
          SELECTION: ${{ steps.render.outputs.selection }}
          CONFIDENCE: ${{ steps.render.outputs.confidence }}
          TESTS: ${{ steps.render.outputs.tests }}
          SF_INSTANCE_URL: ${{ vars.SF_INSTANCE_URL }}
          SF_ACCESS_TOKEN: ${{ secrets.SF_ACCESS_TOKEN }}
        run: |
          scope=$(python3 - <<'PYTHON'
          import json, os
          from pathlib import Path
          rows = [json.loads(line) for line in (Path(os.environ['RUNNER_TEMP']) / 'changed.jsonl').read_text().splitlines()]
          print('changed' if any(r.get('row') == 'file' and (r.get('package') is not None or r.get('gaps')) for r in rows) else 'empty')
          PYTHON
          )
          if [[ "$scope" == empty ]]; then
            echo 'No Apex/metadata in scope changed; skipping the test run explicitly.'
            exit 0
          fi
          if [[ "$SELECTION" != selected ]]; then
            echo '::error::No selected tests for changed Apex/metadata. Review these untested symbols and the artifact:'
            python3 - <<'PYTHON'
          import json, os
          from pathlib import Path
          untested = json.loads((Path(os.environ['RUNNER_TEMP']) / 'impact.json').read_text())['untested']
          for node in untested:
              print(f"  {node['symbol']!r} ({node['file']!r})")
          if not untested:
              print('  No symbol could be listed; inspect confidence reasons in the impact.json artifact.')
          PYTHON
            exit 1
          fi
          if [[ "$CONFIDENCE" == partial ]]; then
            echo '::warning::Partial confidence: running selected tests; review the confidence reasons in the comment.'
          fi
          sf org login access-token --instance-url "$SF_INSTANCE_URL" --alias validation --no-prompt
          IFS=',' read -r -a selected_tests <<< "$TESTS"
          sf project deploy validate --target-org validation --source-dir force-app \
            --test-level RunSpecifiedTests --tests "${selected_tests[@]}" --wait 60
```

The JSON artifact contains the complete evidence, including anything omitted
from the comment. For very large repositories an exact-head refs pack cache
is optional; see [What may persist](persistence.md#what-may-persist). To keep the pack
between runs so that extract reuses it, see [Running in CI](ci.md#running-in-ci).
