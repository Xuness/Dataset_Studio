mod api;
mod jobs;
mod query_jobs;
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
        Command::Serve { data_dir, port } => serve(data_dir, port).await,
        Command::Worker { plan } => worker::run(&plan),
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
    let plan = studio_sources::QueryReader::default().explain(&source, spec)?;
    atomic_json(&output, &plan)
}
async fn serve(root: PathBuf, port: u16) -> Result<()> {
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
    let queries = Arc::new(query_jobs::QueryRunner::default());
    let state = api::AppState {
        store: store.clone(),
        connection: connection.clone(),
        io: Arc::new(tokio::sync::Semaphore::new(2)),
        metadata_io: Arc::new(tokio::sync::Semaphore::new(1)),
        metadata: Arc::new(studio_sources::MetadataReader::default()),
        queries: queries.clone(),
        shutdown: shutdown_tx.clone(),
    };
    let token = connection.token.clone();
    let request_store = store.clone();
    let auth = axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let token = token.clone();
            let store = request_store.clone();
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
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);
    let app = api::routes()
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .layer(auth)
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);
    let query_scheduler = tokio::spawn(query_jobs::scheduler(store.clone(), queries.clone()));
    let recovery_store = store.clone();
    let recovery = tokio::task::spawn_blocking(move || match recovery_store.recover_jobs() {
        Ok(issues) => {
            for (id, issue) in issues {
                tracing::warn!(project_id=%id,%issue,"project recovery deferred");
            }
        }
        Err(error) => tracing::warn!(%error,"background recovery unavailable"),
    });
    let scheduler = tokio::spawn(jobs::scheduler(store));
    tracing::info!(endpoint=%connection.endpoint,api_version=API_VERSION,"engine ready");
    let abort = scheduler.abort_handle();
    let query_shutdown = queries.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select!{_ = tokio::signal::ctrl_c()=>{let _=shutdown_tx.send(true);},_ = shutdown_rx.changed()=>{}}
            abort.abort();
            query_shutdown.shutdown();
        })
        .await
        .map_err(Error::io);
    scheduler.abort();
    let _ = scheduler.await;
    queries.shutdown();
    let _ = query_scheduler.await;
    let _ = recovery.await;
    drop(lease);
    result
}
