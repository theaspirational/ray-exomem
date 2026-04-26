# Onboarding Template System — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded three-exom bootstrap (`personal/health`, `work`, `work/example`) with a data-driven template system. The operator chooses which templates are offered to new users via CLI flags; users pick exactly one template (or skip) on a `/welcome` page after first login. The repo ships built-in templates as TOML files and supports an external override directory for company-specific deployments.

**Architecture:**
- Server reads template TOMLs at startup (built-in via `include_dir!` or external via `--onboarding-template-dir`), filters by `--onboarding-templates` whitelist, and exposes the catalog through new `/api/onboarding/*` endpoints. The login handler stops eagerly seeding three exoms; instead it provisions a single empty `{email}/main` and marks the user not-yet-onboarded.
- UI gains a `/welcome` route that fetches the catalog and posts a seed/skip request. Routing logic redirects unbootstrapped users to `/welcome` after login.
- Persistence: `StoredUser` gains `onboarded: bool` and `seeded_templates: Vec<String>`. Both JSONL-backed and Postgres-backed auth stores carry these fields. JSONL replay preserves them across reads.
- The per-template seed includes a sentinel fact `(onboarding/template_id "<id>")` and `(onboarding/template_version "<version>")` for future migration ladders.

**Out of scope of this plan:** Adding aggregate / float / between features to rayforce2 (that work has its own plan at `~/code/<workspace>/rayforce2/docs/plans/2026-04-18-datalog-aggregates-and-onboarding.md`). The Phase B prelude in that plan — deleting `system_schema::native_derived_relations` — is included here as Task 1 because the new health template owns those derivations declaratively and any leftover procedural code would double-fire.

**Tech Stack:** Rust (axum, serde, toml, include_dir), SvelteKit 5 (Tailwind, shadcn-svelte), JSONL persistence, optional Postgres.

**Branch:** `feature/onboarding-templates` on ray-exomem (`git checkout -b feature/onboarding-templates main`).

---

## File map

**Created:**
- `assets/onboarding/empty.toml`
- `assets/onboarding/health.toml`
- `assets/onboarding/workspace.toml`
- `assets/onboarding/project.toml`
- `src/onboarding.rs` — template loading, schema, catalog filtering, seeding
- `src/onboarding/template.rs` — `OnboardingTemplate` struct + TOML deserialization
- `src/onboarding/seed.rs` — apply a template to an exom
- `ui/src/routes/welcome/+page.svelte` — welcome screen
- `ui/src/routes/welcome/+page.ts` — load function for catalog
- `ui/src/lib/onboarding.svelte.ts` — UI client for `/api/onboarding/*`

**Modified:**
- `src/main.rs` — new CLI flags on `serve` and `daemon`
- `src/server.rs` — `AppState::onboarding_catalog` field + setter
- `src/auth/routes.rs` — strip `bootstrap_user_namespace` down to provisioning a single empty exom; add `/api/onboarding/*` routes
- `src/auth/store.rs` — `StoredUser.onboarded` + `seeded_templates`; JSONL replay; new `record_onboarding_seed` / `record_onboarding_skip`
- `src/db/mod.rs` and `src/db/pg_auth.rs` — Postgres mirror of new fields + migration
- `src/system_schema.rs` — delete `native_derived_relations` (Phase B prelude)
- `src/server.rs` — delete `known_derived_samples` block (Phase B prelude)
- `ui/src/lib/auth.svelte.ts` — extend `AuthUser` with `onboarded` field
- `ui/src/routes/+layout.svelte` — redirect unbootstrapped users to `/welcome`
- `ui/src/lib/stores.svelte.ts` — `defaultExomForUser` returns `{email}/main` (drop `/work`)
- `CLAUDE.md` — note new CLI flags and template system

---

## Task 1 — Phase B prelude: delete `native_derived_relations`

The new health template will express water-band / step-band as Datalog rules (uses cmp + neg already in upstream rayforce2 as proven by `feature/datalog-aggregates` test commit). The procedural Rust must be removed first to avoid double derivation when the template seeds rules.

**Files:**
- Modify: `src/system_schema.rs:640-698` (function `native_derived_relations`)
- Modify: `src/system_schema.rs:700-708` (`builtin_rule_specs`)
- Modify: `src/system_schema.rs:80-81` (`HEALTH_WATER_BAND`, `HEALTH_STEP_BAND` constants)
- Modify: `src/server.rs:3583-3641` (`known_derived_samples` block)

- [ ] **Step 1: Inventory call sites**

```bash
grep -n "native_derived_relations\|known_derived_samples\|HEALTH_WATER_BAND\|HEALTH_STEP_BAND" src/
```

Expected hits: `system_schema.rs:80,81,640,703,741,816`, `server.rs:3585,3588,3594,3598`.

- [ ] **Step 2: Replace `known_derived_samples` block in `server.rs`**

Open `src/server.rs` lines 3583-3641. Replace the entire block with:

```rust
let known_derived_samples: HashMap<String, Vec<Vec<serde_json::Value>>> = HashMap::new();
```

If `known_derived_samples` is unused after this change, remove the variable and any tuple-binding update at the return site.

- [ ] **Step 3: Delete `native_derived_relations` from `system_schema.rs`**

Delete lines 640-698 (the function body) and remove the call inside `builtin_rule_specs` at 702-706 — the function should now return only `static_builtin_rule_specs(exom)`. Remove `HEALTH_WATER_BAND` / `HEALTH_STEP_BAND` constants at 80-81 and their `*_FACT_ID` siblings if they are no longer referenced. Remove `latest_active_fact` if unused.

- [ ] **Step 4: Delete the obsolete test**

Remove `system_schema.rs:816` test if it tests the deleted function. Re-run:

```bash
cargo build --release --features postgres
```

Address any compile errors by removing dead imports.

- [ ] **Step 5: Re-test**

```bash
cargo test
```

Expected: green. The `health/water-band` etc. predicates will not be derivable until Task 9 ships the new health template, but no test should depend on them right now.

- [ ] **Step 6: Commit**

```bash
git add src/system_schema.rs src/server.rs
git commit -m "refactor(system_schema): remove procedural native_derived_relations"
```

---

## Task 2 — Strip `bootstrap_user_namespace` to a single empty exom

The login handler currently force-creates three exoms with hardcoded data. Replace with a minimal provisioning step that creates only `{email}/main` and leaves seeding to the user's choice on `/welcome`.

**Files:**
- Modify: `src/auth/routes.rs:138-344` (delete bootstrap helpers + rewrite `bootstrap_user_namespace`)

- [ ] **Step 1: Delete obsolete helpers**

Remove the following functions from `src/auth/routes.rs`:
- `health_bootstrap_facts` (152-169)
- `work_main_bootstrap_facts` (171-185)
- `work_example_bootstrap_facts` (187-201)
- `health_bootstrap_rules` (204-225)

Keep `BOOTSTRAP_SENTINEL_*`, `BootstrapFactSpec`, `bootstrap_ctx`, `exom_is_bootstrapped`, and `seed_bootstrap_exom` — they will be reused by the template seeder in Task 7.

- [ ] **Step 2: Rewrite `bootstrap_user_namespace`**

Replace lines 272-344 with:

```rust
async fn provision_user_namespace(
    state: &AppState,
    email: &str,
) -> Result<(), ApiError> {
    if state.auth_store.is_none() {
        return Ok(());
    }
    let Some(tree_root) = state.tree_root.as_ref() else {
        return Ok(());
    };

    let path: crate::path::TreePath = email
        .parse()
        .map_err(|e: crate::path::PathError| ApiError::new("bad_path", e.to_string()))?;
    let main_disk = path
        .join("main")
        .map_err(|e| ApiError::new("bad_path", e.to_string()))?
        .to_disk_path(tree_root);

    let was_missing = crate::tree::classify(&main_disk) == crate::tree::NodeKind::Missing;
    crate::scaffold::init_project(tree_root, &path).map_err(ApiError::from)?;

    if was_missing {
        let _ = state
            .sse_tx
            .send(r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string());
    }
    Ok(())
}
```

- [ ] **Step 3: Update `login` handler**

In the login function (around line 436), change the call:

```rust
- bootstrap_user_namespace(&state, &identity.email, &session_id).await?;
+ provision_user_namespace(&state, &identity.email).await?;
```

- [ ] **Step 4: Build + commit**

```bash
cargo build --release --features postgres
git add src/auth/routes.rs
git commit -m "refactor(auth): provision empty {email}/main; drop hardcoded 3-exom bootstrap"
```

---

## Task 3 — Template schema and TOML deserialization

**Files:**
- Create: `src/onboarding.rs`
- Create: `src/onboarding/template.rs`
- Create: `src/onboarding/seed.rs`
- Modify: `src/lib.rs` (add `pub mod onboarding;`)
- Modify: `Cargo.toml` (add `toml = "0.8"`, `include_dir = "0.7"` if missing)

- [ ] **Step 1: Add dependencies**

```bash
cargo add toml
cargo add include_dir --features glob
```

- [ ] **Step 2: Write the failing test for template parsing**

Create `src/onboarding/template.rs` with:

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OnboardingTemplate {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    pub version: String,
    pub exom_suffix: String,
    #[serde(default)]
    pub facts: Vec<TemplateFact>,
    #[serde(default)]
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TemplateFact {
    pub fact_id: String,
    pub predicate: String,
    pub value: String,
}

impl OnboardingTemplate {
    pub fn from_toml(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).map_err(|e| anyhow::anyhow!("template parse: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_template() {
        let toml = r#"
id = "empty"
label = "Empty workspace"
description = "Start with a blank exom."
version = "v1"
exom_suffix = "main"
"#;
        let t = OnboardingTemplate::from_toml(toml).unwrap();
        assert_eq!(t.id, "empty");
        assert_eq!(t.exom_suffix, "main");
        assert!(t.facts.is_empty());
        assert!(t.rules.is_empty());
    }

    #[test]
    fn parses_template_with_facts_and_rules() {
        let toml = r#"
id = "health"
label = "Personal health demo"
description = "Wellness facts and derived bands."
version = "v1"
exom_suffix = "personal/health/main"
icon = "heart"

[[facts]]
fact_id = "health/profile/age"
predicate = "profile/age"
value = "30"

[[facts]]
fact_id = "health/profile/height_cm"
predicate = "profile/height_cm"
value = "175"

rules = [
  "(rule {exom} (health/water-band \"small\") (?w_id 'profile/weight_kg ?w) (< ?w 60))",
]
"#;
        let t = OnboardingTemplate::from_toml(toml).unwrap();
        assert_eq!(t.id, "health");
        assert_eq!(t.facts.len(), 2);
        assert_eq!(t.facts[0].predicate, "profile/age");
        assert_eq!(t.rules.len(), 1);
    }
}
```

- [ ] **Step 3: Create catalog module shell**

Create `src/onboarding.rs`:

```rust
pub mod seed;
pub mod template;

pub use template::{OnboardingTemplate, TemplateFact};

use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OnboardingCatalog {
    pub templates: Vec<OnboardingTemplate>,
}

impl OnboardingCatalog {
    pub fn empty() -> Self {
        Self { templates: Vec::new() }
    }

    /// Always-present pseudo-template that seeds nothing. Returned in addition
    /// to any whitelisted templates so the user always has a "Skip" option
    /// implemented as picking the empty template.
    pub fn empty_template() -> OnboardingTemplate {
        OnboardingTemplate {
            id: "empty".to_string(),
            label: "Empty workspace".to_string(),
            description: "Start with a blank exom you can build up yourself.".to_string(),
            icon: Some("square".to_string()),
            version: "v1".to_string(),
            exom_suffix: "main".to_string(),
            facts: Vec::new(),
            rules: Vec::new(),
        }
    }

    pub fn from_builtins(filter: Option<&[String]>) -> Result<Self> {
        let mut templates = vec![Self::empty_template()];
        for raw in BUILTIN_TEMPLATE_SOURCES {
            let t = OnboardingTemplate::from_toml(raw)?;
            if let Some(allow) = filter {
                if !allow.iter().any(|id| id == &t.id) {
                    continue;
                }
            }
            if t.id == "empty" {
                continue; // already inserted above
            }
            templates.push(t);
        }
        Ok(Self { templates })
    }

    pub fn from_directory(dir: &Path, filter: Option<&[String]>) -> Result<Self> {
        let mut templates = vec![Self::empty_template()];
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            let t = OnboardingTemplate::from_toml(&text)?;
            if let Some(allow) = filter {
                if !allow.iter().any(|id| id == &t.id) {
                    continue;
                }
            }
            if t.id == "empty" {
                continue;
            }
            templates.push(t);
        }
        Ok(Self { templates })
    }

    pub fn find(&self, id: &str) -> Option<&OnboardingTemplate> {
        self.templates.iter().find(|t| t.id == id)
    }
}

const BUILTIN_TEMPLATE_SOURCES: &[&str] = &[
    include_str!("../assets/onboarding/health.toml"),
    include_str!("../assets/onboarding/workspace.toml"),
    include_str!("../assets/onboarding/project.toml"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_template_always_present_in_builtin_catalog() {
        let cat = OnboardingCatalog::from_builtins(None).unwrap();
        assert!(cat.find("empty").is_some());
    }

    #[test]
    fn filter_drops_unlisted_templates() {
        let cat = OnboardingCatalog::from_builtins(Some(&["workspace".to_string()])).unwrap();
        assert!(cat.find("workspace").is_some());
        assert!(cat.find("health").is_none());
        assert!(cat.find("empty").is_some()); // always present
    }
}
```

- [ ] **Step 4: Stub the seed module**

Create `src/onboarding/seed.rs`:

```rust
use anyhow::Result;
use crate::context::MutationContext;
use crate::onboarding::OnboardingTemplate;
use crate::server::AppState;

pub async fn seed_template(
    _state: &AppState,
    _email: &str,
    _ctx: &MutationContext,
    _template: &OnboardingTemplate,
) -> Result<String> {
    todo!("implemented in Task 7")
}
```

- [ ] **Step 5: Wire module into `lib.rs`**

In `src/lib.rs` add (alphabetical with other modules):

```rust
pub mod onboarding;
```

- [ ] **Step 6: Build + run unit tests**

```bash
cargo build --release --features postgres
cargo test onboarding
```

Expected: green. The template-parsing tests pass (they use inline TOML). The catalog tests still fail because no built-in TOML files exist yet — that is fixed in Task 4.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/onboarding.rs src/onboarding/template.rs src/onboarding/seed.rs Cargo.toml Cargo.lock
git commit -m "feat(onboarding): template schema, catalog, and stub seeder"
```

---

## Task 4 — Built-in template TOMLs

**Files:**
- Create: `assets/onboarding/health.toml`
- Create: `assets/onboarding/workspace.toml`
- Create: `assets/onboarding/project.toml`

- [ ] **Step 1: Create assets directory**

```bash
mkdir -p assets/onboarding
```

- [ ] **Step 2: Write `assets/onboarding/health.toml`**

```toml
id = "health"
label = "Personal health"
description = "Profile (age, height, weight) plus declarative water and step bands."
icon = "heart"
version = "v1"
exom_suffix = "personal/health/main"

[[facts]]
fact_id = "health/profile/age"
predicate = "profile/age"
value = "30"

[[facts]]
fact_id = "health/profile/height_cm"
predicate = "profile/height_cm"
value = "175"

[[facts]]
fact_id = "health/profile/weight_kg"
predicate = "profile/weight_kg"
value = "75"

[[facts]]
fact_id = "health/profile/units"
predicate = "profile/units"
value = "metric"

[[facts]]
fact_id = "health/onboarding/disclaimer"
predicate = "onboarding/disclaimer"
value = "general_wellness_example_not_medical_advice"

rules = [
  "(rule {exom} (health/water-band \"small\") (?w_id 'health/profile/weight_kg ?w) (?h_id 'health/profile/height_cm ?h) (< ?w 60) (< ?h 170))",
  "(rule {exom} (health/water-band \"large\") (?w_id 'health/profile/weight_kg ?w) (?h_id 'health/profile/height_cm ?h) (>= ?w 85))",
  "(rule {exom} (health/water-band \"large\") (?w_id 'health/profile/weight_kg ?w) (?h_id 'health/profile/height_cm ?h) (>= ?h 185))",
  "(rule {exom} (health/water-band \"medium\") (?w_id 'health/profile/weight_kg ?w) (?h_id 'health/profile/height_cm ?h) (not (health/water-band \"small\")) (not (health/water-band \"large\")))",
  "(rule {exom} (health/step-band \"high\") (?id 'health/profile/age ?a) (< ?a 30))",
  "(rule {exom} (health/step-band \"medium\") (?id 'health/profile/age ?a) (>= ?a 30) (< ?a 50))",
  "(rule {exom} (health/step-band \"gentle\") (?id 'health/profile/age ?a) (>= ?a 50))",
  "(rule {exom} (health/recommended-water-ml \"2000\") (health/water-band \"small\"))",
  "(rule {exom} (health/recommended-water-ml \"2500\") (health/water-band \"medium\"))",
  "(rule {exom} (health/recommended-water-ml \"3000\") (health/water-band \"large\"))",
  "(rule {exom} (health/recommended-steps-per-day \"10000\") (health/step-band \"high\"))",
  "(rule {exom} (health/recommended-steps-per-day \"9000\") (health/step-band \"medium\"))",
  "(rule {exom} (health/recommended-steps-per-day \"7500\") (health/step-band \"gentle\"))",
]
```

- [ ] **Step 3: Write `assets/onboarding/workspace.toml`**

```toml
id = "workspace"
label = "Personal workspace"
description = "An empty workspace exom for general work and notes."
icon = "briefcase"
version = "v1"
exom_suffix = "work/main"

[[facts]]
fact_id = "workspace/purpose"
predicate = "workspace/purpose"
value = "personal work area"

[[facts]]
fact_id = "workspace/next_step"
predicate = "workspace/next_step"
value = "create projects, facts, or sessions here"

rules = []
```

- [ ] **Step 4: Write `assets/onboarding/project.toml`**

```toml
id = "project"
label = "Example project"
description = "A starter project exom showing how to track status and next steps."
icon = "folder"
version = "v1"
exom_suffix = "work/example/main"

[[facts]]
fact_id = "project/name"
predicate = "project/name"
value = "Example Project"

[[facts]]
fact_id = "project/status"
predicate = "project/status"
value = "active"

[[facts]]
fact_id = "project/next_step"
predicate = "project/next_step"
value = "inspect facts, graph, and sessions"

rules = []
```

- [ ] **Step 5: Re-run catalog tests**

```bash
cargo test onboarding::tests
```

Expected: green — catalog now loads three built-ins plus `empty`.

- [ ] **Step 6: Commit**

```bash
git add assets/onboarding/
git commit -m "feat(onboarding): built-in templates (health, workspace, project)"
```

---

## Task 5 — CLI flags `--onboarding-templates` and `--onboarding-template-dir`

**Files:**
- Modify: `src/main.rs` (the `Serve` and `Daemon` subcommand structs around lines 320-402)

- [ ] **Step 1: Locate both subcommands**

```bash
grep -n "Serve {\|Daemon {" src/main.rs | head
```

- [ ] **Step 2: Add fields to both `Serve` and `Daemon`**

For each subcommand struct, after `database_url`, add:

```rust
        /// Comma-separated list of onboarding template IDs to expose to new users.
        /// Built-in IDs: empty, health, workspace, project. Empty/unset = all built-ins.
        /// "empty" is always available regardless of this filter.
        #[arg(long, value_delimiter = ',')]
        onboarding_templates: Option<Vec<String>>,

        /// Override built-in onboarding templates with TOML files from this directory.
        /// When set, built-ins are NOT loaded; only files matching `*.toml` in this
        /// directory are read. The "empty" pseudo-template is still always available.
        #[arg(long)]
        onboarding_template_dir: Option<PathBuf>,
```

- [ ] **Step 3: Capture flags inside the handler**

In the handler block for `Serve` (around line 1605-1620, where `auth_provider` etc. are wired), add immediately after the auth-store wiring:

```rust
let catalog = match &onboarding_template_dir {
    Some(dir) => match ray_exomem::onboarding::OnboardingCatalog::from_directory(
        dir,
        onboarding_templates.as_deref(),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error loading onboarding templates from {}: {}", dir.display(), e);
            std::process::exit(1);
        }
    },
    None => match ray_exomem::onboarding::OnboardingCatalog::from_builtins(
        onboarding_templates.as_deref(),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error loading built-in onboarding templates: {}", e);
            std::process::exit(1);
        }
    },
};
let s = std::sync::Arc::get_mut(&mut state)
    .expect("state Arc refcount should be 1 at init");
s.onboarding_catalog = Some(std::sync::Arc::new(catalog));
```

Repeat the equivalent block in the `Daemon` handler.

- [ ] **Step 4: Add the field to `AppState`**

In `src/server.rs`, inside `pub struct AppState` (around line 67), add:

```rust
pub onboarding_catalog: Option<Arc<crate::onboarding::OnboardingCatalog>>,
```

In every `AppState` constructor (`new`, `from_data_dir`), default it to `None`.

- [ ] **Step 5: Build + commit**

```bash
cargo build --release --features postgres
git add src/main.rs src/server.rs
git commit -m "feat(cli): --onboarding-templates and --onboarding-template-dir flags"
```

---

## Task 6 — Persist onboarding state on `StoredUser`

`StoredUser` (both JSONL and Postgres backends) gets two new fields. JSONL replay preserves them across reads, mirroring how `active` and `last_login` are handled at `auth/store.rs:561-595`.

**Files:**
- Modify: `src/auth/store.rs` (`StoredUser`, replay, `record_user`)
- Modify: `src/db/mod.rs` (`StoredUser` mirror, AuthDb trait additions)
- Modify: `src/db/pg_auth.rs` (Postgres impl + migration)
- Create: `migrations/202604180001_add_user_onboarding_state.sql` (Postgres migration)

- [ ] **Step 1: Extend `StoredUser` (JSONL backend)**

In `src/auth/store.rs:64-71`, change to:

```rust
#[derive(Debug, Clone)]
pub struct StoredUser {
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub created_at: String,
    pub active: bool,
    pub last_login: Option<String>,
    pub onboarded: bool,
    pub seeded_templates: Vec<String>,
}
```

- [ ] **Step 2: Extend the Postgres mirror**

In `src/db/mod.rs`, locate `pub struct StoredUser` and add the same two fields. In `src/db/pg_auth.rs`, update the `upsert_user` SQL and `get_user` SQL to round-trip the new columns.

- [ ] **Step 3: Write Postgres migration**

Create `migrations/202604180001_add_user_onboarding_state.sql`:

```sql
ALTER TABLE auth_users
    ADD COLUMN IF NOT EXISTS onboarded BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS seeded_templates JSONB NOT NULL DEFAULT '[]'::jsonb;
```

(If migrations are run automatically by `sqlx::migrate!()` they pick this up at startup; otherwise add a manual step in CLAUDE.md per Task 12.)

- [ ] **Step 4: Update JSONL replay (preserve fields)**

In `src/auth/store.rs:555-594`, locate the `"user"` arm of the JSONL replay match. Extend the `StoredUser` construction to read both new fields, with the same `or_else` fallback pattern used by `active` and `last_login`:

```rust
let onboarded = entry
    .get("onboarded")
    .and_then(|v| v.as_bool())
    .or_else(|| {
        self.users
            .lock()
            .unwrap()
            .get(email)
            .map(|user| user.onboarded)
    })
    .unwrap_or(false);
let seeded_templates = entry
    .get("seeded_templates")
    .and_then(|v| v.as_array())
    .map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect::<Vec<_>>()
    })
    .or_else(|| {
        self.users
            .lock()
            .unwrap()
            .get(email)
            .map(|user| user.seeded_templates.clone())
    })
    .unwrap_or_default();
```

Then include both in the `StoredUser { ... }` literal.

- [ ] **Step 5: Update `record_user` to preserve fields**

In `src/auth/store.rs:792-838`, update both the Postgres branch and the JSONL branch to preserve `onboarded` and `seeded_templates` from the previous record. For the JSONL branch:

```rust
let entry = serde_json::json!({
    "kind": "user",
    "email": email,
    "display_name": display_name,
    "provider": provider,
    "created_at": existing
        .as_ref()
        .map(|user| user.created_at.clone())
        .unwrap_or_else(|| now.clone()),
    "active": existing.as_ref().map(|user| user.active).unwrap_or(true),
    "last_login": now,
    "onboarded": existing.as_ref().map(|user| user.onboarded).unwrap_or(false),
    "seeded_templates": existing
        .as_ref()
        .map(|user| user.seeded_templates.clone())
        .unwrap_or_default(),
});
```

- [ ] **Step 6: Add `record_onboarding_seed` and `record_onboarding_skip`**

In `src/auth/store.rs`, add two new methods on `AuthStore`:

```rust
pub async fn record_onboarding_seed(&self, email: &str, template_id: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut existing = self.users.lock().unwrap().get(email).cloned();
    if let Some(ref mut u) = existing {
        u.onboarded = true;
        if !u.seeded_templates.iter().any(|t| t == template_id) {
            u.seeded_templates.push(template_id.to_string());
        }
    }
    let templates = existing
        .as_ref()
        .map(|u| u.seeded_templates.clone())
        .unwrap_or_else(|| vec![template_id.to_string()]);

    if let Some(ref db) = self.auth_db {
        let row = db.get_user(email).await?;
        if let Some(mut row) = row {
            row.onboarded = true;
            row.seeded_templates = templates.clone();
            row.last_login = Some(now);
            db.upsert_user(&row).await?;
        }
        return Ok(());
    }

    let entry = serde_json::json!({
        "kind": "user",
        "email": email,
        "display_name": existing.as_ref().map(|u| u.display_name.clone()).unwrap_or_else(|| email.to_string()),
        "provider": existing.as_ref().map(|u| u.provider.clone()).unwrap_or_else(|| "unknown".to_string()),
        "created_at": existing.as_ref().map(|u| u.created_at.clone()).unwrap_or_else(|| now.clone()),
        "active": existing.as_ref().map(|u| u.active).unwrap_or(true),
        "last_login": now,
        "onboarded": true,
        "seeded_templates": templates,
    });
    self.append_entry(&entry)?;
    self.apply_entry(&entry);
    Ok(())
}

pub async fn record_onboarding_skip(&self, email: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let existing = self.users.lock().unwrap().get(email).cloned();

    if let Some(ref db) = self.auth_db {
        if let Some(mut row) = db.get_user(email).await? {
            row.onboarded = true;
            row.last_login = Some(now);
            db.upsert_user(&row).await?;
        }
        return Ok(());
    }

    let entry = serde_json::json!({
        "kind": "user",
        "email": email,
        "display_name": existing.as_ref().map(|u| u.display_name.clone()).unwrap_or_else(|| email.to_string()),
        "provider": existing.as_ref().map(|u| u.provider.clone()).unwrap_or_else(|| "unknown".to_string()),
        "created_at": existing.as_ref().map(|u| u.created_at.clone()).unwrap_or_else(|| now.clone()),
        "active": existing.as_ref().map(|u| u.active).unwrap_or(true),
        "last_login": now,
        "onboarded": true,
        "seeded_templates": existing.as_ref().map(|u| u.seeded_templates.clone()).unwrap_or_default(),
    });
    self.append_entry(&entry)?;
    self.apply_entry(&entry);
    Ok(())
}
```

- [ ] **Step 7: Tests for replay preservation**

Add to the `tests` module in `src/auth/store.rs`:

```rust
#[tokio::test]
async fn replay_preserves_onboarding_fields() {
    let dir = tempfile::tempdir().unwrap();
    let store = AuthStore::bootstrap(dir.path(), &[]).await.unwrap();
    store.record_user("a@x.com", "A", "google").await;
    store.record_onboarding_seed("a@x.com", "health").await.unwrap();

    // Reopen to force JSONL replay.
    let store2 = AuthStore::bootstrap(dir.path(), &[]).await.unwrap();
    let u = store2.users.lock().unwrap().get("a@x.com").cloned().unwrap();
    assert!(u.onboarded);
    assert_eq!(u.seeded_templates, vec!["health".to_string()]);

    // A subsequent ordinary login must NOT reset onboarded.
    store2.record_user("a@x.com", "A", "google").await;
    let u = store2.users.lock().unwrap().get("a@x.com").cloned().unwrap();
    assert!(u.onboarded);
    assert_eq!(u.seeded_templates, vec!["health".to_string()]);
}
```

This regression matches the gotcha already documented in CLAUDE.md ("JSONL auth replay must preserve `user.active` / `last_login` on repeated `user` entries.").

- [ ] **Step 8: Build, test, commit**

```bash
cargo build --release --features postgres
cargo test auth::store
git add src/auth/store.rs src/db/ migrations/
git commit -m "feat(auth): persist onboarded + seeded_templates on StoredUser"
```

---

## Task 7 — Implement `seed_template`

`seed_template` writes the template's facts and rules into the seeded exom and stamps a sentinel `(onboarding/template_id "<id>")` and `(onboarding/template_version "<version>")` so future migrations can detect them.

**Files:**
- Modify: `src/onboarding/seed.rs`

- [ ] **Step 1: Replace the stub**

```rust
use anyhow::{Context, Result};

use crate::context::MutationContext;
use crate::http_error::ApiError;
use crate::onboarding::OnboardingTemplate;
use crate::server::AppState;

const TEMPLATE_ID_PREDICATE: &str = "onboarding/template_id";
const TEMPLATE_VERSION_PREDICATE: &str = "onboarding/template_version";

pub async fn seed_template(
    state: &AppState,
    email: &str,
    ctx: &MutationContext,
    template: &OnboardingTemplate,
) -> Result<String> {
    let Some(tree_root) = state.tree_root.as_ref() else {
        anyhow::bail!("seed_template requires persistent storage");
    };

    let suffix_path: crate::path::TreePath =
        format!("{email}/{}", template.exom_suffix).parse().context("template exom_suffix")?;
    let parent = suffix_path.parent().unwrap_or_else(crate::path::TreePath::root);
    let main_disk = suffix_path.to_disk_path(tree_root);
    crate::scaffold::init_project(tree_root, &parent).context("scaffold parent")?;
    let exom_str = suffix_path.to_string();

    crate::server::mutate_exom_async(state, &exom_str, |es| {
        // Idempotent: if the same template_id is already present, do nothing.
        let already = es.brain.current_facts().iter().any(|f| {
            f.predicate == TEMPLATE_ID_PREDICATE && f.value == template.id
        });
        if already {
            return Ok(());
        }

        for f in &template.facts {
            es.brain.assert_fact(
                &f.fact_id,
                &f.predicate,
                &f.value,
                1.0,
                "onboarding",
                None,
                None,
                ctx,
            )?;
        }

        // Sentinel facts.
        es.brain.assert_fact(
            &format!("onboarding/{}", TEMPLATE_ID_PREDICATE),
            TEMPLATE_ID_PREDICATE,
            &template.id,
            1.0,
            "onboarding",
            None,
            None,
            ctx,
        )?;
        es.brain.assert_fact(
            &format!("onboarding/{}", TEMPLATE_VERSION_PREDICATE),
            TEMPLATE_VERSION_PREDICATE,
            &template.version,
            1.0,
            "onboarding",
            None,
            None,
            ctx,
        )?;

        for raw in &template.rules {
            let rendered = raw.replace("{exom}", &exom_str);
            es.rules.push(crate::rules::parse_rule_line(
                &rendered,
                ctx.clone(),
                crate::brain::now_iso(),
            )?);
        }
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("seed_template: {e}"))?;

    let _ = state
        .sse_tx
        .send(r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string());
    let _ = main_disk; // touched to verify scaffold; unused otherwise.

    Ok(exom_str)
}
```

- [ ] **Step 2: Unit test the seeder against a tempdir state**

Add `#[cfg(test)] mod tests { ... }` exercising `seed_template` end-to-end with the `empty` template (must succeed and stamp the sentinel) and the `health` template (must produce a queryable `health/water-band` derivation through the engine for the default profile).

- [ ] **Step 3: Build + test**

```bash
cargo build --release --features postgres
cargo test onboarding::seed
```

- [ ] **Step 4: Commit**

```bash
git add src/onboarding/seed.rs
git commit -m "feat(onboarding): seed_template applies facts + rules to target exom"
```

---

## Task 8 — `/api/onboarding/*` HTTP endpoints

**Files:**
- Modify: `src/auth/routes.rs` (extend `auth_router` and add three handlers)

- [ ] **Step 1: Extend the router**

In `src/auth/routes.rs:24-36`, extend `auth_router`:

```rust
pub fn auth_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/info", get(auth_info))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/session", get(session))
        .route("/me", get(me))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{key_id}", delete(revoke_api_key))
        .route("/shares", get(list_shares).post(create_share))
        .route("/shares/{share_id}", delete(revoke_share))
        .route("/shared-with-me", get(shared_with_me))
        .route("/onboarding/catalog", get(onboarding_catalog))
        .route("/onboarding/seed", post(onboarding_seed))
        .route("/onboarding/skip", post(onboarding_skip))
}
```

- [ ] **Step 2: Implement `onboarding_catalog`**

```rust
#[derive(Serialize)]
struct OnboardingCatalogResponse {
    templates: Vec<OnboardingCatalogEntry>,
}

#[derive(Serialize)]
struct OnboardingCatalogEntry {
    id: String,
    label: String,
    description: String,
    icon: Option<String>,
    exom_suffix: String,
    version: String,
}

async fn onboarding_catalog(
    State(state): State<Arc<AppState>>,
    _user: User,
) -> Result<impl IntoResponse, ApiError> {
    let cat = state
        .onboarding_catalog
        .as_ref()
        .ok_or_else(|| ApiError::new("onboarding_disabled", "onboarding not configured"))?;
    let templates = cat
        .templates
        .iter()
        .map(|t| OnboardingCatalogEntry {
            id: t.id.clone(),
            label: t.label.clone(),
            description: t.description.clone(),
            icon: t.icon.clone(),
            exom_suffix: t.exom_suffix.clone(),
            version: t.version.clone(),
        })
        .collect();
    Ok(Json(OnboardingCatalogResponse { templates }))
}
```

- [ ] **Step 3: Implement `onboarding_seed`**

```rust
#[derive(Deserialize)]
struct OnboardingSeedRequest {
    template_id: String,
}

#[derive(Serialize)]
struct OnboardingSeedResponse {
    seeded_exom: String,
    template_id: String,
}

async fn onboarding_seed(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(body): Json<OnboardingSeedRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    let cat = state
        .onboarding_catalog
        .as_ref()
        .ok_or_else(|| ApiError::new("onboarding_disabled", "onboarding not configured"))?;
    let template = cat
        .find(&body.template_id)
        .ok_or_else(|| ApiError::new("unknown_template", format!("template {}", body.template_id)))?
        .clone();

    let session_id = user.session_id.clone().unwrap_or_default();
    let ctx = bootstrap_ctx(&user.email, &session_id);

    let seeded_exom = if template.id == "empty" {
        // Empty template only marks the user onboarded; no seeding needed beyond the
        // already-provisioned {email}/main exom.
        format!("{}/main", user.email)
    } else {
        crate::onboarding::seed::seed_template(&state, &user.email, &ctx, &template)
            .await
            .map_err(|e| ApiError::new("seed_failed", e.to_string()).with_status(500))?
    };

    store
        .record_onboarding_seed(&user.email, &template.id)
        .await
        .map_err(|e| ApiError::new("record_failed", e.to_string()).with_status(500))?;

    Ok(Json(OnboardingSeedResponse {
        seeded_exom,
        template_id: template.id,
    }))
}
```

- [ ] **Step 4: Implement `onboarding_skip`**

```rust
async fn onboarding_skip(
    State(state): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_auth_store(&state)?;
    store
        .record_onboarding_skip(&user.email)
        .await
        .map_err(|e| ApiError::new("record_failed", e.to_string()).with_status(500))?;
    Ok(Json(serde_json::json!({ "skipped": true })))
}
```

- [ ] **Step 5: Surface `onboarded` in `/auth/session` and `/auth/me`**

In `AuthUserResponse` (around line 56), add:

```rust
#[serde(default)]
onboarded: bool,
```

In `auth_user_response`, set:

```rust
onboarded: store_lookup_onboarded(state, &user.email).await,
```

(Add a small helper that reads `store.users` lock or calls the AuthDb to fetch the latest persisted state.)

- [ ] **Step 6: Build + smoke test endpoints**

```bash
cargo build --release --features postgres
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
ray-exomem stop
set -a; source .env; set +a
ray-exomem serve --bind 127.0.0.1:9780 \
  --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL" &
SERVE_PID=$!
sleep 3
# Log in via the UI in a browser at https://devmem.trydev.app/ray-exomem/, copy session cookie, then:
COOKIE='<paste>'
curl -s -H "Cookie: $COOKIE" http://127.0.0.1:9780/ray-exomem/auth/onboarding/catalog | jq
curl -s -H "Cookie: $COOKIE" -H 'content-type: application/json' \
  -d '{"template_id":"workspace"}' \
  http://127.0.0.1:9780/ray-exomem/auth/onboarding/seed | jq
kill $SERVE_PID
```

Expected: catalog returns the configured templates; seed returns `{ seeded_exom: "<email>/work/main", template_id: "workspace" }` and the user record now has `onboarded: true` and `seeded_templates: ["workspace"]`.

- [ ] **Step 7: Commit**

```bash
git add src/auth/routes.rs
git commit -m "feat(api): GET /onboarding/catalog, POST /onboarding/{seed,skip}"
```

---

## Task 9 — UI client + `/welcome` route

**Files:**
- Create: `ui/src/lib/onboarding.svelte.ts`
- Create: `ui/src/routes/welcome/+page.svelte`
- Create: `ui/src/routes/welcome/+page.ts`
- Modify: `ui/src/lib/auth.svelte.ts`
- Modify: `ui/src/routes/+layout.svelte`
- Modify: `ui/src/lib/stores.svelte.ts`

- [ ] **Step 1: Extend `AuthUser` interface**

In `ui/src/lib/auth.svelte.ts`, add to `AuthUser`:

```ts
onboarded?: boolean;
```

- [ ] **Step 2: Drop the `/work` default**

In `ui/src/lib/stores.svelte.ts`, change:

```ts
defaultExomForUser(email: string): string {
    return `${email}/main`;
}
```

- [ ] **Step 3: Create the onboarding client**

`ui/src/lib/onboarding.svelte.ts`:

```ts
import { getExomemBaseUrl } from '$lib/exomem-base';

function authBase(): string {
    return getExomemBaseUrl().replace('/ray-exomem', '');
}

export interface OnboardingTemplate {
    id: string;
    label: string;
    description: string;
    icon?: string | null;
    exom_suffix: string;
    version: string;
}

export interface OnboardingCatalog {
    templates: OnboardingTemplate[];
}

export async function fetchOnboardingCatalog(): Promise<OnboardingCatalog> {
    const r = await fetch(`${authBase()}/auth/onboarding/catalog`, {
        credentials: 'include'
    });
    if (!r.ok) throw new Error(`onboarding catalog: ${r.status}`);
    return (await r.json()) as OnboardingCatalog;
}

export async function seedOnboarding(template_id: string): Promise<{ seeded_exom: string }> {
    const r = await fetch(`${authBase()}/auth/onboarding/seed`, {
        method: 'POST',
        credentials: 'include',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ template_id })
    });
    if (!r.ok) throw new Error(`onboarding seed: ${r.status}`);
    return (await r.json()) as { seeded_exom: string };
}

export async function skipOnboarding(): Promise<void> {
    const r = await fetch(`${authBase()}/auth/onboarding/skip`, {
        method: 'POST',
        credentials: 'include'
    });
    if (!r.ok) throw new Error(`onboarding skip: ${r.status}`);
}
```

- [ ] **Step 4: Create welcome page load function**

`ui/src/routes/welcome/+page.ts`:

```ts
import { fetchOnboardingCatalog } from '$lib/onboarding.svelte';

export const ssr = false;

export async function load() {
    const cat = await fetchOnboardingCatalog();
    return { catalog: cat };
}
```

- [ ] **Step 5: Create welcome page**

`ui/src/routes/welcome/+page.svelte`:

```svelte
<script lang="ts">
    import { goto } from '$app/navigation';
    import { base } from '$app/paths';
    import { auth } from '$lib/auth.svelte';
    import { app } from '$lib/stores.svelte';
    import { seedOnboarding, skipOnboarding, type OnboardingTemplate } from '$lib/onboarding.svelte';
    import { Button } from '$lib/components/ui/button/index.js';

    let { data } = $props();
    let busy = $state<string | null>(null);
    let error = $state<string | null>(null);

    async function pick(t: OnboardingTemplate) {
        busy = t.id;
        error = null;
        try {
            if (t.id === 'empty') {
                await skipOnboarding();
                if (auth.user) auth.user.onboarded = true;
                app.switchExom(`${auth.user!.email}/main`);
            } else {
                const { seeded_exom } = await seedOnboarding(t.id);
                if (auth.user) auth.user.onboarded = true;
                app.switchExom(seeded_exom);
            }
            await goto(`${base}/`);
        } catch (e) {
            error = (e as Error).message;
        } finally {
            busy = null;
        }
    }
</script>

<div class="mx-auto flex min-h-screen max-w-3xl flex-col justify-center gap-8 px-6 py-16">
    <header class="space-y-2">
        <h1 class="text-3xl font-semibold text-zinc-100">Welcome to ray-exomem</h1>
        <p class="text-zinc-400">Pick a starting template. You can always create more exoms later.</p>
    </header>

    {#if error}
        <p class="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-300">{error}</p>
    {/if}

    <ul class="grid gap-3 md:grid-cols-2">
        {#each data.catalog.templates as t (t.id)}
            <li class="flex flex-col gap-3 rounded-lg border border-zinc-700 bg-zinc-900 p-5">
                <h2 class="text-lg font-medium text-zinc-100">{t.label}</h2>
                <p class="flex-1 text-sm text-zinc-400">{t.description}</p>
                <div class="flex items-center justify-between">
                    <code class="text-xs text-zinc-500">{t.exom_suffix}</code>
                    <Button onclick={() => pick(t)} disabled={busy !== null}>
                        {busy === t.id ? 'Setting up...' : 'Choose'}
                    </Button>
                </div>
            </li>
        {/each}
    </ul>
</div>
```

- [ ] **Step 6: Wire redirect into `+layout.svelte`**

In `ui/src/routes/+layout.svelte:60-91`, locate the `onMount` and `$effect` blocks. Extend the authenticated branch to redirect to `/welcome` when `!user.onboarded`:

```svelte
const isWelcomeRoute = $derived(page.url.pathname.startsWith(`${base}/welcome`));

// inside onMount:
if (auth.isAuthenticated && !isLoginRoute) {
    if (!auth.user?.onboarded && !isWelcomeRoute) {
        void goto(`${base}/welcome`, { replaceState: true });
        return;
    }
    app.ensureAuthenticatedDefaultExom(auth.user?.email ?? null);
    startApp();
}

// equivalent guard inside the $effect block.
```

- [ ] **Step 7: Build the UI**

```bash
cd ui && npm run check && npm run build
```

Expected: green. The Rust build will pick up the new `ui/build/` artifacts on the next `cargo build --release`.

- [ ] **Step 8: Commit**

```bash
git add ui/src/lib/onboarding.svelte.ts ui/src/routes/welcome/ \
        ui/src/lib/auth.svelte.ts ui/src/routes/+layout.svelte ui/src/lib/stores.svelte.ts
git commit -m "feat(ui): /welcome route, onboarding client, post-login redirect"
```

---

## Task 10 — End-to-end daemon verification

- [ ] **Step 1: Rebuild + redeploy binary**

```bash
cargo build --release --features postgres
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
```

- [ ] **Step 2: Drop and recreate the Postgres database**

```bash
docker exec -u postgres ddd-postgres-1 psql -c "DROP DATABASE IF EXISTS ray_exomem"
docker exec -u postgres ddd-postgres-1 psql -c "CREATE DATABASE ray_exomem OWNER ray_exomem"
```

(Greenfield bias per `CLAUDE.md`: replay rather than migrate.)

- [ ] **Step 3: Restart daemon**

```bash
ray-exomem stop
set -a; source .env; set +a
ray-exomem serve --bind 127.0.0.1:9780 \
  --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL" &
sleep 3
```

- [ ] **Step 4: Manual UI walkthrough**

Open `https://devmem.trydev.app/ray-exomem/` and confirm:

1. Log in as a fresh user (or revoked-onboarded user).
2. Page redirects to `/welcome`.
3. Catalog shows: Empty workspace, Personal health, Personal workspace, Example project.
4. Pick **Personal health** → page redirects to selected exom `<email>/personal/health/main`.
5. Tree drawer shows the seeded path; query view returns `(query (?b) (health/water-band ?b))` → `["medium"]`; `(query (?ml) (health/recommended-water-ml ?ml))` → `["2500"]`.
6. Reload page → no second `/welcome` prompt (user already onboarded).
7. Log out, log back in → still no `/welcome` prompt.

- [ ] **Step 5: Filter walkthrough**

Restart with `--onboarding-templates workspace,project`. Log in with a different fresh user. The catalog must show only `Empty workspace`, `Personal workspace`, `Example project` — no `Personal health` card.

- [ ] **Step 6: External directory walkthrough**

```bash
mkdir -p /tmp/ray-onboarding && cp assets/onboarding/workspace.toml /tmp/ray-onboarding/
ray-exomem stop
ray-exomem serve --bind 127.0.0.1:9780 \
  --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL" \
  --onboarding-template-dir /tmp/ray-onboarding &
sleep 3
```

Catalog must return only `empty` + `workspace`.

- [ ] **Step 7: Idempotent seeding**

Re-issue the `POST /auth/onboarding/seed` for the same template — server must return success without doubling facts. Verify via `(query (?fid) (?fid 'onboarding/template_id ?id))` returning a single row.

- [ ] **Step 8: Commit any verification fixes**

```bash
git add -p
git commit -m "fix(onboarding): verification feedback"
```

---

## Task 11 — Tests

- [ ] **Step 1: Catalog filter + external dir**

In `src/onboarding.rs::tests`, add tests for:
- `from_directory` ignoring non-`.toml` files
- `from_directory` filter behavior matching `from_builtins` filter behavior
- malformed TOML produces a clear error

- [ ] **Step 2: Endpoint tests**

Add an axum integration test (or extend an existing `auth/routes` test harness) covering:
- `GET /auth/onboarding/catalog` requires authentication
- `POST /auth/onboarding/seed` with unknown `template_id` returns 404-ish error
- Seeding is idempotent

- [ ] **Step 3: UI smoke**

```bash
cd ui && npm run test 2>/dev/null || npm run check
```

If a `vitest` setup exists, add a unit test for the `pick` function in `welcome/+page.svelte` mocking `seedOnboarding` and `skipOnboarding`.

- [ ] **Step 4: Commit**

```bash
git add src/onboarding.rs src/auth/routes.rs ui/
git commit -m "test(onboarding): catalog filter, endpoint contracts, ui smoke"
```

---

## Task 12 — Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Document new flags**

Insert under "Local dev with Cloudflare tunnel → Daemon" section, after the existing `serve` invocation example:

```markdown
- **Onboarding templates** (optional): pass `--onboarding-templates health,workspace,project` to filter built-in templates surfaced to new users on `/welcome`. Pass `--onboarding-template-dir /path/to/dir` to replace built-ins entirely with TOML files from the directory. The `empty` pseudo-template is always shown.
```

- [ ] **Step 2: Add new gotcha**

Insert under "Important gotchas":

```markdown
- New users land on `/welcome` and pick exactly one onboarding template. The previous behavior of force-creating `personal/health`, `work`, `work/example` is gone; only `{email}/main` is provisioned at login. Users marked `onboarded = true` skip `/welcome` on subsequent logins.
- The health template's water-band / step-band derivations live in `assets/onboarding/health.toml` as Datalog rules. There is no procedural Rust path for these any longer; if a query for `health/water-band` returns nothing, check the template was actually seeded into the target exom (look for `(?fid 'onboarding/template_id ?id)`).
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: onboarding template CLI flags + new bootstrap behavior"
```

---

## Task 13 — Open PR

- [ ] **Step 1: Push branch**

```bash
git push -u origin feature/onboarding-templates
```

- [ ] **Step 2: Open PR via gh**

```bash
gh pr create --base main --head feature/onboarding-templates \
  --title "feat: data-driven onboarding template system" \
  --body "$(cat <<'EOF'
## Summary
- Replace hardcoded 3-exom bootstrap with TOML-driven template system.
- New CLI flags --onboarding-templates and --onboarding-template-dir.
- /welcome UI lets users pick exactly one template (or empty).
- StoredUser gains onboarded + seeded_templates; JSONL replay preserves them.
- Health template owns water-band / step-band derivations declaratively;
  procedural native_derived_relations deleted.

## Test plan
- [ ] cargo test
- [ ] cd ui && npm run check && npm run build
- [ ] Fresh DB + fresh login → /welcome shows 4 templates → pick "Personal health" → query returns water-band/medium and recommended-water-ml/2500
- [ ] Restart with --onboarding-templates workspace,project → catalog is empty + workspace + project
- [ ] Restart with --onboarding-template-dir /tmp/foo containing only workspace.toml → catalog is empty + workspace
- [ ] Log out + log in → no second /welcome prompt
EOF
)"
```

---

## Self-review checklist

- [ ] All 13 tasks reference exact file paths and line numbers from the current codebase (verified at plan write time against ray-exomem HEAD `c3ab18e`).
- [ ] No placeholders ("TBD", "implement later", "appropriate error handling").
- [ ] `OnboardingTemplate` field names are consistent across Rust struct, TOML files, API response, and UI client.
- [ ] `seed_template` is idempotent against re-seeding the same template (sentinel check on `onboarding/template_id`).
- [ ] JSONL replay preserves `onboarded` + `seeded_templates` per the same fallback pattern used for `active` + `last_login`.
- [ ] Postgres migration is additive (no destructive `ALTER TYPE` etc.).
- [ ] CLAUDE.md update covers both new CLI surface and the "no health logic in core" invariant.

## Verification summary

After Task 13, the following must all hold:

1. `cargo test && cd ui && npm run check && npm run build` is green.
2. `grep -r "native_derived_relations" src/ ui/` returns no hits.
3. `grep -r "bootstrap_user_namespace\|health_bootstrap_facts\|work_main_bootstrap_facts\|work_example_bootstrap_facts" src/` returns no hits.
4. Fresh DB walkthrough produces the seven expected outcomes from Task 10 Step 4.
5. CLI walkthrough from Task 10 Steps 5-6 confirms filtering and external dir replacement.
6. Two-login walkthrough (Task 10 Step 4 #6-7) confirms onboarded persistence.
