use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, routing::get};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, session::local::LocalSessionManager,
    tower::StreamableHttpService,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;

use crate::messages::{SessionInfo, WorkspaceInfo};
use crate::orchestrator::Orchestrator;

/// Default port for the MCP server (spells "mcp" on a phone keypad).
pub const DEFAULT_MCP_PORT: u16 = 3847;

/// Shared state accessible from MCP tool handlers.
#[derive(Clone)]
pub struct McpContext {
    orchestrator: Arc<std::sync::Mutex<Orchestrator>>,
}

impl McpContext {
    pub fn new(orchestrator: Arc<std::sync::Mutex<Orchestrator>>) -> Self {
        Self { orchestrator }
    }
}

// ── MCP Service ─────────────────────────────────────────────────────────

/// The UBIQ MCP service, implementing the ServerHandler trait.
#[derive(Clone)]
pub struct UbiqMcpService {
    ctx: McpContext,
    tool_router: ToolRouter<UbiqMcpService>,
}

#[tool_router]
impl UbiqMcpService {
    pub fn new(ctx: McpContext) -> Self {
        Self {
            ctx,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "sessionMetadata",
        description = "Get metadata for a Ubiq session: session info, running workspaces, and available agent types."
    )]
    async fn session_metadata(
        &self,
        Parameters(params): Parameters<SessionMetadataParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let sid = params
            .session_id
            .parse::<uuid::Uuid>()
            .map_err(|e| rmcp::ErrorData {
                code: rmcp::model::ErrorCode(-32602),
                message: format!("Invalid session_id: {e}").into(),
                data: None,
            })?;

        let orch = self.ctx.orchestrator.lock().map_err(|e| rmcp::ErrorData {
            code: rmcp::model::ErrorCode(-32603),
            message: format!("Lock error: {e}").into(),
            data: None,
        })?;

        let session_info: SessionInfo =
            orch.get_session(sid).ok_or_else(|| rmcp::ErrorData {
                code: rmcp::model::ErrorCode(-32602),
                message: format!("Session {} not found", params.session_id).into(),
                data: None,
            })?;

        let workspaces: Vec<WorkspaceInfo> = orch.get_session_workspaces(sid);

        let agent_types_dedup: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            workspaces
                .iter()
                .map(|w| w.agent_type.clone())
                .filter(|a| seen.insert(a.clone()))
                .collect()
        };

        let result = serde_json::json!({
            "session": {
                "id": session_info.id.to_string(),
                "name": session_info.name,
                "home_folder": session_info.home_folder,
                "created_at": session_info.created_at,
            },
            "workspaces": workspaces.iter().map(|w| {
                serde_json::json!({
                    "id": w.id.to_string(),
                    "agent_type": w.agent_type,
                    "folder": w.folder,
                    "cols": w.cols,
                    "rows": w.rows,
                    "running": w.running,
                })
            }).collect::<Vec<_>>(),
            "agent_types": agent_types_dedup,
        });

        let text = serde_json::to_string_pretty(&result).map_err(|e| rmcp::ErrorData {
            code: rmcp::model::ErrorCode(-32603),
            message: format!("Serialization error: {e}").into(),
            data: None,
        })?;

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }
}

/// Parameters for the `sessionMetadata` tool.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct SessionMetadataParams {
    /// The session ID to query metadata for.
    pub session_id: String,
}

#[tool_handler]
impl ServerHandler for UbiqMcpService {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Ubiq MCP Server — Provides session and workspace metadata for agent orchestration. \
                 Use the sessionMetadata tool to query session state.",
            )
    }
}

// ── HTTP Server ─────────────────────────────────────────────────────────

/// Start the MCP HTTP server on the given address.
///
/// This spawns a new thread with its own tokio runtime so it doesn't
/// interfere with Tauri's runtime.
pub fn start_mcp_server(
    addr: SocketAddr,
    orchestrator: Arc<std::sync::Mutex<Orchestrator>>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("mcp-server")
            .build()
            .expect("Failed to create MCP server tokio runtime");

        rt.block_on(async move {
            if let Err(e) = run_server(addr, orchestrator).await {
                eprintln!("MCP server error: {e}");
            }
        });
    });
}

async fn run_server(
    addr: SocketAddr,
    orchestrator: Arc<std::sync::Mutex<Orchestrator>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ctx = McpContext::new(orchestrator);

    let mcp_service: StreamableHttpService<UbiqMcpService, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(UbiqMcpService::new(ctx.clone())),
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default(),
        );

    let app = Router::new()
        .route("/health", get(health_check))
        .nest_service("/mcp", mcp_service);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("MCP server listening on http://{addr}/mcp");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("MCP server shutting down");
        })
        .await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "ok"
}
