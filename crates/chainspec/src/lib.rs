use fraxtal::FRAXTAL_MAINNET;
use fraxtal_hoodi_testnet::FRAXTAL_HOODI_TESTNET;
use reth_cli::chainspec::{ChainSpecParser, parse_genesis};
use reth_optimism_chainspec::OpChainSpec;
use std::sync::Arc;

pub mod bootnodes;
mod fraxtal;
mod fraxtal_hoodi_testnet;

pub use bootnodes::{
    FRAXTAL_HOODI_TESTNET_BOOTNODES, FRAXTAL_MAINNET_BOOTNODES, fraxtal_bootnodes,
    fraxtal_hoodi_testnet_nodes, fraxtal_mainnet_nodes,
};

/// Fraxtal chain specification parser.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FraxtalChainSpecParser;

impl ChainSpecParser for FraxtalChainSpecParser {
    type ChainSpec = OpChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] = &["fraxtal", "fraxtal-hoodi-testnet"];

    fn parse(s: &str) -> eyre::Result<Arc<Self::ChainSpec>> {
        chain_value_parser(s)
    }
}

/// Clap value parser for [`OpChainSpec`]s.
///
/// The value parser matches either a known chain, the path
/// to a json file, or a json formatted string in-memory. The json needs to be a Genesis struct.
pub fn chain_value_parser(s: &str) -> eyre::Result<Arc<OpChainSpec>, eyre::Error> {
    Ok(match s {
        "fraxtal" => FRAXTAL_MAINNET.clone(),
        "fraxtal-hoodi-testnet" => FRAXTAL_HOODI_TESTNET.clone(),
        _ => Arc::new(parse_genesis(s)?.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_chain_spec() {
        for &chain in FraxtalChainSpecParser::SUPPORTED_CHAINS {
            assert!(
                <FraxtalChainSpecParser as ChainSpecParser>::parse(chain).is_ok(),
                "Failed to parse {chain}"
            );
        }
    }
}

/// Convenience accessors for the built-in Fraxtal chain specs.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FraxtalChainSpec;

impl FraxtalChainSpec {
    pub fn mainnet() -> Arc<OpChainSpec> {
        FRAXTAL_MAINNET.clone()
    }

    pub fn hoodi() -> Arc<OpChainSpec> {
        FRAXTAL_HOODI_TESTNET.clone()
    }
}
