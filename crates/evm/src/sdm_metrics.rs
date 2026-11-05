//! Zero-registration and pass counting for the `0x7D` post-exec validation series.
//!
//! Mirrors the crate-private parts of `reth_optimism_evm::sdm_metrics` that `FraxtalEvmConfig`
//! needs. Failures are reported through the public
//! [`reth_optimism_evm::report_post_exec_validation_failure`], which counts into the same series.
//!
//! `new_with_labels` resolves the recorder on every call instead of caching it, so a report before
//! the recorder is installed drops that one sample rather than silencing the counter. The same
//! applies to the zero-registration: it only reaches the exporter when a configuration is
//! constructed after the recorder is installed.

#[cfg(feature = "std")]
use op_alloy_consensus::PostExecPayloadValidationError;
#[cfg(feature = "std")]
use reth_metrics::{
    Metrics,
    metrics::{self, Counter},
};

#[cfg(feature = "std")]
const RESULT_OK: &str = "ok";
#[cfg(feature = "std")]
const RESULT_FAIL: &str = "fail";
#[cfg(feature = "std")]
const ALL_RESULTS: [&str; 2] = [RESULT_OK, RESULT_FAIL];

#[cfg(feature = "std")]
#[derive(Metrics)]
#[metrics(scope = "op_sdm")]
struct SdmPostExecValidationMetrics {
    /// Post-exec structural validation failures, by the first rule the parser rejected on.
    post_exec_validation_failure_total: Counter,
}

#[cfg(feature = "std")]
#[derive(Metrics)]
#[metrics(scope = "op_sdm")]
struct SdmPostExecResultMetrics {
    /// Post-exec validations by outcome: `ok` when a block's post-exec transaction passes
    /// structural validation, `fail` on any rejection. Blocks carrying no post-exec transaction
    /// count under neither outcome, so the series is an SDM activity signal, not a block counter.
    post_exec_validation_total: Counter,
}

#[cfg(feature = "std")]
fn failure_counter(reason: &'static str) -> Counter {
    SdmPostExecValidationMetrics::new_with_labels(&[("reason", reason)])
        .post_exec_validation_failure_total
}

#[cfg(feature = "std")]
fn result_counter(result: &'static str) -> Counter {
    SdmPostExecResultMetrics::new_with_labels(&[("result", result)]).post_exec_validation_total
}

/// Registers every series this module can emit at zero.
///
/// The counters are otherwise label-lazy: a series would first exist at the event that creates
/// it, and `increase()` never counts a series birth, so the first failure a process ever sees
/// could fire no alert. With the zeros registered — and scraped at least once — that failure is
/// a 0 -> 1 step on an existing series. A call before the recorder is installed is dropped
/// silently, and registration is idempotent, so callers run it on every configuration
/// construction.
pub(crate) fn register_sdm_metrics_at_zero() {
    #[cfg(feature = "std")]
    {
        for reason in PostExecPayloadValidationError::ALL_REASONS {
            failure_counter(reason).increment(0);
        }
        for result in ALL_RESULTS {
            result_counter(result).increment(0);
        }
    }
}

/// Counts a block whose post-exec transaction passed structural validation.
pub(crate) fn report_post_exec_validation_ok() {
    #[cfg(feature = "std")]
    result_counter(RESULT_OK).increment(1);
}
