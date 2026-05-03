use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderName, HeaderValue, StatusCode, Uri},
    middleware,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use include_dir::{include_dir, Dir};
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tower_http::cors::{Any, CorsLayer};

// ---------------------------------------------------------------------------
// Embedded UI
// ---------------------------------------------------------------------------

static EMBEDDED_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/ui/build");

// ---------------------------------------------------------------------------
// Public constants (previously in web.rs)
// ---------------------------------------------------------------------------

/// Sub-path mount, baked at compile time from `$RAY_EXOMEM_BASE_PATH`.
/// Empty string means root mount. When non-empty, must start with `/` and
/// must not end with `/` (build.rs enforces this).
pub const BASE_PATH: &str = env!("RAY_EXOMEM_BASE_PATH");
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:9780";
pub const DEFAULT_EXOM: &str = "main";

use crate::{
    auth::{middleware::MaybeUser, User},
    backend::RayforceEngine,
    brain::{self, Brain, MergePolicy},
    context::{self, MutationContext},
    ffi,
    http_error::ApiError,
    rayfall_ast::{self, CanonicalForm, CanonicalQuery, LoweringOptions},
    rayfall_parser,
    rules::{self, ParsedRule},
    storage::{self, RayObj},
    system_schema,
};

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

pub struct ExomState {
    pub brain: Brain,
    pub datoms: RayObj,
    /// Per-type fact sub-tables. Rebound to the fixed env names
    /// `facts_i64` / `facts_str` / `facts_sym` immediately before each query
    /// so rule bodies can use `(facts_i64 ?e ?a ?v)` with live, typed values.
    pub typed_facts: storage::TypedFactTables,
    pub rules: Vec<ParsedRule>,
    pub exom_disk: Option<PathBuf>,
    /// Cached `ExomMeta.created_by` (email of the creator). Empty string
    /// means ownerless (system exom or unmigrated legacy). The auth layer
    /// reads this when resolving access in the `public/*` namespace —
    /// only the creator gets FullAccess; everyone else is ReadOnly+fork.
    pub created_by: String,
    /// Cached `ExomMeta.acl_mode`. The auth layer reads this for `public/*`
    /// to elevate non-creators from ReadOnly to ReadWrite when co-edit is
    /// on. The brain layer reads this in `precheck_write` to short-circuit
    /// `claim_branch` on the `main` trunk when co-edit is on.
    pub acl_mode: crate::exom::AclMode,
}

pub struct AppState {
    pub exoms: Mutex<HashMap<String, ExomState>>,
    pub engine: crate::backend::RayforceEngine,
    pub tree_root: Option<PathBuf>,
    pub sym_path: Option<PathBuf>,
    pub start_time: Instant,
    /// SSE event channel. The first tuple element is the exom path the
    /// event belongs to (`Some(slash_path)` for per-exom events, `None`
    /// for global structural events like `tree:changed` or
    /// `system:factory_reset`). The second is the JSON payload.
    /// Subscribers in `api_sse` filter `Some`-tagged events through
    /// `resolve_access` so a subscriber never sees activity in an exom
    /// they cannot read.
    pub sse_tx: broadcast::Sender<(Option<String>, String)>,
    pub auth_store: Option<Arc<crate::auth::store::AuthStore>>,
    pub auth_provider: Option<Arc<dyn crate::auth::provider::AuthProvider>>,
    pub ui_state: Option<Arc<dyn crate::db::UiStateDb>>,
    pub bind_addr: Option<String>,
    /// When non-empty, exposes a loopback-only `GET /auth/dev-login` route that
    /// mints a session for one of these emails without OAuth. Set via repeated
    /// `--dev-login-email` flags on `serve`. The route picks the email from
    /// `?email=` (must be in this allow-list) or defaults to the first entry.
    /// Refuses to start if the bind address is non-loopback.
    pub dev_login_emails: Vec<String>,
}

impl AppState {
    pub fn new(
        engine: RayforceEngine,
        exoms: HashMap<String, ExomState>,
        tree_root: Option<PathBuf>,
        sym_path: Option<PathBuf>,
    ) -> Self {
        let (sse_tx, _) = broadcast::channel(512);
        Self {
            exoms: Mutex::new(exoms),
            engine,
            tree_root,
            sym_path,
            start_time: Instant::now(),
            sse_tx,
            auth_store: None,
            auth_provider: None,
            ui_state: None,
            bind_addr: None,
            dev_login_emails: Vec::new(),
        }
    }

    /// Build AppState by loading a data directory (same logic as web::serve).
    pub fn from_data_dir(data_dir: Option<PathBuf>) -> anyhow::Result<Arc<Self>> {
        let mut exoms: HashMap<String, ExomState> = HashMap::new();
        let (engine, tree_root, sym_path) = match data_dir {
            Some(ref root) => {
                let sym = root.join("sym");
                // Always create the engine with fresh builtins at their
                // canonical slots. If an old sym file exists, the
                // rewrite below re-interns each persisted string and
                // remaps on-disk splays through the shift. This
                // decouples on-disk string identity from rayforce2's
                // slot layout, so builtin-shape refactors upstream
                // (e.g. commit 7db37e4 turning flat `.sys.gc` into a
                // dotted sym) no longer require wiping the sym file.
                // See archive/2026-04-24_sym-rewrite-migration/design.md.
                let engine = RayforceEngine::new()?;

                let tree_dir = root.join("tree");
                std::fs::create_dir_all(&tree_dir).ok();

                match crate::sym_rewrite::run_sym_rewrite(&sym, &tree_dir) {
                    Ok(outcome) => log_sym_rewrite_outcome(&outcome),
                    Err(e) => {
                        return Err(e.context(
                            "sym rewrite failed; refusing to boot with potentially \
                             corrupt on-disk state. See diagnostic above and \
                             archive/2026-04-24_sym-rewrite-migration/design.md",
                        ));
                    }
                }

                load_tree_exoms_into(&tree_dir, &sym, &mut exoms);
                // No auto-scaffolded `tree/main`. Under the privacy model
                // ownership is required for every read/write, and a bare
                // root-level `main` is owned by no one — so the only thing
                // an auto-create would do is mint an unreachable directory.
                // Fresh deployments boot with an empty exoms map; projects
                // and user namespaces are added explicitly via `init` or
                // `exom-new`. First authenticated login additionally seeds
                // the `public/*` paths declared by `bootstrap/*.json`
                // fixtures, but never auto-creates `{email}/main`.
                (engine, Some(tree_dir), Some(sym))
            }
            None => {
                // No persistence: ephemeral engine, empty exoms map. Same
                // contract as the persistent fresh-state path above —
                // exoms are added explicitly, never auto-created.
                let engine = RayforceEngine::new()?;
                (engine, None, None)
            }
        };

        // Bind all exoms into the engine.
        for (name, es) in &exoms {
            engine.bind_named_db(storage::sym_intern(name), &es.datoms)?;
        }

        // Run a canonical smoke query against every loaded exom to surface
        // sym/engine incompatibilities at startup instead of at first
        // request. Non-fatal — the daemon still boots.
        engine_health_probe(&engine, &exoms);

        Ok(Arc::new(AppState::new(engine, exoms, tree_root, sym_path)))
    }
}

/// Surface the sym-rewrite outcome to stderr. `FreshBoot` and `FastPath`
/// are the quiet cases — single line each, since nothing interesting
/// happened. `Remapped` is the loud case: the on-disk layout shifted
/// relative to the current binary (expected after a rayforce2 upgrade
/// that changed builtin interning shape) and we just rewrote every
/// splay. An operator seeing this on boot should know the migration
/// fired so they can correlate it with the rayforce2 version change.
fn log_sym_rewrite_outcome(outcome: &crate::sym_rewrite::RewriteOutcome) {
    use crate::sym_rewrite::RewriteOutcome;
    match outcome {
        RewriteOutcome::FreshBoot => {
            eprintln!("[ray-exomem] sym rewrite: fresh boot (no sym file yet)");
        }
        RewriteOutcome::FastPath { persisted } => {
            eprintln!(
                "[ray-exomem] sym rewrite: fast-path ({persisted} persisted strings, layout unchanged)"
            );
        }
        RewriteOutcome::Remapped {
            persisted,
            splays_rewritten,
        } => {
            eprintln!();
            eprintln!("[ray-exomem] ========================================================");
            eprintln!("[ray-exomem] sym table was MIGRATED");
            eprintln!("[ray-exomem]");
            eprintln!("[ray-exomem] {persisted} persisted strings re-interned under the current");
            eprintln!("[ray-exomem] binary's canonical layout; {splays_rewritten} on-disk splays");
            eprintln!("[ray-exomem] rewritten through the old→new remap.");
            eprintln!("[ray-exomem]");
            eprintln!("[ray-exomem] This is expected after a rayforce2 upgrade that");
            eprintln!("[ray-exomem] reshaped builtin interning (e.g. a builtin moving");
            eprintln!("[ray-exomem] from flat to dotted sym). Data was preserved via");
            eprintln!("[ray-exomem] the remap; next boot should hit the fast-path.");
            eprintln!("[ray-exomem] ========================================================");
            eprintln!();
        }
    }
}

/// Run the canonical typed-facts query against every loaded exom using the
/// same expand + execute path the live HTTP handler uses. Any failure is
/// logged loudly so an operator sees it at daemon start (in
/// `/tmp/ray-exomem.log`) rather than at first request.
///
/// Typical failure mode post-rayforce2-upgrade: queries return
/// `RAY_ERROR code=domain` with an empty msg because a persisted
/// sym entry (often a dotted-name builtin like `.sys.gc`) collides
/// with a reshaped builtin in the new binary. See CLAUDE.md
/// "sym compatibility" bullet and the sym-rewrite migration spec.
fn engine_health_probe(
    engine: &crate::backend::RayforceEngine,
    exoms: &HashMap<String, ExomState>,
) {
    let mut failures: Vec<(String, String)> = Vec::new();
    for name in exoms.keys() {
        let source = format!(
            "(query {} (find ?e ?a ?v) (where (facts_i64 ?e ?a ?v)))",
            name
        );
        let result: anyhow::Result<()> = (|| {
            let query = lower_query_request(&source, None, "startup health probe")?;
            let expanded = expand_canonical_query(exoms, engine, source.clone(), &query)?;
            execute_prepared_query(engine, exoms, &expanded)?;
            Ok(())
        })();
        if let Err(e) = result {
            failures.push((name.clone(), e.to_string()));
        }
    }
    if failures.is_empty() {
        return;
    }

    eprintln!();
    eprintln!("[ray-exomem] ========================================================");
    eprintln!("[ray-exomem] WARNING: engine health probe FAILED on startup");
    eprintln!("[ray-exomem]");
    eprintln!("[ray-exomem] The sym table or bound tables appear incompatible with");
    eprintln!("[ray-exomem] the current rayforce2 build. This usually happens after");
    eprintln!("[ray-exomem] an upstream change to builtin interning shape (e.g. a");
    eprintln!("[ray-exomem] builtin moving from flat to dotted sym).");
    eprintln!("[ray-exomem]");
    eprintln!("[ray-exomem] Per-exom failures:");
    for (name, err) in &failures {
        eprintln!("[ray-exomem]   - {}: {}", name, err);
    }
    eprintln!("[ray-exomem]");
    eprintln!("[ray-exomem] DO NOT wipe ~/.ray-exomem/sym as a reflex — every");
    eprintln!("[ray-exomem] persisted RAY_SYM column (fact ids, predicates, etc.)");
    eprintln!("[ray-exomem] encodes sym IDs by slot and would be stranded. See");
    eprintln!("[ray-exomem] CLAUDE.md 'sym compatibility' bullet for the forward");
    eprintln!("[ray-exomem] path (upstream issue or sym-rewrite migration).");
    eprintln!("[ray-exomem] ========================================================");
    eprintln!();
}

fn load_tree_exoms_into(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    out: &mut HashMap<String, ExomState>,
) {
    fn walk(
        tree_root: &std::path::Path,
        current: &std::path::Path,
        sym_path: &std::path::Path,
        out: &mut HashMap<String, ExomState>,
    ) {
        let Ok(rd) = std::fs::read_dir(current) else {
            return;
        };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let disk = entry.path();
            let rel = disk.strip_prefix(tree_root).unwrap_or(&disk);
            let slash_key = rel
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join("/");
            if slash_key.is_empty() {
                continue;
            }
            let meta_p = disk.join(crate::exom::META_FILENAME);
            if meta_p.exists() {
                eprintln!("[ray-exomem] loading tree exom '{}'", slash_key);
                match load_exom_from_tree_path_inner(&disk, sym_path, &slash_key) {
                    Ok(es) => {
                        out.insert(slash_key.clone(), es);
                    }
                    Err(e) => {
                        eprintln!(
                            "[ray-exomem] WARNING: failed to load '{}': {}",
                            slash_key, e
                        );
                    }
                }
                continue;
            }
            walk(tree_root, &disk, sym_path, out);
        }
    }
    walk(tree_root, tree_root, sym_path, out);
}

fn load_exom_from_tree_path_inner(
    exom_disk: &std::path::Path,
    sym_path: &std::path::Path,
    slash_key: &str,
) -> anyhow::Result<ExomState> {
    load_exom_from_tree_path(exom_disk, sym_path, slash_key)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(api_status))
        .route("/tree", get(api_tree))
        .route("/welcome/summary", get(api_welcome_summary))
        .route("/guide", get(api_guide))
        .route("/actions/init", post(api_init))
        .route("/actions/folder-new", post(api_folder_new))
        .route("/actions/delete", post(api_delete))
        .route("/actions/exom-new", post(api_exom_new))
        .route("/actions/exom-fork", post(api_exom_fork))
        .route("/actions/exom-mode", post(api_exom_mode))
        .route("/actions/session-new", post(api_session_new))
        .route("/actions/session-join", post(api_session_join))
        .route("/actions/branch-create", post(api_branch_create))
        .route("/actions/rename", post(api_rename))
        .route("/actions/assert-fact", post(api_assert_fact))
        // Query
        .route("/query", get(api_query_get).post(api_query_post))
        .route("/expand-query", post(api_expand_query))
        // Eval
        .route("/actions/eval", post(api_eval))
        .route("/actions/evaluate", post(api_evaluate_noop))
        // Facts
        .route("/facts", get(api_facts_list))
        .route("/facts/valid-at", get(api_facts_valid_at))
        .route("/facts/bitemporal", get(api_facts_bitemporal))
        .route("/facts/{id}", get(api_fact_detail))
        .route("/beliefs", get(api_beliefs_list))
        .route("/observations", get(api_observations_list))
        // Branches
        .route("/branches", get(api_list_branches).post(api_create_branch))
        .route(
            "/branches/{id}",
            get(api_branch_detail).delete(api_delete_branch_handler),
        )
        .route("/branches/{id}/diff", get(api_branch_diff_handler))
        .route("/branches/{id}/merge", post(api_merge_branch_handler))
        // Explain
        .route("/explain", get(api_explain))
        // Export / Import
        .route("/actions/export", get(api_export))
        .route("/actions/export-json", get(api_export_json))
        .route("/actions/import-json", post(api_import_json))
        // Mutations
        .route("/actions/retract-all", post(api_retract_all))
        .route("/actions/wipe", post(api_wipe))
        .route("/actions/factory-reset", post(api_factory_reset))
        // Schema / graph / clusters / logs / provenance / relation-graph
        .route("/schema", get(api_schema))
        .route("/graph", get(api_graph))
        .route("/clusters", get(api_clusters))
        .route("/clusters/{id}", get(api_cluster_detail_handler))
        .route("/logs", get(api_logs))
        .route("/provenance", get(api_provenance))
        .route("/relation-graph", get(api_relation_graph))
        // Derived / beliefs
        .route("/derived/{pred}", get(api_derived_handler))
        .route("/beliefs/{id}/support", get(api_belief_support_handler))
        // UI state
        .route(
            "/ui/graph-layout",
            get(api_get_graph_layout).put(api_put_graph_layout),
        )
        .fallback(api_not_found)
}

// ---------------------------------------------------------------------------
// SSE endpoint
// ---------------------------------------------------------------------------

async fn api_sse(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.sse_tx.subscribe();
    // Snapshot the subscriber's identity for the per-event filter. The
    // `require_auth` middleware on `/events` guarantees `maybe_user.0` is
    // `Some` whenever `auth_store` is configured; the `None` arm only
    // fires in single-user dev mode where every event is delivered.
    let subscriber = maybe_user.0;
    let state_for_filter = state.clone();
    let events = BroadcastStream::new(rx)
        .filter_map(move |r| {
            let state = state_for_filter.clone();
            let subscriber = subscriber.clone();
            async move {
                let (exom, payload) = r.ok()?;
                match (exom, &state.auth_store, subscriber) {
                    // No exom tag = global event (tree:changed, factory_reset). Always deliver.
                    (None, _, _) => Some(payload),
                    // Per-exom event but auth disabled (single-user dev). Deliver.
                    (Some(_), None, _) => Some(payload),
                    // Per-exom event, auth on, but no subscriber identity. Drop.
                    (Some(_), Some(_), None) => None,
                    // Per-exom event with an authenticated subscriber. Filter by access.
                    (Some(exom_path), Some(store), Some(user)) => {
                        let owner = lookup_owner(&state, &exom_path);
                        let level =
                            crate::auth::access::resolve_access(&user, &exom_path, store, owner)
                                .await;
                        if level.can_read() {
                            Some(payload)
                        } else {
                            None
                        }
                    }
                }
            }
        })
        .map(|payload| Ok(Event::default().data(payload)));
    let pings = IntervalStream::new(tokio::time::interval(Duration::from_secs(15)))
        .map(|_| Ok(Event::default().event("ping").data("{}")));
    Sse::new(futures::stream::select(events, pings))
}

async fn set_response_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin-allow-popups"),
    );
    response
}

// ---------------------------------------------------------------------------
// SPA fallback
// ---------------------------------------------------------------------------

fn content_type_for_ext(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn spa_fallback(uri: Uri) -> impl IntoResponse {
    // When mounted under `.nest(BASE_PATH, ...)`, the inner handlers see the
    // request path with BASE_PATH already stripped. When mounted at root the
    // path is unchanged. Either way, just trim the leading slash.
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = EMBEDDED_UI.get_file(path) {
        let ct = content_type_for_ext(path);
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, ct)],
            file.contents(),
        )
            .into_response();
    }

    // SPA client-side routing fallback
    if let Some(index) = EMBEDDED_UI.get_file("index.html") {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            index.contents(),
        )
            .into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

pub async fn serve(bind: &str, state: Arc<AppState>) -> anyhow::Result<()> {
    if let Some(ref auth_store) = state.auth_store {
        if let Some(auth_db) = &auth_store.auth_db {
            let cleanup_db = auth_db.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(900));
                loop {
                    interval.tick().await;
                    if let Err(e) = cleanup_db.cleanup_expired_sessions().await {
                        eprintln!("auth: cleanup_expired_sessions: {}", e);
                    }
                }
            });
        }
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let require_auth_layer =
        axum::middleware::from_fn_with_state(state.clone(), crate::auth::middleware::require_auth);

    // Routes that go through `require_auth_layer`. The layer is no-op when
    // `state.auth_store` is `None` (single-user dev mode).
    let protected = Router::new()
        .nest("/api", api_router())
        .route(
            "/mcp",
            get(crate::mcp::mcp_stream_handler)
                .post(crate::mcp::mcp_handler)
                .delete(crate::mcp::mcp_delete_handler),
        )
        .route(
            "/mcp/sse",
            get(crate::mcp::mcp_stream_handler)
                .post(crate::mcp::mcp_handler)
                .delete(crate::mcp::mcp_delete_handler),
        )
        .route("/events", get(api_sse))
        .layer(require_auth_layer);

    let inner = protected
        .nest("/auth", crate::auth::routes::auth_router())
        .nest("/auth/admin", crate::auth::admin::admin_router())
        .fallback(spa_fallback);

    let app_router = if BASE_PATH.is_empty() {
        inner
    } else {
        Router::new().nest(BASE_PATH, inner)
    };

    let app = app_router
        .with_state(state)
        .layer(middleware::map_response(set_response_headers))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn server_tree_root(state: &AppState) -> PathBuf {
    state
        .tree_root
        .clone()
        .unwrap_or_else(crate::storage::tree_root)
}

fn server_sym_path(state: &AppState) -> PathBuf {
    state
        .sym_path
        .clone()
        .unwrap_or_else(|| crate::storage::data_dir().join("sym"))
}

/// Read three-axis attribution headers (`x-agent`, `x-model`) for a write
/// route. Cookie-auth UI writes typically omit both; Bearer-auth scripts pass
/// either or both to override the API-key label and the model.
fn read_attribution_headers(headers: &axum::http::HeaderMap) -> (Option<String>, Option<String>) {
    let read = |name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    (read("x-agent"), read("x-model"))
}

/// Lazy-load an exom by slash key, inserting into the map if found on disk.
fn get_or_load_exom<'a>(
    exoms: &'a mut HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    slash_key: &str,
    tree_root: Option<&std::path::Path>,
    sym_path: Option<&std::path::Path>,
) -> anyhow::Result<&'a mut ExomState> {
    if exoms.contains_key(slash_key) {
        return Ok(exoms.get_mut(slash_key).unwrap());
    }
    if let (Some(tr), Some(sp)) = (tree_root, sym_path) {
        let disk = tr.join(slash_key);
        let meta_p = disk.join(crate::exom::META_FILENAME);
        if meta_p.exists() {
            let es = load_exom_from_tree_path(&disk, sp, slash_key)?;
            engine.bind_named_db(storage::sym_intern(slash_key), &es.datoms)?;
            exoms.insert(slash_key.to_string(), es);
            return Ok(exoms.get_mut(slash_key).unwrap());
        }
    }
    Err(anyhow::anyhow!("unknown exom '{}'", slash_key))
}

pub fn load_exom_from_tree_path(
    exom_disk: &std::path::Path,
    sym_path: &std::path::Path,
    slash_key: &str,
) -> anyhow::Result<ExomState> {
    let brain = Brain::open_exom(exom_disk, sym_path)?;
    // Load meta (and run the Model A migration if `created_by` is empty:
    // backfill from `main`'s TOFU claimer once, persist, and surface the
    // owner email for caching on ExomState).
    let mut created_by = String::new();
    let mut acl_mode = crate::exom::AclMode::SoloEdit;
    let meta_p = exom_disk.join(crate::exom::META_FILENAME);
    if meta_p.exists() {
        if let Ok(mut meta) = crate::exom::read_meta(exom_disk) {
            if meta.created_by.is_empty() {
                if let Some(claimer) = brain
                    .branches()
                    .iter()
                    .find(|b| b.branch_id == "main")
                    .and_then(|b| b.claimed_by_user_email.clone())
                {
                    meta.created_by = claimer;
                    if let Err(e) = crate::exom::write_meta(exom_disk, &meta) {
                        eprintln!(
                            "[ray-exomem] WARNING: failed to backfill created_by for '{}': {}",
                            slash_key, e
                        );
                    } else {
                        eprintln!(
                            "[ray-exomem] migration: backfilled created_by='{}' for '{}'",
                            meta.created_by, slash_key
                        );
                    }
                }
            }
            created_by = meta.created_by.clone();
            acl_mode = meta.acl_mode;
        }
    }
    let datoms = storage::build_datoms_table(&brain, brain::MAIN_BRANCH)?;
    let typed_facts = storage::build_typed_fact_tables(&brain, brain::MAIN_BRANCH)?;
    let rules_p = exom_disk.join("rules.ray");
    let rules = if rules_p.exists() {
        let src = std::fs::read_to_string(&rules_p)?;
        let mut out = Vec::new();
        for line in src.lines().map(str::trim).filter(|l| !l.is_empty()) {
            out.push(crate::rules::parse_rule_line(
                line,
                context::MutationContext::default(),
                String::new(),
            )?);
        }
        out
    } else {
        Vec::new()
    };
    let schema_p = exom_disk.join(system_schema::SCHEMA_FILENAME);
    let ontology =
        system_schema::build_exom_ontology(slash_key, &brain, brain::MAIN_BRANCH, &rules);
    let _ = system_schema::save_exom_ontology(&schema_p, &ontology);
    Ok(ExomState {
        brain,
        datoms,
        typed_facts,
        rules,
        exom_disk: Some(exom_disk.to_path_buf()),
        created_by,
        acl_mode,
    })
}

fn combined_rules(exom: &str, user_rules: &[ParsedRule]) -> anyhow::Result<Vec<ParsedRule>> {
    let mut rules = system_schema::builtin_rules(exom)?;
    rules.extend_from_slice(user_rules);
    Ok(rules)
}

/// Rebuild this exom's datoms + typed-fact tables for `branch_id` and rebind
/// them in the engine. No disk persistence. Use this when retargeting the
/// query plane for an explicit branch read.
pub(crate) fn rebind_datoms_only(
    state: &AppState,
    exoms: &mut HashMap<String, ExomState>,
    exom_name: &str,
    branch_id: &str,
) -> anyhow::Result<()> {
    let es = exoms
        .get_mut(exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    if !es.brain.branch_exists(branch_id) {
        anyhow::bail!("unknown branch '{}'", branch_id);
    }
    es.datoms = storage::build_datoms_table(&es.brain, branch_id)?;
    es.typed_facts = storage::build_typed_fact_tables(&es.brain, branch_id)?;
    state
        .engine
        .bind_named_db(storage::sym_intern(exom_name), &es.datoms)?;
    Ok(())
}

fn refresh_exom_binding(
    state: &AppState,
    exoms: &mut HashMap<String, ExomState>,
    exom_name: &str,
) -> anyhow::Result<()> {
    let es = exoms
        .get_mut(exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    es.datoms = storage::build_datoms_table(&es.brain, brain::MAIN_BRANCH)?;
    es.typed_facts = storage::build_typed_fact_tables(&es.brain, brain::MAIN_BRANCH)?;
    state
        .engine
        .bind_named_db(storage::sym_intern(exom_name), &es.datoms)?;
    if let Some(disk) = es.exom_disk.as_ref() {
        es.brain.save()?;
        let rules_p = disk.join("rules.ray");
        let body: String = es
            .rules
            .iter()
            .map(|r| r.full_text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !body.is_empty() {
            std::fs::write(&rules_p, format!("{}\n", body))?;
        }
        let schema_p = disk.join(system_schema::SCHEMA_FILENAME);
        let ontology =
            system_schema::build_exom_ontology(exom_name, &es.brain, brain::MAIN_BRANCH, &es.rules);
        let _ = system_schema::save_exom_ontology(&schema_p, &ontology);
    }
    Ok(())
}

fn evict_cached_exom(state: &AppState, exom_name: &str) {
    state.exoms.lock().unwrap().remove(exom_name);
}

/// Bind the executing exom's per-type fact sub-tables under the shared env
/// names (`facts_i64` / `facts_str` / `facts_sym`) right before running a
/// query. Rayforce2's auto-EDB hook (`ray_query_fn`) then picks them up for
/// any rule body that references `(facts_i64 ?e ?a ?v)` etc.
///
/// The bindings are per-process (globally shared in the runtime env), so
/// concurrent queries against different exoms race unless serialized. The
/// existing `state.exoms` mutex already serializes the mutation path;
/// callers MUST hold `state.exoms.lock()` before invoking this helper.
pub(crate) fn bind_typed_facts_for_exom(
    engine: &crate::backend::RayforceEngine,
    exoms: &HashMap<String, ExomState>,
    exom_name: &str,
) -> anyhow::Result<()> {
    let es = exoms
        .get(exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    engine.bind_named_db(
        storage::sym_intern(storage::FACTS_I64_ENV),
        &es.typed_facts.facts_i64,
    )?;
    engine.bind_named_db(
        storage::sym_intern(storage::FACTS_STR_ENV),
        &es.typed_facts.facts_str,
    )?;
    engine.bind_named_db(
        storage::sym_intern(storage::FACTS_SYM_ENV),
        &es.typed_facts.facts_sym,
    )?;
    Ok(())
}

pub fn mutate_exom<T>(
    state: &AppState,
    exom_name: &str,
    f: impl FnOnce(&mut ExomState) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    if !exoms.contains_key(exom_name) {
        let _ = get_or_load_exom(&mut exoms, &state.engine, exom_name, tree_root, sym_path);
    }
    let result = {
        let es = exoms
            .get_mut(exom_name)
            .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
        f(es)
    };
    let out = result?;
    refresh_exom_binding(state, &mut exoms, exom_name)?;
    let _ = state.sse_tx.send((
        Some(exom_name.to_string()),
        format!(r#"{{"kind":"memory","exom":"{}"}}"#, exom_name),
    ));
    Ok(out)
}

pub async fn mutate_exom_async<T>(
    state: &AppState,
    exom_name: &str,
    f: impl FnOnce(&mut ExomState) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    mutate_exom(state, exom_name, f)
}

fn emit_tree_changed(state: &AppState) {
    let _ = state.sse_tx.send((
        None,
        r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string(),
    ));
}

/// Look up the public-namespace owner state for an exom path.
///
/// - In-memory `ExomState` with non-empty `created_by` → `Owner(email)`
/// - Loaded ExomState with empty `created_by` (migration-failed legacy)
///   → `Ownerless`
/// - Path not loaded (folder, missing exom, or pre-load) → `Unknown`
///
/// Caller passes the result to `resolve_access` for the Model A decision
/// on `public/*` paths. Outside `public/*` the value is ignored.
pub fn lookup_owner(state: &AppState, exom_slash: &str) -> crate::auth::access::PublicOwner {
    use crate::auth::access::PublicOwner;
    let exoms = state.exoms.lock().unwrap();
    match exoms.get(exom_slash) {
        Some(es) if !es.created_by.is_empty() => PublicOwner::Owner {
            email: es.created_by.clone(),
            acl_mode: es.acl_mode,
        },
        Some(_) => PublicOwner::Ownerless,
        None => PublicOwner::Unknown,
    }
}

async fn guard_read(
    state: &AppState,
    maybe_user: &MaybeUser,
    exom_slash: &str,
) -> Option<axum::response::Response> {
    if let Some(ref auth_store) = state.auth_store {
        if let Some(ref user) = maybe_user.0 {
            let owner = lookup_owner(state, exom_slash);
            let level =
                crate::auth::access::resolve_access(user, exom_slash, auth_store, owner).await;
            if !level.can_read() {
                return Some(
                    ApiError::new("forbidden", format!("read access denied to {}", exom_slash))
                        .with_status(403)
                        .into_response(),
                );
            }
        }
    }
    None
}

async fn guard_write(
    state: &AppState,
    maybe_user: &MaybeUser,
    exom_slash: &str,
) -> Option<axum::response::Response> {
    if let Some(ref auth_store) = state.auth_store {
        if let Some(ref user) = maybe_user.0 {
            let owner = lookup_owner(state, exom_slash);
            let level =
                crate::auth::access::resolve_access(user, exom_slash, auth_store, owner).await;
            if !level.can_write() {
                return Some(
                    ApiError::new(
                        "forbidden",
                        format!("write access denied to {}", exom_slash),
                    )
                    .with_status(403)
                    .into_response(),
                );
            }
        }
    }
    None
}

async fn guard_owner(
    state: &AppState,
    maybe_user: &MaybeUser,
    exom_slash: &str,
) -> Option<axum::response::Response> {
    if let Some(ref auth_store) = state.auth_store {
        if let Some(ref user) = maybe_user.0 {
            let owner = lookup_owner(state, exom_slash);
            let level =
                crate::auth::access::resolve_access(user, exom_slash, auth_store, owner).await;
            if !level.is_owner() {
                return Some(
                    ApiError::new(
                        "forbidden",
                        format!("owner access required for {}", exom_slash),
                    )
                    .with_status(403)
                    .into_response(),
                );
            }
        }
    }
    None
}

fn tree_path_starts_with(path: &crate::path::TreePath, prefix: &crate::path::TreePath) -> bool {
    path.len() >= prefix.len()
        && path
            .segments()
            .iter()
            .zip(prefix.segments().iter())
            .all(|(a, b)| a == b)
}

fn namespace_path(namespace: &str) -> std::io::Result<crate::path::TreePath> {
    namespace.parse().map_err(|e: crate::path::PathError| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid namespace {:?}: {}", namespace, e),
        )
    })
}

/// Top-level shared root visible to every authenticated user. See
/// `auth::access::resolve_access` for the matching authorization rule.
const PUBLIC_NAMESPACE: &str = "public";

fn append_public_subtree(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    children: &mut Vec<crate::tree::TreeNode>,
    opts: &crate::tree::WalkOptions,
) -> std::io::Result<()> {
    let public_root: crate::path::TreePath = PUBLIC_NAMESPACE
        .parse()
        .expect("'public' is a valid TreePath segment");
    children.push(crate::tree::walk_or_empty(
        tree_root,
        sym_path,
        &public_root,
        opts,
    )?);
    Ok(())
}

pub(crate) async fn build_tree_root_for_user(
    state: &AppState,
    user: &User,
    opts: &crate::tree::WalkOptions,
) -> std::io::Result<crate::tree::TreeNode> {
    let tree_root = server_tree_root(state);
    let sym_path = server_sym_path(state);
    let own_root = namespace_path(user.namespace_root())?;
    let mut children = vec![crate::tree::walk_or_empty(
        &tree_root, &sym_path, &own_root, opts,
    )?];

    if let Some(ref auth_store) = state.auth_store {
        let mut by_owner: BTreeMap<String, Vec<crate::path::TreePath>> = BTreeMap::new();
        for grant in auth_store.shares_for_grantee(&user.email).await {
            let parsed: crate::path::TreePath = match grant.path.parse() {
                Ok(path) => path,
                Err(err) => {
                    eprintln!(
                        "[ray-exomem] skipping invalid share path {:?}: {}",
                        grant.path, err
                    );
                    continue;
                }
            };
            let Some(owner) = parsed.segments().first().cloned() else {
                continue;
            };
            if owner == user.namespace_root() {
                continue;
            }
            by_owner.entry(owner).or_default().push(parsed);
        }

        for (owner, shared_paths) in by_owner {
            let owner_root = namespace_path(&owner)?;
            children.push(crate::tree::walk_shared_projection(
                &tree_root,
                &sym_path,
                &owner_root,
                &shared_paths,
                opts,
            )?);
        }
    }

    append_public_subtree(&tree_root, &sym_path, &mut children, opts)?;

    Ok(crate::tree::TreeNode::Folder {
        name: String::new(),
        path: String::new(),
        children,
    })
}

async fn build_tree_path_for_user(
    state: &AppState,
    user: &User,
    requested: &crate::path::TreePath,
    opts: &crate::tree::WalkOptions,
) -> Result<crate::tree::TreeNode, ApiError> {
    let Some(ref auth_store) = state.auth_store else {
        let tree_root = server_tree_root(state);
        let sym_path = server_sym_path(state);
        return crate::tree::walk(&tree_root, &sym_path, requested, opts)
            .map_err(|e| ApiError::new("io", e.to_string()));
    };

    let tree_root = server_tree_root(state);
    let sym_path = server_sym_path(state);
    let requested_slash = requested.to_slash_string();
    let direct_owner = lookup_owner(state, &requested_slash);
    let direct_level =
        crate::auth::access::resolve_access(user, &requested_slash, auth_store, direct_owner).await;
    if direct_level.can_read() {
        let walk_result = if requested.len() == 1 {
            crate::tree::walk_or_empty(&tree_root, &sym_path, requested, opts)
        } else {
            crate::tree::walk(&tree_root, &sym_path, requested, opts)
        };
        return walk_result.map_err(|e| ApiError::new("io", e.to_string()));
    }

    let shared_paths: Vec<crate::path::TreePath> = auth_store
        .shares_for_grantee(&user.email)
        .await
        .into_iter()
        .filter_map(|grant| match grant.path.parse() {
            Ok(path) => Some(path),
            Err(err) => {
                eprintln!(
                    "[ray-exomem] skipping invalid share path {:?}: {}",
                    grant.path, err
                );
                None
            }
        })
        .filter(|grant_path| {
            tree_path_starts_with(requested, grant_path)
                || tree_path_starts_with(grant_path, requested)
        })
        .collect();

    if shared_paths.is_empty() {
        return Err(ApiError::new(
            "forbidden",
            format!("read access denied to {}", requested_slash),
        )
        .with_status(403));
    }

    crate::tree::walk_shared_projection(&tree_root, &sym_path, requested, &shared_paths, opts)
        .map_err(|e| ApiError::new("io", e.to_string()))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn api_status(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root_val = state.tree_root.as_deref();
    let sym_path_val = state.sym_path.as_deref();
    if !exoms.contains_key(&exom_slash) {
        let _ = get_or_load_exom(
            &mut exoms,
            &state.engine,
            &exom_slash,
            tree_root_val,
            sym_path_val,
        );
    }
    let Some(es) = exoms.get(&exom_slash) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "exom not found"})),
        )
            .into_response();
    };
    let brain = &es.brain;
    let uptime = state.start_time.elapsed().as_secs();
    let facts = brain.facts_on_branch(brain::MAIN_BRANCH);
    let beliefs = brain.beliefs_on_branch(brain::MAIN_BRANCH);
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let derived_names: Vec<String> = rules::derived_predicates(&all_rules)
        .into_iter()
        .map(|(n, _)| n)
        .take(24)
        .collect();
    let ontology =
        system_schema::build_exom_ontology(&exom_slash, brain, brain::MAIN_BRANCH, &es.rules);
    let status = serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "server": {
            "name": "ray-exomem",
            "version": crate::frontend_version(),
            "uptime_sec": uptime,
            "tree_root": server_tree_root(&state).display().to_string(),
            "build": {
                "git_sha": crate::build_git_sha(),
                "built_unix": crate::build_unix_timestamp(),
                "identity": crate::build_identity(),
            }
        },
        "storage": {
            "exom_path": "in-memory"
        },
        "stats": {
            "relations": 3,
            "facts": facts.len(),
            "derived_tuples": beliefs.len(),
            "intervals": facts.iter().filter(|f| f.valid_to.is_some()).count(),
            "directives": 0,
            "events_logged": brain.transactions().len(),
            "sym_entries": storage::sym_count(),
            "rules": {
                "count": es.rules.len(),
                "derived_predicates": derived_names,
            }
        },
        "schema": {
            "path": serde_json::Value::Null,
            "system_attribute_count": ontology.system_attributes.len(),
            "coordination_attribute_count": ontology.coordination_attributes.len(),
            "builtin_view_count": ontology.builtin_views.len(),
            "user_predicates": ontology.user_predicates,
        }
    });
    (StatusCode::OK, Json(status)).into_response()
}

#[derive(Deserialize, Default)]
struct TreeQuery {
    path: Option<String>,
    depth: Option<usize>,
    archived: Option<String>,
    branches: Option<String>,
    activity: Option<String>,
}

async fn api_tree(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<TreeQuery>,
) -> impl IntoResponse {
    if state.auth_store.is_some() && maybe_user.0.is_none() {
        return ApiError::new("unauthorized", "authentication required")
            .with_status(401)
            .into_response();
    }

    let tree_root = server_tree_root(&state);
    let sym_path = server_sym_path(&state);
    let opts = crate::tree::WalkOptions {
        depth: q.depth.or(Some(usize::MAX)),
        include_archived: q.archived.as_deref() == Some("true"),
        include_branches: q.branches.as_deref() == Some("true"),
        include_activity: q.activity.as_deref() == Some("true"),
    };
    let result = if let Some(ref user) = maybe_user.0 {
        match q.path.as_deref().filter(|s| !s.is_empty()) {
            None => build_tree_root_for_user(&state, user, &opts).await,
            Some(p) => match p.parse::<crate::path::TreePath>() {
                Ok(tp) => match build_tree_path_for_user(&state, user, &tp, &opts).await {
                    Ok(node) => return Json(node).into_response(),
                    Err(err) => return err.into_response(),
                },
                Err(e) => {
                    let err = ApiError::new("bad_path", e.to_string());
                    return err.into_response();
                }
            },
        }
    } else {
        match q.path.as_deref().filter(|s| !s.is_empty()) {
            None => crate::tree::walk_root(&tree_root, &sym_path, &opts),
            Some(p) => match p.parse::<crate::path::TreePath>() {
                Ok(tp) => crate::tree::walk(&tree_root, &sym_path, &tp, &opts),
                Err(e) => {
                    let err = ApiError::new("bad_path", e.to_string());
                    return err.into_response();
                }
            },
        }
    };
    match result {
        Ok(node) => Json(node).into_response(),
        Err(e) => ApiError::new("io", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// /api/welcome/summary
//
// Single fan-in endpoint for the cold-visitor welcome page (see
// archive/2026-04-25_ui-ux-redesign/brief.md). Returns:
//   - totals: aggregate fact / exom / branch counts and the most-recent
//     change across all bootstrap-seeded exoms.
//   - featured: top entities by recency, fact-count tiebreak, with
//     name / type / summary / docs_url pulled from the predicate
//     registry.
//   - latest: post-bootstrap transactions (tx_time within 30d of now).
//
// Auth-gated like the rest of the API. Iterates over
// `bootstrap_seed_exom_paths()` so a daemon with no fixtures still
// returns sensible empty arrays.
// ---------------------------------------------------------------------------
async fn api_welcome_summary(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
) -> impl IntoResponse {
    if state.auth_store.is_some() && maybe_user.0.is_none() {
        return ApiError::new("unauthorized", "authentication required")
            .with_status(401)
            .into_response();
    }

    let seeded_paths = crate::auth::routes::bootstrap_seed_exom_paths();
    let tree_root_val = state.tree_root.as_deref();
    let sym_path_val = state.sym_path.as_deref();

    let mut total_facts: usize = 0;
    let mut total_branches: usize = 0;
    let mut total_exoms: usize = 0;
    let mut latest_change: Option<(String, String, String)> = None; // (tx_time, actor, note)

    #[derive(Default)]
    struct EntityAgg {
        exom: String,
        entity: String,
        fact_count: usize,
        last_tx_time: String,
        name: Option<String>,
        ty: Option<String>,
        summary: Option<String>,
        docs_url: Option<String>,
    }
    let mut entities: HashMap<String, EntityAgg> = HashMap::new();

    let mut all_txs: Vec<(String, crate::brain::Tx)> = Vec::new();

    {
        let mut exoms = state.exoms.lock().unwrap();
        for path in &seeded_paths {
            if get_or_load_exom(&mut exoms, &state.engine, path, tree_root_val, sym_path_val)
                .is_err()
            {
                continue;
            }
            let Some(es) = exoms.get(path) else { continue };
            total_exoms += 1;
            let brain = &es.brain;
            let facts = brain.facts_on_branch(brain::MAIN_BRANCH);
            total_facts += facts.len();
            total_branches += brain.branches().len();

            for tx in brain.transactions() {
                if latest_change
                    .as_ref()
                    .map(|(t, _, _)| tx.tx_time.as_str() > t.as_str())
                    .unwrap_or(true)
                {
                    latest_change = Some((
                        tx.tx_time.clone(),
                        tx.user_email.clone().unwrap_or_else(|| "system".into()),
                        tx.note.clone(),
                    ));
                }
                all_txs.push((path.clone(), tx.clone()));
            }

            for f in &facts {
                let entity = match f.fact_id.find('#') {
                    Some(idx) => f.fact_id[..idx].to_string(),
                    None => f.fact_id.clone(),
                };
                let key = format!("{path}::{entity}");
                let agg = entities.entry(key).or_insert_with(|| EntityAgg {
                    exom: path.clone(),
                    entity: entity.clone(),
                    ..Default::default()
                });
                agg.fact_count += 1;

                let tx_time = brain
                    .transactions()
                    .iter()
                    .find(|t| t.tx_id == f.created_by_tx)
                    .map(|t| t.tx_time.clone())
                    .unwrap_or_default();
                if tx_time > agg.last_tx_time {
                    agg.last_tx_time = tx_time;
                }

                match f.predicate.as_str() {
                    "entity/name" => agg.name = Some(value_string(&f.value)),
                    "entity/type" => agg.ty = Some(value_string(&f.value)),
                    "concept/summary" => agg.summary = Some(value_string(&f.value)),
                    "concept/docs_url" | "concept/scalar_docs_url" => {
                        agg.docs_url = Some(value_string(&f.value));
                    }
                    _ => {}
                }
            }
        }
    }

    let mut featured: Vec<&EntityAgg> = entities.values().collect();
    featured.sort_by(|a, b| {
        b.last_tx_time
            .cmp(&a.last_tx_time)
            .then_with(|| b.fact_count.cmp(&a.fact_count))
    });
    let featured_json: Vec<serde_json::Value> = featured
        .iter()
        .filter(|e| e.name.is_some())
        .take(6)
        .map(|e| {
            serde_json::json!({
                "exom": e.exom,
                "entity": e.entity,
                "name": e.name,
                "type": e.ty,
                "summary": e.summary,
                "docs_url": e.docs_url,
                "fact_count": e.fact_count,
                "last_tx_time": e.last_tx_time,
            })
        })
        .collect();

    // Split transactions on the 30-day watershed: anything older is
    // treated as bootstrap seed activity, anything newer as live changes.
    let now = chrono::Utc::now();
    let watershed = (now - chrono::Duration::days(30)).to_rfc3339();
    let mut latest: Vec<(String, &crate::brain::Tx)> = all_txs
        .iter()
        .filter(|(_, t)| t.tx_time > watershed)
        .map(|(p, t)| (p.clone(), t))
        .collect();
    latest.sort_by(|a, b| b.1.tx_time.cmp(&a.1.tx_time));

    let serialize_tx = |(exom, tx): &(String, &crate::brain::Tx)| {
        serde_json::json!({
            "exom": exom,
            "tx_id": tx.tx_id,
            "tx_time": tx.tx_time,
            "user_email": tx.user_email,
            "agent": tx.agent,
            "model": tx.model,
            "action": tx.action.to_string(),
            "refs": tx.refs,
            "note": tx.note,
            "branch_id": tx.branch_id,
        })
    };

    let totals = serde_json::json!({
        "facts": total_facts,
        "exoms": total_exoms,
        "branches": total_branches,
        "last_change": latest_change.as_ref().map(|(t, a, n)| serde_json::json!({
            "tx_time": t,
            "actor": a,
            "note": n,
        })).unwrap_or(serde_json::Value::Null),
    });

    Json(serde_json::json!({
        "totals": totals,
        "featured": featured_json,
        "latest": latest.iter().take(10).map(serialize_tx).collect::<Vec<_>>(),
    }))
    .into_response()
}

fn value_string(v: &crate::fact_value::FactValue) -> String {
    v.display()
}

async fn api_guide() -> impl IntoResponse {
    let body = crate::agent_guide::doctrine();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        body,
    )
}

async fn api_not_found(uri: Uri) -> impl IntoResponse {
    ApiError::new("not_found", format!("unknown API route {}", uri.path()))
        .with_status(404)
        .into_response()
}

#[derive(Deserialize)]
struct PathBody {
    path: Option<String>,
    /// Optional `acl_mode` for the created exom (or each created exom in
    /// `init`'s case). Defaults to `solo-edit`. Ignored on Session exoms.
    #[serde(default)]
    acl_mode: Option<crate::exom::AclMode>,
}

async fn api_init(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let path_slash = path.to_slash_string();
    if let Some(resp) = guard_write(&state, &maybe_user, &path_slash).await {
        return resp;
    }
    let tree_root = server_tree_root(&state);
    let created_by = maybe_user
        .0
        .as_ref()
        .map(|u| u.email.as_str())
        .unwrap_or("");
    let acl_mode = body.acl_mode.unwrap_or(crate::exom::AclMode::SoloEdit);
    match crate::scaffold::init_project(&tree_root, &path, created_by) {
        Ok(()) => {
            // Stamp acl_mode on the project's `main` exom only. Sessions
            // under `<path>/sessions/` are always SoloEdit (Q7).
            if acl_mode == crate::exom::AclMode::CoEdit {
                let main_disk = path.to_disk_path(&tree_root).join("main");
                if let Ok(mut m) = crate::exom::read_meta(&main_disk) {
                    m.acl_mode = acl_mode;
                    if let Err(e) = crate::exom::write_meta(&main_disk, &m) {
                        eprintln!(
                            "[ray-exomem] WARNING: init created but acl_mode stamp failed on main: {}",
                            e
                        );
                    }
                }
            }
            emit_tree_changed(&state);
            Json(serde_json::json!({
                "ok": true,
                "path": path.to_slash_string(),
                "acl_mode": acl_mode,
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

async fn api_folder_new(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let path_slash = path.to_slash_string();
    if let Some(resp) = guard_write(&state, &maybe_user, &path_slash).await {
        return resp;
    }
    let tree_root = server_tree_root(&state);
    match crate::scaffold::new_folder(&tree_root, &path) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "path": path.to_slash_string()})).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

async fn api_delete(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if state.auth_store.is_some() && path.len() == 1 {
        return ApiError::new(
            "namespace_root_immutable",
            "cannot delete a top-level namespace root",
        )
        .with_status(403)
        .into_response();
    }
    let path_slash = path.to_slash_string();
    if let Some(resp) = guard_write(&state, &maybe_user, &path_slash).await {
        return resp;
    }
    let tree_root = server_tree_root(&state);

    let exoms_under = match crate::scaffold::collect_exoms_under(&tree_root, &path) {
        Ok(v) => v,
        Err(e) => return ApiError::from(e).into_response(),
    };

    // Drop in-memory ExomState entries for everything we're about to remove.
    // Hold the guard only for the structural mutation; reconcile_engine and
    // the disk delete run after so the lock isn't held across IO.
    {
        let mut exoms = state.exoms.lock().unwrap();
        for slash in &exoms_under {
            exoms.remove(slash);
        }
    }

    if let Err(e) = crate::scaffold::delete_subtree(&tree_root, &path) {
        // Engine bindings are now stale w.r.t. what's still on disk, but
        // reconcile will rebind only what remains in `state.exoms`. Forcing
        // a reconcile here keeps the engine consistent with the in-memory map.
        let exoms = state.exoms.lock().unwrap();
        reconcile_engine(&state, &exoms);
        return ApiError::from(e).into_response();
    }

    {
        let exoms = state.exoms.lock().unwrap();
        reconcile_engine(&state, &exoms);
    }

    if let Some(ref auth_store) = state.auth_store {
        auth_store.delete_shares_under(&path_slash).await;
    }

    emit_tree_changed(&state);

    Json(serde_json::json!({
        "ok": true,
        "deleted": path_slash,
        "removed_exoms": exoms_under,
    }))
    .into_response()
}

async fn api_exom_new(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    let path_slash = path.to_slash_string();
    if let Some(resp) = guard_write(&state, &maybe_user, &path_slash).await {
        return resp;
    }
    let tree_root = server_tree_root(&state);
    let created_by = maybe_user
        .0
        .as_ref()
        .map(|u| u.email.as_str())
        .unwrap_or("");
    let acl_mode = body.acl_mode.unwrap_or(crate::exom::AclMode::SoloEdit);
    match crate::scaffold::new_bare_exom(&tree_root, &path, created_by) {
        Ok(()) => {
            // Stamp acl_mode on the freshly-created exom.json. Only persists
            // a non-default mode; SoloEdit is the constructor's default.
            if acl_mode == crate::exom::AclMode::CoEdit {
                let disk = path.to_disk_path(&tree_root);
                if let Ok(mut m) = crate::exom::read_meta(&disk) {
                    m.acl_mode = acl_mode;
                    if let Err(e) = crate::exom::write_meta(&disk, &m) {
                        eprintln!(
                            "[ray-exomem] WARNING: exom-new created but acl_mode stamp failed: {}",
                            e
                        );
                    }
                }
            }
            emit_tree_changed(&state);
            Json(serde_json::json!({
                "ok": true,
                "path": path.to_slash_string(),
                "acl_mode": acl_mode,
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct ExomForkBody {
    /// Path of the exom to fork (must be readable by the caller; not a session exom).
    source: String,
    /// Optional target path. When omitted, the default is
    /// `{user.email}/forked/<source_subpath>`:
    /// - `public/X/Y/Z` → `{user.email}/forked/X/Y/Z` (drop `public/` prefix).
    /// - `{other_email}/X/Y` → `{user.email}/forked/{other_email}/X/Y`
    ///   (preserve the source owner's email so lineage is readable in the path).
    /// - `{user.email}/X/Y` (self-fork) → `{user.email}/forked/X/Y`.
    /// Suffixed with `-2`, `-3`, ... on the leaf segment if the default is taken.
    #[serde(default)]
    target: Option<String>,
}

/// `POST /api/actions/exom-fork`
///
/// Model A's contribution path. Read-share gives you read; if you want to
/// write, you fork into your own namespace (or any path you can write to)
/// and own the result. The new exom carries:
/// - `created_by = user.email` (Model A ownership stamp)
/// - `forked_from = { source_path, source_tx_id, forked_at }` (lineage for
///   future sync-request flows; never overwritten)
/// - all currently-active facts from `source`'s `main`, asserted as new
///   tx records attributed to the forker. Original `fact_id`s are
///   preserved so cross-exom diffs can match identities.
///
/// Refused for session exoms (sessions are time-bounded multi-agent
/// contexts, not knowledge artifacts; fork the parent project instead).
async fn api_exom_fork(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<ExomForkBody>,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };

    let source_path: crate::path::TreePath = match body.source.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_source", e.to_string()).into_response(),
    };
    let source_slash = source_path.to_slash_string();

    // Read access on source.
    if let Some(resp) = guard_read(&state, &maybe_user, &source_slash).await {
        return resp;
    }

    // Default target: `{user.email}/forked/<source-subpath>` with collision
    // suffixes on the leaf segment. The `forked/` prefix groups every fork
    // the user has made into one place in their personal namespace, and the
    // sub-path mirrors the source so a fork of `public/work/team/proj/main`
    // lands at `{user.email}/forked/work/team/proj/main` — readable lineage
    // without having to read the `forked_from` block.
    //
    // The actual derivation lives in `crate::exom::default_fork_target`,
    // which is also called from `mcp::tool_exom_fork` so the two transports
    // cannot drift apart again.
    let target_slash = match body.target.as_deref() {
        Some(t) => t.to_string(),
        None => match crate::exom::default_fork_target(
            &server_tree_root(&state),
            &user.email,
            &source_slash,
            source_path.last(),
        ) {
            Ok(t) => t,
            Err(_) => {
                return ApiError::new(
                    "fork_collision",
                    "could not find a free target path; pass `target` explicitly",
                )
                .into_response();
            }
        },
    };
    let target_path: crate::path::TreePath = match target_slash.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_target", e.to_string()).into_response(),
    };

    // Write access on target.
    if let Some(resp) = guard_write(&state, &maybe_user, &target_slash).await {
        return resp;
    }

    let tree_root = server_tree_root(&state);
    let sym_path = server_sym_path(&state);

    // Refuse session exoms.
    let source_disk = source_path.to_disk_path(&tree_root);
    let source_meta = match crate::exom::read_meta(&source_disk) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::new("no_such_exom", format!("no such exom {}", source_slash))
                .with_status(404)
                .into_response();
        }
        Err(e) => return ApiError::new("io", e.to_string()).into_response(),
    };
    if source_meta.kind == crate::exom::ExomKind::Session {
        return ApiError::new(
            "fork_session_unsupported",
            "session exoms cannot be forked; fork the parent project's main instead",
        )
        .with_status(400)
        .into_response();
    }

    // Target must not exist (avoid clobber). The default-target loop above
    // already auto-suffixed; a caller-supplied target gets the strict check.
    let target_disk = target_path.to_disk_path(&tree_root);
    if target_disk.exists() {
        return ApiError::new(
            "target_exists",
            format!("target path already exists: {}", target_slash),
        )
        .with_status(409)
        .into_response();
    }

    // Snapshot source facts + the source tip tx_id under the lock.
    let snapshot: Vec<(
        String,
        String,
        crate::fact_value::FactValue,
        f64,
        String,
        String,
        Option<String>,
    )>;
    let source_tip_tx: u64;
    {
        let mut exoms = state.exoms.lock().unwrap();
        let es = match get_or_load_exom(
            &mut exoms,
            &state.engine,
            &source_slash,
            Some(&tree_root),
            Some(&sym_path),
        ) {
            Ok(es) => es,
            Err(e) => return ApiError::new("source_load_failed", e.to_string()).into_response(),
        };
        // Tip = max created_by_tx among current_facts (fine for v1; if no
        // facts, use 0). This is what `forked_at_tx` records.
        source_tip_tx = es
            .brain
            .facts_on_branch(brain::MAIN_BRANCH)
            .iter()
            .map(|f| f.created_by_tx as u64)
            .max()
            .unwrap_or(0);
        snapshot = es
            .brain
            .facts_on_branch(brain::MAIN_BRANCH)
            .iter()
            .map(|f| {
                (
                    f.fact_id.clone(),
                    f.predicate.clone(),
                    f.value.clone(),
                    f.confidence,
                    f.provenance.clone(),
                    f.valid_from.clone(),
                    f.valid_to.clone(),
                )
            })
            .collect();
    }

    // Create the target exom (bare; same shape as exom-new).
    if let Err(e) = crate::scaffold::new_bare_exom(&tree_root, &target_path, &user.email) {
        return ApiError::from(e).into_response();
    }

    // Stamp lineage into the target's ExomMeta.
    let lineage = crate::exom::ForkLineage {
        source_path: source_slash.clone(),
        source_tx_id: source_tip_tx,
        forked_at: crate::exom::now_iso8601_basic(),
    };
    if let Ok(mut tmeta) = crate::exom::read_meta(&target_disk) {
        tmeta.forked_from = Some(lineage.clone());
        if let Err(e) = crate::exom::write_meta(&target_disk, &tmeta) {
            eprintln!(
                "[ray-exomem] WARNING: forked exom created but lineage stamp failed: {}",
                e
            );
        }
    }

    // Replay the snapshot into the new exom under the forker's identity.
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let write_ctx = MutationContext::from_user(user, header_agent, header_model);
    let mut copied: usize = 0;
    {
        // mutate_exom_async lazy-loads from disk, so the new exom will be
        // picked up. We loop sequentially; each assert_fact is its own tx.
        for (fact_id, predicate, value, confidence, provenance, valid_from, valid_to) in
            snapshot.iter()
        {
            let target = target_slash.clone();
            let fact_id_for_log = fact_id.clone();
            let target_for_log = target_slash.clone();
            let source_for_log = source_slash.clone();
            let fact_id_owned = fact_id.clone();
            let predicate_owned = predicate.clone();
            let value_owned = value.clone();
            let confidence_owned = *confidence;
            let provenance_owned = provenance.clone();
            let valid_from_owned = valid_from.clone();
            let valid_to_owned = valid_to.clone();
            let ctx = write_ctx.clone();
            let r = mutate_exom_async(&state, &target, move |ex| {
                ex.brain.assert_fact(
                    brain::MAIN_BRANCH,
                    &fact_id_owned,
                    &predicate_owned,
                    value_owned,
                    confidence_owned,
                    &provenance_owned,
                    Some(valid_from_owned.as_str()),
                    valid_to_owned.as_deref(),
                    &ctx,
                )
            })
            .await;
            match r {
                Ok(_) => copied += 1,
                Err(e) => {
                    eprintln!(
                        "[ray-exomem] fork: failed to copy fact {} from {} to {}: {}",
                        fact_id_for_log, source_for_log, target_for_log, e
                    );
                }
            }
        }
    }

    emit_tree_changed(&state);

    Json(serde_json::json!({
        "ok": true,
        "source": source_slash,
        "target": target_slash,
        "copied_facts": copied,
        "forked_from": {
            "source_path": lineage.source_path,
            "source_tx_id": lineage.source_tx_id,
            "forked_at": lineage.forked_at,
        }
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ExomModeBody {
    exom: String,
    mode: crate::exom::AclMode,
}

/// `POST /api/actions/exom-mode`
///
/// Flip an exom's `acl_mode` between `solo-edit` and `co-edit`.
/// Creator-only; rejected on Session exoms (Q7). Side effects:
///
/// - `solo-edit → co-edit`: clears `main.claimed_by_user_email = None` so
///   any auth-admitted writer lands on the shared trunk.
/// - `co-edit → solo-edit`: deterministically re-claims `main` for
///   `created_by` so the creator regains exclusive trunk-write predictably.
///
/// Non-`main` branches are not touched (Q5 invariant).
async fn api_exom_mode(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<ExomModeBody>,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };

    let exom_path: crate::path::TreePath = match body.exom.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    let tree_root = server_tree_root(&state);
    let disk = exom_path.to_disk_path(&tree_root);

    let mut meta = match crate::exom::read_meta(&disk) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::new("no_such_exom", format!("no such exom {}", exom_slash))
                .with_status(404)
                .into_response();
        }
        Err(e) => return ApiError::new("io", e.to_string()).into_response(),
    };

    if meta.created_by != user.email {
        return ApiError::new(
            "not_creator",
            "only the exom creator may change the acl_mode",
        )
        .with_status(403)
        .into_response();
    }

    if meta.kind == crate::exom::ExomKind::Session {
        return ApiError::new(
            "acl_mode_not_applicable",
            "session exoms use orchestrator-allocated branches; co-edit is not applicable to this exom kind",
        )
        .with_status(400)
        .into_response();
    }

    let prev_mode = meta.acl_mode;
    let new_mode = body.mode;

    if prev_mode == new_mode {
        return Json(serde_json::json!({
            "ok": true,
            "exom": exom_slash,
            "mode": new_mode,
            "previous_mode": prev_mode,
            "changed": false,
        }))
        .into_response();
    }

    // Persist the new mode in exom.json BEFORE touching the brain. `acl_mode`
    // is owned by `ExomMeta`, not by `Brain`, so any concurrent reader (e.g.
    // a parallel `precheck_write` call) needs to observe the new mode the
    // moment we publish it. `write_meta` is the single source of truth.
    meta.acl_mode = new_mode;
    if let Err(e) = crate::exom::write_meta(&disk, &meta) {
        return ApiError::new("io", format!("failed to write exom.json: {}", e)).into_response();
    }

    // Decide what the new `main` claim should be. `co-edit` clears it so any
    // auth-admitted writer lands on the shared trunk; `solo-edit` re-claims
    // for the exom creator so they regain deterministic trunk write.
    let new_claim: Option<(String, Option<String>, Option<String>)> = match new_mode {
        crate::exom::AclMode::CoEdit => None,
        crate::exom::AclMode::SoloEdit => Some((user.email.clone(), None, None)),
    };

    // Append an audit fact tx so the flip is visible in history alongside
    // every other mutation. `_meta/acl_mode` is a single fact_id so
    // assert_fact's last-write-wins gives a clean "current mode" view. Run
    // the claim mutation in the SAME `mutate_exom_async` closure so the
    // in-memory `Brain.branches` and the disk splay table stay in lockstep:
    // `refresh_exom_binding`'s tail `brain.save()` then writes back exactly
    // the state we just set, instead of clobbering a fresh disk write with
    // stale in-memory branches (the original 2026-05-02 regression).
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let write_ctx = MutationContext::from_user(user, header_agent, header_model);
    let mode_str = match new_mode {
        crate::exom::AclMode::CoEdit => "co-edit",
        crate::exom::AclMode::SoloEdit => "solo-edit",
    };
    let exom_for_assert = exom_slash.clone();
    let ctx = write_ctx.clone();
    if let Err(e) = mutate_exom_async(&state, &exom_for_assert, move |ex| {
        // Sync the in-memory ExomState mode field so subsequent
        // guard_write / lookup_owner calls observe the new mode without
        // a daemon restart.
        ex.acl_mode = new_mode;
        ex.brain
            .set_branch_claim(brain::MAIN_BRANCH, new_claim)
            .map_err(|e| anyhow::anyhow!("set_branch_claim failed: {}", e))?;
        ex.brain.assert_fact(
            brain::MAIN_BRANCH,
            "_meta/acl_mode",
            "_meta/acl_mode",
            crate::fact_value::FactValue::Str(mode_str.to_string()),
            1.0,
            "exom-mode-flip",
            None,
            None,
            &ctx,
        )?;
        Ok(())
    })
    .await
    {
        return ApiError::new("io", format!("exom-mode flip failed: {}", e)).into_response();
    }

    emit_tree_changed(&state);

    Json(serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "mode": new_mode,
        "previous_mode": prev_mode,
        "changed": true,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SessionNewBody {
    project_path: Option<String>,
    #[serde(rename = "type")]
    session_type: Option<String>,
    label: Option<String>,
    actor: Option<String>,
    agent: Option<String>,
    model: Option<String>,
    #[serde(default)]
    agents: Vec<String>,
}

async fn api_session_new(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<SessionNewBody>,
) -> impl IntoResponse {
    let project_path_str = body.project_path.unwrap_or_default();
    let project_path: crate::path::TreePath = match project_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if let Some(resp) = guard_write(&state, &maybe_user, &project_path.to_slash_string()).await {
        return resp;
    }
    let session_type = match body.session_type.as_deref().unwrap_or("") {
        "multi" => crate::exom::SessionType::Multi,
        "single" => crate::exom::SessionType::Single,
        other => {
            return ApiError::new(
                "bad_session_type",
                format!("unknown session type {:?}; use 'multi' or 'single'", other),
            )
            .into_response();
        }
    };
    let label = body.label.unwrap_or_default();
    let actor = maybe_user
        .0
        .as_ref()
        .map(|u| u.email.clone())
        .or(body.actor)
        .unwrap_or_default();
    if actor.is_empty() {
        return ApiError::from(brain::WriteError::ActorRequired).into_response();
    }
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let agent = header_agent
        .or_else(|| maybe_user.0.as_ref().and_then(|u| u.api_key_label.clone()))
        .or(body.agent);
    let model = header_model.or(body.model);
    let tree_root = server_tree_root(&state);
    let sym_path = server_sym_path(&state);
    match brain::session_new(
        &tree_root,
        &sym_path,
        &project_path,
        session_type,
        &label,
        actor.as_str(),
        agent.as_deref(),
        model.as_deref(),
        &body.agents,
    ) {
        Ok(session_path) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({
                "ok": true,
                "session_path": session_path.to_slash_string(),
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct SessionJoinBody {
    session_path: Option<String>,
    /// Sub-agent branch label within the session to claim. Branch must have
    /// been pre-created by `session_new`.
    #[serde(default, alias = "actor")]
    agent_label: Option<String>,
    user_email: Option<String>,
    agent: Option<String>,
    model: Option<String>,
}

async fn api_session_join(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<SessionJoinBody>,
) -> impl IntoResponse {
    let session_path_str = body.session_path.unwrap_or_default();
    let agent_label = body.agent_label.unwrap_or_default();
    let actor = maybe_user
        .0
        .as_ref()
        .map(|u| u.email.clone())
        .or(body.user_email)
        .unwrap_or_else(|| agent_label.clone());
    if actor.is_empty() {
        return ApiError::from(brain::WriteError::ActorRequired).into_response();
    }
    if agent_label.is_empty() {
        return ApiError::new("agent_label_required", "agent_label required")
            .with_suggestion("pass agent_label in request body")
            .into_response();
    }
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let agent = header_agent
        .or_else(|| maybe_user.0.as_ref().and_then(|u| u.api_key_label.clone()))
        .or(body.agent);
    let model = header_model.or(body.model);
    let session_path: crate::path::TreePath = match session_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if let Some(resp) = guard_write(&state, &maybe_user, &session_path.to_slash_string()).await {
        return resp;
    }
    let tree_root = server_tree_root(&state);
    let sym_path = server_sym_path(&state);
    match brain::session_join(
        &tree_root,
        &sym_path,
        &session_path,
        &agent_label,
        actor.as_str(),
        agent.as_deref(),
        model.as_deref(),
    ) {
        Ok(branch) => Json(serde_json::json!({
            "ok": true,
            "session_path": session_path.to_slash_string(),
            "user_email": actor,
            "branch": branch,
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct BranchCreateBody {
    exom_path: Option<String>,
    branch_name: Option<String>,
    actor: Option<String>,
}

async fn api_branch_create(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<BranchCreateBody>,
) -> impl IntoResponse {
    let exom_path_str = body.exom_path.unwrap_or_default();
    let branch_name = body.branch_name.unwrap_or_default();
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        body.actor.unwrap_or_default()
    };
    let exom_path: crate::path::TreePath = match exom_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if let Some(resp) = guard_write(&state, &maybe_user, &exom_path.to_slash_string()).await {
        return resp;
    }
    if branch_name.is_empty() {
        return ApiError::new("branch_name_required", "branch_name required").into_response();
    }
    if actor.is_empty() {
        return ApiError::new("actor_required", "actor required")
            .with_suggestion("pass actor in request body")
            .into_response();
    }
    let tree_root = server_tree_root(&state);
    let sym_path = server_sym_path(&state);
    let disk = exom_path.to_disk_path(&tree_root);
    let meta = match crate::exom::read_meta(&disk) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::new("no_such_exom", format!("no such exom {exom_path_str}"))
                .with_path(exom_path_str)
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    if let Some(sess) = &meta.session {
        if sess.initiated_by != actor {
            return ApiError::new(
                "not_orchestrator",
                format!(
                    "only the session orchestrator ({}) may create branches",
                    sess.initiated_by
                ),
            )
            .with_actor(actor)
            .into_response();
        }
    }
    match brain::create_branch(&tree_root, &sym_path, &exom_path, &branch_name) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({
                "ok": true,
                "exom_path": exom_path.to_slash_string(),
                "branch_name": branch_name,
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct RenameBody {
    path: Option<String>,
    new_segment: Option<String>,
}

async fn api_rename(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let new_segment = body.new_segment.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if state.auth_store.is_some() && path.len() == 1 {
        return ApiError::new(
            "namespace_root_immutable",
            "cannot rename a user namespace root",
        )
        .with_status(403)
        .into_response();
    }
    if let Some(resp) = guard_write(&state, &maybe_user, &path.to_slash_string()).await {
        return resp;
    }
    let tree_root = server_tree_root(&state);
    let disk = path.to_disk_path(&tree_root);
    if crate::tree::classify(&disk) == crate::tree::NodeKind::Exom {
        if let Ok(meta) = crate::exom::read_meta(&disk) {
            if meta.kind == crate::exom::ExomKind::Session {
                return ApiError::new(
                    "session_id_immutable",
                    "cannot rename session id; use session/label to change the display label",
                )
                .into_response();
            }
        }
    }
    match crate::tree::rename_last_segment(&tree_root, &path, &new_segment) {
        Ok(new_path) => {
            if let Some(ref auth_store) = state.auth_store {
                let old_slash = path.to_slash_string();
                let new_slash = new_path.to_slash_string();
                auth_store.update_share_paths(&old_slash, &new_slash).await;
            }
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "new_path": new_path.to_slash_string()}))
                .into_response()
        }
        Err(e) => ApiError::new("rename_failed", e).into_response(),
    }
}

#[derive(Deserialize)]
struct AssertFactBody {
    #[serde(default)]
    exom: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default, alias = "user_email")]
    actor: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    model: Option<String>,
    fact_id: Option<String>,
    predicate: String,
    /// Typed fact value. Accepts JSON:
    ///   * `20` → `FactValue::I64`
    ///   * `"Basil"` → `FactValue::Str`
    ///   * `{"$sym": "active"}` → `FactValue::Sym`
    value: crate::fact_value::FactValue,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    provenance: Option<String>,
    #[serde(default)]
    valid_from: Option<String>,
    #[serde(default)]
    valid_to: Option<String>,
}

fn default_confidence() -> f64 {
    1.0
}

async fn api_assert_fact(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(req): Json<AssertFactBody>,
) -> impl IntoResponse {
    let exom_raw = req.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if let Err(e) = brain::validate_predicate_name(&req.predicate) {
        return ApiError::new("invalid_predicate", e.to_string()).into_response();
    }

    if let Some(resp) = guard_write(&state, &maybe_user, &exom_slash).await {
        return resp;
    }

    let (header_agent, header_model) = read_attribution_headers(&headers);
    let actor = maybe_user
        .0
        .as_ref()
        .map(|u| u.email.clone())
        .or_else(|| req.actor.clone())
        .unwrap_or_default();
    if actor.is_empty() {
        return ApiError::from(brain::WriteError::ActorRequired).into_response();
    }
    let agent = header_agent
        .or_else(|| maybe_user.0.as_ref().and_then(|u| u.api_key_label.clone()))
        .or_else(|| req.agent.clone());
    let model = header_model.or_else(|| req.model.clone());
    let write_ctx = MutationContext {
        user_email: Some(actor.clone()),
        agent,
        model,
        session: None,
    };

    let branch_str = req
        .branch
        .as_deref()
        .unwrap_or(brain::MAIN_BRANCH)
        .to_string();

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        let sym_path = server_sym_path(&state);
        if let Err(e) = brain::precheck_write(
            &tree_root,
            &sym_path,
            &exom_path,
            &branch_str,
            actor.as_str(),
            write_ctx.agent.as_deref(),
            write_ctx.model.as_deref(),
        ) {
            return ApiError::from(e).into_response();
        }
        evict_cached_exom(&state, &exom_slash);
    }

    let fact_id = req.fact_id.clone().unwrap_or_else(|| req.predicate.clone());
    let provenance = req
        .source
        .as_deref()
        .or(req.provenance.as_deref())
        .unwrap_or("api")
        .to_string();
    let predicate = req.predicate.clone();
    let value = req.value.clone();
    let confidence = req.confidence;
    let valid_from = req.valid_from.clone();
    let valid_to = req.valid_to.clone();

    let result = mutate_exom_async(&state, &exom_slash, |ex| {
        ex.brain.assert_fact(
            &branch_str,
            &fact_id,
            &predicate,
            value.clone(),
            confidence,
            &provenance,
            valid_from.as_deref(),
            valid_to.as_deref(),
            &write_ctx,
        )
    })
    .await;

    match result {
        Ok(tx_id) => {
            if state.tree_root.is_some()
                && matches!(
                    predicate.as_str(),
                    "session/label" | "session/closed_at" | "session/archived_at"
                )
            {
                let tree_root = server_tree_root(&state);
                let display_value = value.display();
                if let Err(e) = brain::mirror_session_meta_to_disk(
                    &tree_root,
                    &exom_path,
                    &predicate,
                    &display_value,
                ) {
                    eprintln!(
                        "[ray-exomem] mirror_session_meta_to_disk failed (best-effort): {}",
                        e
                    );
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "fact_id": fact_id,
                "tx_id": tx_id
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Query helpers (shared by query + eval + expand-query handlers)
// ---------------------------------------------------------------------------

pub struct ExpandedQuery {
    pub original_source: String,
    pub normalized_query: String,
    pub expanded_query: String,
    #[allow(dead_code)]
    pub exom_name: String,
}

fn lower_query_request(
    source: &str,
    default_exom: Option<&str>,
    surface: &str,
) -> anyhow::Result<CanonicalQuery> {
    let forms = rayfall_ast::parse_forms(source)?;
    if forms.len() != 1 {
        anyhow::bail!("{surface} expects exactly one top-level Rayfall query form");
    }
    let lowered = rayfall_ast::lower_top_level(
        &forms[0],
        LoweringOptions {
            default_query_exom: default_exom,
            default_rule_exom: Some(DEFAULT_EXOM),
        },
    )?;
    match lowered.as_slice() {
        [CanonicalForm::Query(query)] => Ok(query.clone()),
        [CanonicalForm::AssertFact(_) | CanonicalForm::RetractFact(_) | CanonicalForm::Rule(_)] => {
            anyhow::bail!("{surface} only accepts a Rayfall (query ...) form")
        }
        [] => anyhow::bail!("{surface} expects exactly one top-level Rayfall query form"),
        _ => anyhow::bail!("{surface} accepts exactly one logical query form"),
    }
}

enum EvalForm {
    Canonical(CanonicalForm),
    Raw(String),
}

fn lower_eval_forms(source: &str) -> anyhow::Result<Vec<EvalForm>> {
    let forms = rayfall_ast::parse_forms(source)?;
    let mut out = Vec::new();
    for form in forms {
        match form
            .as_list()
            .and_then(|items| items.first())
            .and_then(|item| item.as_symbol())
        {
            Some("assert-fact" | "retract-fact" | "rule" | "query" | "in-exom") => {
                let lowered = rayfall_ast::lower_top_level(
                    &form,
                    LoweringOptions {
                        default_query_exom: None,
                        default_rule_exom: Some(DEFAULT_EXOM),
                    },
                )?;
                out.extend(lowered.into_iter().map(EvalForm::Canonical));
            }
            _ => out.push(EvalForm::Raw(form.emit())),
        }
    }
    Ok(out)
}

fn expand_canonical_query(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    original_source: String,
    query: &CanonicalQuery,
) -> anyhow::Result<ExpandedQuery> {
    let exom_name = query.exom.clone();
    let es = exoms
        .get(&exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    let rule_inline_bodies: Vec<String> = combined_rules(&exom_name, &es.rules)?
        .into_iter()
        .map(|rule| rule.inline_body)
        .collect();

    // Pin body-atom string literals to sym tags when the schema demands it.
    // Two paths:
    //   - direct EAV `(?e 'attr "literal")` — rewritten in place.
    //   - rule-call `(rule-name ?id "literal" ?v)` — the literal arrives at
    //     the inlined rule body's value slot only after rayforce2 expands
    //     the rule, so we rewrite at the call site using a per-rule head-
    //     param→attr map derived from each rule's inline body.
    let mut rule_attr_map: std::collections::HashMap<String, Vec<Option<String>>> =
        std::collections::HashMap::new();
    for body in &rule_inline_bodies {
        if let Some((name, attrs)) = rayfall_ast::derive_rule_param_attrs(body) {
            rule_attr_map.insert(name, attrs);
        }
    }
    let mut query = query.clone();
    query.rewrite_body_literals_with_schema_and_rules(
        |attr| es.brain.value_kind_for_attr(attr),
        |name| rule_attr_map.get(name).cloned(),
    );

    let normalized_query = query.emit();
    let expanded_query =
        rayfall_parser::rewrite_query_with_rules(&normalized_query, &rule_inline_bodies)?;
    let _ = engine; // engine is accessed separately for eval
    Ok(ExpandedQuery {
        original_source,
        normalized_query,
        expanded_query,
        exom_name,
    })
}

pub fn expand_query(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    source: &str,
    default_exom: Option<&str>,
    surface: &str,
) -> anyhow::Result<ExpandedQuery> {
    let query = lower_query_request(source, default_exom, surface)?;
    expand_canonical_query(exoms, engine, source.to_string(), &query)
}

/// Like `expand_query`, but also runs `validate_query_body` so the caller
/// gets `unknown relation 'X' (did you mean 'Y'?)` instead of a silent
/// empty table when the query references a relation the engine hasn't been
/// taught about. Use this for callers (MCP, ad-hoc tools) that want the
/// same friendly diagnostics the HTTP `/api/query` route gives.
pub fn expand_query_validated(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    source: &str,
    default_exom: Option<&str>,
    surface: &str,
) -> anyhow::Result<ExpandedQuery> {
    let query = lower_query_request(source, default_exom, surface)?;
    let expanded = expand_canonical_query(exoms, engine, source.to_string(), &query)?;
    let es = exoms
        .get(&expanded.exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", expanded.exom_name))?;
    let known = known_relations_for_exom(&expanded.exom_name, &es.rules);
    let arities = known_relation_arities_for_exom(&expanded.exom_name, &es.rules);
    validate_query_body(&query, &known, &arities)?;
    Ok(expanded)
}

/// Body-position operators that aren't relation references. Logical
/// compounds descend into their children; comparison and `between` are
/// terminal (their args are values, not predicate names).
fn is_logical_body_op(name: &str) -> bool {
    matches!(name, "and" | "or" | "not")
}

fn is_cmp_body_op(name: &str) -> bool {
    matches!(name, "<" | "<=" | ">" | ">=" | "=" | "!=" | "between")
}

/// Aggregate forms: `(sum ?v pred [col] [by ?k key_col ...])`. The
/// second argument is the source relation and gets validated; everything
/// else is a var or column index.
fn is_aggregate_body_op(name: &str) -> bool {
    matches!(name, "count" | "sum" | "min" | "max" | "avg")
}

/// Compute the relation names a rule body in this exom may reference
/// without being treated as a typo: the three typed-EDBs, the exom's own
/// datoms table, all builtin view IDBs, and any user-defined rule heads.
fn known_relations_for_exom(exom: &str, rules: &[ParsedRule]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    names.insert(storage::FACTS_I64_ENV.to_string());
    names.insert(storage::FACTS_STR_ENV.to_string());
    names.insert(storage::FACTS_SYM_ENV.to_string());
    names.insert(exom.to_string());
    for builtin in system_schema::builtin_views(exom) {
        names.insert(builtin.name);
    }
    for r in rules {
        names.insert(r.head_predicate.clone());
    }
    names
}

/// Like `known_relations_for_exom`, but maps name → arity so callers can
/// reject calls that pass the wrong number of args.
fn known_relation_arities_for_exom(
    exom: &str,
    rules: &[ParsedRule],
) -> std::collections::BTreeMap<String, usize> {
    let mut arities = std::collections::BTreeMap::new();
    // Typed EDBs and the datoms table are arity-3 (entity / attr / value).
    arities.insert(storage::FACTS_I64_ENV.to_string(), 3);
    arities.insert(storage::FACTS_STR_ENV.to_string(), 3);
    arities.insert(storage::FACTS_SYM_ENV.to_string(), 3);
    arities.insert(exom.to_string(), 3);
    for builtin in system_schema::builtin_views(exom) {
        arities.insert(builtin.name, builtin.arity);
    }
    for r in rules {
        // If a user redefines the same head with multiple arities, the
        // rule registry preserves both — keep the largest so we accept
        // the broadest call shape.
        arities
            .entry(r.head_predicate.clone())
            .and_modify(|a| *a = (*a).max(r.head_arity))
            .or_insert(r.head_arity);
    }
    arities
}

/// Walk the `(where ...)` clause of a canonical query and reject any
/// body atom whose leading symbol isn't a known relation, a logical
/// compound, a comparison, or an aggregate. Rayforce2 historically
/// silently produced an empty table when a rule body referenced an
/// unknown EDB; this surfaces the mistake as a 400 with a "did you
/// mean" hint before ever reaching the engine.
fn validate_query_body(
    query: &CanonicalQuery,
    known: &BTreeSet<String>,
    arities: &std::collections::BTreeMap<String, usize>,
) -> anyhow::Result<()> {
    for clause in &query.clauses {
        let Some(items) = clause.as_list() else {
            continue;
        };
        let Some(head) = items.first().and_then(|e| e.as_symbol()) else {
            continue;
        };
        if head == "where" {
            for atom in &items[1..] {
                validate_body_atom(atom, known, arities)?;
            }
        }
    }
    Ok(())
}

fn validate_body_atom(
    atom: &rayfall_ast::Expr,
    known: &BTreeSet<String>,
    arities: &std::collections::BTreeMap<String, usize>,
) -> anyhow::Result<()> {
    let Some(items) = atom.as_list() else {
        return Ok(());
    };
    let Some(first) = items.first() else {
        return Ok(());
    };
    let Some(sym) = first.as_symbol() else {
        return Ok(());
    };
    if sym.starts_with('?') {
        return Ok(());
    }
    if is_logical_body_op(sym) {
        for child in &items[1..] {
            validate_body_atom(child, known, arities)?;
        }
        return Ok(());
    }
    if is_cmp_body_op(sym) {
        return Ok(());
    }
    if is_aggregate_body_op(sym) {
        if let Some(pred) = items.get(2).and_then(|e| e.as_symbol()) {
            if !pred.starts_with('?') && !known.contains(pred) {
                return Err(unknown_relation_error(pred, known));
            }
        }
        return Ok(());
    }
    if !known.contains(sym) {
        return Err(unknown_relation_error(sym, known));
    }
    if let Some(&declared) = arities.get(sym) {
        let actual = items.len() - 1;
        if actual != declared {
            return Err(anyhow::anyhow!(
                "rule '{}' expects {} args, got {}",
                sym,
                declared,
                actual
            ));
        }
    }
    Ok(())
}

fn unknown_relation_error(unknown: &str, known: &BTreeSet<String>) -> anyhow::Error {
    let suggestion = known
        .iter()
        .find(|k| {
            k.as_str() == unknown
                || k.starts_with(unknown)
                || unknown.starts_with(k.as_str())
                || k.contains(unknown)
                || unknown.contains(k.as_str())
        })
        .cloned();
    match suggestion {
        Some(hint) if hint != unknown => {
            anyhow::anyhow!("unknown relation '{unknown}' in query body (did you mean '{hint}'?)")
        }
        _ => anyhow::anyhow!(
            "unknown relation '{unknown}' in query body; \
             bind it as an EDB, define a rule with head '{unknown}', \
             or pick from: {}",
            known.iter().cloned().collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Bind typed-fact tables and evaluate a prepared query, returning the
/// decoded output. Rejects non-TABLE engine returns instead of silently
/// formatting them as scalars: a `(query ...)` form that doesn't
/// produce a table means rayforce2 failed mid-eval, and the caller
/// should see that as an error.
///
/// Caller MUST hold `state.exoms.lock()` since `bind_typed_facts_for_exom`
/// mutates globally-shared env bindings.
fn execute_prepared_query(
    engine: &crate::backend::RayforceEngine,
    exoms: &HashMap<String, ExomState>,
    expanded: &ExpandedQuery,
) -> anyhow::Result<(String, Option<serde_json::Value>)> {
    bind_typed_facts_for_exom(engine, exoms, &expanded.exom_name)?;
    let raw = engine.eval_raw(&expanded.expanded_query)?;
    let obj_type = unsafe { ffi::ray_obj_type(raw.as_ptr()) };
    if obj_type != ffi::RAY_TABLE {
        anyhow::bail!(
            "query evaluator returned a non-table result (ray obj type {obj_type}); \
             this usually means the rule failed to compile or evaluate inside rayforce2"
        );
    }
    let decoded = storage::decode_query_table(&raw, &expanded.normalized_query)?;
    Ok((storage::format_decoded_query_table(&decoded), Some(decoded)))
}

fn eval_query_form(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    query: &CanonicalQuery,
) -> anyhow::Result<(String, Option<serde_json::Value>)> {
    let expanded = expand_canonical_query(exoms, engine, query.emit(), query)?;
    let es = exoms
        .get(&expanded.exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", expanded.exom_name))?;
    let known = known_relations_for_exom(&expanded.exom_name, &es.rules);
    let arities = known_relation_arities_for_exom(&expanded.exom_name, &es.rules);
    validate_query_body(query, &known, &arities)?;
    execute_prepared_query(engine, exoms, &expanded)
}

fn query_relation_rows(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    exom: &str,
    predicate: &str,
    arity: usize,
) -> anyhow::Result<Vec<Vec<serde_json::Value>>> {
    let vars: Vec<String> = (0..arity).map(|idx| format!("?v{idx}")).collect();
    let vars_joined = vars.join(" ");
    let source = format!("(query {exom} (find {vars_joined}) (where ({predicate} {vars_joined})))");
    let query = lower_query_request(&source, None, "schema relation sample")?;
    let expanded = expand_canonical_query(exoms, engine, source, &query)?;
    let (_, decoded) = execute_prepared_query(engine, exoms, &expanded)?;
    let decoded = decoded.unwrap_or(serde_json::Value::Null);
    Ok(decoded["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| row.as_array().cloned())
        .collect())
}

pub(crate) fn reconcile_engine(state: &AppState, exoms: &HashMap<String, ExomState>) {
    if let Err(e) = state.engine.reconcile_lang_env() {
        eprintln!("[ray-exomem] reconcile_lang_env failed: {}", e);
        return;
    }
    for (name, es) in exoms {
        let _ = state
            .engine
            .bind_named_db(storage::sym_intern(name), &es.datoms);
    }
}

// ---------------------------------------------------------------------------
// GET/POST /query
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct QueryParams {
    #[serde(rename = "exom")]
    _exom: Option<String>,
    /// Optional branch to evaluate the query against. When supplied, the
    /// engine's datoms binding is rebuilt from that branch's view for this
    /// request without mutating daemon-wide branch state.
    branch: Option<String>,
}

async fn api_query_get(
    State(state): State<Arc<AppState>>,
    _maybe_user: MaybeUser,
    Query(params): Query<QueryParams>,
) -> impl IntoResponse {
    // GET with body is non-standard; just return an error directing to POST
    let _ = params;
    let _ = state;
    ApiError::new(
        "use_post",
        "Use POST /api/query with a Rayfall query in the request body",
    )
    .with_status(405)
    .into_response()
}

async fn api_query_post(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(params): Query<QueryParams>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let source = String::from_utf8_lossy(&body).into_owned();
    let query = match lower_query_request(&source, None, "api/query") {
        Ok(q) => q,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    };
    if let Some(resp) = guard_read(&state, &maybe_user, &query.exom).await {
        return resp;
    }
    let target_branch = params
        .branch
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(brain::MAIN_BRANCH);
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &query.exom, tree_root, sym_path) {
        Ok(es) => es,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    if !es.brain.branch_exists(target_branch) {
        return ApiError::new(
            "unknown_branch",
            format!("unknown branch '{target_branch}'"),
        )
        .with_status(400)
        .into_response();
    }
    if let Err(e) = rebind_datoms_only(&state, &mut exoms, &query.exom, target_branch) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("rebind failed: {e}")
            })),
        )
            .into_response();
    }
    let outcome: axum::response::Response = 'body: {
        let expanded = match expand_canonical_query(&exoms, &state.engine, source.clone(), &query) {
            Ok(e) => e,
            Err(err) => {
                break 'body (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": err.to_string()})),
                )
                    .into_response();
            }
        };
        let es = match exoms.get(&expanded.exom_name) {
            Some(es) => es,
            None => {
                break 'body (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("unknown exom '{}'", expanded.exom_name)
                    })),
                )
                    .into_response();
            }
        };
        let known = known_relations_for_exom(&expanded.exom_name, &es.rules);
        let arities = known_relation_arities_for_exom(&expanded.exom_name, &es.rules);
        if let Err(err) = validate_query_body(&query, &known, &arities) {
            break 'body (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
        match execute_prepared_query(&state.engine, &exoms, &expanded) {
            Ok((output, decoded)) => {
                let mut payload = serde_json::json!({
                    "ok": true,
                    "output": output,
                    "mutated_exom": serde_json::Value::Null,
                    "mutation_count": 0,
                    "normalized_query": expanded.normalized_query,
                    "expanded_query": expanded.expanded_query
                });
                if let Some(decoded) = decoded {
                    if let (Some(dst), Some(src)) = (payload.as_object_mut(), decoded.as_object()) {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
                Json(payload).into_response()
            }
            Err(err) => {
                reconcile_engine(&state, &exoms);
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": err.to_string()})),
                )
                    .into_response()
            }
        }
    };

    drop(exoms);
    outcome
}

// ---------------------------------------------------------------------------
// POST /expand-query
// ---------------------------------------------------------------------------

async fn api_expand_query(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let source = String::from_utf8_lossy(&body).into_owned();
    let query = match lower_query_request(&source, None, "api/expand-query") {
        Ok(q) => q,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    };
    if let Some(resp) = guard_read(&state, &maybe_user, &query.exom).await {
        return resp;
    }
    let exoms = state.exoms.lock().unwrap();
    match expand_canonical_query(&exoms, &state.engine, source.clone(), &query) {
        Ok(expanded) => Json(serde_json::json!({
            "ok": true,
            "original_source": expanded.original_source,
            "normalized_query": expanded.normalized_query,
            "expanded_query": expanded.expanded_query,
            "exom": expanded.exom_name,
        }))
        .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/eval
// ---------------------------------------------------------------------------

async fn api_eval(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(params): Query<QueryParams>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let ctx = match maybe_user.0.as_ref() {
        Some(user) => MutationContext::from_user(user, header_agent, header_model),
        None => MutationContext {
            user_email: None,
            agent: header_agent,
            model: header_model,
            session: None,
        },
    };
    let branch_id = params
        .branch
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| brain::MAIN_BRANCH.to_string());
    api_eval_inner(state, maybe_user, ctx, branch_id, body).await
}

async fn api_eval_inner(
    state: Arc<AppState>,
    maybe_user: MaybeUser,
    ctx: MutationContext,
    branch_id: String,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let source = String::from_utf8_lossy(&body).into_owned();
    let forms = match lower_eval_forms(&source) {
        Ok(forms) => forms,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    };
    if let Some(ref auth_store) = state.auth_store {
        if let Some(ref user) = maybe_user.0 {
            let canonical_forms: Vec<_> = forms
                .iter()
                .filter_map(|f| match f {
                    EvalForm::Canonical(c) => Some(c.clone()),
                    _ => None,
                })
                .collect();
            if !canonical_forms.is_empty() {
                let state_for_owner = state.clone();
                if let Err(e) = crate::auth::access::authorize_rayfall(
                    user,
                    &canonical_forms,
                    auth_store,
                    |path| lookup_owner(&state_for_owner, path),
                )
                .await
                {
                    return ApiError::new("forbidden", e.to_string())
                        .with_status(403)
                        .into_response();
                }
            }
        }
    }
    let mut last_result = String::new();
    let mut last_decoded: Option<serde_json::Value> = None;

    for form in forms {
        let exec: anyhow::Result<()> = match form {
            EvalForm::Canonical(CanonicalForm::AssertFact(mutation)) => {
                let exom = mutation.exom.clone();
                let pred = mutation.predicate.clone();
                let fact_id = mutation.fact_id.clone();
                let value = mutation.value.clone();
                let r = (|| {
                    let mut exoms = state.exoms.lock().unwrap();
                    let es = exoms
                        .get_mut(&exom)
                        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom))?;
                    es.brain.assert_fact(
                        &branch_id,
                        &fact_id,
                        &pred,
                        &value,
                        1.0,
                        "rayfall-eval",
                        None,
                        None,
                        &ctx,
                    )?;
                    es.datoms = storage::build_datoms_table(&es.brain, &branch_id)?;
                    state
                        .engine
                        .bind_named_db(storage::sym_intern(&exom), &es.datoms)?;
                    if let Some(disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                        let body_str: String = es
                            .rules
                            .iter()
                            .map(|r| r.full_text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !body_str.is_empty() {
                            std::fs::write(disk.join("rules.ray"), format!("{}\n", body_str))?;
                        }
                    }
                    let _ = state.sse_tx.send((
                        Some(exom.clone()),
                        format!(r#"{{"v":1,"kind":"memory","op":"eval_assert_fact","exom":"{}","user_email":"{}","predicate":"{}"}}"#, exom, ctx.user_email.as_deref().unwrap_or("system"), pred),
                    ));
                    Ok(())
                })();
                r
            }
            EvalForm::Canonical(CanonicalForm::RetractFact(mutation)) => {
                let exom = mutation.exom.clone();
                let pred = mutation.predicate.clone();
                let fact_id = mutation.fact_id.clone();
                let value = mutation.value.clone();
                let r = (|| {
                    let mut exoms = state.exoms.lock().unwrap();
                    let es = exoms
                        .get_mut(&exom)
                        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom))?;
                    es.brain
                        .retract_fact_exact(&branch_id, &fact_id, &pred, &value, &ctx)?;
                    es.datoms = storage::build_datoms_table(&es.brain, &branch_id)?;
                    state
                        .engine
                        .bind_named_db(storage::sym_intern(&exom), &es.datoms)?;
                    if let Some(_disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                    }
                    let _ = state.sse_tx.send((
                        Some(exom.clone()),
                        format!(r#"{{"v":1,"kind":"memory","op":"eval_retract_fact","exom":"{}","user_email":"{}","predicate":"{}"}}"#, exom, ctx.user_email.as_deref().unwrap_or("system"), pred),
                    ));
                    Ok(())
                })();
                r
            }
            EvalForm::Canonical(CanonicalForm::Rule(rule)) => {
                let full = rule.emit();
                let exom_name = rule.exom.clone();
                let r = (|| {
                    let pr = rules::parse_rule_line(&full, ctx.clone(), brain::now_iso())?;
                    let mut exoms = state.exoms.lock().unwrap();
                    let es = exoms
                        .get_mut(&exom_name)
                        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
                    es.rules.push(pr);
                    es.datoms = storage::build_datoms_table(&es.brain, &branch_id)?;
                    state
                        .engine
                        .bind_named_db(storage::sym_intern(&exom_name), &es.datoms)?;
                    if let Some(disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                        let body_str: String = es
                            .rules
                            .iter()
                            .map(|r| r.full_text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        std::fs::write(disk.join("rules.ray"), format!("{}\n", body_str))?;
                    }
                    let _ = state.sse_tx.send((
                        Some(exom_name.clone()),
                        format!(
                            r#"{{"v":1,"kind":"memory","op":"rule_append","exom":"{}","user_email":"{}"}}"#,
                            exom_name,
                            ctx.user_email.as_deref().unwrap_or("system")
                        ),
                    ));
                    Ok(())
                })();
                r
            }
            EvalForm::Canonical(CanonicalForm::Query(query)) => {
                let mut exoms = state.exoms.lock().unwrap();
                if let Err(e) = rebind_datoms_only(&state, &mut exoms, &query.exom, &branch_id) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": e.to_string()})),
                    )
                        .into_response();
                }
                match eval_query_form(&exoms, &state.engine, &query) {
                    Ok((output, decoded)) => {
                        last_result = output;
                        last_decoded = decoded;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            EvalForm::Raw(source) => match state.engine.eval(&source) {
                Ok(out) => {
                    last_result = out;
                    last_decoded = None;
                    Ok(())
                }
                Err(e) => Err(e),
            },
        };

        if let Err(err) = exec {
            let exoms = state.exoms.lock().unwrap();
            reconcile_engine(&state, &exoms);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    }

    let mut payload = serde_json::json!({
        "ok": true,
        "output": last_result,
        "mutated_exom": serde_json::Value::Null,
        "mutation_count": 0
    });
    if let Some(decoded) = last_decoded {
        if let (Some(dst), Some(src)) = (payload.as_object_mut(), decoded.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
    Json(payload).into_response()
}

// ---------------------------------------------------------------------------
// POST /actions/evaluate (noop)
// ---------------------------------------------------------------------------

async fn api_evaluate_noop() -> impl IntoResponse {
    Json(serde_json::json!({"ok": true, "new_derivations": 0, "duration_ms": 0}))
}

// ---------------------------------------------------------------------------
// GET /facts/valid-at
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExomQuery {
    exom: Option<String>,
}

#[derive(Deserialize)]
struct BranchExomQuery {
    exom: Option<String>,
    branch: Option<String>,
}

#[derive(Deserialize)]
struct ValidAtQuery {
    exom: Option<String>,
    timestamp: Option<String>,
    branch: Option<String>,
}

async fn api_facts_valid_at(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ValidAtQuery>,
) -> impl IntoResponse {
    let timestamp = match q.timestamp.as_deref().filter(|s| !s.is_empty()) {
        Some(t) => t.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "timestamp query parameter is required"})),
            )
                .into_response();
        }
    };
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let entries: Vec<_> = es
        .brain
        .facts_valid_at_on_branch(&branch_id, &timestamp)
        .iter()
        .map(|f| fact_to_json(f))
        .collect();
    Json(serde_json::json!({
        "ok": true,
        "timestamp": timestamp,
        "facts": entries,
        "count": entries.len()
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /facts/bitemporal
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BitemporalQuery {
    exom: Option<String>,
    timestamp: Option<String>,
    tx_id: Option<u64>,
}

async fn api_facts_bitemporal(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<BitemporalQuery>,
) -> impl IntoResponse {
    let timestamp = q.timestamp.as_deref().unwrap_or("");
    let tx_id = q.tx_id.unwrap_or(0);
    if timestamp.is_empty() || tx_id == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "timestamp and tx_id query parameters are required"})),
        )
            .into_response();
    }
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let entries: Vec<_> = es
        .brain
        .facts_bitemporal(tx_id, timestamp)
        .iter()
        .map(|f| fact_to_json(f))
        .collect();
    Json(serde_json::json!({
        "ok": true,
        "timestamp": timestamp,
        "tx_id": tx_id,
        "facts": entries,
        "count": entries.len()
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /facts/:id
// ---------------------------------------------------------------------------

async fn api_fact_detail(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;
    let history = brain.fact_history(&id);
    let tx_index: std::collections::HashMap<crate::brain::TxId, &crate::brain::Tx> =
        brain.transactions().iter().map(|t| (t.tx_id, t)).collect();
    match history.last() {
        Some(f) => {
            let status = if f.revoked_by_tx.is_some() {
                "retracted"
            } else {
                "active"
            };
            let touch_history: Vec<_> = brain
                .explain(&id)
                .iter()
                .map(|tx| {
                    serde_json::json!({
                        "event_id": format!("tx{}", tx.tx_id),
                        "event_type": tx.action.to_string(),
                        "user_email": tx.user_email,
                        "agent": tx.agent,
                        "model": tx.model,
                    })
                })
                .collect();
            let creator = tx_index.get(&f.created_by_tx);
            Json(serde_json::json!({
                "fact": {
                    "id": f.fact_id,
                    "predicate": f.predicate,
                    "tuple": [f.fact_id, f.predicate, f.value, f.confidence],
                    "interval": {
                        "start": f.valid_from,
                        "end": f.valid_to.as_deref().unwrap_or("inf")
                    },
                    "status": status,
                    "cluster_ids": [format!("cluster:{}", f.predicate)]
                },
                "metadata": {
                    "predicate": f.predicate,
                    "value": f.value,
                    "confidence": f.confidence,
                    "provenance": f.provenance,
                    "created_at": f.created_at,
                    "valid_from": f.valid_from,
                    "valid_to": f.valid_to,
                    "created_by": format!("tx/{}", f.created_by_tx),
                    "superseded_by": f.superseded_by_tx.map(|tx| format!("tx/{}", tx)),
                    "revoked_by": f.revoked_by_tx.map(|tx| format!("tx/{}", tx)),
                    "user_email": creator.and_then(|t| t.user_email.as_deref()),
                    "agent": creator.and_then(|t| t.agent.as_deref()),
                    "model": creator.and_then(|t| t.model.as_deref()),
                },
                "provenance": {"type": "base"},
                "touch_history": touch_history
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "fact not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Fact JSON helper
// ---------------------------------------------------------------------------

fn fact_to_json(f: &crate::brain::Fact) -> serde_json::Value {
    serde_json::json!({
        "fact_id": f.fact_id,
        "predicate": f.predicate,
        "value": f.value,
        "confidence": f.confidence,
        "valid_from": f.valid_from,
        "valid_to": f.valid_to,
        "created_by_tx": f.created_by_tx,
        "provenance": f.provenance
    })
}

fn resolve_branch_key<'a>(brain: &'a Brain, key: &str) -> Option<&'a str> {
    brain
        .branches()
        .iter()
        .find(|b| b.branch_id == key || b.name == key)
        .map(|b| b.branch_id.as_str())
}

fn resolve_view_branch(brain: &Brain, key: Option<&str>) -> Result<String, String> {
    let requested = key.filter(|s| !s.is_empty()).unwrap_or(brain::MAIN_BRANCH);
    resolve_branch_key(brain, requested)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("no branch matching {:?}", requested))
}

fn fact_json_enriched(brain: &Brain, f: &crate::brain::Fact) -> serde_json::Value {
    let tx = brain
        .transactions()
        .iter()
        .find(|t| t.tx_id == f.created_by_tx);
    let (actor, branch_id, tx_time) = match tx {
        Some(t) => (
            t.user_email.as_deref().unwrap_or(""),
            t.branch_id.as_str(),
            t.tx_time.as_str(),
        ),
        None => ("", "", ""),
    };
    let branch_name = brain
        .branches()
        .iter()
        .find(|b| b.branch_id == branch_id)
        .map(|b| b.name.as_str())
        .unwrap_or("");
    serde_json::json!({
        "fact_id": f.fact_id,
        "predicate": f.predicate,
        "value": f.value,
        "confidence": f.confidence,
        "valid_from": f.valid_from,
        "valid_to": f.valid_to,
        "created_by_tx": f.created_by_tx,
        "provenance": f.provenance,
        "actor": actor,
        "branch_id": branch_id,
        "branch_name": branch_name,
        "tx_time": tx_time,
    })
}

#[derive(Deserialize)]
struct FactsListQuery {
    exom: Option<String>,
    /// Branch id or name; omit = main.
    branch: Option<String>,
    /// Deduped union of visible facts on every non-archived branch, sorted by `tx_time`.
    #[serde(default)]
    all_branches: bool,
}

async fn api_facts_list(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<FactsListQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;

    let entries: Vec<serde_json::Value> = if q.all_branches {
        let mut seen: HashMap<String, &crate::brain::Fact> = HashMap::new();
        for b in brain.branches() {
            if b.archived {
                continue;
            }
            for f in brain.facts_on_branch(&b.branch_id) {
                seen.entry(f.fact_id.clone()).or_insert(f);
            }
        }
        let mut rows: Vec<_> = seen
            .into_values()
            .map(|f| fact_json_enriched(brain, f))
            .collect();
        rows.sort_by(|a, b| {
            let ta = a["tx_time"].as_str().unwrap_or("");
            let tb = b["tx_time"].as_str().unwrap_or("");
            ta.cmp(tb).then_with(|| {
                let fa = a["fact_id"].as_str().unwrap_or("");
                let fb = b["fact_id"].as_str().unwrap_or("");
                fa.cmp(fb)
            })
        });
        rows
    } else {
        let bid = match resolve_view_branch(brain, q.branch.as_deref()) {
            Ok(id) => id,
            Err(e) => return ApiError::new("unknown_branch", e).into_response(),
        };
        brain
            .facts_on_branch(&bid)
            .into_iter()
            .map(|f| fact_json_enriched(brain, f))
            .collect()
    };

    Json(serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "facts": entries,
        "count": entries.len()
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /beliefs + GET /observations
// ---------------------------------------------------------------------------

async fn api_beliefs_list(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let tx_index: HashMap<crate::brain::TxId, &crate::brain::Tx> = es
        .brain
        .transactions()
        .iter()
        .map(|t| (t.tx_id, t))
        .collect();
    let beliefs: Vec<_> = es
        .brain
        .beliefs_on_branch(&branch_id)
        .into_iter()
        .map(|belief| {
            let tx = tx_index.get(&belief.created_by_tx).copied();
            let origin_branch = tx.map(|t| t.branch_id.as_str()).unwrap_or("");
            let branch_name = es
                .brain
                .branches()
                .iter()
                .find(|b| b.branch_id == origin_branch)
                .map(|b| b.name.as_str())
                .unwrap_or(origin_branch);
            serde_json::json!({
                "belief_id": belief.belief_id,
                "claim_text": belief.claim_text,
                "status": belief.status.to_string(),
                "confidence": belief.confidence,
                "supported_by": belief.supported_by,
                "rationale": belief.rationale,
                "valid_from": belief.valid_from,
                "valid_to": belief.valid_to,
                "created_by_tx": belief.created_by_tx,
                "tx_time": tx.map(|t| t.tx_time.as_str()).unwrap_or(""),
                "actor": tx.and_then(|t| t.user_email.as_deref()).unwrap_or(""),
                "branch_id": origin_branch,
                "branch_name": branch_name,
            })
        })
        .collect();
    let count = beliefs.len();
    Json(serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "branch": branch_id,
        "beliefs": beliefs,
        "count": count
    }))
    .into_response()
}

async fn api_observations_list(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let tx_index: HashMap<crate::brain::TxId, &crate::brain::Tx> = es
        .brain
        .transactions()
        .iter()
        .map(|t| (t.tx_id, t))
        .collect();
    let observations: Vec<_> = es
        .brain
        .observations()
        .iter()
        .map(|obs| {
            let tx = tx_index.get(&obs.tx_id).copied();
            let origin_branch = tx.map(|t| t.branch_id.as_str()).unwrap_or("");
            let branch_name = es
                .brain
                .branches()
                .iter()
                .find(|b| b.branch_id == origin_branch)
                .map(|b| b.name.as_str())
                .unwrap_or(origin_branch);
            serde_json::json!({
                "obs_id": obs.obs_id,
                "source_type": obs.source_type,
                "source_ref": obs.source_ref,
                "content": obs.content,
                "confidence": obs.confidence,
                "tags": obs.tags,
                "valid_from": obs.valid_from,
                "valid_to": obs.valid_to,
                "created_at": obs.created_at,
                "tx_id": obs.tx_id,
                "tx_time": tx.map(|t| t.tx_time.as_str()).unwrap_or(""),
                "actor": tx.and_then(|t| t.user_email.as_deref()).unwrap_or(""),
                "branch_id": origin_branch,
                "branch_name": branch_name,
            })
        })
        .collect();
    let count = observations.len();
    Json(serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "observations": observations,
        "count": count
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /branches + POST /branches
// ---------------------------------------------------------------------------

async fn api_list_branches(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branches: Vec<_> = es
        .brain
        .branches()
        .iter()
        .map(|b| {
            serde_json::json!({
                "branch_id": b.branch_id,
                "name": b.name,
                "parent_branch_id": b.parent_branch_id,
                "created_tx_id": b.created_tx_id,
                "archived": b.archived,
                "fact_count": es.brain.facts_on_branch(&b.branch_id).len(),
                "claimed_by_user_email": b.claimed_by_user_email,
                "claimed_by_agent": b.claimed_by_agent,
                "claimed_by_model": b.claimed_by_model,
            })
        })
        .collect();
    Json(serde_json::json!({"branches": branches})).into_response()
}

#[derive(Deserialize)]
struct CreateBranchBody {
    exom: Option<String>,
    branch_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_branch_id: Option<String>,
}

async fn api_create_branch(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateBranchBody>,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let ctx = MutationContext::from_user(user, header_agent, header_model);

    let exom_raw = body.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if let Some(resp) = guard_write(&state, &maybe_user, &exom_slash).await {
        return resp;
    }

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        let sym_path = server_sym_path(&state);
        if let Err(e) = brain::precheck_write(
            &tree_root,
            &sym_path,
            &exom_path,
            body.parent_branch_id
                .as_deref()
                .unwrap_or(brain::MAIN_BRANCH),
            user.email.as_str(),
            ctx.agent.as_deref(),
            ctx.model.as_deref(),
        ) {
            return ApiError::from(e).into_response();
        }
        evict_cached_exom(&state, &exom_slash);
    }

    let bid = body.branch_id.clone();
    let name = body.name.unwrap_or_else(|| bid.clone());
    let parent = body
        .parent_branch_id
        .unwrap_or_else(|| brain::MAIN_BRANCH.to_string());
    let bid2 = bid.clone();
    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        ex.brain.create_branch(&parent, &bid2, &name, &ctx)
    })
    .await;
    match result {
        Ok(tx_id) => Json(serde_json::json!({"branch_id": bid, "tx_id": tx_id})).into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /branches/:id
// ---------------------------------------------------------------------------

async fn api_branch_detail(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    match es
        .brain
        .branches()
        .iter()
        .find(|b| b.branch_id == branch_id)
    {
        Some(b) => Json(serde_json::json!({
            "branch_id": b.branch_id,
            "name": b.name,
            "parent_branch_id": b.parent_branch_id,
            "created_tx_id": b.created_tx_id,
            "archived": b.archived,
            "fact_count": es.brain.facts_on_branch(&b.branch_id).len(),
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("branch '{}' not found", branch_id)})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// DELETE /branches/:id
// ---------------------------------------------------------------------------

async fn api_delete_branch_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let _ctx = MutationContext::from_user(user, header_agent, header_model);
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_write(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let bid = branch_id.clone();
    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        ex.brain.archive_branch(&bid)?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => Json(serde_json::json!({"archived": branch_id})).into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /branches/:id/diff
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DiffQuery {
    exom: Option<String>,
    base: Option<String>,
}

async fn api_branch_diff_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<DiffQuery>,
) -> impl IntoResponse {
    let base = q.base.as_deref().unwrap_or("main");
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_facts = es.brain.facts_on_branch(&branch_id);
    let base_facts = es.brain.facts_on_branch(base);
    let base_map: HashMap<&str, &&crate::brain::Fact> =
        base_facts.iter().map(|f| (f.fact_id.as_str(), f)).collect();
    let branch_map: HashMap<&str, &&crate::brain::Fact> = branch_facts
        .iter()
        .map(|f| (f.fact_id.as_str(), f))
        .collect();
    let added: Vec<_> = branch_facts
        .iter()
        .filter(|f| !base_map.contains_key(f.fact_id.as_str()))
        .map(|f| fact_to_json(f))
        .collect();
    let removed: Vec<_> = base_facts
        .iter()
        .filter(|f| !branch_map.contains_key(f.fact_id.as_str()))
        .map(|f| fact_to_json(f))
        .collect();
    let changed: Vec<_> = branch_facts
        .iter()
        .filter_map(|f| {
            base_map
                .get(f.fact_id.as_str())
                .filter(|bf| bf.value != f.value)
                .map(|bf| {
                    serde_json::json!({
                        "fact_id": f.fact_id,
                        "predicate": f.predicate,
                        "base_value": bf.value,
                        "branch_value": f.value,
                    })
                })
        })
        .collect();
    Json(serde_json::json!({"added": added, "removed": removed, "changed": changed}))
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /branches/:id/merge
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct MergeBranchBody {
    exom: Option<String>,
    target_branch: Option<String>,
    #[serde(default)]
    policy: Option<String>,
}

async fn api_merge_branch_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(source_branch): AxumPath<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<MergeBranchBody>,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let ctx = MutationContext::from_user(user, header_agent, header_model);
    let policy = match body.policy.as_deref().unwrap_or("last-writer-wins") {
        "last-writer-wins" => MergePolicy::LastWriterWins,
        "keep-target" => MergePolicy::KeepTarget,
        "manual" => MergePolicy::Manual,
        _ => return ApiError::new("bad_policy", "unknown merge policy").into_response(),
    };
    let exom_raw = body.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_write(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let target_branch = body
        .target_branch
        .unwrap_or_else(|| brain::MAIN_BRANCH.to_string());
    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        let sym_path = server_sym_path(&state);
        if let Err(e) = brain::precheck_write(
            &tree_root,
            &sym_path,
            &exom_path,
            &target_branch,
            user.email.as_str(),
            ctx.agent.as_deref(),
            ctx.model.as_deref(),
        ) {
            return ApiError::from(e).into_response();
        }
        evict_cached_exom(&state, &exom_slash);
    }
    let src = source_branch.clone();
    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        ex.brain.merge_branch(&src, &target_branch, policy, &ctx)
    })
    .await;
    match result {
        Ok(merge_result) => Json(serde_json::json!({
            "added": merge_result.added,
            "conflicts": merge_result.conflicts.iter().map(|c| serde_json::json!({
                "fact_id": c.fact_id,
                "predicate": c.predicate,
                "source_value": c.source_value,
                "target_value": c.target_value,
            })).collect::<Vec<_>>(),
            "tx_id": merge_result.tx_id,
        }))
        .into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /explain
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExplainQuery {
    exom: Option<String>,
    branch: Option<String>,
    predicate: Option<String>,
    #[allow(dead_code)]
    terms: Option<String>,
}

async fn api_explain(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExplainQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let predicate = q.predicate.as_deref().unwrap_or("").to_string();
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };
    let defining_rules: Vec<&str> = all_rules
        .iter()
        .filter(|r| r.head_predicate == predicate)
        .map(|r| r.full_text.as_str())
        .collect();
    if !defining_rules.is_empty() {
        return Json(serde_json::json!({
            "kind": "derived",
            "predicate": predicate,
            "derived_by_rules": defining_rules,
        }))
        .into_response();
    }
    let facts = es.brain.facts_on_branch(&branch_id);
    let matching = facts
        .iter()
        .find(|f| f.predicate == predicate || f.fact_id == predicate);
    match matching {
        Some(f) => Json(serde_json::json!({
            "predicate": f.predicate,
            "terms": [f.fact_id, f.predicate, f.value],
            "tree": {
                "id": f.fact_id,
                "predicate": f.predicate,
                "terms": [f.fact_id, f.predicate, f.value],
                "kind": "base",
                "source": f.provenance,
                "confidence": f.confidence,
                "asserted_at": f.created_by_tx
            },
            "meta": {
                "source": f.provenance,
                "confidence": f.confidence,
                "asserted_at": f.created_by_tx
            }
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": format!("no fact matching predicate '{}'", predicate)}),
            ),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /actions/export (Rayfall text)
// ---------------------------------------------------------------------------

async fn api_export(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let facts = es.brain.facts_on_branch(&branch_id);
    let mut out = String::new();
    out.push_str(&format!(";; ray-exomem export — exom: {}\n", exom_slash));
    for f in &facts {
        out.push_str(&format!(
            "(assert-fact {} \"{}\" '{} \"{}\")",
            exom_slash,
            f.fact_id.replace('"', "\\\""),
            f.predicate.replace('"', "\\\""),
            f.value.display().replace('"', "\\\""),
        ));
        let valid_to_str = f.valid_to.as_deref().unwrap_or("inf");
        out.push_str(&format!(" ;; @valid[{}, {}]", f.valid_from, valid_to_str));
        out.push('\n');
    }
    for rule in &es.rules {
        out.push_str(&rule.full_text);
        out.push('\n');
    }
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        out,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /actions/export-json
// ---------------------------------------------------------------------------

async fn api_export_json(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let payload = serde_json::json!({
        "exom": exom_slash,
        "version": 1,
        "facts": es.brain.all_facts(),
        "transactions": es.brain.transactions(),
        "observations": es.brain.observations(),
        "beliefs": es.brain.all_beliefs(),
        "branches": es.brain.branches(),
        "rules": es.rules.iter().map(|r| &r.full_text).collect::<Vec<_>>(),
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(body) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/import-json
// ---------------------------------------------------------------------------

async fn api_import_json(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    use crate::brain::{Belief, Branch, Fact, Observation, Tx};

    #[derive(Deserialize)]
    struct ImportPayload {
        facts: Vec<Fact>,
        transactions: Vec<Tx>,
        #[serde(default)]
        observations: Vec<Observation>,
        #[serde(default)]
        beliefs: Vec<Belief>,
        #[serde(default)]
        branches: Vec<Branch>,
        #[serde(default)]
        rules: Vec<String>,
    }

    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let actor = user.email.clone();
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if let Some(resp) = guard_write(&state, &maybe_user, &exom_slash).await {
        return resp;
    }

    let payload: ImportPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return ApiError::new(
                "invalid_payload",
                format!("invalid JSON import payload: {}", e),
            )
            .into_response()
        }
    };

    let _ctx = MutationContext::from_user(user, header_agent, header_model);

    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        ex.brain.replace_state(
            payload.facts,
            payload.transactions,
            payload.observations,
            payload.beliefs,
            payload.branches,
        )?;
        let mut parsed_rules = Vec::new();
        for line in &payload.rules {
            let line = line.trim();
            if !line.is_empty() {
                parsed_rules.push(rules::parse_rule_line(
                    line,
                    MutationContext::default(),
                    String::new(),
                )?);
            }
        }
        ex.rules = parsed_rules;
        let n_facts = ex.brain.all_facts().len();
        let n_txs = ex.brain.transactions().len();
        Ok((n_facts, n_txs))
    })
    .await;

    match result {
        Ok((n_facts, n_txs)) => {
            // Re-bind all exoms after import to ensure consistency
            let exoms = state.exoms.lock().unwrap();
            reconcile_engine(&state, &exoms);
            let _ = state.sse_tx.send((
                Some(exom_slash.clone()),
                format!(
                    r#"{{"v":1,"kind":"memory","op":"import_json","exom":"{}","actor":"{}"}}"#,
                    exom_slash, actor
                ),
            ));
            Json(serde_json::json!({
                "ok": true,
                "imported": {"facts": n_facts, "transactions": n_txs}
            }))
            .into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/retract-all
// ---------------------------------------------------------------------------

async fn api_retract_all(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let ctx = MutationContext::from_user(user, header_agent, header_model);
    let actor = user.email.clone();
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if let Some(resp) = guard_owner(&state, &maybe_user, &exom_slash).await {
        return resp;
    }

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        let sym_path = server_sym_path(&state);
        if let Err(e) = brain::precheck_write(
            &tree_root,
            &sym_path,
            &exom_path,
            "main",
            user.email.as_str(),
            ctx.agent.as_deref(),
            ctx.model.as_deref(),
        ) {
            return ApiError::from(e).into_response();
        }
        evict_cached_exom(&state, &exom_slash);
    }

    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        let fact_ids: Vec<String> = ex
            .brain
            .facts_on_branch(brain::MAIN_BRANCH)
            .iter()
            .map(|f| f.fact_id.clone())
            .collect();
        let count = fact_ids.len();
        for id in &fact_ids {
            let _ = ex.brain.retract_fact(brain::MAIN_BRANCH, id, &ctx);
        }
        ex.rules.clear();
        Ok(count)
    })
    .await;

    match result {
        Ok(count) => {
            let _ = state.sse_tx.send((
                Some(exom_slash.clone()),
                format!(
                    r#"{{"v":1,"kind":"memory","op":"retract_all","exom":"{}","actor":"{}"}}"#,
                    exom_slash, actor
                ),
            ));
            Json(serde_json::json!({"ok": true, "tuples_removed": count})).into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/wipe
// ---------------------------------------------------------------------------

async fn api_wipe(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let user = match maybe_user.0.as_ref() {
        Some(u) => u,
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let actor = user.email.clone();
    let (header_agent, header_model) = read_attribution_headers(&headers);
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if let Some(resp) = guard_owner(&state, &maybe_user, &exom_slash).await {
        return resp;
    }

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        let sym_path = server_sym_path(&state);
        if let Err(e) = brain::precheck_write(
            &tree_root,
            &sym_path,
            &exom_path,
            "main",
            user.email.as_str(),
            header_agent.as_deref(),
            header_model.as_deref(),
        ) {
            return ApiError::from(e).into_response();
        }
        evict_cached_exom(&state, &exom_slash);
    }

    let result = mutate_exom_async(&state, &exom_slash, |ex| {
        ex.brain.reset();
        ex.rules.clear();
        if let Some(disk) = ex.exom_disk.as_ref() {
            if disk.exists() {
                std::fs::remove_dir_all(disk)?;
            }
            std::fs::create_dir_all(disk)?;
        }
        Ok(())
    })
    .await;

    match result {
        Ok(()) => {
            let _ = state.sse_tx.send((
                Some(exom_slash.clone()),
                format!(
                    r#"{{"v":1,"kind":"memory","op":"wipe","exom":"{}","actor":"{}"}}"#,
                    exom_slash, actor
                ),
            ));
            Json(serde_json::json!({"ok": true, "wiped": exom_slash})).into_response()
        }
        Err(e) => ApiError::new("error", e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /actions/factory-reset
// ---------------------------------------------------------------------------

async fn api_factory_reset(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(ref _auth_store) = state.auth_store {
        match maybe_user.0 {
            Some(ref user) if user.is_top_admin() => {}
            Some(_) => {
                return ApiError::new("forbidden", "factory-reset requires top-admin access")
                    .with_status(403)
                    .into_response();
            }
            None => {
                return ApiError::new("forbidden", "factory-reset requires top-admin access")
                    .with_status(403)
                    .into_response();
            }
        }
    }
    let actor = match maybe_user.0.as_ref() {
        Some(u) => u.email.clone(),
        None => return ApiError::from(brain::WriteError::ActorRequired).into_response(),
    };
    let _ = &headers;
    // The brain/tree wipe is fully sync. Wrap it in a closure so the
    // std::sync::MutexGuard on `state.exoms` is released before we await
    // the auth-store wipe below — otherwise the future would not be Send.
    let sync_result: Result<Vec<String>, ApiError> = (|| {
        let mut exoms = state.exoms.lock().unwrap();
        let old_names: Vec<String> = exoms.keys().cloned().collect();
        exoms.clear();

        // Nuke persisted splay state on disk before declaring success. Without
        // this the next lazy-load or daemon restart resurrects the "deleted"
        // exoms from disk, silently contradicting the response.
        if let Some(ref tree_root) = state.tree_root {
            if tree_root.exists() {
                let entries = std::fs::read_dir(tree_root).map_err(|e| {
                    ApiError::new("factory_reset_failed", e.to_string()).with_status(500)
                })?;
                for entry in entries.flatten() {
                    let p = entry.path();
                    let remove = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        std::fs::remove_dir_all(&p)
                    } else {
                        std::fs::remove_file(&p)
                    };
                    remove.map_err(|e| {
                        ApiError::new(
                            "factory_reset_failed",
                            format!("remove {}: {}", p.display(), e),
                        )
                        .with_status(500)
                    })?;
                }
            }
        }

        reconcile_engine(&state, &exoms);
        Ok(old_names)
    })();

    let old_names = match sync_result {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    // Wipe user-derived auth state (users, sessions, api keys, shares) and
    // log the caller out by clearing their session cookie. allowed_domains
    // is preserved as policy config. Without this, a top-admin remained
    // logged in after a factory reset, the SPA kept its old selected exom,
    // and the next page load 400'd against an exom that no longer existed.
    if let Some(ref auth_store) = state.auth_store {
        if let Err(e) = auth_store.factory_reset_state().await {
            return ApiError::new("factory_reset_failed", format!("auth wipe: {e}"))
                .with_status(500)
                .into_response();
        }
    }

    let _ = state.sse_tx.send((
        None,
        format!(
            r#"{{"v":1,"kind":"system","op":"factory_reset","actor":"{}"}}"#,
            actor
        ),
    ));

    let cookie = crate::auth::middleware::clear_session_cookie();
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "ok": true,
            "removed_exoms": old_names,
            "state": "clean",
            "logged_out": true,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /schema
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SchemaQuery {
    exom: Option<String>,
    branch: Option<String>,
    include_samples: Option<String>,
    sample_limit: Option<usize>,
    relation: Option<String>,
}

async fn api_schema(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<SchemaQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let include_samples = q.include_samples.as_deref() == Some("true");
    let sample_limit = q.sample_limit.unwrap_or(10);
    let filter_relation = q.relation.clone();
    let branch_id = q
        .branch
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(brain::MAIN_BRANCH)
        .to_string();

    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let mut relations = Vec::new();
    let (
        fact_groups,
        has_intervals_map,
        obs_tuples,
        belief_tuples,
        has_belief_intervals,
        all_rules,
        ontology,
    ) = {
        let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path)
        {
            Ok(e) => e,
            Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
        };
        let brain = &es.brain;
        let all_rules = match combined_rules(&exom_slash, &es.rules) {
            Ok(r) => r,
            Err(e) => return ApiError::new("error", e.to_string()).into_response(),
        };

        if !brain.branch_exists(&branch_id) {
            return ApiError::new("unknown_branch", format!("unknown branch '{}'", branch_id))
                .with_status(400)
                .into_response();
        }

        let mut fact_groups: HashMap<String, Vec<Vec<serde_json::Value>>> = HashMap::new();
        let mut has_intervals_map: HashMap<String, bool> = HashMap::new();
        for fact in brain.facts_on_branch(&branch_id) {
            let entry = fact_groups.entry(fact.predicate.clone()).or_default();
            entry.push(vec![
                serde_json::Value::String(fact.fact_id.clone()),
                serde_json::Value::String(fact.predicate.clone()),
                serde_json::to_value(&fact.value)
                    .unwrap_or_else(|_| serde_json::Value::String(fact.value.display())),
                serde_json::json!(fact.confidence),
                serde_json::json!({
                    "valid_from": fact.valid_from,
                    "valid_to": fact.valid_to,
                    "branch_origin": brain.tx_branch(fact.created_by_tx).unwrap_or(""),
                        "branch_role": brain.fact_branch_role(fact, &branch_id),
                }),
            ]);
            if fact.valid_to.is_some() {
                has_intervals_map.insert(fact.predicate.clone(), true);
            }
        }

        let obs_tuples: Vec<Vec<serde_json::Value>> = brain
            .observations()
            .iter()
            .map(|obs| {
                vec![
                    serde_json::Value::String(obs.obs_id.clone()),
                    serde_json::Value::String(obs.source_type.clone()),
                    serde_json::Value::String(obs.content.clone()),
                    serde_json::json!(obs.confidence),
                ]
            })
            .collect();

        let beliefs = brain.beliefs_on_branch(&branch_id);
        let has_belief_intervals = beliefs.iter().any(|belief| belief.valid_to.is_some());
        let belief_tuples: Vec<Vec<serde_json::Value>> = beliefs
            .iter()
            .map(|belief| {
                vec![
                    serde_json::Value::String(belief.belief_id.clone()),
                    serde_json::Value::String(belief.claim_text.clone()),
                    serde_json::json!(belief.confidence),
                    serde_json::Value::String(belief.status.to_string()),
                    serde_json::json!({
                        "valid_from": belief.valid_from,
                        "valid_to": belief.valid_to
                    }),
                ]
            })
            .collect();

        let ontology =
            system_schema::build_exom_ontology(&exom_slash, brain, &branch_id, &es.rules);
        (
            fact_groups,
            has_intervals_map,
            obs_tuples,
            belief_tuples,
            has_belief_intervals,
            all_rules,
            ontology,
        )
    };

    for (pred, tuples) in &fact_groups {
        if let Some(ref filter) = filter_relation {
            if pred != filter {
                continue;
            }
        }
        let has_intervals = has_intervals_map.get(pred).copied().unwrap_or(false);
        let mut rel = serde_json::json!({
            "name": pred,
            "arity": 5,
            "kind": "base",
            "cardinality": tuples.len(),
            "has_intervals": has_intervals,
            "defined_by": []
        });
        if include_samples {
            rel["sample_tuples"] = serde_json::json!(tuples
                .iter()
                .take(sample_limit)
                .cloned()
                .collect::<Vec<_>>());
        }
        relations.push(rel);
    }

    if filter_relation.is_none() || filter_relation.as_deref() == Some("observation") {
        let mut rel = serde_json::json!({
            "name": "observation",
            "arity": 4,
            "kind": "base",
            "cardinality": obs_tuples.len(),
            "has_intervals": false,
            "defined_by": []
        });
        if include_samples && !obs_tuples.is_empty() {
            rel["sample_tuples"] = serde_json::json!(obs_tuples
                .into_iter()
                .take(sample_limit)
                .collect::<Vec<_>>());
        }
        relations.push(rel);
    }

    if filter_relation.is_none() || filter_relation.as_deref() == Some("belief") {
        let mut rel = serde_json::json!({
            "name": "belief",
            "arity": 5,
            "kind": "derived",
            "cardinality": belief_tuples.len(),
            "has_intervals": has_belief_intervals,
            "defined_by": ["belief-revision"]
        });
        if include_samples && !belief_tuples.is_empty() {
            rel["sample_tuples"] = serde_json::json!(belief_tuples
                .into_iter()
                .take(sample_limit)
                .collect::<Vec<_>>());
        }
        relations.push(rel);
    }

    let mut base_names: std::collections::HashSet<String> =
        fact_groups.keys().map(|s| (*s).to_string()).collect();
    base_names.insert("observation".into());
    base_names.insert("belief".into());

    let derived_preds = rules::derived_predicates(&all_rules);
    for (pred_name, arity) in derived_preds {
        if base_names.contains(&pred_name) {
            continue;
        }
        if let Some(ref filter) = filter_relation {
            if filter != pred_name.as_str() {
                continue;
            }
        }
        let defined_by_rules: Vec<usize> = all_rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.head_predicate == pred_name)
            .map(|(i, _)| i)
            .collect();
        let mut rel = serde_json::json!({
            "name": pred_name, "arity": arity, "kind": "derived",
            "cardinality": serde_json::Value::Null,
            "has_intervals": false, "defined_by": defined_by_rules,
        });
        if include_samples {
            let rows = query_relation_rows(&exoms, &state.engine, &exom_slash, &pred_name, arity)
                .ok()
                .filter(|rows| !rows.is_empty());
            if let Some(rows) = rows {
                rel["cardinality"] = serde_json::json!(rows.len());
                rel["sample_tuples"] =
                    serde_json::json!(rows.into_iter().take(sample_limit).collect::<Vec<_>>());
            }
        }
        relations.push(rel);
    }

    let base_count = relations.iter().filter(|r| r["kind"] == "base").count();
    let derived_count = relations.iter().filter(|r| r["kind"] == "derived").count();
    let largest = relations
        .iter()
        .max_by_key(|r| r["cardinality"].as_u64().unwrap_or(0))
        .map(|r| serde_json::json!({"name": r["name"], "cardinality": r["cardinality"]}));

    Json(serde_json::json!({
        "relations": relations,
        "ontology": ontology,
        "directives": [],
        "summary": {
            "relation_count": relations.len(),
            "base_relation_count": base_count,
            "derived_relation_count": derived_count,
            "largest_relation": largest
        }
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /graph
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GraphQuery {
    exom: Option<String>,
    limit: Option<usize>,
    branch: Option<String>,
}

async fn api_graph(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<GraphQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let limit = q.limit.unwrap_or(500);
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let facts = es.brain.facts_on_branch(&branch_id);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_nodes = std::collections::HashSet::new();
    for (i, f) in facts.iter().take(limit).enumerate() {
        let entity_id = &f.fact_id;
        let pred_id = format!("pred:{}", f.predicate);
        if seen_nodes.insert(entity_id.clone()) {
            nodes.push(serde_json::json!({"id": entity_id, "type": "entity", "label": format!("{} = {}", f.predicate, f.value), "degree": 1}));
        }
        if seen_nodes.insert(pred_id.clone()) {
            nodes.push(serde_json::json!({"id": pred_id, "type": "entity", "label": f.predicate, "degree": 1}));
        }
        edges.push(serde_json::json!({"id": format!("e{}", i), "type": "fact", "source": entity_id, "target": pred_id, "label": f.value}));
    }
    Json(serde_json::json!({
        "nodes": nodes, "edges": edges, "clusters": [],
        "summary": {"node_count": nodes.len(), "edge_count": edges.len(), "cluster_count": 0}
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /clusters
// ---------------------------------------------------------------------------

async fn api_clusters(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let facts = es.brain.facts_on_branch(&branch_id);
    let mut groups: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        *groups.entry(&f.predicate).or_default() += 1;
    }
    let clusters: Vec<_> = groups
        .iter()
        .map(|(pred, count)| {
            serde_json::json!({
                "id": format!("cluster:{}", pred), "label": pred, "kind": "shared_predicate",
                "fact_count": count, "active_count": count, "deprecated_count": 0
            })
        })
        .collect();
    Json(serde_json::json!({"clusters": clusters})).into_response()
}

// ---------------------------------------------------------------------------
// GET /clusters/:id
// ---------------------------------------------------------------------------

async fn api_cluster_detail_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let pred = id.strip_prefix("cluster:").unwrap_or(&id);
    let facts = es.brain.facts_on_branch(&branch_id);
    let matching: Vec<_> = facts.iter().filter(|f| f.predicate == pred).collect();
    let nodes: Vec<_> = matching.iter().map(|f| serde_json::json!({"id": f.fact_id, "type": "fact", "label": format!("{} = {}", f.predicate, f.value)})).collect();
    let fact_entries: Vec<_> = matching.iter().map(|f| serde_json::json!({
        "id": f.fact_id, "tuple": [f.fact_id, f.predicate, f.value, f.confidence],
        "status": "active", "interval": {"start": f.valid_from, "end": f.valid_to.as_deref().unwrap_or("inf")}
    })).collect();
    Json(serde_json::json!({
        "id": id, "label": pred, "kind": "shared_predicate",
        "stats": {"fact_count": matching.len(), "active_count": matching.len(), "deprecated_count": 0},
        "nodes": nodes, "facts": fact_entries, "related_clusters": []
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /logs
// ---------------------------------------------------------------------------

async fn api_logs(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let events: Vec<_> = es
        .brain
        .transactions()
        .iter()
        .rev()
        .take(24)
        .map(|tx| {
            serde_json::json!({
                "id": format!("tx{}", tx.tx_id), "type": tx.action.to_string(),
                "timestamp": tx.tx_time, "pattern": tx.note,
                "source": tx.user_email,
                "agent": tx.agent,
                "model": tx.model,
            })
        })
        .collect();
    Json(serde_json::json!({"events": events})).into_response()
}

// ---------------------------------------------------------------------------
// GET /provenance
// ---------------------------------------------------------------------------

async fn api_provenance(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let facts = es.brain.facts_on_branch(&branch_id);
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };
    let derived_n = rules::derived_predicates(&all_rules).len();
    let base_facts: Vec<_> = facts.iter().map(|f| serde_json::json!({
        "id": f.fact_id, "predicate": f.predicate, "terms": [f.fact_id, f.predicate, f.value],
        "kind": "base", "source": f.provenance, "confidence": f.confidence, "asserted_at": f.created_by_tx
    })).collect();
    Json(serde_json::json!({
        "derivations": [], "base_facts": base_facts, "edges": [], "timeline": [],
        "summary": {"derived_count": derived_n, "base_count": base_facts.len(), "edge_count": 0, "event_count": 0}
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /relation-graph
// ---------------------------------------------------------------------------

fn relation_graph_subject(fact_id: &str) -> String {
    fact_id
        .split_once('#')
        .map(|(entity, _)| entity)
        .unwrap_or(fact_id)
        .to_string()
}

fn relation_graph_label(id: &str) -> String {
    let raw = id
        .rsplit(['/', ':', '#'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(id);
    if raw.len() > 48 {
        format!("{}...", &raw[..45])
    } else {
        raw.to_string()
    }
}

fn value_looks_like_entity(value: &str) -> bool {
    if value.starts_with("repo:") || value.starts_with("doc:") || value.starts_with("command:") {
        return true;
    }
    value.contains('/') && !value.contains(char::is_whitespace)
}

fn relation_graph_target(fact: &crate::brain::Fact) -> (String, String) {
    let display = fact.value.display();
    if value_looks_like_entity(&display) {
        (display.clone(), relation_graph_label(&display))
    } else {
        (
            format!("{}={display}", fact.predicate),
            relation_graph_label(&display),
        )
    }
}

async fn api_relation_graph(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let branch_id = match resolve_view_branch(&es.brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let facts = es.brain.facts_on_branch(&branch_id);
    let mut nodes: HashMap<String, (String, usize)> = HashMap::new();
    let mut edges = Vec::new();
    for fact in &facts {
        let source = relation_graph_subject(&fact.fact_id);
        let (target, target_label) = relation_graph_target(fact);
        if source == target {
            continue;
        }
        nodes
            .entry(source.clone())
            .and_modify(|(_, degree)| *degree += 1)
            .or_insert_with(|| (relation_graph_label(&source), 1));
        nodes
            .entry(target.clone())
            .and_modify(|(_, degree)| *degree += 1)
            .or_insert((target_label, 1));
        edges.push(serde_json::json!({
            "source": source,
            "target": target,
            "label": fact.predicate,
            "predicate": fact.predicate,
            "kind": "base",
        }));
    }
    let edge_count = edges.len();
    let nodes: Vec<_> = nodes
        .into_iter()
        .map(
            |(id, (label, degree))| serde_json::json!({"id": id, "label": label, "degree": degree}),
        )
        .collect();
    let node_count = nodes.len();
    Json(serde_json::json!({
        "nodes": nodes, "edges": edges,
        "summary": {"node_count": node_count, "edge_count": edge_count}
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /derived/:pred
// ---------------------------------------------------------------------------

async fn api_derived_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(pred_name): AxumPath<String>,
    Query(q): Query<ExomQuery>,
) -> impl IntoResponse {
    if pred_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing predicate"})),
        )
            .into_response();
    }
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let all_rules = match combined_rules(&exom_slash, &es.rules) {
        Ok(r) => r,
        Err(e) => return ApiError::new("error", e.to_string()).into_response(),
    };
    let arity = match all_rules
        .iter()
        .find(|r| r.head_predicate == pred_name)
        .map(|r| r.head_arity)
    {
        Some(a) => a,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "unknown derived predicate"})),
            )
                .into_response()
        }
    };
    let find_vars: Vec<String> = (0..arity).map(|i| format!("?v{i}")).collect();
    let find_vars_str = find_vars.join(" ");
    let bodies: Vec<String> = all_rules.iter().map(|r| r.inline_body.clone()).collect();
    let rules_clause = bodies.join(" ");
    let rayfall = format!(
        "(query {exom_slash} (find {find_vars_str}) (where ({pred_name} {find_vars_str})) (rules {rules_clause}))"
    );
    match state.engine.eval(&rayfall) {
        Ok(output) => Json(serde_json::json!({"predicate": pred_name, "kind": "derived", "arity": arity, "rows": output})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /beliefs/:id/support
// ---------------------------------------------------------------------------

async fn api_belief_support_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(belief_id): AxumPath<String>,
    Query(q): Query<BranchExomQuery>,
) -> impl IntoResponse {
    let exom_raw = q.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();
    if let Some(resp) = guard_read(&state, &maybe_user, &exom_slash).await {
        return resp;
    }
    let bid = belief_id.trim().to_string();
    if bid.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing belief id"})),
        )
            .into_response();
    }
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let es = match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path) {
        Ok(e) => e,
        Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
    };
    let brain = &es.brain;
    let branch_id = match resolve_view_branch(brain, q.branch.as_deref()) {
        Ok(id) => id,
        Err(e) => return ApiError::new("unknown_branch", e).into_response(),
    };
    let beliefs: Vec<_> = brain
        .beliefs_on_branch(&branch_id)
        .into_iter()
        .filter(|b| b.belief_id == bid)
        .collect();
    let b = match beliefs.first() {
        Some(x) => *x,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("belief '{}' not found", bid)})),
            )
                .into_response()
        }
    };
    let mut support_facts = Vec::new();
    let mut support_obs = Vec::new();
    let mut unresolved: Vec<&str> = Vec::new();
    let branch_facts = brain.facts_on_branch(&branch_id);
    for id in &b.supported_by {
        if let Some(f) = branch_facts.iter().find(|f| f.fact_id == *id) {
            support_facts.push(fact_to_json(f));
        } else if let Some(o) = brain.observations().iter().find(|o| o.obs_id == *id) {
            support_obs.push(serde_json::json!({"obs_id": o.obs_id, "source_type": o.source_type, "source_ref": o.source_ref, "content": o.content}));
        } else {
            unresolved.push(id.as_str());
        }
    }
    Json(serde_json::json!({
        "ok": true,
        "belief_id": b.belief_id,
        "claim_text": b.claim_text,
        "supported_by_resolved": {"facts": support_facts, "observations": support_obs},
        "supported_by_unresolved": unresolved,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET / PUT /ui/graph-layout
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GraphLayoutQuery {
    scope: String,
}

async fn api_get_graph_layout(
    State(state): State<Arc<AppState>>,
    user: User,
    Query(q): Query<GraphLayoutQuery>,
) -> Response {
    let scope = q.scope.trim();
    if scope.is_empty() {
        return ApiError::new("missing_scope", "scope is required").into_response();
    }
    let Some(ui_state) = state.ui_state.as_ref() else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"layout": null})))
            .into_response();
    };
    match ui_state.get_graph_layout(&user.email, scope).await {
        Ok(Some(layout)) => Json(serde_json::json!({"layout": layout})).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"layout": null})),
        )
            .into_response(),
        Err(e) => ApiError::new("db_error", e.to_string()).into_response(),
    }
}

async fn api_put_graph_layout(
    State(state): State<Arc<AppState>>,
    user: User,
    Query(q): Query<GraphLayoutQuery>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let scope = q.scope.trim();
    if scope.is_empty() {
        return ApiError::new("missing_scope", "scope is required").into_response();
    }
    let Some(ui_state) = state.ui_state.as_ref() else {
        return ApiError::new("not_configured", "ui state persistence is not configured")
            .into_response();
    };
    if !body.is_object() {
        return ApiError::new("bad_payload", "layout body must be a JSON object")
            .into_response();
    }
    match ui_state.upsert_graph_layout(&user.email, scope, &body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ApiError::new("db_error", e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod query_validation_tests {
    use super::*;

    fn parse_query(source: &str) -> CanonicalQuery {
        lower_query_request(source, None, "test").expect("parse")
    }

    fn known() -> BTreeSet<String> {
        known_relations_for_exom("main", &[])
    }

    fn arities() -> std::collections::BTreeMap<String, usize> {
        known_relation_arities_for_exom("main", &[])
    }

    #[test]
    fn accepts_builtin_edb() {
        let q = parse_query("(query main (find ?p ?v) (where (facts_i64 ?e ?p ?v)))");
        validate_query_body(&q, &known(), &arities()).expect("facts_i64 is a bound EDB");
    }

    #[test]
    fn accepts_eav_form() {
        let q = parse_query("(query main (find ?f ?p) (where (?f 'fact/predicate ?p)))");
        validate_query_body(&q, &known(), &arities())
            .expect("EAV atoms with variable leads should pass");
    }

    #[test]
    fn accepts_builtin_view_idb() {
        let q = parse_query("(query main (find ?f ?p ?v) (where (fact-row ?f ?p ?v)))");
        validate_query_body(&q, &known(), &arities()).expect("fact-row is a builtin view");
    }

    #[test]
    fn accepts_nested_logical_and_cmp() {
        let q = parse_query("(query main (find ?v) (where (and (facts_i64 ?e ?p ?v) (> ?v 5))))");
        validate_query_body(&q, &known(), &arities()).expect("and/cmp should descend and pass");
    }

    #[test]
    fn accepts_aggregate_over_known_relation() {
        let q = parse_query("(query main (find ?s) (where (sum ?s facts_i64 2)))");
        validate_query_body(&q, &known(), &arities()).expect("agg over known relation");
    }

    #[test]
    fn rejects_typo_in_relation_name() {
        let q = parse_query("(query main (find ?v) (where (facts_164 ?e ?p ?v)))");
        let err = validate_query_body(&q, &known(), &arities()).expect_err("typo should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("facts_164"),
            "error should mention the typo: {msg}"
        );
        assert!(msg.contains("facts_i64"), "should suggest the fix: {msg}");
    }

    #[test]
    fn rejects_unknown_relation_in_aggregate() {
        let q = parse_query("(query main (find ?s) (where (sum ?s bogus_rel 2)))");
        let err = validate_query_body(&q, &known(), &arities()).expect_err("unknown agg source");
        assert!(err.to_string().contains("bogus_rel"));
    }

    #[test]
    fn rejects_unknown_relation() {
        let q = parse_query("(query main (find ?v) (where (no_such_relation ?e ?p ?v)))");
        let err = validate_query_body(&q, &known(), &arities()).expect_err("unknown relation");
        assert!(err.to_string().contains("no_such_relation"));
    }

    #[test]
    fn known_relations_includes_user_rule_heads() {
        let parsed = crate::rules::parse_rule_line(
            "(rule main (heavy ?f ?w) (facts_i64 ?f 'weight_kg ?w) (> ?w 60))",
            MutationContext::default(),
            "test".to_string(),
        )
        .expect("parse user rule");
        let known = known_relations_for_exom("main", std::slice::from_ref(&parsed));
        let arities = known_relation_arities_for_exom("main", std::slice::from_ref(&parsed));
        let q = parse_query("(query main (find ?f ?w) (where (heavy ?f ?w)))");
        validate_query_body(&q, &known, &arities).expect("user rule head should be accepted");
    }

    #[test]
    fn rejects_wrong_arity_on_builtin_view() {
        // fact-row is arity 3; calling it with 1 arg must fail.
        let q = parse_query("(query main (find ?x) (where (fact-row ?x)))");
        let err =
            validate_query_body(&q, &known(), &arities()).expect_err("arity mismatch should fail");
        let msg = err.to_string();
        assert!(msg.contains("fact-row"), "error should mention rule: {msg}");
        assert!(
            msg.contains("3"),
            "error should mention declared arity: {msg}"
        );
        assert!(
            msg.contains("1"),
            "error should mention actual arity: {msg}"
        );
    }

    #[test]
    fn rejects_wrong_arity_on_typed_edb() {
        // facts_i64 is arity 3; calling with 2 args should fail.
        let q = parse_query("(query main (find ?e ?p) (where (facts_i64 ?e ?p)))");
        let err =
            validate_query_body(&q, &known(), &arities()).expect_err("arity mismatch should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("facts_i64"),
            "error should mention rule: {msg}"
        );
    }
}
