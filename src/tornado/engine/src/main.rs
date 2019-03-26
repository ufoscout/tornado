pub mod collector;
pub mod config;
pub mod dispatcher;
pub mod engine;
pub mod executor;

use crate::dispatcher::{ActixEventBus, DispatcherActor};
use crate::engine::{EventMessage, MatcherActor};
use crate::executor::icinga2::Icinga2ApiClientMessage;
use crate::executor::ActionMessage;
use crate::executor::ExecutorActor;
use actix::prelude::*;
use failure::Fail;
use log::*;
use std::sync::Arc;
use tornado_common::actors::json_event_reader::JsonEventReaderActor;
use tornado_common::actors::tcp_server::listen_to_tcp;
use tornado_common_logger::setup_logger;
use tornado_engine_matcher::dispatcher::Dispatcher;
use tornado_engine_matcher::matcher::Matcher;

fn main() -> Result<(), Box<std::error::Error>> {
    let conf = config::Conf::build();

    if let Some(command) = &conf.command {
        return command.execute(&conf);
    }

    start_tornado(conf)
}

fn start_tornado(conf: config::Conf) -> Result<(), Box<std::error::Error>> {
    setup_logger(&conf.logger).map_err(|e| e.compat())?;

    let (config_rules, archive_config, icinga2_client_config) = config::parse_config_files(&conf)?;

    // Start matcher
    let matcher = Arc::new(Matcher::build(&config_rules).map_err(|e| e.compat())?);

    // start system
    System::run(move || {
        let cpus = num_cpus::get();
        info!("Available CPUs: {}", cpus);

        // Start archive executor actor
        let archive_executor_addr = SyncArbiter::start(1, move || {
            let executor = tornado_executor_archive::ArchiveExecutor::new(&archive_config);
            ExecutorActor { executor }
        });

        // Start script executor actor
        let script_executor_addr = SyncArbiter::start(1, move || {
            let executor = tornado_executor_script::ScriptExecutor::new();
            ExecutorActor { executor }
        });

        // Start Icinga2 Client Actor
        let icinga2_client_addr =
            executor::icinga2::Icinga2ApiClientActor::start_new(icinga2_client_config);

        // Start icinga2 executor actor
        let icinga2_executor_addr = SyncArbiter::start(1, move || {
            let icinga2_client_addr_clone = icinga2_client_addr.clone();
            let executor = tornado_executor_icinga2::Icinga2Executor::new(move |icinga2action| {
                icinga2_client_addr_clone
                    .do_send(Icinga2ApiClientMessage { message: icinga2action });
                Ok(())
            });
            ExecutorActor { executor }
        });

        // Configure action dispatcher
        let event_bus = {
            let event_bus = ActixEventBus {
                callback: move |action| {
                    match action.id.as_ref() {
                        "archive" => archive_executor_addr.do_send(ActionMessage { action }),
                        "icinga2" => icinga2_executor_addr.do_send(ActionMessage { action }),
                        "script" => script_executor_addr.do_send(ActionMessage { action }),
                        _ => error!("There are not executors for action id [{}]", &action.id),
                    };
                },
            };
            Arc::new(event_bus)
        };

        // Start dispatcher actor
        let dispatcher_addr = SyncArbiter::start(1, move || {
            let dispatcher =
                Dispatcher::build(event_bus.clone()).expect("Cannot build the dispatcher");
            DispatcherActor { dispatcher }
        });

        // Start matcher actor
        let matcher_addr = SyncArbiter::start(cpus, move || MatcherActor {
            matcher: matcher.clone(),
            dispatcher_addr: dispatcher_addr.clone(),
        });

        // Start Event Json TCP listener
        let tcp_address = format!("{}:{}", conf.io.event_socket_ip, conf.io.event_socket_port);
        let json_matcher_addr_clone = matcher_addr.clone();
        listen_to_tcp(tcp_address.clone(), move |msg| {
            let json_matcher_addr_clone = json_matcher_addr_clone.clone();
            JsonEventReaderActor::start_new(msg, move |event| {
                json_matcher_addr_clone.do_send(EventMessage { event })
            });
        })
        .and_then(|_| {
            info!("Started TCP server at [{}]. Listening for incoming events", tcp_address);
            Ok(())
        })
        // here we are forced to unwrap by the Actix API. See: https://github.com/actix/actix/issues/203
        .unwrap_or_else(|err| {
            error!("Cannot start TCP server at [{}]. Err: {}", tcp_address, err);
            std::process::exit(1);
        });

        // Start snmptrapd Json UDS listener
        let snmptrapd_tpc_address =
            format!("{}:{}", conf.io.snmptrapd_socket_ip, conf.io.snmptrapd_socket_port);
        let snmptrapd_matcher_addr_clone = matcher_addr.clone();
        listen_to_tcp(snmptrapd_tpc_address.clone(), move |msg| {
            collector::snmptrapd::SnmptrapdJsonReaderActor::start_new(
                msg,
                snmptrapd_matcher_addr_clone.clone(),
            );
        })
        .and_then(|_| {
            info!(
                "Started TCP server at [{}]. Listening for incoming SNMPTRAPD events",
                snmptrapd_tpc_address
            );
            Ok(())
        })
        // here we are forced to unwrap by the Actix API. See: https://github.com/actix/actix/issues/203
        .unwrap_or_else(|err| {
            error!("Cannot start TCP server at [{}]. Err: {}", snmptrapd_tpc_address, err);
            std::process::exit(1);
        });
    });

    Ok(())
}
