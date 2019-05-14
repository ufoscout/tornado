use crate::engine::{EventMessageWithReply, MatcherActor, ProcessType};
use actix::Addr;
use backend::api::handler::ApiHandler;
use backend::error::ApiError;
use futures::future::{Future, FutureResult};
use tornado_common_api::Event;
use tornado_engine_matcher::config::{MatcherConfig, MatcherConfigManager};
use tornado_engine_matcher::model::ProcessedEvent;

pub struct MatcherApiHandler {
    matcher: Addr<MatcherActor>,
    config_manager: Box<MatcherConfigManager>,
}

impl ApiHandler for MatcherApiHandler {

    fn get_config(&self) -> Box<Future<Item = MatcherConfig, Error = ApiError>> {
        Box::new(FutureResult::from(self.config_manager.read().map_err(ApiError::from)))
    }

    fn send_event(&self, event: Event) -> Box<Future<Item = ProcessedEvent, Error = ApiError>> {
        let request = self
            .matcher
            .send(EventMessageWithReply { event, process_type: ProcessType::SkipActions });

        let response = request.map_err(ApiError::from).and_then(|res| Ok(res?));

        Box::new(response)
    }
}

impl MatcherApiHandler {
    pub fn new(
        config_manager: Box<MatcherConfigManager>,
        matcher: Addr<MatcherActor>,
    ) -> MatcherApiHandler {
        MatcherApiHandler { config_manager, matcher }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::dispatcher::{ActixEventBus, DispatcherActor};
    use actix::{Arbiter, SyncArbiter, System};
    use std::sync::Arc;
    use tornado_engine_matcher::config::fs::FsMatcherConfigManager;
    use tornado_engine_matcher::dispatcher::Dispatcher;
    use tornado_engine_matcher::matcher::Matcher;

    #[test]
    fn should_send_an_event_to_the_matcher_and_return_the_processed_event() {
        // Arrange
        let path = "./config/rules.d";
        let config = FsMatcherConfigManager::new(path);
        let matcher = Arc::new(config.read().and_then(|config| Matcher::build(&config)).unwrap());

        System::run(move || {
            let event_bus = Arc::new(ActixEventBus { callback: |_| {} });

            let dispatcher_addr = SyncArbiter::start(1, move || {
                let dispatcher = Dispatcher::build(event_bus.clone()).unwrap();
                DispatcherActor { dispatcher }
            });

            let matcher_addr = SyncArbiter::start(1, move || MatcherActor {
                matcher: matcher.clone(),
                dispatcher_addr: dispatcher_addr.clone(),
            });

            let api = MatcherApiHandler { matcher: matcher_addr, config_manager: Box::new(config) };

            // Act
            Arbiter::spawn({
                api.send_event(Event::new("test-type")).then(|res| {
                    // Verify
                    assert!(res.is_ok());
                    assert_eq!(Some("test-type"), res.unwrap().event.event_type.get_text());
                    System::current().stop();
                    Ok(())
                })
            });
        })
        .unwrap();
    }

}
