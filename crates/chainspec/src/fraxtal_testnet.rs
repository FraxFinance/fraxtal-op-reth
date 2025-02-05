//! Chain specification for the Base Mainnet network.

use std::sync::{Arc, LazyLock};

use alloy_chains::Chain;
use alloy_primitives::{b256, U256};
use reth_chainspec::{once_cell_set, BaseFeeParams, BaseFeeParamsKind, ChainSpec};
use reth_ethereum_forks::EthereumHardfork;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_forks::OpHardfork;

/// The Base mainnet spec
pub(crate) static FRAXTAL_TESTNET: LazyLock<Arc<OpChainSpec>> = LazyLock::new(|| {
    OpChainSpec {
        inner: ChainSpec {
            chain: Chain::fraxtal_testnet(),
            genesis: serde_json::from_str(include_str!("../res/genesis/testnet.json"))
                .expect("Can't deserialize Fraxtal testnet genesis json"),
            genesis_hash: once_cell_set(b256!(
                "910f5c4084b63fd860d0c2f9a04615115a5a991254700b39ba072290dbd77489"
            )),
            paris_block_and_final_difficulty: Some((0, U256::from(0))),
            hardforks: OpHardfork::base_mainnet(),
            base_fee_params: BaseFeeParamsKind::Variable(
                vec![
                    (EthereumHardfork::London.boxed(), BaseFeeParams::optimism()),
                    (OpHardfork::Canyon.boxed(), BaseFeeParams::optimism_canyon()),
                ]
                .into(),
            ),
            ..Default::default()
        },
    }
    .into()
});
