mod aesthetic;
mod api;
mod artifacts;
mod cache_config;
mod jobs;
mod lake_locations;
mod lake_updates;
mod lake_worker_bundle;
mod llm_invocations;
mod previews;
mod query_budget;
mod query_cache;
mod query_jobs;
mod ranked_indexes;
mod ranking;
mod ranking_query;
mod ranking_reads;
mod source_indexes;
mod sources;
mod tool_inputs;
mod worker;
use clap::{Parser, Subcommand};
use fs2::FileExt;
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    sync::Arc,
};
use studio_domain::*;
use studio_protocol::{API_VERSION, EngineConnection};
use studio_storage::{SqliteStore, atomic_json};
use utoipa::OpenApi;
#[derive(Parser)]
#[command(name = "studio-engine", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        data_dir: PathBuf,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    Worker {
        #[arg(long)]
        plan: PathBuf,
    },
    Schema {
        #[arg(long)]
        output: PathBuf,
    },
    ProbeDuckdb {
        #[arg(long)]
        dll: PathBuf,
        #[arg(long)]
        database: PathBuf,
    },
    ExplainQuery {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "studio_engine=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let result = match Cli::parse().command {
        Command::Serve {
            data_dir,
            port,
            cache_dir,
        } => serve(data_dir, port, cache_dir).await,
        Command::Worker { plan } => worker::run_reported(&plan),
        Command::Schema { output } => {
            if let Some(parent) = output.parent() {
                let _ = fs::create_dir_all(parent);
            }
            atomic_json(&output, &api::ApiDoc::openapi())
        }
        Command::ProbeDuckdb { dll, database } => {
            studio_sources::duckdb_probe::probe(&dll, &database).map(|result| println!("{result}"))
        }
        Command::ExplainQuery {
            source,
            spec,
            output,
        } => explain_query(source, spec, output),
    };
    if let Err(error) = result {
        tracing::error!(code=error.code,message=%error.message,"engine stopped");
        std::process::exit(1);
    }
}
fn explain_query(source: PathBuf, spec: PathBuf, output: PathBuf) -> Result<()> {
    let source: Source =
        serde_json::from_slice(&fs::read(source).map_err(Error::io)?).map_err(Error::io)?;
    let spec: QuerySpec =
        serde_json::from_slice(&fs::read(spec).map_err(Error::io)?).map_err(Error::io)?;
    use studio_application::QueryAdapter;
    let resources = Arc::new(studio_resources::ReadCoordinator::default());
    let service = sources::SourceService::new(
        studio_sources::registry(
            output
                .parent()
                .ok_or_else(|| Error::invalid("缺少输出目录"))?
                .join("query-temp"),
            None,
            None,
        )?,
        resources,
    );
    let read = service.background(
        ReadClass::NativeQuery,
        QUERY_MEMORY_BYTES,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )?;
    let plan = read
        .query(QUERY_MEMORY_BYTES, false)
        .explain(&source, spec)?;
    atomic_json(&output, &plan)
}
async fn serve(root: PathBuf, port: u16, cache_dir: Option<PathBuf>) -> Result<()> {
    fs::create_dir_all(&root).map_err(Error::io)?;
    let root = root.canonicalize().map_err(Error::io)?;
    let lease = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("engine.lock"))
        .map_err(Error::io)?;
    lease
        .try_lock_exclusive()
        .map_err(|_| Error::new("ENGINE_BUSY", "已有引擎管理这个应用目录"))?;
    remove_abandoned_query_temps(&root.join("query-temp"), "query-")?;
    remove_abandoned_query_temps(&root.join("browse-index"), "index-build-")?;
    remove_abandoned_query_temps(&root.join("ranked-index"), "rank-build-")?;
    let store = Arc::new(SqliteStore::new(root.clone())?);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(Error::io)?;
    let connection = EngineConnection {
        api_version: API_VERSION,
        instance_id: new_id(),
        pid: std::process::id(),
        endpoint: format!("http://{}", listener.local_addr().map_err(Error::io)?),
        token: format!("{}{}", new_id(), new_id()),
    };
    atomic_json(&root.join("engine.json"), &connection)?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let coordinator = Arc::new(studio_resources::ReadCoordinator::default());
    let query_budget = Arc::new(query_budget::QueryBudget::open(
        root.join("query-settings.json"),
        coordinator.clone(),
    )?);
    let resources: Arc<dyn studio_application::ReadResources> = coordinator;
    let cache_path = cache_dir
        .or_else(|| std::env::var_os("STUDIO_CACHE_DIR").map(PathBuf::from))
        .unwrap_or_else(|| root.join("preview-cache"));
    let preview_cache = studio_resources::PreviewCache::open(&cache_path)?;
    let cache = Arc::new(query_cache::CacheControl::open(
        root.join("query-cache.json"),
        (preview_cache.metrics().quota_bytes >> 20) as u32,
    )?);
    let handles = source_indexes::SourceIndexHandles::new(root.join("browse-index"));
    let sources = sources::SourceService::new(
        studio_sources::registry(
            root.join("query-temp"),
            Some(handles.identities.clone()),
            Some(handles.ratings.clone()),
        )?,
        resources.clone(),
    );
    let source_indexes = source_indexes::SourceIndexService::new(
        handles,
        sources.clone(),
        query_budget.clone(),
        cache.clone(),
        root.join("query-temp"),
    );
    let previews = previews::PreviewService::new(resources.clone(), preview_cache, sources.clone());
    let queries = Arc::new(query_jobs::QueryRunner::new(
        root.join("query-temp"),
        query_budget,
        cache,
        source_indexes,
        sources.clone(),
        root.join("browse-index"),
    ));
    previews
        .cache
        .set_quota(u64::from(queries.cache.config()?.preview_mib) << 20)?;
    let aesthetic = Arc::new(aesthetic::Runner::default());
    let aesthetic_analysis = Arc::new(aesthetic::analysis::Runner::default());
    use studio_application::lake_updates::LakeUpdateBackend;
    let lake_updates = Arc::new(lake_updates::Backend::new(root.clone()));
    let lake_update_supervisor = tokio::spawn(lake_updates.clone().supervise(store.clone()));
    if lake_updates.configured()
        && let Err(error) = lake_updates.ensure_worker()
    {
        tracing::warn!(
            code = error.code,
            "lake update worker unavailable; browsing remains available"
        );
    }
    let state = api::AppState {
        lake_updates: lake_updates.clone(),
        aesthetic: aesthetic.clone(),
        aesthetic_analysis: aesthetic_analysis.clone(),
        llm: {
            let credentials = Arc::new(studio_llm::credentials::CredentialVault::new(
                root.join("llm-credentials"),
            ));
            let backend = Arc::new(studio_llm::RemoteLlm::new(credentials.clone()));
            Arc::new(studio_application::llm::LlmService::new(
                store.clone(),
                backend,
                credentials,
            ))
        },
        llm_invocations: Default::default(),
        store: store.clone(),
        connection: connection.clone(),
        resources: resources.clone(),
        previews: previews.clone(),
        sources: sources.clone(),
        queries: queries.clone(),
        ranking_reads: Arc::new(ranking_reads::RankingReadCache::default()),
        shutdown: shutdown_tx.clone(),
    };
    store.initialize_lake_inputs()?;
    let lake_input_supervisor = tokio::spawn(api::lake_inputs::supervise(state.clone()));
    let token = connection.token.clone();
    let request_store = store.clone();
    let request_previews = previews.clone();
    let auth = axum::middleware::from_fn(
        move |mut request: axum::extract::Request, next: axum::middleware::Next| {
            let token = token.clone();
            let store = request_store.clone();
            let previews = request_previews.clone();
            async move {
                use axum::response::IntoResponse;
                let supplied = request
                    .headers()
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "));
                if supplied != Some(token.as_str()) {
                    return api::Failure::from(Error::new("UNAUTHORIZED", "引擎连接凭据无效"))
                        .into_response();
                }
                let session = request
                    .headers()
                    .get("x-studio-session")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                if let Some(id) = &session
                    && let Err(error) = studio_domain::validate_id(id)
                {
                    return api::Failure::from(error).into_response();
                }
                request.extensions_mut().insert(api::ClientSession(session));
                let segments = request.uri().path().split('/').collect::<Vec<_>>();
                let _project_lease = if segments.len() >= 4
                    && segments[1..3] == ["v1", "projects"]
                    && studio_domain::validate_id(segments[3]).is_ok()
                    && !matches!(segments.get(4), Some(&"open" | &"close"))
                {
                    match store.request_lease(segments[3]) {
                        Ok(lease) => Some(lease),
                        Err(error) => return api::Failure::from(error).into_response(),
                    }
                } else {
                    None
                };
                let read_id = request
                    .headers()
                    .get("x-studio-read-id")
                    .and_then(|v| v.to_str().ok());
                let read_operation = request.method() == axum::http::Method::GET
                    || (request.method() == axum::http::Method::POST
                        && matches!(
                            segments.as_slice(),
                            ["", "v1", "projects", _, "ranking-browse", "assets"]
                                | [
                                    "",
                                    "v1",
                                    "projects",
                                    _,
                                    "artifacts",
                                    _,
                                    "ranking",
                                    "rows" | "count"
                                ]
                                | ["", "v1", "projects", _, "selection", "members"]
                                | ["", "v1", "projects", _, "assets", "summaries"]
                        ));
                let _read_ticket = if read_operation
                    && segments.len() >= 4
                    && segments[1..3] == ["v1", "projects"]
                    && studio_domain::validate_id(segments[3]).is_ok()
                    && let Some(read_id) = read_id
                {
                    match previews.ticket(segments[3], read_id) {
                        Ok(ticket) => Some(ticket),
                        Err(error) => return api::Failure::from(error).into_response(),
                    }
                } else {
                    None
                };
                let cancelled = _read_ticket
                    .as_ref()
                    .map(|t| t.cancelled.clone())
                    .unwrap_or_else(|| Arc::new(std::sync::atomic::AtomicBool::new(false)));
                let priority = match request
                    .headers()
                    .get("x-studio-read-priority")
                    .and_then(|v| v.to_str().ok())
                {
                    Some("prefetch") => ReadPriority::Prefetch,
                    Some("background") => ReadPriority::Background,
                    _ => ReadPriority::Interactive,
                };
                request.extensions_mut().insert(api::RequestReadContext {
                    cancelled,
                    priority,
                });
                next.run(request).await
            }
        },
    );
    let origins = [
        "http://127.0.0.1:1420",
        "http://localhost:1420",
        "http://tauri.localhost",
        "https://tauri.localhost",
        "tauri://localhost",
    ]
    .into_iter()
    .map(|s| s.parse::<axum::http::HeaderValue>().expect("static origin"))
    .collect::<Vec<_>>();
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PATCH,
            axum::http::Method::PUT,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderName::from_static("x-studio-read-id"),
            axum::http::HeaderName::from_static("x-studio-read-priority"),
            axum::http::HeaderName::from_static("x-studio-session"),
        ])
        .expose_headers(
            [
                "x-studio-cache",
                "x-studio-freshness",
                "x-studio-verified-ms",
            ]
            .map(axum::http::HeaderName::from_static),
        );
    let app = api::routes()
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .layer(auth)
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);
    let query_scheduler = tokio::spawn(query_jobs::scheduler(store.clone(), queries.clone()));
    let cache_maintenance = tokio::spawn(query_jobs::cache_maintenance(
        store.clone(),
        queries.clone(),
        previews.cache.clone(),
    ));
    let recovery_store = store.clone();
    let recovery = tokio::task::spawn_blocking(move || match recovery_store.recover_jobs() {
        Ok(issues) => {
            for (id, issue) in issues {
                tracing::warn!(project_id=%id,%issue,"project recovery deferred");
            }
        }
        Err(error) => tracing::warn!(%error,"background recovery unavailable"),
    });
    let preview_scheduler = tokio::spawn(previews.clone().run());
    let scheduler = tokio::spawn(jobs::scheduler(store.clone(), resources, sources));
    tracing::info!(endpoint=%connection.endpoint,api_version=API_VERSION,"engine ready");
    let abort = scheduler.abort_handle();
    let query_shutdown = queries.clone();
    let preview_shutdown = previews.clone();
    let aesthetic_shutdown = aesthetic.clone();
    let analysis_shutdown = aesthetic_analysis.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select!{_ = tokio::signal::ctrl_c()=>{let _=shutdown_tx.send(true);},_ = shutdown_rx.changed()=>{}}
            abort.abort();
            jobs::shutdown();
            api::lake_inputs::shutdown();
            store.stop_member_writes();
            query_shutdown.shutdown();
            preview_shutdown.shutdown();
            aesthetic_shutdown.shutdown();
            analysis_shutdown.shutdown();
        })
        .await
        .map_err(Error::io);
    scheduler.abort();
    let _ = scheduler.await;
    queries.shutdown();
    let _ = query_scheduler.await;
    let _ = cache_maintenance.await;
    while queries.ranked_indexes.busy() {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let _ = recovery.await;
    previews.shutdown();
    let _ = preview_scheduler.await;
    while aesthetic.busy() || aesthetic_analysis.busy() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let _ = lake_input_supervisor.await;
    drop(lease);
    lake_updates.shutdown();
    let _ = lake_update_supervisor.await;
    result
}
fn remove_abandoned_query_temps(directory: &std::path::Path, prefix: &str) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let root = directory.canonicalize().map_err(Error::io)?;
    for entry in fs::read_dir(&root).map_err(Error::io)? {
        let entry = entry.map_err(Error::io)?;
        if !entry.file_name().to_string_lossy().starts_with(prefix) {
            continue;
        }
        let path = entry.path();
        let target = path.canonicalize().map_err(Error::io)?;
        if !target.starts_with(&root) || entry.file_type().map_err(Error::io)?.is_symlink() {
            return Err(Error::invalid("查询临时文件超出应用目录"));
        }
        if entry.file_type().map_err(Error::io)?.is_dir() {
            fs::remove_dir_all(&target).map_err(Error::io)?;
        } else {
            fs::remove_file(&target).map_err(Error::io)?;
        }
    }
    Ok(())
}
