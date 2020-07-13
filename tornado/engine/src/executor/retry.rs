use crate::executor::ActionMessage;
use actix::dev::ToEnvelope;
use actix::{Actor, Addr, Context, Handler};
use log::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tornado_executor_common::ExecutorError;

/// Defines the strategy to apply in case of a failure.
/// This is applied, for example, when an action execution fails
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RetryStrategy {
    pub retry_policy: RetryPolicy,
    pub backoff_policy: BackoffPolicy,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self { retry_policy: RetryPolicy::None, backoff_policy: BackoffPolicy::None }
    }
}

impl RetryStrategy {
    /// Returns whether a retry attempt should be performed and an optional backoff time
    pub fn should_retry(&self, failed_attempts: u32) -> (bool, Option<Duration>) {
        (
            self.retry_policy.should_retry(failed_attempts),
            self.backoff_policy.should_wait(failed_attempts),
        )
    }
}

// Defines the retry policy of a RetryStrategy
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type")]
pub enum RetryPolicy {
    /// No Retry attempts defined
    None,
    /// The operation will be retried for a max number of times.
    MaxRetries { retries: u32 },
    /// The operation will be retried an infinite number of times.
    Infinite,
    // Timeout,
}

impl RetryPolicy {
    fn should_retry(&self, failed_attempts: u32) -> bool {
        if failed_attempts == 0 {
            true
        } else {
            match self {
                RetryPolicy::None => false,
                RetryPolicy::Infinite => true,
                RetryPolicy::MaxRetries { retries: attempts } => *attempts + 1 > failed_attempts,
            }
        }
    }
}

// Defines the backoff policy of a RetryStrategy
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type")]
pub enum BackoffPolicy {
    /// No backoff, the retry will be attempted without waiting
    None,
    /// A fixed amount ot time will be waited between each retry attempt
    Fixed { ms: u32 },
    /// Permits to specify the amount of time between two consecutive retry attempts.
    /// The time to wait after 'i' retries is specified in the vector at position 'i'.
    /// If the number of retries is bigger than the vector length, then the last value in the vector is used.
    /// For example:
    /// ms = [111,222,333] -> It waits 111 ms after the first failure, 222 ms after the second failure and then 333 ms for all following failures.
    Variable { ms: Vec<u32> },
    // Exponential
}

impl BackoffPolicy {
    fn should_wait(&self, failed_attempts: u32) -> Option<Duration> {
        if failed_attempts == 0 {
            None
        } else {
            match self {
                BackoffPolicy::None => None,
                BackoffPolicy::Fixed { ms } => {
                    if *ms > 0 {
                        Some(Duration::from_millis(*ms as u64))
                    } else {
                        None
                    }
                }
                BackoffPolicy::Variable { ms } => {
                    let index = (failed_attempts - 1) as usize;
                    let option_wait_ms = if ms.len() > index { ms.get(index) } else { ms.last() };
                    match option_wait_ms {
                        Some(wait_ms) => {
                            if *wait_ms > 0 {
                                Some(Duration::from_millis(*wait_ms as u64))
                            } else {
                                None
                            }
                        }
                        None => None,
                    }
                }
            }
        }
    }
}

pub struct RetryActor<A: Actor + actix::Handler<ActionMessage>>
where
    <A as Actor>::Context: ToEnvelope<A, ActionMessage>,
{
    executor_addr: Addr<A>,
    retry_strategy: Arc<RetryStrategy>,
}

impl<A: Actor + actix::Handler<ActionMessage>> Actor for RetryActor<A>
where
    <A as Actor>::Context: ToEnvelope<A, ActionMessage>,
{
    type Context = Context<Self>;
}

impl<A: Actor + actix::Handler<ActionMessage>> RetryActor<A>
where
    <A as Actor>::Context: ToEnvelope<A, ActionMessage>,
{
    pub fn start_new<F>(retry_strategy: Arc<RetryStrategy>, factory: F) -> Addr<Self>
    where
        F: FnOnce() -> Addr<A>,
    {
        let executor_addr = factory();
        Self { retry_strategy, executor_addr }.start()
    }
}

impl<A: Actor + actix::Handler<ActionMessage>> Handler<ActionMessage> for RetryActor<A>
where
    <A as Actor>::Context: ToEnvelope<A, ActionMessage>,
{
    type Result = Result<(), ExecutorError>;

    fn handle(&mut self, mut msg: ActionMessage, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("RetryActor - received new message");

        let executor_addr = self.executor_addr.clone();
        let retry_strategy = self.retry_strategy.clone();

        actix::spawn(async move {
            let mut should_retry = true;
            while should_retry {
                should_retry = false;
                let result = executor_addr.send(msg.clone()).await;
                match result {
                    Ok(response) => {
                        if response.is_err() {
                            msg.failed_attempts += 1;
                            let (new_should_retry, should_wait) =
                                retry_strategy.should_retry(msg.failed_attempts);
                            should_retry = new_should_retry;
                            if should_retry {
                                debug!("The failed message will be reprocessed based on the current RetryPolicy. Message: {:?}", msg);
                                if let Some(delay_for) = should_wait {
                                    debug!("Wait for {:?} before retrying.", delay_for);
                                    actix::clock::delay_for(delay_for).await;
                                }
                            } else {
                                warn!("The failed message will not be retried any more in respect of the current RetryPolicy. Message: {:?}", msg)
                            }
                        }
                    }
                    Err(e) => error!("MailboxError: {}", e),
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::executor::ExecutorActor;
    use actix::SyncArbiter;
    use rand::Rng;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
    use tornado_common_api::Action;
    use tornado_executor_common::Executor;

    #[test]
    fn retry_policy_should_return_when_to_retry() {
        // None
        assert!(RetryPolicy::None.should_retry(0));
        assert!(!RetryPolicy::None.should_retry(1));
        assert!(!RetryPolicy::None.should_retry(10));
        assert!(!RetryPolicy::None.should_retry(100));

        // Max
        assert!(RetryPolicy::MaxRetries { retries: 0 }.should_retry(0));
        assert!(!RetryPolicy::MaxRetries { retries: 0 }.should_retry(1));
        assert!(!RetryPolicy::MaxRetries { retries: 0 }.should_retry(10));
        assert!(!RetryPolicy::MaxRetries { retries: 0 }.should_retry(100));

        assert!(RetryPolicy::MaxRetries { retries: 1 }.should_retry(0));
        assert!(RetryPolicy::MaxRetries { retries: 1 }.should_retry(1));
        assert!(!RetryPolicy::MaxRetries { retries: 1 }.should_retry(2));
        assert!(!RetryPolicy::MaxRetries { retries: 1 }.should_retry(10));
        assert!(!RetryPolicy::MaxRetries { retries: 1 }.should_retry(100));

        assert!(RetryPolicy::MaxRetries { retries: 10 }.should_retry(0));
        assert!(RetryPolicy::MaxRetries { retries: 10 }.should_retry(1));
        assert!(RetryPolicy::MaxRetries { retries: 10 }.should_retry(10));
        assert!(!RetryPolicy::MaxRetries { retries: 10 }.should_retry(11));
        assert!(!RetryPolicy::MaxRetries { retries: 10 }.should_retry(100));

        // Infinite
        assert!(RetryPolicy::Infinite.should_retry(0));
        assert!(RetryPolicy::Infinite.should_retry(1));
        assert!(RetryPolicy::Infinite.should_retry(10));
        assert!(RetryPolicy::Infinite.should_retry(100));
    }

    #[test]
    fn backoff_policy_should_return_the_wait_time() {
        // None
        assert_eq!(None, BackoffPolicy::None.should_wait(0));
        assert_eq!(None, BackoffPolicy::None.should_wait(1));
        assert_eq!(None, BackoffPolicy::None.should_wait(10));
        assert_eq!(None, BackoffPolicy::None.should_wait(100));

        // Fixed
        assert_eq!(None, BackoffPolicy::Fixed { ms: 100 }.should_wait(0));
        assert_eq!(
            Some(Duration::from_millis(100)),
            BackoffPolicy::Fixed { ms: 100 }.should_wait(1)
        );
        assert_eq!(
            Some(Duration::from_millis(100)),
            BackoffPolicy::Fixed { ms: 100 }.should_wait(10)
        );
        assert_eq!(
            Some(Duration::from_millis(1123)),
            BackoffPolicy::Fixed { ms: 1123 }.should_wait(100)
        );

        assert_eq!(None, BackoffPolicy::Fixed { ms: 0 }.should_wait(0));
        assert_eq!(None, BackoffPolicy::Fixed { ms: 0 }.should_wait(1));
        assert_eq!(None, BackoffPolicy::Fixed { ms: 0 }.should_wait(10));

        // Variable
        assert_eq!(None, BackoffPolicy::Variable { ms: vec!() }.should_wait(0));
        assert_eq!(None, BackoffPolicy::Variable { ms: vec!() }.should_wait(1));
        assert_eq!(None, BackoffPolicy::Variable { ms: vec!() }.should_wait(200));

        assert_eq!(None, BackoffPolicy::Variable { ms: vec!(0) }.should_wait(0));
        assert_eq!(None, BackoffPolicy::Variable { ms: vec!(0) }.should_wait(1));
        assert_eq!(None, BackoffPolicy::Variable { ms: vec!(0) }.should_wait(100));

        assert_eq!(None, BackoffPolicy::Variable { ms: vec!(100) }.should_wait(0));
        assert_eq!(
            Some(Duration::from_millis(100)),
            BackoffPolicy::Variable { ms: vec!(100) }.should_wait(1)
        );
        assert_eq!(
            Some(Duration::from_millis(100)),
            BackoffPolicy::Variable { ms: vec!(100) }.should_wait(2)
        );
        assert_eq!(
            Some(Duration::from_millis(100)),
            BackoffPolicy::Variable { ms: vec!(100) }.should_wait(10)
        );
        assert_eq!(
            Some(Duration::from_millis(100)),
            BackoffPolicy::Variable { ms: vec!(100) }.should_wait(100)
        );

        assert_eq!(None, BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(0));
        assert_eq!(
            Some(Duration::from_millis(111)),
            BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(1)
        );
        assert_eq!(
            Some(Duration::from_millis(222)),
            BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(2)
        );
        assert_eq!(None, BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(3));
        assert_eq!(
            Some(Duration::from_millis(444)),
            BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(4)
        );
        assert_eq!(
            Some(Duration::from_millis(444)),
            BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(5)
        );
        assert_eq!(
            Some(Duration::from_millis(444)),
            BackoffPolicy::Variable { ms: vec!(111, 222, 0, 444) }.should_wait(100_000)
        );
    }

    #[test]
    fn retry_policy_should_return_whether_to_retry() {
        let retry_strategy = RetryStrategy {
            retry_policy: RetryPolicy::MaxRetries { retries: 1 },
            backoff_policy: BackoffPolicy::Fixed { ms: 34 },
        };
        assert_eq!((true, None), retry_strategy.should_retry(0));
        assert_eq!((true, Some(Duration::from_millis(34))), retry_strategy.should_retry(1));
        assert_eq!((false, Some(Duration::from_millis(34))), retry_strategy.should_retry(2));
    }

    #[actix_rt::test]
    async fn should_retry_if_failure() {
        let (sender, mut receiver) = unbounded_channel();
        let attempts = rand::thread_rng().gen_range(10, 250);
        let retry_strategy = RetryStrategy {
            retry_policy: RetryPolicy::MaxRetries { retries: attempts },
            backoff_policy: BackoffPolicy::None,
        };

        let action = Arc::new(Action::new("hello"));

        let executor_addr = RetryActor::start_new(Arc::new(retry_strategy.clone()), move || {
            SyncArbiter::start(2, move || {
                let executor = AlwaysFailExecutor { sender: sender.clone() };
                ExecutorActor { executor }
            })
        });

        executor_addr.do_send(ActionMessage { action, failed_attempts: 0 });

        for _i in 0..=attempts {
            let received = receiver.recv().await.unwrap();
            assert_eq!("hello", received.id);
        }

        actix::clock::delay_for(Duration::from_millis(25)).await;
        // there should be no other messages on the channel
        assert!(receiver.try_recv().is_err());
    }

    #[actix_rt::test]
    async fn should_not_retry_if_ok() {
        let (sender, mut receiver) = unbounded_channel();
        let attempts = rand::thread_rng().gen_range(10, 250);
        let retry_strategy = RetryStrategy {
            retry_policy: RetryPolicy::MaxRetries { retries: attempts },
            backoff_policy: BackoffPolicy::None,
        };

        let action = Arc::new(Action::new("hello"));

        let executor_addr = RetryActor::start_new(Arc::new(retry_strategy.clone()), move || {
            SyncArbiter::start(2, move || {
                let executor = AlwaysOkExecutor { sender: sender.clone() };
                ExecutorActor { executor }
            })
        });

        executor_addr.do_send(ActionMessage { action, failed_attempts: 0 });

        let received = receiver.recv().await.unwrap();
        assert_eq!("hello", received.id);

        actix::clock::delay_for(Duration::from_millis(25)).await;
        // there should be no other messages on the channel
        assert!(receiver.try_recv().is_err());
    }

    #[actix_rt::test]
    async fn should_apply_the_backoff_policy_on_failure() {
        let (sender, mut receiver) = unbounded_channel();
        let wait_times = vec![10, 30, 20, 40, 50];
        let attempts = 4;
        let retry_strategy = RetryStrategy {
            retry_policy: RetryPolicy::MaxRetries { retries: attempts },
            backoff_policy: BackoffPolicy::Variable { ms: wait_times.clone() },
        };

        let action = Arc::new(Action::new("hello_world"));

        let executor_addr = RetryActor::start_new(Arc::new(retry_strategy.clone()), move || {
            SyncArbiter::start(2, move || {
                let executor = AlwaysFailExecutor { sender: sender.clone() };
                ExecutorActor { executor }
            })
        });

        executor_addr.do_send(ActionMessage { action, failed_attempts: 0 });

        for i in 0..=(attempts as usize) {
            let before_ms = chrono::Local::now().timestamp_millis();
            let received = receiver.recv().await.unwrap();
            let after_ms = chrono::Local::now().timestamp_millis();

            assert_eq!("hello_world", received.id);
            if i > 0 {
                let passed = after_ms - before_ms;
                let expected = wait_times[i - 1] as i64;
                println!("passed: {} - expected: {}", passed, expected);
                assert!(passed >= expected);
            }
        }

        actix::clock::delay_for(Duration::from_millis(25)).await;
        // there should be no other messages on the channel
        assert!(receiver.try_recv().is_err());
    }

    struct AlwaysFailExecutor {
        sender: UnboundedSender<Action>,
    }

    impl Executor for AlwaysFailExecutor {
        fn execute(&mut self, action: &Action) -> Result<(), ExecutorError> {
            self.sender.send(action.clone()).unwrap();
            Err(ExecutorError::ActionExecutionError { message: "".to_owned() })
        }
    }

    impl std::fmt::Display for AlwaysFailExecutor {
        fn fmt(&self, _fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
            Ok(())
        }
    }

    struct AlwaysOkExecutor {
        sender: UnboundedSender<Action>,
    }

    impl Executor for AlwaysOkExecutor {
        fn execute(&mut self, action: &Action) -> Result<(), ExecutorError> {
            self.sender.send(action.clone()).unwrap();
            Ok(())
        }
    }

    impl std::fmt::Display for AlwaysOkExecutor {
        fn fmt(&self, _fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
            Ok(())
        }
    }
}
