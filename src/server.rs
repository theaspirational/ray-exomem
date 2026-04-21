use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
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

pub const UI_MOUNT_PATH: &str = "/ray-exomem";
pub const API_PREFIX: &str = "/ray-exomem/api/";
pub const EVENTS_PATH: &str = "/ray-exomem/events";
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:9780";
pub const DEFAULT_EXOM: &str = "main";

use crate::{
    auth::{middleware::MaybeUser, User},
    backend::RayforceEngine,
    brain::{self, Belief, Brain, Branch, Fact, MergePolicy, Observation, Tx},
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
}

pub struct AppState {
    pub exoms: Mutex<HashMap<String, ExomState>>,
    pub engine: crate::backend::RayforceEngine,
    pub tree_root: Option<PathBuf>,
    pub sym_path: Option<PathBuf>,
    pub start_time: Instant,
    pub sse_tx: broadcast::Sender<String>,
    pub auth_store: Option<Arc<crate::auth::store::AuthStore>>,
    pub auth_provider: Option<Arc<dyn crate::auth::provider::AuthProvider>>,
    pub bind_addr: Option<String>,
    pub exom_db: Option<Arc<dyn crate::db::ExomDb>>,
}

impl AppState {
    pub fn new(
        engine: RayforceEngine,
        exoms: HashMap<String, ExomState>,
        tree_root: Option<PathBuf>,
        sym_path: Option<PathBuf>,
        exom_db: Option<Arc<dyn crate::db::ExomDb>>,
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
            bind_addr: None,
            exom_db,
        }
    }

    /// Build AppState by loading a data directory (same logic as web::serve).
    pub fn from_data_dir(data_dir: Option<PathBuf>) -> anyhow::Result<Arc<Self>> {
        let mut exoms: HashMap<String, ExomState> = HashMap::new();
        let (engine, tree_root, sym_path) = match data_dir {
            Some(ref root) => {
                let sym = root.join("sym");
                // Load sym INSIDE ray_runtime_create, before builtins intern
                // their names. This keeps persisted symbol IDs stable across
                // binary upgrades — builtins get appended after, not before.
                let engine = if sym.exists() {
                    match RayforceEngine::new_with_sym(&sym) {
                        Ok(e) => e,
                        Err(_) => {
                            eprintln!(
                                "[ray-exomem] WARNING: symbol table incompatible. \
                                 Recovering from JSONL sidecars: {}",
                                root.display()
                            );
                            if sym.exists() {
                                let _ = std::fs::remove_file(&sym);
                            }
                            let sym_lk = root.join("sym.lk");
                            if sym_lk.exists() {
                                let _ = std::fs::remove_file(&sym_lk);
                            }
                            RayforceEngine::new()?
                        }
                    }
                } else {
                    RayforceEngine::new()?
                };
                let tree_dir = root.join("tree");
                std::fs::create_dir_all(&tree_dir).ok();
                load_tree_exoms_into(&tree_dir, &sym, &mut exoms);
                if exoms.is_empty() {
                    let default_path: crate::path::TreePath = "main".parse().unwrap();
                    let _ = crate::scaffold::new_bare_exom(&tree_dir, &default_path);
                    load_tree_exoms_into(&tree_dir, &sym, &mut exoms);
                }
                (engine, Some(tree_dir), Some(sym))
            }
            None => {
                let engine = RayforceEngine::new()?;
                exoms.insert(
                    DEFAULT_EXOM.to_string(),
                    ExomState {
                        brain: Brain::new(),
                        datoms: storage::build_datoms_table(&Brain::new())?,
                        typed_facts: storage::build_typed_fact_tables(&Brain::new())?,
                        rules: Vec::new(),
                        exom_disk: None,
                    },
                );
                (engine, None, None)
            }
        };

        // Bind all exoms into the engine.
        for (name, es) in &exoms {
            engine.bind_named_db(storage::sym_intern(name), &es.datoms)?;
        }

        Ok(Arc::new(AppState::new(
            engine, exoms, tree_root, sym_path, None,
        )))
    }

    /// Re-load exom data from ExomDb when available. Call after initial disk loading.
    pub async fn reload_exoms_from_db(&self) {
        let Some(ref exom_db) = self.exom_db else {
            return;
        };
        let Some(ref tree_root) = self.tree_root else {
            return;
        };
        let Some(ref sym_path) = self.sym_path else {
            return;
        };

        let keys: Vec<String> = {
            let exoms = self.exoms.lock().unwrap();
            exoms.keys().cloned().collect()
        };

        for key in keys {
            let disk = tree_root.join(&key);
            match load_exom_preferring_db(Some(exom_db), &disk, sym_path, &key).await {
                Ok(es) => {
                    let mut exoms = self.exoms.lock().unwrap();
                    exoms.insert(key.clone(), es);
                    let datoms = &exoms.get(&key).expect("just inserted").datoms;
                    let _ = self.engine.bind_named_db(storage::sym_intern(&key), datoms);
                }
                Err(e) => eprintln!("[ray-exomem] WARNING: DB reload for '{}': {}", key, e),
            }
        }
    }
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

/// Load an exom, preferring Postgres data when ExomDb is available and has data.
/// Falls back to disk (splay/JSONL) otherwise.
async fn load_exom_preferring_db(
    exom_db: Option<&Arc<dyn crate::db::ExomDb>>,
    exom_disk: &std::path::Path,
    sym_path: &std::path::Path,
    slash_key: &str,
) -> anyhow::Result<ExomState> {
    if let Some(db) = exom_db {
        let txs = db.load_transactions(slash_key).await?;
        if !txs.is_empty() {
            let mut brain =
                Brain::open_exom_from_db(db.as_ref(), slash_key, exom_disk, sym_path).await?;

            let meta_p = exom_disk.join(crate::exom::META_FILENAME);
            if meta_p.exists() {
                if let Ok(meta) = crate::exom::read_meta(exom_disk) {
                    if brain
                        .branches()
                        .iter()
                        .any(|b| b.branch_id == meta.current_branch && !b.archived)
                    {
                        let _ = brain.switch_branch(&meta.current_branch);
                    }
                }
            }
            let datoms = storage::build_datoms_table(&brain)?;
            let typed_facts = storage::build_typed_fact_tables(&brain)?;
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
            let ontology = system_schema::build_exom_ontology(slash_key, &brain, &rules);
            let _ = system_schema::save_exom_ontology(&schema_p, &ontology);
            return Ok(ExomState {
                brain,
                datoms,
                typed_facts,
                rules,
                exom_disk: Some(exom_disk.to_path_buf()),
            });
        }
    }
    load_exom_from_tree_path(exom_disk, sym_path, slash_key)
}

/// Same tree walk as [`load_tree_exoms_into`], but uses [`load_exom_preferring_db`].
#[allow(dead_code)]
async fn load_tree_exoms_into_async(
    tree_root: &Path,
    sym_path: &Path,
    exom_db: Option<&Arc<dyn crate::db::ExomDb>>,
    out: &mut HashMap<String, ExomState>,
) {
    fn collect_exom_paths(tree_root: &Path, current: &Path, paths: &mut Vec<(PathBuf, String)>) {
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
            if disk.join(crate::exom::META_FILENAME).exists() {
                paths.push((disk, slash_key));
                continue;
            }
            collect_exom_paths(tree_root, &disk, paths);
        }
    }
    let mut paths = Vec::new();
    collect_exom_paths(tree_root, tree_root, &mut paths);
    for (disk, slash_key) in paths {
        eprintln!("[ray-exomem] loading tree exom '{}'", slash_key);
        match load_exom_preferring_db(exom_db, &disk, sym_path, &slash_key).await {
            Ok(es) => {
                out.insert(slash_key, es);
            }
            Err(e) => eprintln!(
                "[ray-exomem] WARNING: failed to load '{}': {}",
                slash_key, e
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(api_status))
        .route("/tree", get(api_tree))
        .route("/guide", get(api_guide))
        .route("/exoms", get(api_exoms_gone).post(api_exoms_gone))
        .route("/actions/init", post(api_init))
        .route("/actions/exom-new", post(api_exom_new))
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
        // Branches
        .route("/branches", get(api_list_branches).post(api_create_branch))
        .route(
            "/branches/{id}",
            get(api_branch_detail).delete(api_delete_branch_handler),
        )
        .route("/branches/{id}/switch", post(api_switch_branch_handler))
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
        .route(
            "/actions/consolidate-propose",
            post(api_consolidate_propose),
        )
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
        // Exom manage (removed)
        .route("/exoms/{name}/manage", post(api_exoms_gone))
        // Old start-session compat → 410
        .route("/actions/start-session", post(api_start_session_gone))
}

// ---------------------------------------------------------------------------
// SSE endpoint
// ---------------------------------------------------------------------------

async fn api_sse(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.sse_tx.subscribe();
    let events = BroadcastStream::new(rx)
        .filter_map(|r| async { r.ok() })
        .map(|data| Ok(Event::default().data(data)));
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
    let raw = uri.path().trim_start_matches('/');
    let path = raw.strip_prefix("ray-exomem/").unwrap_or(raw);
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

    state.reload_exoms_from_db().await;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Nested under /ray-exomem/api
        .nest("/ray-exomem/api", api_router())
        // MCP JSON-RPC endpoint
        .route("/mcp", post(crate::mcp::mcp_handler))
        // Auth routes
        .nest("/auth", crate::auth::routes::auth_router())
        .nest("/auth/admin", crate::auth::admin::admin_router())
        // SSE event stream
        .route("/ray-exomem/events", get(api_sse))
        .route("/sse", get(api_sse))
        // Compat shim: smoke test calls /api/status
        .route("/api/status", get(api_status))
        .fallback(spa_fallback)
        .with_state(state)
        .layer(middleware::map_response(set_response_headers))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
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
    let mut brain = {
        let b = Brain::open_exom(exom_disk, sym_path)?;
        b.save_all_jsonl()?;
        b
    };
    let meta_p = exom_disk.join(crate::exom::META_FILENAME);
    if meta_p.exists() {
        if let Ok(meta) = crate::exom::read_meta(exom_disk) {
            if brain
                .branches()
                .iter()
                .any(|b| b.branch_id == meta.current_branch && !b.archived)
            {
                let _ = brain.switch_branch(&meta.current_branch);
            }
        }
    }
    let datoms = storage::build_datoms_table(&brain)?;
    let typed_facts = storage::build_typed_fact_tables(&brain)?;
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
    let ontology = system_schema::build_exom_ontology(slash_key, &brain, &rules);
    let _ = system_schema::save_exom_ontology(&schema_p, &ontology);
    Ok(ExomState {
        brain,
        datoms,
        typed_facts,
        rules,
        exom_disk: Some(exom_disk.to_path_buf()),
    })
}

fn combined_rules(exom: &str, user_rules: &[ParsedRule]) -> anyhow::Result<Vec<ParsedRule>> {
    let mut rules = system_schema::builtin_rules(exom)?;
    rules.extend_from_slice(user_rules);
    Ok(rules)
}

fn refresh_exom_binding(
    state: &AppState,
    exoms: &mut HashMap<String, ExomState>,
    exom_name: &str,
) -> anyhow::Result<()> {
    let es = exoms
        .get_mut(exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    es.datoms = storage::build_datoms_table(&es.brain)?;
    es.typed_facts = storage::build_typed_fact_tables(&es.brain)?;
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
        let ontology = system_schema::build_exom_ontology(exom_name, &es.brain, &es.rules);
        let _ = system_schema::save_exom_ontology(&schema_p, &ontology);
    }
    Ok(())
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
    let _ = state
        .sse_tx
        .send(format!(r#"{{"kind":"memory","exom":"{}"}}"#, exom_name));
    Ok(out)
}

/// After sync splay rebuild (`mutate_exom`), persist the exom snapshot to Postgres when configured.
/// Best-effort: logs a warning on failure and does not fail the HTTP mutation.
async fn exom_db_save_brain_snapshot(state: &AppState, exom_name: &str) {
    let Some(ref db) = state.exom_db else {
        return;
    };
    let path = exom_name.to_string();
    let snapshot = {
        let exoms = state.exoms.lock().unwrap();
        let Some(es) = exoms.get(exom_name) else {
            return;
        };
        (
            es.brain.transactions().to_vec(),
            es.brain.all_facts().to_vec(),
            es.brain.observations().to_vec(),
            es.brain.all_beliefs().to_vec(),
            es.brain.branches().to_vec(),
        )
    };
    let db = db.clone();
    let (txs, facts, observations, beliefs, branches) = snapshot;
    if let Err(e) = async {
        db.save_transactions(&path, &txs).await?;
        db.save_facts(&path, &facts).await?;
        db.save_observations(&path, &observations).await?;
        db.save_beliefs(&path, &beliefs).await?;
        db.save_branches(&path, &branches).await?;
        anyhow::Ok(())
    }
    .await
    {
        eprintln!("[ray-exomem] warning: ExomDb snapshot failed for {path}: {e}");
    }
}

/// Sync JSONL on disk under `disk` to Postgres (e.g. session exoms not loaded in `AppState`).
async fn exom_db_sync_from_disk(
    db: &Arc<dyn crate::db::ExomDb>,
    exom_slash: &str,
    disk: &std::path::Path,
) -> anyhow::Result<()> {
    let txs: Vec<Tx> = storage::load_jsonl(&disk.join("tx.jsonl"))?;
    let facts: Vec<Fact> = storage::load_jsonl(&disk.join("fact.jsonl"))?;
    let observations: Vec<Observation> = storage::load_jsonl(&disk.join("observation.jsonl"))?;
    let beliefs: Vec<Belief> = storage::load_jsonl(&disk.join("belief.jsonl"))?;
    let branches: Vec<Branch> = storage::load_jsonl(&disk.join("branch.jsonl"))?;
    db.save_transactions(exom_slash, &txs).await?;
    db.save_facts(exom_slash, &facts).await?;
    db.save_observations(exom_slash, &observations).await?;
    db.save_beliefs(exom_slash, &beliefs).await?;
    db.save_branches(exom_slash, &branches).await?;
    Ok(())
}

/// After in-memory mutation + splay rebuild, write JSONL sidecars or mirror to Postgres.
async fn persist_exom_storage(state: &AppState, exom_name: &str) {
    if state.exom_db.is_some() {
        exom_db_save_brain_snapshot(state, exom_name).await;
    } else {
        let exoms = state.exoms.lock().unwrap();
        if let Some(es) = exoms.get(exom_name) {
            if let Err(e) = es.brain.save_all_jsonl() {
                eprintln!(
                    "[ray-exomem] warning: JSONL write failed for {}: {e}",
                    exom_name
                );
            }
        }
    }
}

pub async fn mutate_exom_async<T>(
    state: &AppState,
    exom_name: &str,
    f: impl FnOnce(&mut ExomState) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let result = mutate_exom(state, exom_name, f)?;
    persist_exom_storage(state, exom_name).await;
    Ok(result)
}

fn emit_tree_changed(state: &AppState) {
    let _ = state
        .sse_tx
        .send(r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string());
}

async fn guard_read(
    state: &AppState,
    maybe_user: &MaybeUser,
    exom_slash: &str,
) -> Option<axum::response::Response> {
    if let Some(ref auth_store) = state.auth_store {
        if let Some(ref user) = maybe_user.0 {
            let level = crate::auth::access::resolve_access(user, exom_slash, auth_store).await;
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
            let level = crate::auth::access::resolve_access(user, exom_slash, auth_store).await;
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
            let level = crate::auth::access::resolve_access(user, exom_slash, auth_store).await;
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

async fn build_tree_root_for_admin(
    state: &AppState,
    user: &User,
    opts: &crate::tree::WalkOptions,
) -> std::io::Result<crate::tree::TreeNode> {
    let tree_root = server_tree_root(state);
    let mut namespaces = BTreeSet::new();
    namespaces.insert(user.namespace_root().to_string());
    if let Some(ref auth_store) = state.auth_store {
        for stored in auth_store.list_users().await {
            namespaces.insert(stored.email);
        }
    }

    let mut children = Vec::new();
    for namespace in namespaces {
        let root_path = namespace_path(&namespace)?;
        children.push(crate::tree::walk_or_empty(&tree_root, &root_path, opts)?);
    }

    Ok(crate::tree::TreeNode::Folder {
        name: String::new(),
        path: String::new(),
        children,
    })
}

async fn build_tree_root_for_user(
    state: &AppState,
    user: &User,
    opts: &crate::tree::WalkOptions,
) -> std::io::Result<crate::tree::TreeNode> {
    let tree_root = server_tree_root(state);
    let own_root = namespace_path(user.namespace_root())?;
    let mut children = vec![crate::tree::walk_or_empty(&tree_root, &own_root, opts)?];

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
                &owner_root,
                &shared_paths,
                opts,
            )?);
        }
    }

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
        return crate::tree::walk(&tree_root, requested, opts)
            .map_err(|e| ApiError::new("io", e.to_string()));
    };

    let tree_root = server_tree_root(state);
    let requested_slash = requested.to_slash_string();
    let direct_level =
        crate::auth::access::resolve_access(user, &requested_slash, auth_store).await;
    if direct_level.can_read() {
        let walk_result = if requested.len() == 1 {
            crate::tree::walk_or_empty(&tree_root, requested, opts)
        } else {
            crate::tree::walk(&tree_root, requested, opts)
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

    crate::tree::walk_shared_projection(&tree_root, requested, &shared_paths, opts)
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
    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();
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
    let ontology = system_schema::build_exom_ontology(&exom_slash, brain, &es.rules);
    let status = serde_json::json!({
        "ok": true,
        "exom": exom_slash,
        "current_branch": brain.current_branch_id(),
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
    let opts = crate::tree::WalkOptions {
        depth: q.depth.or(Some(usize::MAX)),
        include_archived: q.archived.as_deref() == Some("true"),
        include_branches: q.branches.as_deref() == Some("true"),
        include_activity: q.activity.as_deref() == Some("true"),
    };
    let result = if let Some(ref user) = maybe_user.0 {
        match q.path.as_deref().filter(|s| !s.is_empty()) {
            None => {
                if user.is_admin() {
                    build_tree_root_for_admin(&state, user, &opts).await
                } else {
                    build_tree_root_for_user(&state, user, &opts).await
                }
            }
            Some(p) => match p.parse::<crate::path::TreePath>() {
                Ok(tp) => {
                    if user.is_admin() {
                        let walk_result = if tp.len() == 1 {
                            crate::tree::walk_or_empty(&tree_root, &tp, &opts)
                        } else {
                            crate::tree::walk(&tree_root, &tp, &opts)
                        };
                        walk_result
                    } else {
                        match build_tree_path_for_user(&state, user, &tp, &opts).await {
                            Ok(node) => return Json(node).into_response(),
                            Err(err) => return err.into_response(),
                        }
                    }
                }
                Err(e) => {
                    let err = ApiError::new("bad_path", e.to_string());
                    return err.into_response();
                }
            },
        }
    } else {
        match q.path.as_deref().filter(|s| !s.is_empty()) {
            None => crate::tree::walk_root(&tree_root, &opts),
            Some(p) => match p.parse::<crate::path::TreePath>() {
                Ok(tp) => crate::tree::walk(&tree_root, &tp, &opts),
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

async fn api_exoms_gone() -> impl IntoResponse {
    ApiError::new("gone", "/api/exoms is removed; use /api/tree instead")
        .with_status(410)
        .into_response()
}

#[derive(Deserialize)]
struct PathBody {
    path: Option<String>,
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
    match crate::scaffold::init_project(&tree_root, &path) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "path": path.to_slash_string()})).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
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
    match crate::scaffold::new_bare_exom(&tree_root, &path) {
        Ok(()) => {
            emit_tree_changed(&state);
            Json(serde_json::json!({"ok": true, "path": path.to_slash_string()})).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
struct SessionNewBody {
    project_path: Option<String>,
    #[serde(rename = "type")]
    session_type: Option<String>,
    label: Option<String>,
    actor: Option<String>,
    #[serde(default)]
    agents: Vec<String>,
}

async fn api_session_new(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
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
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        body.actor.unwrap_or_default()
    };
    let tree_root = server_tree_root(&state);
    match brain::session_new(
        &tree_root,
        &project_path,
        session_type,
        &label,
        &actor,
        &body.agents,
    ) {
        Ok(session_path) => {
            emit_tree_changed(&state);
            if let Some(ref db) = state.exom_db {
                let disk = session_path.to_disk_path(&tree_root);
                let slash = session_path.to_slash_string();
                let db = db.clone();
                if let Err(e) = exom_db_sync_from_disk(&db, &slash, &disk).await {
                    eprintln!("[ray-exomem] warning: ExomDb session sync failed for {slash}: {e}");
                }
            }
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
    actor: Option<String>,
}

async fn api_session_join(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    Json(body): Json<SessionJoinBody>,
) -> impl IntoResponse {
    let session_path_str = body.session_path.unwrap_or_default();
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        body.actor.unwrap_or_default()
    };
    let session_path: crate::path::TreePath = match session_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if let Some(resp) = guard_write(&state, &maybe_user, &session_path.to_slash_string()).await {
        return resp;
    }
    if actor.is_empty() {
        return ApiError::new("actor_required", "actor required")
            .with_suggestion("pass actor in request body")
            .into_response();
    }
    let tree_root = server_tree_root(&state);
    match brain::session_join(&tree_root, &session_path, &actor) {
        Ok(branch) => {
            if let Some(ref db) = state.exom_db {
                let disk = session_path.to_disk_path(&tree_root);
                let slash = session_path.to_slash_string();
                let db = db.clone();
                if let Err(e) = exom_db_sync_from_disk(&db, &slash, &disk).await {
                    eprintln!("[ray-exomem] warning: ExomDb session sync failed for {slash}: {e}");
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "session_path": session_path.to_slash_string(),
                "actor": actor,
                "branch": branch,
            }))
            .into_response()
        }
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
    match brain::create_branch(&tree_root, &exom_path, &branch_name) {
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
    actor: Option<String>,
    #[serde(default)]
    branch: Option<String>,
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
    Json(req): Json<AssertFactBody>,
) -> impl IntoResponse {
    let exom_raw = req.exom.as_deref().unwrap_or(DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    if let Some(resp) = guard_write(&state, &maybe_user, &exom_slash).await {
        return resp;
    }

    let actor_str = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        match req.actor.as_deref().filter(|s| !s.is_empty()) {
            Some(a) => a.to_string(),
            None => {
                return ApiError::from(brain::WriteError::ActorRequired).into_response();
            }
        }
    };

    let branch_str = req.branch.as_deref().unwrap_or("main");

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, branch_str, &actor_str) {
            return ApiError::from(e).into_response();
        }
    }

    let user_email = maybe_user.0.as_ref().map(|u| u.email.clone());
    let write_ctx = MutationContext {
        actor: actor_str.clone(),
        session: None,
        model: None,
        user_email,
    };

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
    let rule_inline_bodies: Vec<String> = {
        let es = exoms
            .get(&exom_name)
            .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
        combined_rules(&exom_name, &es.rules)?
            .into_iter()
            .map(|rule| rule.inline_body)
            .collect()
    };
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

fn eval_query_form(
    exoms: &HashMap<String, ExomState>,
    engine: &crate::backend::RayforceEngine,
    query: &CanonicalQuery,
) -> anyhow::Result<(String, Option<serde_json::Value>)> {
    let expanded = expand_canonical_query(exoms, engine, query.emit(), query)?;
    bind_typed_facts_for_exom(engine, exoms, &expanded.exom_name)?;
    let raw = engine.eval_raw(&expanded.expanded_query)?;
    if unsafe { ffi::ray_obj_type(raw.as_ptr()) } == ffi::RAY_TABLE {
        let decoded = storage::decode_query_table(&raw, &expanded.normalized_query)?;
        Ok((storage::format_decoded_query_table(&decoded), Some(decoded)))
    } else {
        Ok((engine.format_obj(&raw)?, None))
    }
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
    let source = format!(
        "(query {exom} (find {vars_joined}) (where ({predicate} {vars_joined})))"
    );
    let query = lower_query_request(&source, None, "schema relation sample")?;
    let expanded = expand_canonical_query(exoms, engine, source, &query)?;
    bind_typed_facts_for_exom(engine, exoms, &expanded.exom_name)?;
    let raw = engine.eval_raw(&expanded.expanded_query)?;
    if unsafe { ffi::ray_obj_type(raw.as_ptr()) } != ffi::RAY_TABLE {
        return Ok(Vec::new());
    }
    let decoded = storage::decode_query_table(&raw, &expanded.normalized_query)?;
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
    let exoms = state.exoms.lock().unwrap();
    let expanded = match expand_canonical_query(&exoms, &state.engine, source.clone(), &query) {
        Ok(e) => e,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response();
        }
    };
    if let Err(e) = bind_typed_facts_for_exom(&state.engine, &exoms, &expanded.exom_name) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("failed to bind typed facts: {e}")})),
        )
            .into_response();
    }
    match state.engine.eval_raw(&expanded.expanded_query) {
        Ok(raw) => {
            let (output, decoded) = if unsafe { ffi::ray_obj_type(raw.as_ptr()) } == ffi::RAY_TABLE
            {
                match storage::decode_query_table(&raw, &expanded.normalized_query) {
                    Ok(d) => (storage::format_decoded_query_table(&d), Some(d)),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": e.to_string()})),
                        )
                            .into_response();
                    }
                }
            } else {
                match state.engine.format_obj(&raw) {
                    Ok(s) => (s, None),
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": e.to_string()})),
                        )
                            .into_response();
                    }
                }
            };
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
            drop(exoms);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.to_string()})),
            )
                .into_response()
        }
    }
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
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let ctx = if let Some(ref user) = maybe_user.0 {
        MutationContext::from_user(
            user,
            headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
        )
    } else {
        let actor = headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string();
        MutationContext {
            actor,
            session: headers
                .get("x-session")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            model: headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            user_email: None,
        }
    };
    api_eval_inner(state, maybe_user, ctx, body).await
}

async fn api_eval_inner(
    state: Arc<AppState>,
    maybe_user: MaybeUser,
    ctx: MutationContext,
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
                if let Err(e) =
                    crate::auth::access::authorize_rayfall(user, &canonical_forms, auth_store).await
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
                        &fact_id,
                        &pred,
                        &value,
                        1.0,
                        "rayfall-eval",
                        None,
                        None,
                        &ctx,
                    )?;
                    es.datoms = storage::build_datoms_table(&es.brain)?;
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
                    let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"eval_assert_fact","exom":"{}","actor":"{}","predicate":"{}"}}"#, exom, ctx.actor, pred));
                    Ok(())
                })();
                match r {
                    Ok(()) => {
                        persist_exom_storage(&state, &exom).await;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
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
                    es.brain.retract_fact_exact(&fact_id, &pred, &value, &ctx)?;
                    es.datoms = storage::build_datoms_table(&es.brain)?;
                    state
                        .engine
                        .bind_named_db(storage::sym_intern(&exom), &es.datoms)?;
                    if let Some(_disk) = es.exom_disk.as_ref() {
                        es.brain.save()?;
                    }
                    let _ = state.sse_tx.send(format!(r#"{{"v":1,"kind":"memory","op":"eval_retract_fact","exom":"{}","actor":"{}","predicate":"{}"}}"#, exom, ctx.actor, pred));
                    Ok(())
                })();
                match r {
                    Ok(()) => {
                        persist_exom_storage(&state, &exom).await;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
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
                    es.datoms = storage::build_datoms_table(&es.brain)?;
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
                    let _ = state.sse_tx.send(format!(
                        r#"{{"v":1,"kind":"memory","op":"rule_append","exom":"{}","actor":"{}"}}"#,
                        exom_name, ctx.actor
                    ));
                    Ok(())
                })();
                match r {
                    Ok(()) => {
                        persist_exom_storage(&state, &exom_name).await;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            EvalForm::Canonical(CanonicalForm::Query(query)) => {
                let exoms = state.exoms.lock().unwrap();
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
// POST /actions/consolidate-propose (501)
// ---------------------------------------------------------------------------

async fn api_consolidate_propose(
    State(_state): State<Arc<AppState>>,
    _maybe_user: MaybeUser,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(
            serde_json::json!({"ok": false, "error": "consolidation propose API is not implemented yet"}),
        ),
    )
}

// ---------------------------------------------------------------------------
// GET /facts/valid-at
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExomQuery {
    exom: Option<String>,
}

#[derive(Deserialize)]
struct ValidAtQuery {
    exom: Option<String>,
    timestamp: Option<String>,
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
    let entries: Vec<_> = es
        .brain
        .facts_valid_at(&timestamp)
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
                        "event_type": tx.action.to_string()
                    })
                })
                .collect();
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
                    "revoked_by": f.revoked_by_tx.map(|tx| format!("tx/{}", tx))
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

fn fact_json_enriched(brain: &Brain, f: &crate::brain::Fact) -> serde_json::Value {
    let tx = brain
        .transactions()
        .iter()
        .find(|t| t.tx_id == f.created_by_tx);
    let (actor, branch_id, tx_time) = match tx {
        Some(t) => (t.actor.as_str(), t.branch_id.as_str(), t.tx_time.as_str()),
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
    /// Branch id or name; omit = current branch.
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
    } else if let Some(ref key) = q.branch {
        let bid = match resolve_branch_key(brain, key) {
            Some(id) => id,
            None => {
                return ApiError::new("unknown_branch", format!("no branch matching {:?}", key))
                    .into_response();
            }
        };
        brain
            .facts_on_branch(bid)
            .into_iter()
            .map(|f| fact_json_enriched(brain, f))
            .collect()
    } else {
        brain
            .current_facts()
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
                "is_current": b.branch_id == es.brain.current_branch_id(),
                "fact_count": es.brain.facts_on_branch(&b.branch_id).len(),
                "claimed_by": b.claimed_by,
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
}

async fn api_create_branch(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateBranchBody>,
) -> impl IntoResponse {
    let ctx = if let Some(ref user) = maybe_user.0 {
        MutationContext::from_user(
            user,
            headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
        )
    } else {
        let actor = headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string();
        MutationContext {
            actor: actor.clone(),
            session: headers
                .get("x-session")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            model: headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            user_email: None,
        }
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

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, "main", &ctx.actor) {
            return ApiError::from(e).into_response();
        }
    }

    let bid = body.branch_id.clone();
    let name = body.name.unwrap_or_else(|| bid.clone());
    let bid2 = bid.clone();
    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        ex.brain.create_branch(&bid2, &name, &ctx)
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
            "is_current": b.branch_id == es.brain.current_branch_id(),
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
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string()
    };
    let _ctx = MutationContext {
        actor: actor.clone(),
        session: headers
            .get("x-session")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        model: headers
            .get("x-model")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        user_email: maybe_user.0.as_ref().map(|u| u.email.clone()),
    };
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
// POST /branches/:id/switch
// ---------------------------------------------------------------------------

async fn api_switch_branch_handler(
    State(state): State<Arc<AppState>>,
    maybe_user: MaybeUser,
    AxumPath(branch_id): AxumPath<String>,
    Query(q): Query<ExomQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string()
    };
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
        ex.brain.switch_branch(&bid)?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => {
            let _ = state.sse_tx.send(format!(
                r#"{{"v":1,"kind":"memory","op":"branch_switch","exom":"{}","actor":"{}"}}"#,
                exom_slash, actor
            ));
            Json(serde_json::json!({"switched_to": branch_id})).into_response()
        }
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
    let ctx = if let Some(ref user) = maybe_user.0 {
        MutationContext::from_user(
            user,
            headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
        )
    } else {
        let actor = headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string();
        MutationContext {
            actor,
            session: headers
                .get("x-session")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            model: headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            user_email: None,
        }
    };
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
    let src = source_branch.clone();
    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        let target = ex.brain.current_branch_id().to_string();
        ex.brain.merge_branch(&src, &target, policy, &ctx)
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
    let facts = es.brain.current_facts();
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
    let facts = es.brain.current_facts();
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

    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string()
    };
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

    let _ctx = MutationContext {
        actor: actor.clone(),
        session: headers
            .get("x-session")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        model: headers
            .get("x-model")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        user_email: maybe_user.0.as_ref().map(|u| u.email.clone()),
    };

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
            let _ = state.sse_tx.send(format!(
                r#"{{"v":1,"kind":"memory","op":"import_json","exom":"{}","actor":"{}"}}"#,
                exom_slash, actor
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
    let ctx = if let Some(ref user) = maybe_user.0 {
        MutationContext::from_user(
            user,
            headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
        )
    } else {
        let actor = headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string();
        MutationContext {
            actor: actor.clone(),
            session: headers
                .get("x-session")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            model: headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            user_email: None,
        }
    };
    let actor = ctx.actor.clone();
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
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, "main", &ctx.actor) {
            return ApiError::from(e).into_response();
        }
    }

    let result = mutate_exom_async(&state, &exom_slash, move |ex| {
        let fact_ids: Vec<String> = ex
            .brain
            .current_facts()
            .iter()
            .map(|f| f.fact_id.clone())
            .collect();
        let count = fact_ids.len();
        for id in &fact_ids {
            let _ = ex.brain.retract_fact(id, &ctx);
        }
        ex.rules.clear();
        Ok(count)
    })
    .await;

    match result {
        Ok(count) => {
            let _ = state.sse_tx.send(format!(
                r#"{{"v":1,"kind":"memory","op":"retract_all","exom":"{}","actor":"{}"}}"#,
                exom_slash, actor
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
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string()
    };
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
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, "main", &actor) {
            return ApiError::from(e).into_response();
        }
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
            let _ = state.sse_tx.send(format!(
                r#"{{"v":1,"kind":"memory","op":"wipe","exom":"{}","actor":"{}"}}"#,
                exom_slash, actor
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
        if let Some(ref user) = maybe_user.0 {
            if !user.is_admin() {
                return ApiError::new("forbidden", "factory-reset requires admin access")
                    .with_status(403)
                    .into_response();
            }
        }
    }
    let actor = if let Some(ref user) = maybe_user.0 {
        user.email.clone()
    } else {
        headers
            .get("x-actor")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anonymous")
            .to_string()
    };
    let mut exoms = state.exoms.lock().unwrap();
    let old_names: Vec<String> = exoms.keys().cloned().collect();
    exoms.clear();

    let default_exom = DEFAULT_EXOM;
    let new_es = ExomState {
        brain: Brain::new(),
        datoms: match storage::build_datoms_table(&Brain::new()) {
            Ok(d) => d,
            Err(e) => {
                return ApiError::new("error", e.to_string()).into_response();
            }
        },
        typed_facts: match storage::build_typed_fact_tables(&Brain::new()) {
            Ok(t) => t,
            Err(e) => {
                return ApiError::new("error", e.to_string()).into_response();
            }
        },
        rules: Vec::new(),
        exom_disk: None,
    };
    exoms.insert(default_exom.to_string(), new_es);

    reconcile_engine(&state, &exoms);
    let _ = state.sse_tx.send(format!(
        r#"{{"v":1,"kind":"memory","op":"factory_reset","exom":"*","actor":"{}"}}"#,
        actor
    ));
    Json(serde_json::json!({
        "ok": true,
        "removed_exoms": old_names,
        "state": "clean"
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /schema
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SchemaQuery {
    exom: Option<String>,
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

    let mut exoms = state.exoms.lock().unwrap();
    let tree_root = state.tree_root.as_deref();
    let sym_path = state.sym_path.as_deref();
    let mut relations = Vec::new();
    let (fact_groups, has_intervals_map, obs_tuples, belief_tuples, has_belief_intervals, all_rules, ontology) =
        {
            let es =
                match get_or_load_exom(&mut exoms, &state.engine, &exom_slash, tree_root, sym_path)
                {
                    Ok(e) => e,
                    Err(e) => return ApiError::new("unknown_exom", e.to_string()).into_response(),
                };
            let brain = &es.brain;
            let all_rules = match combined_rules(&exom_slash, &es.rules) {
                Ok(r) => r,
                Err(e) => return ApiError::new("error", e.to_string()).into_response(),
            };

            let mut fact_groups: HashMap<String, Vec<Vec<serde_json::Value>>> = HashMap::new();
            let mut has_intervals_map: HashMap<String, bool> = HashMap::new();
            for fact in brain.current_facts() {
                let entry = fact_groups.entry(fact.predicate.clone()).or_default();
                entry.push(vec![
                    serde_json::Value::String(fact.fact_id.clone()),
                    serde_json::Value::String(fact.predicate.clone()),
                    serde_json::to_value(&fact.value).unwrap_or_else(|_| {
                        serde_json::Value::String(fact.value.display())
                    }),
                    serde_json::json!(fact.confidence),
                    serde_json::json!({
                        "valid_from": fact.valid_from,
                        "valid_to": fact.valid_to,
                        "branch_origin": brain.tx_branch(fact.created_by_tx).unwrap_or(""),
                        "branch_role": brain.fact_branch_role(fact, brain.current_branch_id()),
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

            let beliefs = brain.current_beliefs();
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

            let ontology = system_schema::build_exom_ontology(&exom_slash, brain, &es.rules);
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
    let facts = es.brain.current_facts();
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
    let facts = es.brain.current_facts();
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
    let pred = id.strip_prefix("cluster:").unwrap_or(&id);
    let facts = es.brain.current_facts();
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
                "timestamp": tx.tx_time, "pattern": tx.note, "source": tx.actor
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
    let facts = es.brain.current_facts();
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

async fn api_relation_graph(
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
    let facts = es.brain.current_facts();
    let mut preds: HashMap<&str, usize> = HashMap::new();
    for f in &facts {
        *preds.entry(&f.predicate).or_default() += 1;
    }
    let nodes: Vec<_> = preds
        .iter()
        .map(|(pred, count)| serde_json::json!({"id": *pred, "label": *pred, "degree": count}))
        .collect();
    Json(serde_json::json!({
        "nodes": nodes, "edges": [],
        "summary": {"node_count": nodes.len(), "edge_count": 0}
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
    let beliefs: Vec<_> = brain
        .current_beliefs()
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
    for id in &b.supported_by {
        if let Some(f) = brain.current_facts().iter().find(|f| f.fact_id == *id) {
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
// POST /actions/start-session → 410
// ---------------------------------------------------------------------------

async fn api_start_session_gone() -> impl IntoResponse {
    ApiError::new(
        "gone",
        "POST /api/actions/start-session is removed; use POST /api/actions/session-new",
    )
    .with_status(410)
    .into_response()
}
