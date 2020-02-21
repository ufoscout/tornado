use actix::prelude::*;
use log::*;
use std::sync::Arc;
use tokio::prelude::*;
use tornado_collector_common::Collector;
use tornado_collector_email::EmailEventCollector;
use tornado_common::actors::message::AsyncReadMessage;
use tornado_common::actors::tcp_client::{EventMessage, TcpClientActor};

pub struct EmailReaderActor {
    pub tpc_client_addr: Addr<TcpClientActor>,
    pub email_collector: Arc<EmailEventCollector>,
}

impl EmailReaderActor {
    pub fn start_new(tpc_client_addr: Addr<TcpClientActor>) -> Addr<EmailReaderActor> {
        EmailReaderActor::create(move |_ctx| EmailReaderActor {
            email_collector: Arc::new(EmailEventCollector::new()),
            tpc_client_addr,
        })
    }
}

impl Actor for EmailReaderActor {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        info!("EmailReaderActor started.");
    }
}

impl<R: AsyncRead + 'static + Unpin> Handler<AsyncReadMessage<R>> for EmailReaderActor {
    type Result = ();

    fn handle(&mut self, mut msg: AsyncReadMessage<R>, _ctx: &mut Context<Self>) -> Self::Result {
        let tcp = self.tpc_client_addr.clone();
        let collector = self.email_collector.clone();
        let fut = async move {
            let mut buf = Vec::new();
            msg.stream.read_to_end(&mut buf).await.unwrap();

            if log_enabled!(Level::Debug) {
                let buf_to_string = String::from_utf8_lossy(&buf);
                debug!("EmailReaderActor - received email:\n{}", buf_to_string);
            }
            match collector.to_event(&buf) {
                Ok(event) => {
                    tcp.do_send(EventMessage { event });
                }
                Err(e) => error!("Error processing incoming email. Err: {}", e),
            };
        };

        actix::spawn(fut);
    }
}
