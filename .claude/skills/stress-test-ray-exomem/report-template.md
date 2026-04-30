# ray-exomem stress-test report

- **Run ID:** `<run-id>`
- **Started:** `<UTC-ISO>`
- **Finished:** `<UTC-ISO>`
- **Base URL:** `<base_url>`
- **Build identity:** `<n/a from MCP-only run — /api/status requires bearer/cookie auth and the MCP guide() tool doesn't expose it; if a build identity is needed, fetch /api/status out-of-band with a bearer token>`
- **Scratch session:** `<session-path>`
- **Flags:** `--with-team=<bool>` `--with-collision-user=<bool>` `--with-admin-probes=<bool>` `--scratch=<public|private>`

## Matrix

| Chapter | Scenario                                  | Status      | Evidence                                                  |
|---------|-------------------------------------------|-------------|-----------------------------------------------------------|
| Ch01    | MCP surface complete                      | <pass/fail> | tools/list contains init / exom_new / tree / merge_branch / archive_branch (+ branch arg on query/eval) |
| Ch01    | guide returns markdown                    | <pass/fail> | <bytes returned>                                          |
| Ch01    | list_exoms ≥ 1                            | <pass/fail> | <count + first-3 paths>                                   |
| Ch01    | exom_status of session                    | <pass/fail> | <{current_branch, facts, beliefs, transactions}> — expect `transactions==0` (genesis is `tx/0`, not counted) |
| Ch01    | schema lists tx/user_email + tx-row arity | <pass/fail> | <attrs found / attrs missing> — `belief-row` is arity 4 (`?belief ?claim ?status ?tx`), `observation-row` is arity 4 (`?obs ?source_type ?content ?tx`) |
| Ch01    | list_branches: main + 2 unclaimed agents  | <pass/fail> | <branch labels list>                                      |
| Ch02    | I64 from JSON number                      | <pass/fail> | <fact_id>                                                 |
| Ch02    | I64 from round-trip string                | <pass/fail> | <fact_id>                                                 |
| Ch02    | Str from non-round-trip "007"             | <pass/fail> | <fact_id>                                                 |
| Ch02    | Str from "+5"                             | <pass/fail> | <fact_id>                                                 |
| Ch02    | Sym from `{"$sym":"active"}`              | <pass/fail> | <fact_id>                                                 |
| Ch02    | facts_i64 EAV returns 2 rows              | <pass/fail> | <returned tuples>                                         |
| Ch02    | facts_str EAV returns 2 rows              | <pass/fail> | <returned tuples>                                         |
| Ch02    | facts_sym EAV returns 1 row               | <pass/fail> | <returned tuple>                                          |
| Ch02    | facts_i64 cmp filter `< 100`              | <pass/fail> | <returned tuple>                                          |
| Ch03    | backfill assert (valid_from past)         | <pass/fail> | <fact_id, tx_id>                                          |
| Ch03    | supersede same fact_id                    | <pass/fail> | <new tx_id>                                               |
| Ch03    | explicit valid_to                         | <pass/fail> | <tx_id, valid_to>                                         |
| Ch03    | retract_fact                              | <pass/fail> | <retract tx_id>                                           |
| Ch03    | fact_history shows 3 value-interval tuples| <pass/fail> | <history rows summary>                                    |
| Ch03    | superseded_by / revoked_by back-pointers  | <pass/fail> | <T1→T2, T2→T3, T3 revoked_by=T4>                          |
| Ch03    | valid_to chains correctly                 | <pass/fail> | <T1.valid_to == T2.valid_from; T3.valid_to == retract_t>  |
| Ch03    | retract event in tx-log                   | <pass/fail> | <1 row, action="retract-fact">                            |
| Ch03    | every history row carries full triple     | <pass/fail> | <user_email/agent/model presence>                         |
| Ch04    | believe v1 with supports=[Ch02 fact]      | <pass/fail> | <belief_id>                                               |
| Ch04    | supersede same belief_id (in-place)       | <pass/fail> | <new tx_id; belief-row claim_text now reflects new revision — view returns 1 row per belief_id, no separate "superseded" row> |
| Ch04    | revoke_belief                             | <pass/fail> | <belief-row status="revoked">                             |
| Ch04    | believe v2 fresh id                       | <pass/fail> | <belief-row status="active">                              |
| Ch04    | belief/supports links to Ch02 fact        | <pass/fail> | <support tuple>                                           |
| Ch04    | belief-row total = 2 (v1 revoked + v2 active) | <pass/fail> | <row count + statuses>                                |
| Ch05    | observe with 3 tags                       | <pass/fail> | <obs_id>                                                  |
| Ch05    | observe second w/ same source_type        | <pass/fail> | <obs_id>                                                  |
| Ch05    | observation-row arity                     | <pass/fail> | <2 rows>                                                  |
| Ch05    | obs/tag triple count                      | <pass/fail> | <3 tag rows for first obs>                                |
| Ch05    | obs/tx recoverable                        | <pass/fail> | <both tx_ids>                                             |
| Ch06    | create_branch feature-x                   | <pass/fail> | <branch row>                                              |
| Ch06    | assert on feature-x                       | <pass/fail> | <fact_id, tx_id>                                          |
| Ch06    | branch isolation: feature-x visible       | <pass/fail> | <fact returned>                                           |
| Ch06    | branch isolation: main NOT visible        | <pass/fail> | <empty rows>                                              |
| Ch06    | list_branches: feature-x present          | <pass/fail> | <is_current=false after cross-branch query>               |
| Ch06    | merge feature-x → main                    | <pass/fail> | <merge tx_id, fx/marker now on main>                      |
| Ch06    | archive feature-x                         | <pass/fail> | <branch/archived="true">                                  |
| Ch07    | session_new single → only main            | <pass/fail> | <list_branches output>                                    |
| Ch07    | session_close blocks writes               | <pass/fail> | <error: "session_closed">                                 |
| Ch07    | multi-session closed_at unset → set       | <pass/fail> | <pre/post values>                                         |
| Ch07    | bad label "/" rejected                    | <pass/fail> | <error: "invalid label">                                  |
| Ch07    | session_join unknown agent_label          | <pass/fail> | <error: "BranchMissing">                                  |
| Ch07    | init scaffolds project                    | <pass/fail> | <main + sessions/ in tree>                                |
| Ch07    | exom_new creates bare exom                | <pass/fail> | <Exom node visible in tree>                               |
| Ch08    | full triple (agent + model)               | <pass/fail> | <tx-row tuple>                                            |
| Ch08    | no agent → API-key-label fallback         | <pass/fail> | <tx/agent value>                                          |
| Ch08    | no model → no tx/model row                | <pass/fail> | <empty EAV vs strict tx-row>                              |
| Ch08    | tx-row arity 8                            | <pass/fail> | <tuple width>                                             |
| Ch10    | view: fact-row                            | <pass/fail> | <row count>                                               |
| Ch10    | view: fact-meta                           | <pass/fail> | <row count>                                               |
| Ch10    | view: fact-with-tx                        | <pass/fail> | <row count>                                               |
| Ch10    | view: tx-row                              | <pass/fail> | <row count>                                               |
| Ch10    | view: observation-row                     | <pass/fail> | <row count>                                               |
| Ch10    | view: belief-row (= 2; Ch04 v1 revoked + v2 active, in-place supersede merges into v1) | <pass/fail> | <row count> |
| Ch10    | view: branch-row                          | <pass/fail> | <row count>                                               |
| Ch10    | view: claim-owner-row (?fact ?owner)      | <pass/fail> | <row count>                                               |
| Ch10    | EDB: facts_i64 / facts_str / facts_sym    | <pass/fail> | <row counts>                                              |
| Ch10    | explain by predicate                      | <pass/fail> | <result snippet>                                          |
| Ch10    | explain by fact_id                        | <pass/fail> | <result snippet>                                          |
| Ch10    | export json                               | <pass/fail> | <bytes>                                                   |
| Ch10    | export jsonl                              | <pass/fail> | <line count>                                              |
| Ch11    | unknown_exom                              | <pass/fail> | <error string>                                            |
| Ch11    | unknown_branch                            | <pass/fail> | <error string>                                            |
| Ch11    | query missing database name               | <pass/fail> | <error string>                                            |
| Ch11    | server-side arity error                   | <pass/fail> | <error: "rule '...' expects N args, got M">               |
| Ch11    | invalid value (array)                     | <pass/fail> | <error string>                                            |
| Ch11    | missing required parameter                | <pass/fail> | <error: "missing required parameter: predicate">          |
| Ch11    | empty-string predicate rejected           | <pass/fail> | <error: "invalid 'predicate': must be non-empty">         |
| Ch11    | BranchOwned                               | <skip/p/f>  | <error string> *(needs --with-collision-user)*            |
| Ch12    | hyphen attr probe (tx/user-email = 0 rows)| <pass/fail> | <row counts>                                              |
| Ch12    | default-fact-id supersede                 | <pass/fail> | <fact_history rows>                                       |
| Ch12    | sym health (no domain error on query)     | <pass/fail> | <ok / RAY_ERROR text>                                     |
| Ch12    | cache staleness post-join                 | <pass/fail> | <claim triple populated immediately>                      |
| Ch12    | cross-branch cursor restoration           | <pass/fail> | <main is_current after non-main query>                    |
| Ch09    | TeamCreate + 2 sub-agents joined          | <skip/p/f>  | <team id, agent ids> *(--with-team)*                      |
| Ch09    | each agent asserted 2 facts on its branch | <skip/p/f>  | <fact_ids per agent>                                      |
| Ch09    | list_branches: full claim triple per      | <skip/p/f>  | <branch-row tuples>                                       |
| Ch09    | cross-branch query agent-a returns its tx | <skip/p/f>  | <tx_id>                                                   |
| Ch09    | second join of same branch idempotent     | <skip/p/f>  | <ok, no error>                                            |

**Summary:** `<P> / <T> passed, <F> failed, <S> skipped`

## Failures (verbatim)

<For each `fail` row, paste the raw evidence: error string, returned tuple, or
diff vs. expected. No paraphrasing.>

## Notes

<Any observations the skill's runner thinks the user should know — for example,
"Build identity unchanged across this run", or "Ch11 BranchOwned skipped because
no second bearer was supplied".>
