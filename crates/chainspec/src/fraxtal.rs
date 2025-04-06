//! Chain specification for the Base Mainnet network.

use std::sync::{Arc, LazyLock};

use alloy_chains::Chain;
use alloy_primitives::b256;
use alloy_primitives::U256;
use reth_chainspec::{BaseFeeParams, BaseFeeParamsKind, ChainHardforks, ChainSpec, ForkCondition};
use reth_ethereum_forks::EthereumHardfork;
use reth_ethereum_forks::Hardfork;
use reth_optimism_chainspec::make_op_genesis_header;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_forks::OpHardfork;
use reth_primitives_traits::SealedHeader;

/// The Base mainnet spec
pub(crate) static FRAXTAL_MAINNET: LazyLock<Arc<OpChainSpec>> = LazyLock::new(|| {
    let genesis = serde_json::from_str(include_str!("../res/genesis/mainnet.json"))
        .expect("Can't deserialize Fraxtal genesis json");
    let hardforks = FRAXTAL_MAINNET_HARDFORKS.clone();
    OpChainSpec {
        inner: ChainSpec {
            chain: Chain::fraxtal(),
            genesis_header: SealedHeader::new(
                make_op_genesis_header(&genesis, &hardforks),
                b256!("0x521982bd54239dc71269eefb58601762cc15cfb2978e0becb46af7962ed6bfaa"),
            ),
            genesis,
            paris_block_and_final_difficulty: Some((0, U256::from(0))),
            hardforks,
            base_fee_params: BaseFeeParamsKind::Variable(
                vec![
                    (EthereumHardfork::London.boxed(), BaseFeeParams::optimism()),
                    (
                        OpHardfork::Canyon.boxed(),
                        BaseFeeParams {
                            max_change_denominator: 250,
                            elasticity_multiplier: 10,
                        },
                    ),
                ]
                .into(),
            ),
            ..Default::default()
        },
    }
    .into()
});

static FRAXTAL_MAINNET_HARDFORKS: LazyLock<ChainHardforks> = LazyLock::new(|| {
    ChainHardforks::new(vec![
        (EthereumHardfork::Frontier.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Homestead.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Tangerine.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::SpuriousDragon.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Byzantium.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::Constantinople.boxed(),
            ForkCondition::Block(0),
        ),
        (
            EthereumHardfork::Petersburg.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Istanbul.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::MuirGlacier.boxed(),
            ForkCondition::Block(0),
        ),
        (EthereumHardfork::Berlin.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::London.boxed(), ForkCondition::Block(0)),
        (
            EthereumHardfork::ArrowGlacier.boxed(),
            ForkCondition::Block(0),
        ),
        (
            EthereumHardfork::GrayGlacier.boxed(),
            ForkCondition::Block(0),
        ),
        (
            EthereumHardfork::Paris.boxed(),
            ForkCondition::TTD {
                activation_block_number: 0,
                fork_block: Some(0),
                total_difficulty: U256::ZERO,
            },
        ),
        (OpHardfork::Bedrock.boxed(), ForkCondition::Block(0)),
        (OpHardfork::Regolith.boxed(), ForkCondition::Timestamp(0)),
        (
            EthereumHardfork::Shanghai.boxed(),
            ForkCondition::Timestamp(0),
        ),
        (OpHardfork::Canyon.boxed(), ForkCondition::Timestamp(0)),
        (
            EthereumHardfork::Cancun.boxed(),
            ForkCondition::Timestamp(1717009201),
        ),
        (
            OpHardfork::Ecotone.boxed(),
            ForkCondition::Timestamp(1717009201),
        ),
        (
            OpHardfork::Fjord.boxed(),
            ForkCondition::Timestamp(1733947201),
        ),
        (
            OpHardfork::Granite.boxed(),
            ForkCondition::Timestamp(1738958401),
        ),
        (
            OpHardfork::Holocene.boxed(),
            ForkCondition::Timestamp(1744052401),
        ),
    ])
});
