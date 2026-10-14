//! Reporting for blocks whose `0x7D` post-exec transaction fails structural validation. The
//! engine surfaces such a block only as a generic invalid payload, so the failed rule is logged
//! and counted here, where it is still known.
//!
//! Recorded from the execution-context builders. The RPC callers that reach them replay stored
//! canonical blocks, which already passed this check at import, so replays cannot inflate the
//! counter. Failures the executor raises while verifying the refunds themselves are a separate
//! class and are not counted here.
//!
//! `new_with_labels` resolves the recorder on every call instead of caching it, so a report before
//! the recorder is installed drops that one sample rather than silencing the counter.

use op_alloy_consensus::PostExecPayloadValidationError;
#[cfg(feature = "std")]
use reth_metrics::{
    Metrics,
    metrics::{self, Counter},
};

#[cfg(feature = "std")]
#[derive(Metrics)]
#[metrics(scope = "op_sdm")]
struct SdmPostExecValidationMetrics {
    /// Post-exec structural validation failures, by the first rule the parser rejected on.
    post_exec_validation_failure_total: Counter,
}

/// Logs a block rejected for its post-exec transaction and, with `std`, counts it under the
/// failed rule.
pub(crate) fn report_post_exec_validation_failure(
    block_number: u64,
    error: PostExecPayloadValidationError,
) {
    let reason = error.as_reason();
    tracing::warn!(
        block_number,
        reason,
        %error,
        "block rejected: post-exec transaction failed structural validation"
    );
    #[cfg(feature = "std")]
    SdmPostExecValidationMetrics::new_with_labels(&[("reason", reason)])
        .post_exec_validation_failure_total
        .increment(1);
}
