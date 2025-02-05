//! Error types for the Fraxtal EVM module.
use reth_evm::execute::BlockExecutionError;

/// Fraxtal Block Executor Errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FraxtalBlockExecutionError {
    /// Thrown when force deploy of frxUSD/sfrxUSD code fails.
    #[error("failed to force frxUSD account code")]
    ForceFrxUSDFail,
}

impl From<FraxtalBlockExecutionError> for BlockExecutionError {
    fn from(err: FraxtalBlockExecutionError) -> Self {
        Self::other(err)
    }
}
