---
id: inbox-qa-checks
title: Proposal — custom tests as QA checks (HTTP, SQL, script) that agents author and Ubiq executes
kind: proposal
status: proposal
summary: A check is a test Ubiq runs itself, without a test framework; agents write checks as declarative files (http, sql and script kinds with assertions, captures, environments and secret references), the test worker runs them with native host-side executors, and they appear in the same explorer, run model, dashboard and ubiq-tests MCP server as framework tests, with approval gates for real environments, a QA-project workflow, interop with Hurl, Postman and JUnit, a phased build order, the decisions it would add and the open questions.
read_when: you are deciding how Ubiq runs HTTP, database or script checks that are not unit tests, how an agent authors and runs them, how they touch real environments safely, or what a QA project is
updated: 2026-10-26
depends_on: [inbox-test-runner, inbox-debugger, inbox-hitl-dialogs, inbox-agents-definitions, feat-workbench-db, feat-workbench-ide, tech-architecture, tech-transport, tech-decisions]
---

**A check is a test that Ubiq executes itself, with no test framework behind it: an HTTP request
with assertions, a SQL query with assertions, or a script whose exit code decides.** Agents author
checks as plain files; the same explorer, run model, results view, dashboard and `ubiq-tests` MCP
server that show framework tests show checks, per the [test-runner proposal](./test-runner-proposal.md).
A **QA project** is then a Ubiq project whose content is checks against environments, with no
application code in it.

"Task" means a kanban card here, so an execution is a **run**, as in the test-runner proposal.

## 1. What exists, what is missing

| Want | Today |
|---|---|
| Call an endpoint and assert on the answer | An agent writes `curl` in a pane and reads text; nothing is kept or rerun |
| Assert on data (an invariant, a row count) | The DB mode runs SQL by hand; `ubiq-sql-read` lets an agent query, but nothing remembers an expectation |
| A repeatable smoke suite per environment | Nothing |
| A check authored by an agent, reviewed by a human | Nothing |

The pieces exist apart. The host already opens database connections for the DB mode
([the DB workbench](../features/workbench-db.md)) with a read-only flag per connection and an agent
access level; the host links an HTTP client (`ureq`, behind the `listener` feature in
`crates/ubiq-host/Cargo.toml`, used for the feedback POST and update checks) and a keychain
(`keyring`). What is missing is the declarative check, its executors, and the model that holds the
results. The test-runner proposal supplies the last.

## 2. Positioning: when a check, when a recipe

Anything that already emits JUnit XML, TAP, TRX or the Ubiq NDJSON schema runs through a **test
recipe**: Playwright, Robot Framework, k6 thresholds, Schemathesis, dbt tests, Soda, Great
Expectations, Newman. Ubiq does not reimplement them. Native checks exist for four things a recipe
does not give:

| Property | Why a recipe is not enough |
|---|---|
| Zero install | Nothing to `pip install` to check that `/health` returns 200 |
| Agent-authored | A small closed schema an agent writes and Ubiq validates, instead of a framework's whole API |
| Secrets-aware | Credentials resolved host-side from references, masked everywhere |
| Inspectable | The exact request, response and rows are attached to the result, not scrolled away |

Rule of thumb: the moment a check needs loops, branching or a library, it is a `script` check or a
recipe, not a richer schema (open question 4).

## 3. Check kinds

A closed set, with Ubiq-owned executors running on the host's side, so a remote host works
unchanged.

| Kind | What it does | Assertions |
|---|---|---|
| `http` | One request or a chain of steps: method, URL, headers, query, body; captures from a response feed later steps (login, then token, then call) | Status, header, JSON path, regex on the body, JSON schema, latency budget |
| `sql` | A query on a saved DB connection | Row count, scalar equals or compare, column values, empty or non-empty, snapshot of the rows |
| `script` | Any command (shell, Python, Node); stdout and stderr captured | Exit code; or the Ubiq NDJSON event schema for a script that reports many results |

Later, not in the first phases: `browser` (Playwright through a recipe rather than embedded), `grpc`,
and `mcp` (call a tool on an MCP server and assert on the answer).

**`sql` reuses the DB mode's connection.** A check names a connection of the project's DB mode; it
never carries a connection string. The engine and drivers are `crates/ubiq-db` behind the host's
`db` feature, and the session code in `crates/ubiq-host/src/db/` already honours a connection's
read-only flag; the SQL MCP servers in `crates/ubiq-host/src/mcp/sql/` already answer through it.
A check run is another client of that session code, not a second driver stack.

## 4. The check file

**TOML**, because `ubiq.toml` and `projects.toml` are TOML and the repository already parses it
(open question 1). The shape is suites, then checks, then steps, then assertions.

```toml
[suite]
name = "orders-smoke"
tags = ["smoke"]
[suite.env.staging]
base_url = "https://staging.example.test"
db = "orders-ro"                     # a DB-mode connection, by name
[[check]]
name = "create then fetch order"
kind = "http"
timeout = "10s"
retries = 1
[[check.step]]
request = { method = "POST", url = "{{env.base_url}}/login", body = { user = "qa", password = "{{secret.qa_password}}" } }
expect = { status = 200 }
capture = { token = "$.token" }
[[check.step]]
request = { method = "GET", url = "{{env.base_url}}/orders/1", headers = { Authorization = "Bearer {{vars.token}}" } }
expect = { status = 200, "$.id" = 1, latency_ms = 800 }
```

| Element | Meaning |
|---|---|
| Suite | A file or a folder of files; carries tags, a default timeout and the environments |
| Environment | `dev`, `staging`, `prod`: base URLs and the DB connection to use; chosen at run time, never in the check |
| Variables | `{{env.*}}` from the environment, `{{vars.*}}` from captures and the suite, `{{secret.*}}` from references only |
| Order and dependencies | Steps run in order; a check may `needs` another, and a failed dependency skips it |
| Tags, timeout, retries | Per suite or per check; a retry is shown as such, so flakiness stays visible |

**Location.** A folder in the project, committed, so a team shares it and a clone just works. That
is the same trade the test-runner proposal states for the recipe: shareable, but a cloned check runs
requests against real systems, which makes the gates in section 6 matter. The alternative is Ubiq's
project store (open question 2).

## 5. Execution and results

The checks run in the **test worker** of the test-runner proposal, as a run with a `TestRunId`; the
native executors are added beside the process spawner, and a check emits the same run events, so the
explorer, results view, dashboard, history and flaky detection need no second path.

| Executor | Built on |
|---|---|
| `http` | The host's HTTP client, behind a host-only feature; timeouts, redirects and TLS settings are check fields. The client is blocking, like the worker threads. Whether `ureq` is reused or another client chosen is open question 5 |
| `sql` | The DB session code and `ubiq-db` drivers, behind the `db` feature |
| `script` | The worker's own piped process path; the NDJSON parser from the closed set |

Each result carries an **attachment** beside the usual message and location: for `http`, the request
as sent (secrets masked), the response status, headers and body (truncated with a count), and the
failing assertion as expected against actual; for `sql`, the query, the timing and the rows (capped).
The results tab shows them: a request and response viewer, and a rows grid reusing
`crates/ubiq/src/ui/db/grid.rs`. A failing JSON assertion shows a diff, not a message.

## 6. Secrets and safety

**A check references credentials, never holds them**, the rule in [`AGENTS.md`](../../AGENTS.md) for
accounts. A `{{secret.name}}` is a reference resolved by the host at run time and never written to a
check file, a result, a log line or an MCP answer; the same value is masked wherever it would be
echoed, including inside a response body that reflects it.

The provider is the open part. The DB mode already seals passwords with AES-256-GCM under a key in
the OS keychain (`crates/ubiq-host/src/db/secrets.rs`, in `<config root>/projects/<ulid>/local/`,
never inside the project). A check's `sql` kind uses exactly that, by connection name. For `http`
and `script` secrets there is no store yet; the candidates are the same sealed-file mechanism, the
keychain directly, or environment variables of the host (open question 6).

Checks touch real environments, so the defaults are conservative.

| Gate | Default |
|---|---|
| Read-only first | `sql` runs on a read-only session unless the check is marked mutating and the connection's agent access allows writes |
| Risky requests | `http` other than GET and HEAD asks for approval before it is sent |
| Environment gating | A run against an environment marked `prod` always asks, whoever starts it |
| Agent-authored checks | A new or changed check needs the user's approval before its first run against a non-local environment, shown as a diff |
| Dry run | Shows every request and query that would be sent, with secrets masked, and sends none |

Approval is a registered dialog through `ubiq-ask`, as in the [debugger proposal](./debugger-proposal.md)
and [HITL dialogs](./hitl-registered-dialogs.md). Which environments count as `prod`, and whether a
human-started run also asks, is open question 7.

## 7. The agent surface

The `ubiq-tests` server of the test-runner proposal grows check tools. Answers are TOON, bodies
truncated with a count, secrets masked.

| Tool | What it does |
|---|---|
| checks_list | Suites and checks with the last status per environment |
| check_propose | Submit a check file; Ubiq validates it against the schema and the user approves a diff before it is saved |
| check_run | A selector (suite, check, tag) and an environment; waiting or async; answers a run id and a summary |
| check_explain_failure | The failing step's request, response or rows, trimmed, with the assertion's expected and actual |
| check_from | Generate checks from a source: an OpenAPI document, a HAR file, a curl command, a SQL statement, a bug report or acceptance criteria |
| check_scratch | Run an ad-hoc unsaved check for exploratory QA, then promote it to a saved check through `check_propose` |

Design rules, as in the test-runner proposal: failures first, shared runs attributed to the agent,
one run per selector at a time. The validation step is the analogue of recipe validation: a check
that does not parse or names an unknown connection or secret is rejected with the reason before the
human sees it.

## 8. QA project workflows

| Workflow | Shape |
|---|---|
| Smoke per environment | A tagged suite run against `staging` or `prod` on demand |
| Regression after deploy | The same suite, run by the agent or the human after a release |
| Data quality | `sql` invariants: no orphan rows, counts within bounds, no nulls in a key column |
| Contract checks | Checks generated from an OpenAPI document; drift shows as new failures |
| Reporting | JUnit XML out for CI, and a markdown summary of a run for a card or a pull request |
| A QA agent | An agent definition ([agent definitions](./agents-definitions.md)) with a QA skill and the `ubiq-tests`, `ubiq-sql-read` and `ubiq-ask` servers preset |

**Triggers.** Ubiq is a desktop application, so a scheduled run on a closed machine is not
available. The first triggers are manual, on project open, and by an agent; scheduling is later and
open question 3.

## 9. Interop and prior art

| Tool | Relation |
|---|---|
| Hurl | Plain-text HTTP tests with JUnit and JSON output; runs as a recipe today; also a candidate engine for `http` (open question 8) |
| Bruno, Postman with Newman | Collections are the main importer source (phase 3); Newman can run as a recipe with JUnit output |
| k6, Schemathesis, Playwright, Robot Framework | Recipes; none is embedded |
| dbt tests, Soda, Great Expectations | Recipes for data quality; `sql` checks cover the small invariants without installing them |

Command lines and output formats of these tools are from memory (verify) before a preset or a
reference recipe relies on them.

## 10. UI changes in IDE mode

The panels are the test-runner proposal's; the delta is small.

| Item | Change |
|---|---|
| Test explorer | Groups by suite and environment; an environment picker in its header; a kind icon per check |
| Test results | A request and response viewer, a rows grid for `sql`, the assertion diff |
| Dashboard | Pass and fail by environment, latency per check over history |
| Check editor | The plain file in the editor; schema-driven completion and validation later; "run this check" from the file and the explorer |
| Theme and status | Tokens and status counts are the test-runner proposal's; an environment badge marks `prod` |

## 11. Phasing

After phase 0 of the test-runner proposal (the worker, the Test family, the explorer and results).

| Phase | Scope |
|---|---|
| 0 | `script` and `http` checks, the file format, environments, secret references, the results viewer, the check MCP tools |
| 1 | `sql` checks through `ubiq-db` and the DB mode's connections |
| 2 | Approval gates hardened, scratch and promote, generation from OpenAPI, curl and HAR |
| 3 | Importers (Postman collection, HAR, curl), JUnit and markdown export, the QA agent definition |
| Later | `browser`, `grpc` and `mcp` kinds; scheduling |

## 12. Decisions this would add

- A check is a first-class test: Ubiq executes `http`, `sql` and `script` checks natively and
  reports them through the test run model, not as a pane. **Cost:** a second source of run events
  beside the parsers, and executors to maintain.
- Checks are declarative files with references to secrets and environments, never inline credentials.
  **Cost:** a secret provider for `http` and `script` that does not exist yet.
- `sql` checks use the DB mode's connections and read-only flag. **Cost:** a check cannot run
  without a connection configured in the project first.
- Real environments are gated: non-GET HTTP, `prod` and first runs of agent-authored checks ask.
  **Cost:** an agent cannot run a fresh QA suite against staging unattended.
- Assertions stay a closed small set; anything richer is a `script` check or a recipe. **Cost:**
  some checks are awkward to express.

## 13. Open questions

1. TOML or YAML for the check file. TOML matches `ubiq.toml`; YAML is what most HTTP tooling users
   expect and nests request bodies more naturally.
2. Where checks live: a committed folder in the project, or Ubiq's project store.
3. Scheduling: none, on project open, or a background schedule while the application runs; and how a
   closed laptop is reported.
4. How far assertions go before a check should be a script; whether a small expression language is
   worth having.
5. Which HTTP client the executor uses, and whether it lives in the base or in Studio, as the same
   seam question in the test-runner proposal.
6. The secret provider for `http` and `script` checks: the DB mode's sealed file, the keychain, or
   the host's environment.
7. The prod policy: which environments are `prod`, whether a human-started run asks too, and whether
   a cloned repository's checks are trusted until a human has seen them.
8. Hurl as the engine for `http`, with Ubiq as its runner and viewer, against a native executor.
9. Which kinds ship first, and whether `sql` can precede `http`.
10. How a check's attachment is persisted in run history without keeping response bodies forever.

## Related docs

- [Test-runner proposal](./test-runner-proposal.md) — the run model, worker, panels and MCP server this extends
- [Debugger proposal](./debugger-proposal.md) — the approval dialogs and shared-session pattern
- [HITL registered dialogs](./hitl-registered-dialogs.md) — the approval gate
- [Agent definitions](./agents-definitions.md) — the QA agent
- [The DB workbench](../features/workbench-db.md) — the connections, read-only flag and sealed passwords `sql` reuses
- [The IDE workbench](../features/workbench-ide.md) — the mode the panels live in
- [Architecture](../tech/architecture.md) — the rules the executors follow
- [Decisions](../tech/decisions.md) — the record this would extend
