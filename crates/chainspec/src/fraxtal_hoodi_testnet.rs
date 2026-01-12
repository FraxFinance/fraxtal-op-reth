//! Chain specification for the Fraxtal Hoodi Testnet network.

use std::sync::{Arc, LazyLock};

use alloy_genesis::Genesis;
use reth_optimism_chainspec::OpChainSpec;

/// The Fraxtal Hoodi Testnet spec.
///
/// This uses `OpChainSpec::from(genesis)` to dynamically extract hardforks
/// from the genesis file instead of hardcoding them, ensuring the computed
/// genesis hash matches the expected value.
pub(crate) static FRAXTAL_HOODI_TESTNET: LazyLock<Arc<OpChainSpec>> = LazyLock::new(|| {
    let genesis: Genesis = serde_json::from_str(include_str!("../res/genesis/hoodi-testnet.json"))
        .expect("Can't deserialize Fraxtal hoodi testnet genesis json");
    Arc::new(OpChainSpec::from(genesis))
});
