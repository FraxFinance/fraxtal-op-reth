//! Error type

/// Fraxtal specific payload building errors.
#[derive(Debug, thiserror::Error)]
pub enum FraxtalPayloadBuilderError {
    /// Thrown when force deploy of frxUSD/sfrxUSD code fails.
    #[error("failed to force frxUSD account code")]
    ForceFrxUSDFail,
}
