# ray-exomem stress-test report

- **Run ID:** `<run-id>`
- **Started:** `<UTC-ISO>`
- **Finished:** `<UTC-ISO>`
- **Base URL:** `<base_url>`
- **user1 email:** `<user1_email>` *(orchestrator)*
- **user2 email:** `<user2_email>` *(cross-user; from `--dev-login-email` allow-list)*
- **Build identity:** `<n/a from MCP-only run — /api/status requires bearer/cookie auth and the MCP guide() tool doesn't expose it; if a build identity is needed, fetch /api/status out-of-band with a bearer token>`
- **Scratch (private):** `<scratch_project>` *(Phase 1+2)*
- **Scratch (public):** `<public_scratch>` *(Phase 3)*
- **Session:** `<session>`
- **Flags:** `--no-cross-user=<bool>` `--with-team=<bool>` `--with-admin-probes=<bool>` `--scratch=<public|private>`

## Matrix

| Phase | Group / Step                                                    | Status      | Evidence |
|-------|-----------------------------------------------------------------|-------------|----------|
| **0** | Preconditions (MCP, Chrome, dev-login, loopback)                | <pass/fail> | <abort cause if any>                              |
| 0     | Discovery: user1 + user2 emails resolved                        | <pass/fail> | <both emails>                                     |
| 0     | Init `<scratch_project>` + session_new                          | <pass/fail> | <ok responses>                                    |
| 0     | Schema snapshot (attrs, view arities)                           | <pass/fail> | <attr count, view arities>                        |
| 0     | Two Chrome contexts dev-login OK                                | <skip/p/f>  | <`/auth/me` per context>                          |
| **1** | A: read-tool surface (guide, list_exoms, exom_status, list_branches) | <pass/fail> | <bytes, count, status, branch count>          |
| 1     | B: typed values (i64×2 + str×2 + sym×1 land in correct EDB)     | <pass/fail> | <fact_ids + EDB row counts>                       |
| 1     | B: cmp filter `< 100` returns `84`                              | <pass/fail> | <result>                                          |
| 1     | C: bitemporal — 4 transitions, 3 value-intervals, back-pointers | <pass/fail> | <T1..T4, history rows>                            |
| 1     | C: retract event in tx-log                                      | <pass/fail> | <tx row>                                          |
| 1     | D: belief lifecycle (believe + supersede + revoke + v2)         | <pass/fail> | <belief_ids, statuses>                            |
| 1     | D: belief-row total = 2                                         | <pass/fail> | <row count>                                       |
| 1     | E: 2 observations + 3-tag obs                                   | <pass/fail> | <obs_ids, tag count>                              |
| 1     | F: builtin-view sweep (8 views, expected ≥ counts)              | <pass/fail> | <per-view row counts>                             |
| 1     | G: attribution triple non-empty                                 | <pass/fail> | <tx-row tuple>                                    |
| 1     | H: explain (predicate + fact_id) + export (canonical + jsonl)   | <pass/fail> | <bytes / line count>                              |
| **2** | A: branch lifecycle (create / assert / isolation / list / merge / archive) | <pass/fail> | <T_fx, merge_tx, archived flag>           |
| 2     | B: session lifecycle (single / bad-label / unknown-agent / close / closed_at) | <pass/fail> | <error strings>                            |
| 2     | C: scaffolding (init / exom_new / folder_new / rename / delete) | <pass/fail> | <`<scratch_bare>` path>                           |
| 2     | D1: hyphen attr probe → 0 rows                                  | <pass/fail> | <count>                                           |
| 2     | D2: default-fact-id supersede → 2 intervals                     | <pass/fail> | <fact_history>                                    |
| 2     | D3: sym health (no domain error)                                | <pass/fail> | <ok / RAY_ERROR text>                             |
| 2     | D4: cache staleness post-join (claim populated immediately)     | <pass/fail> | <list_branches probe-d row>                       |
| **3** | A: Model A 403 (auth-layer; NOT branch_owned)                   | <skip/p/f>  | <verbatim error>                                  |
| 3     | A: created_by stamp + forked_from absent on non-fork            | <skip/p/f>  | <tree node fields>                                |
| 3     | B: fork default-target shape (public → `{email}/forked/...`)    | <skip/p/f>  | <returned target verbatim>                        |
| 3     | B: fork explicit target overrides default                       | <skip/p/f>  | <returned target == explicit value>               |
| 3     | B: fork lineage in tree (created_by, acl_mode, forked_from)     | <skip/p/f>  | <tree node fields>                                |
| 3     | B: fork replayed attribution (every row → forker email)         | <skip/p/f>  | <forker email per row>                            |
| 3     | B: fork auto-suffix on collision (leaf segment `-2`/`-3`)       | <skip/p/f>  | <first/second targets>                            |
| 3     | B: fork default-target shape (`{other_email}/*` source)         | <skip/p/f>  | <returned target preserves owner email subpath>   |
| 3     | B: fork refuses session                                         | <skip/p/f>  | <verbatim `fork_session_unsupported`>             |
| 3     | C1: flip solo→co-edit (creator) returns ok+changed              | <skip/p/f>  | <flip response>                                   |
| 3     | C2: main claim cleared after flip                               | <skip/p/f>  | <list_branches main row>                          |
| 3     | C3: co-edit auth elevation (user2 write succeeds)               | <skip/p/f>  | <ok+tx_id>                                        |
| 3     | C4+5: symmetric retracts (each user retracts the other's fact)  | <skip/p/f>  | <facts removed>                                   |
| 3     | C6: `_meta/acl_mode` audit-trail fact present                   | <skip/p/f>  | <fact row>                                        |
| 3     | C7: non-creator flip → 403 not_creator                          | <skip/p/f>  | <verbatim error>                                  |
| 3     | C8: session flip → 400 acl_mode_not_applicable                  | <skip/p/f>  | <verbatim error>                                  |
| 3     | D: co-edit non-`main` TOFU preserved (user2 → 400 branch_owned) | <skip/p/f>  | <verbatim error>                                  |
| 3     | E: co-edit child session is solo-edit owned by spawner          | <skip/p/f>  | <session tree node>                               |
| 3     | F: `{email}/*` co-edit + rw share → user2 writes successfully   | <skip/p/f>  | <ok+tx_id>                                        |
| 3     | F: flip-back to solo-edit → user2 hits 400 branch_owned         | <skip/p/f>  | <verbatim error>                                  |
| 3     | G: acl_mode persists across daemon restart                      | <skip/p/f>  | <pre/post values per exom>                        |
| **4** | unknown_exom                                                    | <pass/fail> | <verbatim error>                                  |
| 4     | unknown_branch                                                  | <pass/fail> | <verbatim error>                                  |
| 4     | query missing database name                                     | <pass/fail> | <verbatim error>                                  |
| 4     | server-side arity error (non-empty msg)                         | <pass/fail> | <verbatim error>                                  |
| 4     | invalid value (array)                                           | <pass/fail> | <verbatim error>                                  |
| 4     | missing required parameter (predicate)                          | <pass/fail> | <verbatim error>                                  |
| 4     | empty-string predicate rejected                                 | <pass/fail> | <verbatim error>                                  |
| 4     | cannot archive `main`                                           | <pass/fail> | <verbatim error>                                  |
| **5** | TeamCreate + 2 sub-agents joined                                | <skip/p/f>  | <team id> *(--with-team)*                         |
| 5     | each agent claimed its branch (full triple)                     | <skip/p/f>  | <branch-row tuples>                               |
| 5     | cross-branch query returns agent-a's 2 tx                       | <skip/p/f>  | <tx_ids>                                          |
| 5     | second join idempotent                                          | <skip/p/f>  | <ok>                                              |
| 5     | bonus: cross-user write rejected (if `--scratch public`)        | <skip/p/f>  | <verbatim error>                                  |

**Summary:** `<P> / <T> passed, <F> failed, <S> skipped`

## Failures (verbatim)

<For each `fail` row, paste the raw evidence: error string, returned tuple, or
diff vs. expected. No paraphrasing.>

## Notes

<Anything the operator should know — public-namespace exoms left visible after the run, build identity unchanged across this run, Phase 5 skipped because `--with-team` not provided, etc.>
