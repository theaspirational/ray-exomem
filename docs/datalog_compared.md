# Datalog in ray-exomem — for people who've used Datomic

ray-exomem speaks a Datalog dialect with the same EAV-and-time heritage as
Datomic, but a few load-bearing things are different. If you already think
in Datomic, this page is the diff. If you don't — pretend the left column
is "the textbook way" and the right column is "what actually runs here."

## At a glance

| | Datomic | ray-exomem |
|---|---|---|
| **Storage** | Entity-Attribute-Value-Tx triples | Entity-Attribute-Value-Tx triples, branched |
| **Time** | tx-time only ("when did the system learn this?") | tx-time **and** wall-clock `valid_from`/`valid_to` ("when was this true in the world?") |
| **Predicate names** | First-class **attributes** in the schema (`:user/name`) | **Values** inside the EAV store. Project them through `fact-row` to query. |
| **Schema** | Declared up front (`db.install/attribute`) | Open — assert any predicate, the schema reports what's been seen. System attributes are pre-defined. |
| **Branches** | Not a thing | Lightweight copy-on-write namespaces with TOFU ownership |
| **Engine** | Proprietary, JVM | [rayforce2](https://github.com/RayforceDB/rayforce2) (C, Datalog-on-columnar) |
| **Surface** | Peer / Client API, EDN, `[?e :a ?v]` body atoms | MCP / HTTP, Rayfall sexpr, `(rel ?args)` body atoms |

---

## The one gotcha that bites every newcomer

In Datomic, `:user/name` is a **schema attribute** — the name of a column.
In ray-exomem, the literal string `"entity/name"` is **a value** living
inside the universal EAV table. There's no relation called `entity/name`
for the engine to scan; there's just one big triple store.

That means **direct-predicate queries** look the same syntactically but
behave differently:

<table>
<tr><th>Datomic — <code>:user/name</code> is a real attribute</th><th>ray-exomem — predicate names are values; project through <code>fact-row</code></th></tr>
<tr>
<td>

```clojure
;; Find all users named Alice
[:find ?u
 :where [?u :user/name "Alice"]]
```

</td>
<td>

```scheme
; Find all entities whose entity/name is "Alice"
(query <exom>
       (find ?id)
       (where (fact-row ?id "entity/name" "Alice")))
```

</td>
</tr>
</table>

Try the Datomic shape against ray-exomem (`(where (entity/name ?id "Alice"))`)
and you get `unknown relation 'entity/name' (did you mean 'fact-row'?)`.
That error is the system pointing you at the EAV view.

---

## Real-life examples

### 1. "Find every concept tagged as a language-feature"

<table>
<tr><th>Datomic</th><th>ray-exomem</th></tr>
<tr>
<td>

```clojure
[:find ?id ?name
 :where
   [?id :entity/type "language-feature"]
   [?id :entity/name ?name]]
```

</td>
<td>

```scheme
(query <exom>
       (find ?id ?name)
       (where
         (fact-row ?id "entity/type" "language-feature")
         (fact-row ?id "entity/name" ?name)))
```

</td>
</tr>
</table>

Same join shape, same answer. The only structural difference is that you
join on `fact-row` (the EAV projection view) instead of pinning to a typed
attribute — because every predicate in ray-exomem flows through the same
view.

### 2. "What did this fact look like a year ago?"

Both systems are bitemporal-ish, but they slice time differently. Datomic's
`asOf` rewinds the system clock — "show me what the database knew on this
date." ray-exomem also lets you ask "what was *true in the world* on this
date" via `valid_from` / `valid_to`.

<table>
<tr><th>Datomic — system-time travel</th><th>ray-exomem — wall-clock + system time</th></tr>
<tr>
<td>

```clojure
;; Database state as of one year ago.
;; Returns: facts the DB knew at that
;; instant, regardless of when they
;; became true in the modelled world.
(d/q '[:find ?u ?status
       :where [?u :user/status ?status]]
     (d/as-of db one-year-ago))
```

</td>
<td>

```scheme
; Facts whose VALID interval covers
; "one year ago" — the world's view,
; not the system's.
(query <exom>
       (find ?u ?status ?vf ?vt)
       (where
         (fact-row ?u "user/status" ?status)
         (?u 'fact/valid_from ?vf)
         (?u 'fact/valid_to   ?vt)
         (<= ?vf "2025-04-28T00:00:00Z")
         (or (= ?vt nil)
             (>  ?vt "2025-04-28T00:00:00Z"))))
```

</td>
</tr>
</table>

The practical implication: ray-exomem can store "Alice was a junior eng from
2022-03 to 2024-09 (we noted this in 2026)" without confusion. Datomic
needs an explicit modeling pattern for that.

### 3. "Define a derived view"

Both systems compose rules; the surface is mostly cosmetic.

<table>
<tr><th>Datomic</th><th>ray-exomem</th></tr>
<tr>
<td>

```clojure
[[(active-user ?u)
  [?u :user/status :active]]]

;; query
[:find ?u :in $ %
 :where (active-user ?u)]
```

</td>
<td>

```scheme
(rule <exom> (active-user ?u)
  (fact-row ?u "user/status" "active"))

; query
(query <exom>
       (find ?u)
       (where (active-user ?u)))
```

</td>
</tr>
</table>

User-defined rule heads become first-class relations you can put in
`(where ...)`. So if you find yourself repeating `(fact-row ?id
"entity/type" "X")`, lift it into a rule.

### 4. "Aggregate over numeric values"

Datomic uses functions in `:find`. ray-exomem uses aggregation operators
in the body, fed by the typed EDB so values are real ints, not strings.

<table>
<tr><th>Datomic</th><th>ray-exomem</th></tr>
<tr>
<td>

```clojure
;; Average SLA across services
[:find (avg ?ms)
 :where [?s :service/sla_p99_ms ?ms]]
```

</td>
<td>

```scheme
; Same — but you join through facts_i64,
; not fact-row, because aggregation
; needs the I64-typed column.
(query <exom>
       (find ?avg)
       (where
         (avg ?avg facts_i64 2
              by ?attr 1)
         (= ?attr "service/sla_p99_ms")))
```

</td>
</tr>
</table>

Why the typed EDB: assert the value as a JSON number (`75`) and ray-exomem
stores it as `I64`, queryable with `<` / `>` / `sum` / `avg`. Assert as a
string (`"75"`) and it becomes `Str` — fine for equality, useless for math.

### 5. "Try a what-if change without committing to main"

Datomic has no equivalent. ray-exomem has cheap branches:

```scheme
; Create a session with two participants. Each gets a pre-allocated branch.
session_new {
  project_path: "public/work/x/feature-foo",
  session_type: "multi",
  label:        "redesign",
  agents:       ["alice", "bob"]
}

; Alice writes to her branch without disturbing main:
assert_fact {
  exom:      "<session_path>",
  predicate: "design/decision",
  fact_id:   "auth#decision",
  value:     "use opaque tokens",
  branch:    "alice",
  actor:     "alice"
}

; Read her view of the world:
(query <session_path>
       (find ?id ?v)
       (where (fact-row ?id "design/decision" ?v)))
; on branch alice → her decision
; on branch main  → empty
```

TOFU ownership: first writer to a branch claims it; subsequent writers
with a different actor get `branch_owned`. Used to coordinate parallel
agents without a central lock.

### 6. "Negation"

Datomic has `not` and `not-join`. ray-exomem has the same logical `not`
in the body, with the rayforce2 stratifier checking for unstratifiable
cycles before evaluation.

<table>
<tr><th>Datomic</th><th>ray-exomem</th></tr>
<tr>
<td>

```clojure
;; Users without a reported status
[:find ?u
 :where
   [?u :user/name _]
   (not [?u :user/status _])]
```

</td>
<td>

```scheme
(query <exom>
       (find ?u)
       (where
         (fact-row ?u "entity/name" _)
         (not (fact-row ?u "user/status" _))))
```

</td>
</tr>
</table>

If you write a self-referential rule that depends negatively on its own
head (e.g. `r(x) :- not r(x)`), ray-exomem returns
`query: unstratifiable negation cycle` instead of looping forever.

---

## When ray-exomem is the wrong fit

- **You need OLAP-grade joins on billions of rows.** rayforce2 is fast for
  knowledge-base loads, not warehouse loads. Push to a real data warehouse.
- **You need ACID across a remote cluster.** ray-exomem is a single-node
  daemon today; there's no replication or distributed consensus.
- **You need transitive closure beyond a few hops on huge graphs.**
  Stratified Datalog handles it correctly but won't beat a graph DB on
  100M-edge fan-outs.

For "an agent's persistent memory + a small team's shared notebook," the
EAV+bitemporal+branches model lands the right tradeoffs.

---

## See also

- [`agent_guide.md`](./agent_guide.md) — MCP tool reference for agents.
- The bootstrap `getting-started/main` exom — assertions of these
  examples that you can `query` and inspect live.
- [`rayforce2`](https://github.com/RayforceDB/rayforce2) — the Datalog
  engine underneath.
