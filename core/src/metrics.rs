use prometheus::{HistogramVec, IntCounterVec, Opts, Registry};

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppMetrics {
    pub(crate) registry: Registry,
    pub(crate) simulation_latency_seconds: HistogramVec,
    pub(crate) rpc_error_count_total: IntCounterVec,
    pub(crate) simulation_requests_total: IntCounterVec,
    pub(crate) resource_utilization_percent: prometheus::GaugeVec,
    pub host_cpu_usage_percent: prometheus::GaugeVec,
    pub host_memory_usage_percent: prometheus::GaugeVec,
    pub process_memory_bytes: prometheus::GaugeVec,
    pub(crate) indexing_latency_seconds: HistogramVec,
    pub(crate) events_processed_total: IntCounterVec,
    pub(crate) indexing_errors_total: IntCounterVec,
    pub(crate) job_queue_depth: prometheus::GaugeVec,
}

impl AppMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let simulation_latency_seconds = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "simulation_latency_seconds",
                "Latency of simulation requests in seconds",
            ),
            &["endpoint"],
        )?;
        let rpc_error_count_total = IntCounterVec::new(
            Opts::new(
                "rpc_error_count_total",
                "Total number of RPC and simulation errors",
            ),
            &["endpoint", "error_type"],
        )?;
        let simulation_requests_total = IntCounterVec::new(
            Opts::new(
                "simulation_requests_total",
                "Total number of simulation requests by endpoint and cache status",
            ),
            &["endpoint", "cache_status"],
        )?;
        let resource_utilization_percent = prometheus::GaugeVec::new(
            Opts::new(
                "resource_utilization_percent",
                "Resource utilization percentage from latest simulation sample",
            ),
            &["resource"],
        )?;
        let host_cpu_usage_percent = prometheus::GaugeVec::new(
            Opts::new(
                "host_cpu_usage_percent",
                "Host-wide CPU usage percentage (0-100) sampled by the system alarm monitor",
            ),
            &["host"],
        )?;
        let host_memory_usage_percent = prometheus::GaugeVec::new(
            Opts::new(
                "host_memory_usage_percent",
                "Host-wide memory usage percentage (0-100) sampled by the system alarm monitor",
            ),
            &["host"],
        )?;
        let process_memory_bytes = prometheus::GaugeVec::new(
            Opts::new(
                "process_memory_bytes",
                "Resident memory size of the SoroScope process in bytes",
            ),
            &["process"],
        )?;
        let indexing_latency_seconds = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "indexing_latency_seconds",
                "Latency of ledger indexing/collection cycles in seconds",
            ),
            &["stage"],
        )?;
        let events_processed_total = IntCounterVec::new(
            Opts::new(
                "events_processed_total",
                "Total number of ledger events successfully processed",
            ),
            &["stage"],
        )?;
        let indexing_errors_total = IntCounterVec::new(
            Opts::new(
                "indexing_errors_total",
                "Total number of indexing cycle failures",
            ),
            &["stage"],
        )?;
        let job_queue_depth = prometheus::GaugeVec::new(
            Opts::new("job_queue_depth", "Current depth of background job queues"),
            &["queue"],
        )?;

        registry.register(Box::new(simulation_latency_seconds.clone()))?;
        registry.register(Box::new(rpc_error_count_total.clone()))?;
        registry.register(Box::new(simulation_requests_total.clone()))?;
        registry.register(Box::new(resource_utilization_percent.clone()))?;
        registry.register(Box::new(host_cpu_usage_percent.clone()))?;
        registry.register(Box::new(host_memory_usage_percent.clone()))?;
        registry.register(Box::new(process_memory_bytes.clone()))?;
        registry.register(Box::new(indexing_latency_seconds.clone()))?;
        registry.register(Box::new(events_processed_total.clone()))?;
        registry.register(Box::new(indexing_errors_total.clone()))?;
        registry.register(Box::new(job_queue_depth.clone()))?;

        Ok(Self {
            registry,
            simulation_latency_seconds,
            rpc_error_count_total,
            simulation_requests_total,
            resource_utilization_percent,
            host_cpu_usage_percent,
            host_memory_usage_percent,
            process_memory_bytes,
            indexing_latency_seconds,
            events_processed_total,
            indexing_errors_total,
            job_queue_depth,
        })
    }
}
