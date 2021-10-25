use tornado_common_metrics::opentelemetry::metrics::{Counter, ValueRecorder, Unit};

pub const TORNADO_APP: &str = "tornado";

pub struct TornadoMeter {
    /// Counts the total events received
    pub events_received_counter: Counter<u64>,
    /// Counts the total events processed
    pub events_processed_counter: Counter<u64>,
    /// Counts the total events processing seconds
    pub events_processed_duration_seconds: ValueRecorder<f64>,
    /// Counts the total http requests received
    pub http_requests_counter: Counter<u64>,
    /// Counts the total http requests processing seconds
    pub http_requests_duration_seconds: ValueRecorder<f64>,
}

impl Default for TornadoMeter {

    fn default() -> Self {
        let meter = tornado_common_metrics::opentelemetry::global::meter("tornado");

        let events_received_counter = meter
            .u64_counter("events_received_counter")
            .with_description("Events received count")
            .init();

        let events_processed_counter = meter
            .u64_counter("events_processed_counter")
            .with_description("Events processed count")
            .init();

        let events_processed_duration_seconds = meter
            .f64_value_recorder("events_processed_duration_seconds")
            .with_description("Events processed duration")
            .with_unit(Unit::new("seconds"))
            .init();

        let http_requests_counter = meter
            .u64_counter("http_requests.counter")
            .with_description("HTTP requests count")
            .init();

        let http_requests_duration_seconds = meter
            .f64_value_recorder("http_requests.duration_secs")
            .with_description("HTTP requests duration")
            .with_unit(Unit::new("seconds"))
            .init();

        Self {
            events_received_counter,
            events_processed_counter,
            events_processed_duration_seconds,
            http_requests_counter,
            http_requests_duration_seconds
        }
    }

}