use crate::actor::dispatcher::ProcessedEventMessage;
use crate::monitoring::metrics::{TornadoMeter, EVENT_TYPE_LABEL_KEY};
use actix::prelude::*;
use log::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tornado_engine_api::event::api::ProcessType;
use tornado_engine_matcher::config::operation::{matcher_config_filter, NodeFilter};
use tornado_engine_matcher::config::{MatcherConfig, MatcherConfigReader};
use tornado_engine_matcher::error::MatcherError;
use tornado_engine_matcher::matcher::Matcher;
use tornado_engine_matcher::model::{InternalEvent, ProcessedEvent};
use tornado_engine_matcher::{error, matcher};

#[derive(Message)]
#[rtype(result = "Result<ProcessedEvent, error::MatcherError>")]
pub struct EventMessageWithReply {
    pub event: InternalEvent,
    pub config_filter: HashMap<String, NodeFilter>,
    pub process_type: ProcessType,
    pub include_metadata: bool,
}

#[derive(Debug, Message)]
#[rtype(result = "Result<ProcessedEvent, error::MatcherError>")]
pub struct EventMessageAndConfigWithReply {
    pub event: InternalEvent,
    pub matcher_config: MatcherConfig,
    pub process_type: ProcessType,
    pub include_metadata: bool,
}

#[derive(Message)]
#[rtype(result = "Result<(), error::MatcherError>")]
pub struct EventMessage {
    pub event: InternalEvent,
}

#[derive(Message)]
#[rtype(result = "Result<Arc<MatcherConfig>, error::MatcherError>")]
pub struct ReconfigureMessage {}

#[derive(Message)]
#[rtype(result = "Arc<MatcherConfig>")]
pub struct GetCurrentConfigMessage {}

pub struct MatcherActor {
    dispatcher_addr: Recipient<ProcessedEventMessage>,
    matcher_config_manager: Arc<dyn MatcherConfigReader>,
    matcher_config: Arc<MatcherConfig>,
    matcher: Arc<matcher::Matcher>,
    meter: Arc<TornadoMeter>,
}

impl MatcherActor {
    pub async fn start(
        dispatcher_addr: Recipient<ProcessedEventMessage>,
        matcher_config_manager: Arc<dyn MatcherConfigReader>,
        message_mailbox_capacity: usize,
        meter: Arc<TornadoMeter>,
    ) -> Result<Addr<MatcherActor>, MatcherError> {
        let matcher_config = Arc::new(matcher_config_manager.get_config().await?);
        let matcher = Arc::new(Matcher::build(&matcher_config)?);

        Ok(actix::Supervisor::start(move |ctx: &mut Context<MatcherActor>| {
            ctx.set_mailbox_capacity(message_mailbox_capacity);
            MatcherActor { dispatcher_addr, matcher_config_manager, matcher_config, matcher, meter }
        }))
    }

    fn process_event_with_reply(
        &self,
        matcher: &Matcher,
        event: InternalEvent,
        process_type: ProcessType,
        include_metadata: bool,
    ) -> ProcessedEvent {
        let processed_event = self.process(matcher, event, include_metadata);

        match process_type {
            ProcessType::Full => self
                .dispatcher_addr
                .try_send(ProcessedEventMessage { span:  tracing::Span::current(), event: processed_event.clone() }).unwrap_or_else(|err| error!("MatcherActor -  Error while sending ProcessedEventMessage to DispatcherActor. Error: {}", err)),
            ProcessType::SkipActions => {}
        }

        processed_event
    }

    #[inline]
    fn process(
        &self,
        matcher: &Matcher,
        event: InternalEvent,
        include_metadata: bool,
    ) -> ProcessedEvent {
        let timer = SystemTime::now();
        let labels = [EVENT_TYPE_LABEL_KEY.string(
            event
                .event_type
                .get_text()
                .map(|event_type| event_type.to_owned())
                .unwrap_or_else(|| "".to_owned()),
        )];

        let process = matcher.process(event, include_metadata);

        self.meter.events_processed_counter.add(1, &labels);
        self.meter
            .events_processed_duration_seconds
            .record(timer.elapsed().map(|t| t.as_secs_f64()).unwrap_or_default(), &labels);

        process
    }
}

impl Actor for MatcherActor {
    type Context = Context<Self>;
    fn started(&mut self, _ctx: &mut Self::Context) {
        debug!("MatcherActor started.");
    }
}

impl actix::Supervised for MatcherActor {
    fn restarting(&mut self, _ctx: &mut Context<MatcherActor>) {
        debug!("MatcherActor restarted.");
    }
}

impl Handler<EventMessage> for MatcherActor {
    type Result = Result<(), error::MatcherError>;

    fn handle(&mut self, msg: EventMessage, _: &mut Context<Self>) -> Self::Result {
        let trace_id = msg.event.trace_id.as_str();
        let span = tracing::error_span!("MatcherActor", trace_id).entered();
        trace!("MatcherActor - received new EventMessage [{:?}]", &msg.event);

        let processed_event = self.process(&self.matcher, msg.event, false);
        self.dispatcher_addr.try_send(ProcessedEventMessage { span: span.exit(), event: processed_event }).unwrap_or_else(|err| error!("MatcherActor -  Error while sending ProcessedEventMessage to DispatcherActor. Error: {}", err));
        Ok(())
    }
}

impl Handler<EventMessageWithReply> for MatcherActor {
    type Result = Result<ProcessedEvent, error::MatcherError>;

    fn handle(&mut self, msg: EventMessageWithReply, _: &mut Context<Self>) -> Self::Result {
        let trace_id = msg.event.trace_id.as_str();
        let _span = tracing::error_span!("MatcherActor", trace_id).entered();
        trace!("MatcherActor - received new EventMessageWithReply [{:?}]", &msg.event);

        let filtered_config = matcher_config_filter(&self.matcher_config, &msg.config_filter)
            .ok_or_else(|| MatcherError::ConfigurationError {
                message: "The config filter does not match any existing node".to_owned(),
            })?;
        let matcher = Matcher::build(&filtered_config)?;

        Ok(self.process_event_with_reply(
            &matcher,
            msg.event,
            msg.process_type,
            msg.include_metadata,
        ))
    }
}

impl Handler<EventMessageAndConfigWithReply> for MatcherActor {
    type Result = Result<ProcessedEvent, error::MatcherError>;

    fn handle(
        &mut self,
        msg: EventMessageAndConfigWithReply,
        _: &mut Context<Self>,
    ) -> Self::Result {
        let trace_id = msg.event.trace_id.as_str();
        let _span = tracing::error_span!("MatcherActor", trace_id).entered();
        trace!("MatcherActor - received new EventMessageAndConfigWithReply [{:?}]", msg);
        let matcher = Matcher::build(&msg.matcher_config)?;
        Ok(self.process_event_with_reply(
            &matcher,
            msg.event,
            msg.process_type,
            msg.include_metadata,
        ))
    }
}

impl Handler<GetCurrentConfigMessage> for MatcherActor {
    type Result = Arc<MatcherConfig>;

    fn handle(&mut self, _msg: GetCurrentConfigMessage, _: &mut Context<Self>) -> Self::Result {
        trace!("MatcherActor - received new GetCurrentConfigMessage");
        self.matcher_config.clone()
    }
}

impl Handler<ReconfigureMessage> for MatcherActor {
    type Result = ResponseActFuture<Self, Result<Arc<MatcherConfig>, error::MatcherError>>;

    fn handle(&mut self, _msg: ReconfigureMessage, _ctx: &mut Context<Self>) -> Self::Result {
        let matcher_config_manager = self.matcher_config_manager.clone();
        info!("MatcherActor - received ReconfigureMessage.");

        Box::pin(
            async move {
                let matcher_config = Arc::new(matcher_config_manager.get_config().await?);
                let matcher = Arc::new(Matcher::build(&matcher_config)?);
                Ok((matcher, matcher_config))
            }
            .into_actor(self) // converts future to ActorFuture
            .map(|result, this, _ctx| match result {
                Ok((matcher, matcher_config)) => {
                    this.matcher_config = matcher_config.clone();
                    this.matcher = matcher;
                    info!("MatcherActor - Tornado configuration updated successfully.");
                    Ok(matcher_config)
                }
                Err(err) => {
                    error!("MatcherActor - Cannot reconfigure the matcher: {:?}", err);
                    Err(err)
                }
            }),
        )
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::actor::dispatcher::ProcessedEventMessage;
    use crate::command::upgrade_rules::test::prepare_temp_dirs;
    use crate::config::parse_config_files;
    use tornado_common_api::{Event, Value};
    use tornado_engine_matcher::model::ProcessedNode;
    use tornado_engine_matcher::{config::MatcherConfigEditor, model::ProcessedFilterStatus};

    #[actix::test]
    async fn should_reconfigure_the_matcher_and_return_the_new_config() {
        // Arrange
        let tempdir = tempfile::tempdir().unwrap();
        let (config_dir, rules_dir, drafts_dir) = prepare_temp_dirs(&tempdir);

        let configs = parse_config_files(&config_dir, &rules_dir, &drafts_dir).unwrap();

        let config_manager = configs.matcher_config.clone();
        let dispatcher_addr = FakeDispatcher {}.start().recipient();

        let matcher_actor =
            MatcherActor::start(dispatcher_addr, config_manager.clone(), 10, Default::default())
                .await
                .unwrap();

        let draft_id = config_manager.create_draft("user_1".to_owned()).await.unwrap();
        let draft = config_manager.get_draft(&draft_id).await.unwrap();

        // Act
        let request = matcher_actor.send(ReconfigureMessage {}).await.unwrap().unwrap();
        let config_from_response = request.as_ref().clone();

        // Assert
        let matcher_config_after = config_manager.get_config().await.unwrap();
        assert_eq!(config_from_response, matcher_config_after);
        assert_eq!(config_from_response, draft.config);
    }

    #[actix::test]
    async fn should_return_the_current_config() {
        // Arrange
        let tempdir = tempfile::tempdir().unwrap();
        let (config_dir, rules_dir, drafts_dir) = prepare_temp_dirs(&tempdir);

        let configs = parse_config_files(&config_dir, &rules_dir, &drafts_dir).unwrap();

        let config_manager = configs.matcher_config.clone();
        let dispatcher_addr = FakeDispatcher {}.start().recipient();
        let matcher_actor =
            MatcherActor::start(dispatcher_addr, config_manager.clone(), 10, Default::default())
                .await
                .unwrap();

        // Act
        let returned_config = matcher_actor.send(GetCurrentConfigMessage {}).await.unwrap();

        // Assert
        let matcher_config = config_manager.get_config().await.unwrap();
        assert_eq!(&matcher_config, returned_config.as_ref());
    }

    #[actix::test]
    async fn should_execute_event_to_filtered_config_from_root() {
        // Arrange
        let tempdir = tempfile::tempdir().unwrap();
        let (config_dir, rules_dir, drafts_dir) = prepare_temp_dirs(&tempdir);

        let configs = parse_config_files(&config_dir, &rules_dir, &drafts_dir).unwrap();

        let config_manager = configs.matcher_config.clone();
        let dispatcher_addr = FakeDispatcher {}.start().recipient();
        let matcher_actor =
            MatcherActor::start(dispatcher_addr, config_manager.clone(), 10, Default::default())
                .await
                .unwrap();

        let mut event: InternalEvent = Event::new("test").into();
        event.add_to_metadata("tenant_id".to_owned(), Value::Text("alpha".to_owned())).unwrap();

        // Act
        let processed_event: ProcessedEvent = matcher_actor
            .send(EventMessageWithReply {
                event,
                config_filter: HashMap::from([(
                    ROOT_NODE_NAME.to_owned(),
                    NodeFilter::AllChildren,
                )]),
                include_metadata: false,
                process_type: ProcessType::Full,
            })
            .await
            .unwrap()
            .unwrap();

        // Assert
        match processed_event.result {
            ProcessedNode::Filter { nodes, .. } => {
                let tenant_node_matched = nodes.iter().any(|n| match n {
                    ProcessedNode::Filter { name, filter, .. } => {
                        name.eq("tenant_id_alpha")
                            && filter.status == ProcessedFilterStatus::Matched
                    }
                    _ => false,
                });

                assert!(tenant_node_matched);
            }
            _ => assert!(false),
        };
    }

    #[actix::test]
    async fn should_execute_event_to_filtered_config_for_tenant() {
        // Arrange
        let tempdir = tempfile::tempdir().unwrap();
        let (config_dir, rules_dir, drafts_dir) = prepare_temp_dirs(&tempdir);

        let configs = parse_config_files(&config_dir, &rules_dir, &drafts_dir).unwrap();

        let config_manager = configs.matcher_config.clone();
        let dispatcher_addr = FakeDispatcher {}.start().recipient();
        let matcher_actor =
            MatcherActor::start(dispatcher_addr, config_manager.clone(), 10, Default::default())
                .await
                .unwrap();

        let mut event_tenant_alpha: InternalEvent = Event::new("test").into();
        event_tenant_alpha
            .add_to_metadata("tenant_id".to_owned(), Value::Text("alpha".to_owned()))
            .unwrap();

        let mut event_tenant_beta: InternalEvent = Event::new("test").into();
        event_tenant_beta
            .add_to_metadata("tenant_id".to_owned(), Value::Text("beta".to_owned()))
            .unwrap();

        let config_filter = HashMap::from([(
            ROOT_NODE_NAME.to_owned(),
            NodeFilter::SelectedChildren(HashMap::from([(
                "tenant_id_beta".to_owned(),
                NodeFilter::AllChildren,
            )])),
        )]);

        // Act
        let processed_event_alpha: ProcessedEvent = matcher_actor
            .send(EventMessageWithReply {
                event: event_tenant_alpha,
                config_filter: config_filter.clone(),
                include_metadata: false,
                process_type: ProcessType::Full,
            })
            .await
            .unwrap()
            .unwrap();

        let processed_event_beta: ProcessedEvent = matcher_actor
            .send(EventMessageWithReply {
                event: event_tenant_beta,
                config_filter: config_filter.clone(),
                include_metadata: false,
                process_type: ProcessType::Full,
            })
            .await
            .unwrap()
            .unwrap();

        // Assert
        match processed_event_alpha.result {
            ProcessedNode::Filter { nodes, .. } => {
                let tenant_node_matched = nodes.iter().any(|n| match n {
                    ProcessedNode::Filter { name, filter, .. } => {
                        name.eq("tenant_id_alpha")
                            && filter.status == ProcessedFilterStatus::Matched
                    }
                    _ => false,
                });

                assert!(!tenant_node_matched);
            }
            _ => assert!(false),
        };

        match processed_event_beta.result {
            ProcessedNode::Filter { nodes, .. } => {
                let tenant_node_matched = nodes.iter().any(|n| match n {
                    ProcessedNode::Filter { name, filter, .. } => {
                        name.eq("tenant_id_beta") && filter.status == ProcessedFilterStatus::Matched
                    }
                    _ => false,
                });

                assert!(tenant_node_matched);
            }
            _ => assert!(false),
        };
    }

    #[actix::test]
    async fn should_return_error_if_the_filter_matches_no_nodes() {
        // Arrange
        let tempdir = tempfile::tempdir().unwrap();
        let (config_dir, rules_dir, drafts_dir) = prepare_temp_dirs(&tempdir);

        let configs = parse_config_files(&config_dir, &rules_dir, &drafts_dir).unwrap();

        let config_manager = configs.matcher_config.clone();
        let dispatcher_addr = FakeDispatcher {}.start().recipient();
        let matcher_actor =
            MatcherActor::start(dispatcher_addr, config_manager.clone(), 10, Default::default())
                .await
                .unwrap();

        let mut event: InternalEvent = Event::new("test").into();
        event.add_to_metadata("tenant_id".to_owned(), Value::Text("alpha".to_owned())).unwrap();

        // Act
        let processed_event = matcher_actor
            .send(EventMessageWithReply {
                event,
                config_filter: HashMap::from([(
                    ROOT_NODE_NAME.to_owned(),
                    NodeFilter::SelectedChildren(HashMap::from([(
                        "NOT_EXISTING_NODE_NAME".to_owned(),
                        NodeFilter::AllChildren,
                    )])),
                )]),
                include_metadata: false,
                process_type: ProcessType::Full,
            })
            .await
            .unwrap();

        // Assert
        assert!(processed_event.is_err());
    }

    struct FakeDispatcher {}

    impl Actor for FakeDispatcher {
        type Context = Context<Self>;
    }

    impl Handler<ProcessedEventMessage> for FakeDispatcher {
        type Result = Result<(), MatcherError>;
        fn handle(&mut self, _msg: ProcessedEventMessage, _: &mut Context<Self>) -> Self::Result {
            Ok(())
        }
    }
}
