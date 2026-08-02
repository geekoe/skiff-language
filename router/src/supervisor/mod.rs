//! Production Router composition (W-composition; plan §3.2/§5.5/§7).
//!
//! [`RouterSupervisor`] is the only lifecycle owner: config, component
//! construction, listener/task join and shutdown. [`RouterComponents`] is the
//! stable component manifest consumed by the supervisor; each installed
//! session-keyed component appears in the static `SessionLayer` consumer
//! manifest, and the installed lane sinks are injected through the session
//! inbound sink bundle (plan §5.5) before any listener starts.

pub mod actor;
pub mod actor_sink;
pub mod http;
pub mod session_ports;
pub mod sinks;
pub mod ws;

use std::sync::Arc;
use std::time::Duration;

use crate::activation::{
    ActivationCoordinator, ActivationCoordinatorHandle, ActivationCoordinatorOptions,
    ActivationCoordinatorPorts, ActivationHttpHandler, ActivationStateRepository,
    BlockingLoaderCandidatePort, EpochStorePublishPort, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, NoopHealthSink, RoutingCandidateQueryPortAdapter,
    SystemClock,
};
use crate::actor::ActorMethodSpawnExecutionSink;
use crate::bootstrap::{
    ActiveRoutingEpochStore, BootstrapAssemblyError, RouterBootstrapAssembly, RoutingEpoch,
};
use crate::config::RouterConfig;
use crate::dispatch::{ActorMethodSpawnControl, RequestDispatcher, RuntimeDispatcherOptions};
use crate::http::dispatch::HttpDispatchPort;
use crate::http::ingress::{EpochHttpIngressResolver, HttpGatewaySurfaceView};
use crate::http::server::{
    start_http_gateway, GatewayUpgradeHandler, GatewayUpgradeOptions, HttpGatewayServer,
    HttpGatewayServerOptions,
};
use crate::listener::{
    start_runtime_control_listener_with_control, ClientWsContext, ListenerError, ListenerHandle,
    ListenerStartOptions, WsTaskRegistry,
};
use crate::session::consumer::{ConsumerKind, ConsumerManifest};
use crate::session::demux::InboundSinkSet;
use crate::session::health::RuntimeHealthLedger;
use crate::session::layer::{SessionLayer, SessionLayerError, SessionLayerOptions};
use crate::session::SessionConsumer;
use crate::ws::types::SystemClock as WsSystemClock;
use crate::ws::{
    NoopNotificationObserver, WebSocketLane, WebSocketLaneOptions, WebSocketRequestBrokerOptions,
};

use self::actor::{assemble_actor_components, ActorComponents, ActorSessionOwnerConsumer};
use self::actor_sink::{ActorFrameSink, ActorSpawnFrameSink};
use self::http::{DispatcherHttpPort, PendingHttpRouter, RequestFrameSink};
use self::session_ports::{
    ActivationSessionEnqueuePort, DirectoryLeaseRevalidate, DispatcherSessionConsumer,
    LayerSessionAbort, PendingHttpHandle, SessionCandidateViewSource, SessionHandle,
    SessionRuntimePeer, SessionRuntimeViolationSink, StoreRoutingEpochSource,
    WsRuntimeGenerationPeer, WsRuntimeSessionClose,
};
use self::sinks::{ActivationTransactionSink, ConnectionFrameSink};
use self::ws::{
    load_ws_surface_view, LayerWsSessionWriter, ProductionWsConnectSelector, WsConnectSelector,
    WsDispatchStore, WsGatewaySurfaceView, WsInboundDispatch, WsLaneHandle, WsLaneSessionConsumer,
    WsMethodCatalog, WsPendingAdmissionSender, WsSessionWriter,
};

/// Fail-closed supervisor assembly errors; no listener is started and owned
/// bootstrap state is shut down before the error is returned.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("router config environment is required")]
    EnvironmentMissing,
    #[error("activation state repository connect failed: {0}")]
    Repository(String),
    #[error("bootstrap assembly failed: {0}")]
    Bootstrap(#[from] BootstrapAssemblyError),
    #[error("HTTP surface load failed: {0}")]
    Surface(String),
    #[error("actor assembly failed: {0}")]
    Actor(String),
    #[error("dispatcher assembly failed: {0}")]
    Dispatcher(String),
    #[error("session layer assembly failed: {0}")]
    Session(#[from] SessionLayerError),
    #[error("activation recovery start failed: {0}")]
    Recovery(String),
}

/// Stable component manifest (plan §3.2/§5.5). Holds every production owner
/// and the port adapters that wire them together.
#[derive(Debug)]
pub struct RouterComponents {
    pub config: RouterConfig,
    pub assembly: Arc<RouterBootstrapAssembly>,
    pub epoch: Arc<RoutingEpoch>,
    pub epoch_store: Arc<ActiveRoutingEpochStore>,
    pub session: Arc<SessionLayer>,
    pub dispatcher: Arc<RequestDispatcher>,
    pub pending_http: Arc<PendingHttpRouter>,
    pub ws_lane: Arc<WebSocketLane>,
    pub coordinator: ActivationCoordinatorHandle,
    pub actor: Arc<ActorComponents>,
    pub http_dispatcher: Arc<DispatcherHttpPort>,
    pub surface_view: Arc<HttpGatewaySurfaceView>,
    pub request_sink: Arc<RequestFrameSink>,
    pub connection_sink: Arc<ConnectionFrameSink>,
    pub activation_sink: Arc<ActivationTransactionSink>,
    pub ws_surface: Arc<WsGatewaySurfaceView>,
    pub ws_store: Arc<WsDispatchStore>,
    pub ws_selector: Arc<dyn WsConnectSelector>,
    pub client_ws: Arc<ClientWsContext>,
}

impl RouterComponents {
    /// Production entry: connects the Mongo activation repository and runs
    /// the full bootstrap + component assembly.
    pub async fn assemble(config: &RouterConfig) -> Result<Arc<Self>, SupervisorError> {
        let environment = config
            .environment
            .clone()
            .ok_or(SupervisorError::EnvironmentMissing)?;
        let repository = Arc::new(
            MongoActivationStateRepository::connect(
                &config.service_db.mongo_url,
                MongoActivationStateRepositoryOptions::default(),
                Arc::new(SystemClock),
            )
            .await
            .map_err(|error| SupervisorError::Repository(error.to_string()))?,
        ) as Arc<dyn crate::activation::ActivationStateRepository>;
        Self::assemble_with(config, &environment, repository).await
    }

    /// Assembly with an injected repository (tests use the memory fake).
    pub async fn assemble_with(
        config: &RouterConfig,
        environment: &str,
        repository: Arc<dyn ActivationStateRepository>,
    ) -> Result<Arc<Self>, SupervisorError> {
        let assembly = Arc::new(
            RouterBootstrapAssembly::assemble_with(config, environment, repository).await?,
        );
        match Self::assemble_components(config, Arc::clone(&assembly)).await {
            Ok(components) => Ok(components),
            Err(error) => {
                // Fail closed: drain the blocking loader and close the
                // repository before returning; no listener was started.
                assembly.shutdown().await;
                Err(error)
            }
        }
    }

    async fn assemble_components(
        config: &RouterConfig,
        assembly: Arc<RouterBootstrapAssembly>,
    ) -> Result<Arc<Self>, SupervisorError> {
        let epoch = assembly.epoch().clone();
        let epoch_store = assembly.epoch_store();
        let surface_view = Arc::new(
            self::http::load_http_surface_view(&config.artifacts_path, &epoch)
                .map_err(SupervisorError::Surface)?,
        );
        let ws_surface = load_ws_surface_view(&config.artifacts_path, &epoch)
            .map_err(SupervisorError::Surface)?;
        let session_handle = SessionHandle::new();
        let actor = assemble_actor_components(
            Arc::clone(&epoch),
            Arc::clone(&epoch_store),
            session_handle.clone(),
        )
        .map_err(SupervisorError::Actor)?;
        let actor_session_owner = Arc::new(ActorSessionOwnerConsumer::new(Arc::clone(&actor)));

        let dispatcher = Arc::new(
            RequestDispatcher::new(
                RuntimeDispatcherOptions::new(
                    usize::try_from(config.runtime_max_concurrency).unwrap_or(usize::MAX),
                    Arc::new(StoreRoutingEpochSource::new(Arc::clone(&epoch_store))),
                    Arc::new(SessionCandidateViewSource::new(session_handle.clone())),
                    Arc::new(DirectoryLeaseRevalidate::new(session_handle.clone())),
                    Arc::new(
                        SessionRuntimePeer::new(session_handle.clone())
                            .with_spawn_wire_store(Arc::clone(&actor.spawn_wire_store)),
                    ),
                    Arc::new(LayerSessionAbort::new(session_handle.clone())),
                    Arc::clone(&actor.actor_lane_spawn_control) as Arc<dyn ActorMethodSpawnControl>,
                )
                .map_err(SupervisorError::Dispatcher)?,
            )
            .map_err(SupervisorError::Dispatcher)?,
        );
        // The request-parent spawn lookup answers through the dispatcher
        // pending (assembled after the actor lane).
        *actor
            .deferred_dispatcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&dispatcher));

        let session_writer: Arc<dyn WsSessionWriter> =
            Arc::new(LayerWsSessionWriter::new(session_handle.clone()));
        let ws_lane_handle = WsLaneHandle::new();
        let production_selector = Arc::new(ProductionWsConnectSelector::new(
            Arc::clone(&epoch_store),
            Arc::new(SessionCandidateViewSource::new(session_handle.clone())),
            usize::try_from(config.runtime_max_concurrency).unwrap_or(usize::MAX),
        ));
        let ws_pool = production_selector.pool();
        let ws_store = WsDispatchStore::new(
            ws_lane_handle.clone(),
            Arc::clone(&session_writer),
            ws_pool,
            config.request_timeout_ms,
        );
        let ws_selector: Arc<dyn WsConnectSelector> = production_selector;
        let ws_lane = WebSocketLane::new(
            WebSocketLaneOptions {
                broker: WebSocketRequestBrokerOptions {
                    inbound_timeout_ms: config.request_timeout_ms,
                    ..Default::default()
                },
                ..Default::default()
            },
            Arc::new(WsRuntimeGenerationPeer::new(session_handle.clone())),
            Arc::new(WsRuntimeSessionClose::new(session_handle.clone())),
            Arc::new(WsPendingAdmissionSender::new(Arc::clone(&ws_store))),
            Arc::new(WsMethodCatalog::new(Arc::clone(&ws_surface))),
            Arc::new(NoopNotificationObserver),
            Arc::new(SessionRuntimeViolationSink::new(session_handle.clone())),
            Arc::new(WsInboundDispatch::new(Arc::clone(&ws_store))),
        );
        ws_lane_handle.set(Arc::clone(&ws_lane));

        let coordinator_ports = ActivationCoordinatorPorts {
            repository: assembly.repository(),
            loader: Arc::new(BlockingLoaderCandidatePort::new(
                assembly.loader(),
                assembly.strict_loader(),
                assembly.actor_projection(),
            )),
            candidates: Arc::new(RoutingCandidateQueryPortAdapter::new(
                Arc::clone(&epoch_store),
                Arc::new(SessionCandidateViewSource::new(session_handle.clone())),
            )),
            sessions: Arc::new(ActivationSessionEnqueuePort::new(session_handle.clone())),
            publish: Arc::new(EpochStorePublishPort::new(Arc::clone(&epoch_store))),
            health: Arc::new(NoopHealthSink),
        };
        let coordinator = ActivationCoordinator::spawn(
            coordinator_ports,
            ActivationCoordinatorOptions {
                mailbox_capacity: 64,
                ack_deadline: Duration::from_millis(config.activation_prepare_timeout_ms),
                service_db_mongo_url: Some(config.service_db.mongo_url.clone()),
            },
        );

        let pending_http_handle = PendingHttpHandle::new();
        let session = Arc::new(
            SessionLayer::with_options(
                config.clone(),
                SessionLayerOptions {
                    committed_epoch: None,
                    pending_epoch: None,
                    manifest: ConsumerManifest::installed([
                        ConsumerKind::HealthLedger,
                        ConsumerKind::RequestDispatcher,
                        ConsumerKind::RuntimeGenerationPinLedger,
                        ConsumerKind::WebSocketRequestBroker,
                        ConsumerKind::ActorSessionOwner,
                        ConsumerKind::ActivationCoordinator,
                    ]),
                    consumers: {
                        let consumers: Vec<
                            Arc<dyn crate::session::consumer::SessionConsumer>,
                        > = vec![
                            Arc::new(RuntimeHealthLedger::new()),
                            Arc::new(DispatcherSessionConsumer::new(
                                Arc::clone(&dispatcher),
                                pending_http_handle.clone(),
                            )),
                            Arc::clone(&ws_lane.ledger) as Arc<dyn SessionConsumer>,
                            Arc::new(WsLaneSessionConsumer::new(
                                Arc::clone(&ws_lane),
                                Arc::clone(&ws_store),
                                Arc::clone(&ws_lane.broker) as Arc<dyn SessionConsumer>,
                            )) as Arc<dyn SessionConsumer>,
                            Arc::clone(&actor_session_owner) as Arc<dyn SessionConsumer>,
                            Arc::new(coordinator.clone()) as Arc<dyn SessionConsumer>,
                        ];
                        consumers
                    },
                    timing: Default::default(),
                    budgets: Default::default(),
                    writer_delay: None,
                },
            )
            .map_err(SupervisorError::Session)?,
        );
        session.attach_epoch_store(Arc::clone(&epoch_store));
        session_handle.set(Arc::clone(&session));
        session.set_registration_observer(Arc::new(coordinator.clone()));

        // Plan §4.2: a durable pending observed at startup becomes a recovery
        // transaction after the committed epoch is published. The listener
        // starts normally; expected replica registrations rebind through the
        // registration observer above.
        if assembly.pending_recovery().is_some() {
            coordinator
                .start_recovery(assembly.environment().to_string())
                .map_err(|error| SupervisorError::Recovery(error.to_string()))?;
        }

        let pending_http = Arc::new(PendingHttpRouter::new());
        pending_http_handle.set(Arc::clone(&pending_http));
        let request_sink = Arc::new(RequestFrameSink::new_with_ws(
            Arc::clone(&dispatcher),
            Arc::clone(&pending_http),
            Some(Arc::clone(&ws_store)),
        ));
        let connection_sink = Arc::new(ConnectionFrameSink::new(
            Arc::clone(&ws_lane),
            session_handle.clone(),
        ));
        let activation_sink = Arc::new(ActivationTransactionSink::new(coordinator.clone()));
        let actor_sink = Arc::new(ActorFrameSink::new(
            Arc::clone(&actor),
            session_handle.clone(),
            Arc::clone(&epoch_store),
            Arc::clone(&session_writer),
            Arc::new(WsSystemClock),
        ));
        let spawn_sink = Arc::new(ActorSpawnFrameSink::new(
            Arc::clone(&actor),
            Arc::clone(&dispatcher),
            Arc::clone(&epoch_store),
            Arc::clone(&session_writer),
            Arc::new(WsSystemClock),
            config.request_timeout_ms,
        ));
        // E-actor-rust: the real spawn execution owner is the actor frame
        // sink (correlation + owner forward share one surface).
        actor
            .execution_sink
            .set(Arc::clone(&actor_sink) as Arc<dyn ActorMethodSpawnExecutionSink>);
        actor_session_owner.set_sink(Arc::clone(&actor_sink));
        let sinks = InboundSinkSet {
            request: Some(
                Arc::clone(&request_sink) as Arc<dyn crate::session::demux::InboundFrameSink>
            ),
            connection: Some(
                Arc::clone(&connection_sink) as Arc<dyn crate::session::demux::InboundFrameSink>
            ),
            activation_transaction: Some(
                Arc::clone(&activation_sink) as Arc<dyn crate::session::demux::InboundFrameSink>
            ),
            actor: Some(Arc::clone(&actor_sink) as Arc<dyn crate::session::demux::InboundFrameSink>),
            spawn: Some(spawn_sink as Arc<dyn crate::session::demux::InboundFrameSink>),
        };
        session.install_inbound_sinks(Arc::new(sinks));
        crate::supervisor::actor::spawn_actor_lane_timer_pump(
            Arc::clone(&actor),
            Arc::clone(&actor_sink),
            Duration::from_millis(1_000),
            || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            },
        );

        let http_dispatcher = Arc::new(DispatcherHttpPort::new(
            Arc::clone(&dispatcher),
            Arc::clone(&pending_http),
            Duration::from_millis(config.request_timeout_ms),
        ));
        let client_ws = ClientWsContext::new(
            Arc::clone(&ws_surface),
            Arc::clone(&ws_lane),
            Arc::clone(&ws_store),
            Arc::clone(&ws_selector),
            Arc::clone(&epoch_store),
            config.websocket_path.clone(),
            config.request_timeout_ms,
        );

        Ok(Arc::new(Self {
            config: config.clone(),
            assembly: Arc::clone(&assembly),
            epoch,
            epoch_store,
            session,
            dispatcher,
            pending_http,
            ws_lane,
            coordinator,
            actor,
            http_dispatcher,
            surface_view,
            request_sink,
            connection_sink,
            activation_sink,
            ws_surface,
            ws_store,
            ws_selector,
            client_ws,
        }))
    }
}

/// Running supervisor listeners: the public HTTP gateway (production
/// `HttpDispatchPort`) plus the shared runtime/control listener.
pub struct SupervisorListeners {
    pub public_http: HttpGatewayServer,
    pub runtime_control: ListenerHandle,
    pub session: Arc<SessionLayer>,
    pub ws_tasks: Arc<WsTaskRegistry>,
}

impl SupervisorListeners {
    /// C-process-lifecycle order: stop public accept, stop control accept,
    /// drain the session barrier, then join the control listener.
    pub async fn shutdown(self) -> Result<(), ListenerError> {
        let Self {
            public_http,
            runtime_control,
            session,
            ws_tasks,
        } = self;
        let http_result = public_http.shutdown().await;
        runtime_control.begin_shutdown();
        let session_result = session.shutdown().await;
        let control_result = runtime_control.join_shutdown().await;
        ws_tasks.abort_all();
        let mut errors = Vec::new();
        if let Err(error) = http_result {
            errors.push(format!("public http: {error}"));
        }
        if let Err(error) = session_result {
            errors.push(format!("session: {error}"));
        }
        if let Err(error) = control_result {
            errors.push(format!("runtime-control: {error}"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ListenerError::FailStop(errors.join("; ")))
        }
    }
}

/// Unique lifecycle owner (plan §3.2): config, construction, listener/task
/// join and shutdown. Owns no business mutable state.
#[derive(Debug)]
pub struct RouterSupervisor {
    components: Arc<RouterComponents>,
}

impl RouterSupervisor {
    pub async fn assemble(config: &RouterConfig) -> Result<Self, SupervisorError> {
        let components = RouterComponents::assemble(config).await?;
        Ok(Self { components })
    }

    pub async fn assemble_with(
        config: &RouterConfig,
        environment: &str,
        repository: Arc<dyn ActivationStateRepository>,
    ) -> Result<Self, SupervisorError> {
        let components = RouterComponents::assemble_with(config, environment, repository).await?;
        Ok(Self { components })
    }

    pub fn components(&self) -> &Arc<RouterComponents> {
        &self.components
    }

    /// Starts the public HTTP gateway and the runtime/control listener
    /// against the assembled components.
    pub async fn start_listeners(
        &self,
        options: &ListenerStartOptions,
    ) -> Result<SupervisorListeners, ListenerError> {
        let components = Arc::clone(&self.components);
        let public_addr = match options.public_bind {
            Some(addr) => addr,
            None => crate::listener::resolve_listener_addr(
                &components.config.host,
                components.config.http_port,
            )?,
        };
        let http_options = HttpGatewayServerOptions::new(
            public_addr,
            usize::try_from(components.config.http_max_request_bytes).unwrap_or(usize::MAX),
            usize::try_from(components.config.http_max_response_bytes).unwrap_or(usize::MAX),
        );
        let http_options = HttpGatewayServerOptions {
            request_timeout: Duration::from_millis(components.config.request_timeout_ms),
            websocket_upgrade: Some(GatewayUpgradeOptions {
                path: components.config.websocket_path.clone(),
                handler: Arc::clone(&components.client_ws) as Arc<dyn GatewayUpgradeHandler>,
            }),
            ..http_options
        };
        let resolver = Arc::new(EpochHttpIngressResolver::new(Arc::clone(
            &components.surface_view,
        )));
        let public_http = start_http_gateway(
            http_options,
            Arc::clone(&components.epoch),
            resolver,
            Arc::clone(&components.http_dispatcher) as Arc<dyn HttpDispatchPort>,
        )
        .await
        .map_err(|error| ListenerError::Http(error.to_string()))?;
        let activation_deadline =
            Duration::from_millis(components.config.activation_prepare_timeout_ms)
                .saturating_mul(2)
                .max(Duration::from_secs(30));
        let activation_http = Arc::new(ActivationHttpHandler::with_deadline(
            components.coordinator.clone(),
            activation_deadline,
        ));
        let runtime_control = start_runtime_control_listener_with_control(
            &components.config,
            options,
            Arc::clone(&components.session),
            Some(activation_http),
        )
        .await?;
        Ok(SupervisorListeners {
            public_http,
            runtime_control,
            session: Arc::clone(&components.session),
            ws_tasks: Arc::clone(&components.client_ws.tasks),
        })
    }

    /// Shuts down the bootstrap assembly (loader drain + repository close)
    /// after the listeners/sessions have shut down.
    pub async fn shutdown(&self) {
        self.components.assembly.shutdown().await;
    }
}
