use crate::actors::message::{BytesMessage, TornadoCommonActorError};
use crate::actors::nats_publisher::NatsClientConfig;
use crate::TornadoError;
use actix::prelude::*;
use futures::StreamExt;
use log::*;
use serde_derive::{Deserialize, Serialize};
use tornado_common_api::Event;

#[derive(Deserialize, Serialize, Clone)]
pub struct NatsSubscriberConfig {
    pub client: NatsClientConfig,
    pub subject: String,
}

pub async fn subscribe_to_nats<
    F: 'static + FnMut(Event) -> Result<(), TornadoCommonActorError> + Sized + Unpin,
>(
    config: NatsSubscriberConfig,
    message_mailbox_capacity: usize,
    callback: F,
) -> Result<(), TornadoError> {
    let subject = config.subject.parse().map_err(|err| TornadoError::ConfigurationError {
        message: format! {"NatsSubscriberActor - Cannot parse subject. Err: {}", err},
    })?;

    let client = config.client.new_client().await?;
    client.connect().await;

    let (_, subscription) = client.subscribe(&subject, message_mailbox_capacity).await.map_err(|err| {
        TornadoError::ConfigurationError { message: format! {"NatsSubscriberActor - Cannot subscribe to subject [{}]. Err: {}", subject, err} }
    })?;

    NatsStreamingSubscriberActor::create(|ctx| {
        ctx.set_mailbox_capacity(message_mailbox_capacity);
        ctx.add_message_stream(
            Box::leak(Box::new(subscription))
                .map(|message| BytesMessage { msg: message.into_payload() }),
        );
        NatsStreamingSubscriberActor { callback }
    });

    Ok(())
}

struct NatsStreamingSubscriberActor<F>
where
    F: 'static + FnMut(Event) -> Result<(), TornadoCommonActorError> + Sized + Unpin,
{
    callback: F,
}

impl<F> Actor for NatsStreamingSubscriberActor<F>
where
    F: 'static + FnMut(Event) -> Result<(), TornadoCommonActorError> + Sized + Unpin,
{
    type Context = Context<Self>;
}

impl<F> Handler<BytesMessage> for NatsStreamingSubscriberActor<F>
where
    F: 'static + FnMut(Event) -> Result<(), TornadoCommonActorError> + Sized + Unpin,
{
    type Result = Result<(), TornadoCommonActorError>;

    fn handle(&mut self, msg: BytesMessage, _: &mut Context<Self>) -> Self::Result {
        trace!("NatsStreamingSubscriberActor - message received");
        let event = serde_json::from_slice(&msg.msg)
            .map_err(|err| TornadoCommonActorError::SerdeError { message: format! {"{}", err} })?;
        trace!("NatsStreamingSubscriberActor - event from message received: {:#?}", event);
        (&mut self.callback)(event)
    }
}