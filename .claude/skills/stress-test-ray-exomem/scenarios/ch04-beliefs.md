# Ch04 — Beliefs (believe / supersede / revoke / supports)

Verify the belief lifecycle: a single `belief_id` can have one active row, get
superseded, get revoked, and link back to facts via `belief/supports`.

## Setup

Use Ch02's `test/n` fact_id as the support target — call it `F_n`.

## Steps

1. **First version** —
   `believe { exom: <session>, belief_id: "belief/topic#v1", claim_text: "n is 84", confidence: 0.9, supports: [F_n] }`.
   Capture the returned `belief_id`. Also capture the `tx_id`.

2. **Supersede** — same `belief_id`, new claim:
   `believe { exom: <session>, belief_id: "belief/topic#v1", claim_text: "n is actually 84 (refined)", confidence: 0.95, supports: [F_n] }`.
   This should mark the prior row `superseded`.

3. **Revoke** —
   `revoke_belief { exom: <session>, belief_id: "belief/topic#v1" }`.
   The active row from step 2 should now show `status="revoked"`.

4. **Fresh belief** —
   `believe { exom: <session>, belief_id: "belief/topic#v2", claim_text: "n was 84 at backfill time", confidence: 0.7, supports: [F_n] }`.
   Should be `status="active"`.

## Verification queries

- `belief-row` view scoped to the scratch session — **arity 4**, not 5:
  `(query <session> (find ?bid ?text ?status ?tx) (where (belief-row ?bid ?text ?status ?tx)))`.
  `belief/confidence` lives on the entity but is not projected by the view —
  query it via direct EAV (`(?bid 'belief/confidence ?conf)`) if you need it.

  Expect **2 rows** — supersede mutates the existing belief entity in place
  (replaces `claim_text` and `belief/created_by`), it does NOT emit a separate
  `superseded` row:
  - `belief/topic#v1` with `claim_text = "n is actually 84 (refined)"` and
    `status = "revoked"` — one entity, latest revision's claim, latest
    lifecycle status.
  - `belief/topic#v2` with `status = "active"`.

- `belief/supports` triple:
  `(query <session> (find ?bid ?fid) (where (?bid 'belief/supports ?fid)))`.
  Expect rows linking each belief id to `F_n`.

## Pass criteria

- 2 rows on `belief-row`. Statuses: one `revoked` (v1), one `active` (v2).
- Both carry a non-empty `tx_id` and `tx/user_email`.
- `belief/supports` returns a row whose object equals `F_n` for both v1 and v2.

## Evidence

- The 4 captured tx_ids.
- Verbatim `belief-row` rows.
- Verbatim `belief/supports` rows.

## Notes

- Don't conflate "supersede" (same `belief_id`, new claim) with "revoke"
  (active row marked revoked, no replacement). Both must be possible.
- Supersede is **in-place** on the entity — `belief-row` returns one row per
  `belief_id` reflecting the latest claim and lifecycle status, never two
  rows where one is `superseded` and one is the new revision. The full
  revision chain lives in `belief_history` (and on the underlying tx log
  with `tx/action = "revise-belief"`); the row-view collapses it to the
  current state.
- If the supersede leaves *two* rows for `belief/topic#v1` in `belief-row`,
  the view regressed in the other direction (was emitting separate rows
  per revision).
