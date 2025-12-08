use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Extension, Router};

mod cli;
mod clients;
mod config;
mod download;
mod handlers;
mod model;
#[cfg(test)]
mod test_utils;

use cli::{Cli, Commands};
use tokio::select;
use tokio::signal::unix::{SignalKind, signal};
use tracing::info;
use url::Url;

use crate::clients::TiledClient;
use crate::config::GlazedConfig;
use crate::handlers::{download_handler, graphiql_handler, graphql_handler};
use crate::model::TiledQuery;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::init();
    let config;

    if let Some(config_filepath) = cli.config_filepath {
        info!("Loading config from {config_filepath:?}");
        config = GlazedConfig::from_file(&config_filepath)?;
        info!("Config loaded");
    } else {
        info!("Using default config");
        config = GlazedConfig::default();
    }
    match cli.command {
        Commands::Serve => serve(config).await,
    }
}

#[derive(Clone)]
pub struct RootAddress(Url);

async fn serve(config: GlazedConfig) -> Result<(), Box<dyn std::error::Error>> {
    let client = TiledClient::new(config.tiled_client.address);
    let public_address = config
        .public_address
        .clone()
        .unwrap_or_else(|| Url::parse(&format!("http://{}", config.bind_address)).unwrap());
    let schema = Schema::build(TiledQuery, EmptyMutation, EmptySubscription)
        .data(RootAddress(public_address))
        .data(client.clone())
        .finish();

    let graphql_endpoint = config
        .public_address
        .map(|u| u.join("graphql").unwrap().to_string());

    let app = Router::new()
        .route("/graphql", post(graphql_handler).get(graphql_get_warning))
        .route("/graphiql", get(|| graphiql_handler(graphql_endpoint)))
        .route("/asset/{run}/{stream}/{det}/{id}", get(download_handler))
        .with_state(client)
        .fallback((
            StatusCode::NOT_FOUND,
            Html(include_str!("../static/404.html")),
        ))
        .layer(Extension(schema));

    let listener = tokio::net::TcpListener::bind(config.bind_address).await?;
    info!("Serving glazed at {:?}", config.bind_address);

    Ok(axum::serve(listener, app)
        .with_graceful_shutdown(signal_handler())
        .await?)
}

async fn graphql_get_warning() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [("Allow", "POST")],
        Html(include_str!("../static/get_graphql_warning.html")),
    )
}

async fn signal_handler() {
    let mut term = signal(SignalKind::terminate()).expect("Failed to create SIGTERM listener");
    let mut int = signal(SignalKind::interrupt()).expect("Failed to create SIGINT listener");
    let mut quit = signal(SignalKind::quit()).expect("Failed to create SIGQUIT listener");
    let sig = select! {
         _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
        _ = quit.recv() => "SIGQUIT",
    };
    info!("Server interrupted by {sig}");
}
