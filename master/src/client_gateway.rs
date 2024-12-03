use std::sync::Arc;
use axum::{Json, Router};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use proto::master::ModelType;
use crate::worker_gateway::{WorkerGateway, WorkerGatewayCommandError};

pub struct ClientGateway {
    cancellation_token: CancellationToken,
    port: usize,
    client_state: Arc<ClientState>,
}

struct ClientState {
    worker_gateway: Arc<WorkerGateway>,
}

#[derive(Error, Debug, Clone)]
pub enum ClientGatewayError {
    #[error("failed to bind port: {0}")]
    BindError(usize),
    
    #[error("failed to start http server")]
    HttpServerStartError,
}

#[derive(Deserialize)]
struct GenerateRequest {
    prompt: String,
    
    #[serde(deserialize_with = "deserialize_model_type")]
    model_type: ModelType
}

#[derive(Serialize)]
struct GenerateResponse {
    answer: String,
}

impl ClientGateway {
    pub fn new(port: usize, worker_gateway: Arc<WorkerGateway>, cancellation_token: CancellationToken) -> Self {
        Self {
            cancellation_token,
            port,
            client_state: Arc::new(ClientState {
                worker_gateway,
            }),
        }
    }
    
    #[tracing::instrument(skip_all)]
    pub async fn start(&self) -> Result<(), ClientGatewayError> {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .await
            .map_err(|_| ClientGatewayError::BindError(self.port))?;

        let cancellation_token = self.cancellation_token.to_owned();
        let app = Router::new()
            .route("/generate", post(Self::generate))
            .with_state(Arc::clone(&self.client_state));
        
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                cancellation_token.cancelled().await;
                tracing::info!("Stopping client gateway");
            })
            .await
            .map_err(|_| ClientGatewayError::HttpServerStartError)?;
        
        Ok(())
    }
    
    #[tracing::instrument(skip_all)]
    async fn generate(
        State(state): State<Arc<ClientState>>,
        Json(payload): Json<GenerateRequest>,
    ) -> Result<(StatusCode, Json<GenerateResponse>), WorkerGatewayCommandError> {
        tracing::debug!("Received generate request");
        match state.worker_gateway.generate_response(payload.model_type, payload.prompt).await {
            Ok(response) => {
                tracing::debug!("Generated response successfully");
                Ok((StatusCode::OK, Json(GenerateResponse { answer: response })))
            },
            Err(e) => {
                tracing::error!("Failed to generate response: {}", e);
                Err(e)
            },
        }
    }
}

fn deserialize_model_type<'de, D>(deserializer: D) -> Result<ModelType, D::Error>
where
    D: Deserializer<'de>
{
    match Deserialize::deserialize(deserializer)? {
        "llama3" => Ok(ModelType::Llama3v2_1B),
        _ => Err(serde::de::Error::custom("Invalid model type")),
    }
}

impl IntoResponse for WorkerGatewayCommandError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self),
        ).into_response()
    }
}