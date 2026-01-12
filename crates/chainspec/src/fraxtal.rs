//! Chain specification for the Fraxtal Mainnet network.

use std::sync::{Arc, LazyLock};

use alloy_genesis::Genesis;
use reth_optimism_chainspec::OpChainSpec;

/// The Fraxtal Mainnet spec.
///
/// This uses `OpChainSpec::from(genesis)` to dynamically extract hardforks
/// from the genesis file instead of hardcoding them, ensuring the computed
/// genesis hash matches the expected value.
pub(crate) static FRAXTAL_MAINNET: LazyLock<Arc<OpChainSpec>> = LazyLock::new(|| {
    let genesis: Genesis = serde_json::from_str(include_str!("../res/genesis/mainnet.json"))
        .expect("Can't deserialize Fraxtal mainnet genesis json");
    Arc::new(OpChainSpec::from(genesis))
});
