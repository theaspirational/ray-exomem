use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};

use crate::{
    backend::RayforceEngine,
    brain::{self, Brain},
    context::{self, MutationContext},
    http_error::ApiError,
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
        }
    }

    /// Build AppState by loading a data directory (same logic as web::serve).
    pub fn from_data_dir(data_dir: Option<PathBuf>) -> anyhow::Result<Arc<Self>> {
        use crate::{exom::ExomDir, web::DEFAULT_EXOM};

        let engine = RayforceEngine::new()?;
        let mut exoms: HashMap<String, ExomState> = HashMap::new();
        let (tree_root, sym_path) = match data_dir {
            Some(ref root) => {
                let ed = ExomDir::open(root.clone())?;
                let sym = ed.sym_path();
                let tree_dir = root.join("tree");
                if tree_dir.exists() {
                    load_tree_exoms_into(&tree_dir, &sym, &mut exoms);
                }
                // Load legacy flat exoms.
                for name in ed.list_exoms()? {
                    if exoms.contains_key(&name) {
                        continue;
                    }
                    eprintln!("[ray-exomem] loading flat exom '{}'", name);
                    if let Ok(es) = load_flat_exom(&ed, &name) {
                        exoms.insert(name, es);
                    }
                }
                if exoms.is_empty() {
                    ed.create_exom(DEFAULT_EXOM)?;
                    if let Ok(es) = load_flat_exom(&ed, DEFAULT_EXOM) {
                        exoms.insert(DEFAULT_EXOM.to_string(), es);
                    }
                }
                let tree_root = root.join("tree");
                std::fs::create_dir_all(&tree_root).ok();
                (Some(tree_root), Some(sym))
            }
            None => {
                exoms.insert(DEFAULT_EXOM.to_string(), ExomState {
                    brain: Brain::new(),
                    datoms: storage::build_datoms_table(&Brain::new())?,
                    rules: Vec::new(),
                    exom_disk: None,
                });
                (None, None)
            }
        };

        // Bind all exoms into the engine.
        for (name, es) in &exoms {
            engine.bind_named_db(storage::sym_intern(name), &es.datoms)?;
        }

        Ok(Arc::new(AppState::new(engine, exoms, tree_root, sym_path)))
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
        let Ok(rd) = std::fs::read_dir(current) else { return; };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue; };
            if !ft.is_dir() { continue; }
            let disk = entry.path();
            let rel = disk.strip_prefix(tree_root).unwrap_or(&disk);
            let slash_key = rel.components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>().join("/");
            if slash_key.is_empty() { continue; }
            let meta_p = disk.join(crate::exom::META_FILENAME);
            if meta_p.exists() {
                eprintln!("[ray-exomem] loading tree exom '{}'", slash_key);
                match load_exom_from_tree_path_inner(&disk, sym_path, &slash_key) {
                    Ok(es) => { out.insert(slash_key.clone(), es); }
                    Err(e) => { eprintln!("[ray-exomem] WARNING: failed to load '{}': {}", slash_key, e); }
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

fn load_flat_exom(ed: &crate::exom::ExomDir, name: &str) -> anyhow::Result<ExomState> {
    let brain = {
        let b = Brain::open_exom(&ed.exom_path(name), &ed.sym_path())?;
        b.ensure_jsonl_sidecars()?;
        b
    };
    let datoms = storage::build_datoms_table(&brain)?;
    let rules_p = ed.exom_path(name).join("rules.ray");
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
    Ok(ExomState { brain, datoms, rules, exom_disk: None })
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
}

pub async fn serve(bind: &str, state: Arc<AppState>) -> anyhow::Result<()> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Nested under /ray-exomem/api
        .nest("/ray-exomem/api", api_router())
        // Compat shim: smoke test calls /api/status
        .route("/api/status", get(api_status))
        .with_state(state)
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

fn load_exom_from_tree_path(
    exom_disk: &std::path::Path,
    sym_path: &std::path::Path,
    slash_key: &str,
) -> anyhow::Result<ExomState> {
    let mut brain = {
        let b = Brain::open_exom(exom_disk, sym_path)?;
        b.ensure_jsonl_sidecars()?;
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
        rules,
        exom_disk: Some(exom_disk.to_path_buf()),
    })
}

fn combined_rules(exom: &str, user_rules: &[ParsedRule]) -> anyhow::Result<Vec<ParsedRule>> {
    let mut rules = system_schema::builtin_rules(exom)?;
    rules.extend_from_slice(user_rules);
    Ok(rules)
}

fn refresh_exom_binding(state: &AppState, exoms: &mut HashMap<String, ExomState>, exom_name: &str) -> anyhow::Result<()> {
    let es = exoms
        .get_mut(exom_name)
        .ok_or_else(|| anyhow::anyhow!("unknown exom '{}'", exom_name))?;
    es.datoms = storage::build_datoms_table(&es.brain)?;
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
    }
    Ok(())
}

fn mutate_exom<T>(
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
    let _ = state.sse_tx.send(format!(r#"{{"kind":"memory","exom":"{}"}}"#, exom_name));
    Ok(out)
}

fn emit_tree_changed(state: &AppState) {
    let _ = state
        .sse_tx
        .send(r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string());
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn api_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let exom = crate::web::DEFAULT_EXOM;
    let mut exoms = state.exoms.lock().unwrap();
    let tree_root_val = state.tree_root.as_deref();
    let sym_path_val = state.sym_path.as_deref();
    if !exoms.contains_key(exom) {
        let _ = get_or_load_exom(&mut exoms, &state.engine, exom, tree_root_val, sym_path_val);
    }
    let Some(es) = exoms.get(exom) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "exom not found"})),
        );
    };
    let brain = &es.brain;
    let uptime = state.start_time.elapsed().as_secs();
    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();
    let all_rules = match combined_rules(exom, &es.rules) {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))),
    };
    let derived_names: Vec<String> = rules::derived_predicates(&all_rules)
        .into_iter()
        .map(|(n, _)| n)
        .take(24)
        .collect();
    let ontology = system_schema::build_exom_ontology(exom, brain, &es.rules);
    let tree_root_display = server_tree_root(&state).display().to_string();
    let status = serde_json::json!({
        "ok": true,
        "exom": exom,
        "current_branch": brain.current_branch_id(),
        "server": {
            "name": "ray-exomem",
            "version": crate::frontend_version(),
            "uptime_sec": uptime,
            "tree_root": tree_root_display,
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
    (StatusCode::OK, Json(status))
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
    Query(q): Query<TreeQuery>,
) -> impl IntoResponse {
    let tree_root = server_tree_root(&state);
    let opts = crate::tree::WalkOptions {
        depth: q.depth.or(Some(usize::MAX)),
        include_archived: q.archived.as_deref() == Some("true"),
        include_branches: q.branches.as_deref() == Some("true"),
        include_activity: q.activity.as_deref() == Some("true"),
    };
    let result = match q.path.as_deref().filter(|s| !s.is_empty()) {
        None => crate::tree::walk_root(&tree_root, &opts),
        Some(p) => match p.parse::<crate::path::TreePath>() {
            Ok(tp) => crate::tree::walk(&tree_root, &tp, &opts),
            Err(e) => {
                let err = ApiError::new("bad_path", e.to_string());
                return err.into_response();
            }
        },
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
        [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
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
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
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
    Json(body): Json<PathBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
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
    Json(body): Json<SessionNewBody>,
) -> impl IntoResponse {
    let project_path_str = body.project_path.unwrap_or_default();
    let project_path: crate::path::TreePath = match project_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
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
    let actor = body.actor.unwrap_or_default();
    let tree_root = server_tree_root(&state);
    match brain::session_new(&tree_root, &project_path, session_type, &label, &actor, &body.agents) {
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
    actor: Option<String>,
}

async fn api_session_join(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionJoinBody>,
) -> impl IntoResponse {
    let session_path_str = body.session_path.unwrap_or_default();
    let actor = body.actor.unwrap_or_default();
    let session_path: crate::path::TreePath = match session_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
    if actor.is_empty() {
        return ApiError::new("actor_required", "actor required")
            .with_suggestion("pass actor in request body")
            .into_response();
    }
    let tree_root = server_tree_root(&state);
    match brain::session_join(&tree_root, &session_path, &actor) {
        Ok(branch) => Json(serde_json::json!({
            "ok": true,
            "session_path": session_path.to_slash_string(),
            "actor": actor,
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
    Json(body): Json<BranchCreateBody>,
) -> impl IntoResponse {
    let exom_path_str = body.exom_path.unwrap_or_default();
    let branch_name = body.branch_name.unwrap_or_default();
    let actor = body.actor.unwrap_or_default();
    let exom_path: crate::path::TreePath = match exom_path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
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
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    let path_str = body.path.unwrap_or_default();
    let new_segment = body.new_segment.unwrap_or_default();
    let path: crate::path::TreePath = match path_str.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_path", e.to_string()).into_response(),
    };
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
    value: String,
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
    Json(req): Json<AssertFactBody>,
) -> impl IntoResponse {
    let exom_raw = req.exom.as_deref().unwrap_or(crate::web::DEFAULT_EXOM);
    let exom_path: crate::path::TreePath = match exom_raw.parse() {
        Ok(p) => p,
        Err(e) => return ApiError::new("bad_exom_path", e.to_string()).into_response(),
    };
    let exom_slash = exom_path.to_slash_string();

    let actor_str = match req.actor.as_deref().filter(|s| !s.is_empty()) {
        Some(a) => a.to_string(),
        None => {
            return ApiError::from(brain::WriteError::ActorRequired).into_response();
        }
    };

    let branch_str = req.branch.as_deref().unwrap_or("main");

    if state.tree_root.is_some() {
        let tree_root = server_tree_root(&state);
        if let Err(e) = brain::precheck_write(&tree_root, &exom_path, branch_str, &actor_str) {
            return ApiError::from(e).into_response();
        }
    }

    let write_ctx = MutationContext {
        actor: actor_str.clone(),
        session: None,
        model: None,
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

    let result = mutate_exom(&state, &exom_slash, |ex| {
        ex.brain.assert_fact(
            &fact_id,
            &predicate,
            &value,
            confidence,
            &provenance,
            valid_from.as_deref(),
            valid_to.as_deref(),
            &write_ctx,
        )
    });

    match result {
        Ok(tx_id) => {
            if state.tree_root.is_some()
                && matches!(
                    predicate.as_str(),
                    "session/label" | "session/closed_at" | "session/archived_at"
                )
            {
                let tree_root = server_tree_root(&state);
                if let Err(e) = brain::mirror_session_meta_to_disk(
                    &tree_root,
                    &exom_path,
                    &predicate,
                    &value,
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
