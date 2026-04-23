//! Bootnodes for the Fraxtal networks.

use crate::{fraxtal::FRAXTAL_MAINNET, fraxtal_hoodi_testnet::FRAXTAL_HOODI_TESTNET};
use reth_chainspec::EthChainSpec;
use reth_network_peers::{NodeRecord, parse_nodes};

/// Bootnodes for Fraxtal mainnet.
pub static FRAXTAL_MAINNET_BOOTNODES: &[&str] = &[
    "enode://3628cd2691d1fded97bc02ac312dea9cf77e0f2a3f3ec682acc8fea6029b56c0d56940f0d6960fb58ae198929499ab7965dd6826b6a7056c5495b4300980265b@44.237.102.237:30301",
    "enode://3628cd2691d1fded97bc02ac312dea9cf77e0f2a3f3ec682acc8fea6029b56c0d56940f0d6960fb58ae198929499ab7965dd6826b6a7056c5495b4300980265b@44.237.102.237:9200",
    "enode://ef3189491c952c132722b8909390a1cb136a2067a72cdb1b8f236f021aad024c3e50841b8e566f14d7b5cea0ba44d9010c07e5a20d497ddccb5ff7a6e35b73bd@44.214.254.33:30301",
    "enode://ef3189491c952c132722b8909390a1cb136a2067a72cdb1b8f236f021aad024c3e50841b8e566f14d7b5cea0ba44d9010c07e5a20d497ddccb5ff7a6e35b73bd@44.214.254.33:9200",
    "enode://9a1f3aed3d059873001c149585adb1a56cd52c042d1512109c1f4d916e0108c06fbd00da6ef1844937542e9b3f89b582a1c907345e44ad52b90dfee00beac761@34.243.69.12:30301",
    "enode://9a1f3aed3d059873001c149585adb1a56cd52c042d1512109c1f4d916e0108c06fbd00da6ef1844937542e9b3f89b582a1c907345e44ad52b90dfee00beac761@34.243.69.12:9200",
];

/// Bootnodes for Fraxtal Hoodi testnet.
pub static FRAXTAL_HOODI_TESTNET_BOOTNODES: &[&str] = &[
    "enode://47163be5a7d0806d0f986cef90825127ca47950cb5bc72117bdaa61485ac744f1fd7344070163791ec60331ca4cd1127f1869f22af64373487683f229a9bfa86@40.160.27.54:30306",
    "enode://47163be5a7d0806d0f986cef90825127ca47950cb5bc72117bdaa61485ac744f1fd7344070163791ec60331ca4cd1127f1869f22af64373487683f229a9bfa86@40.160.27.54:30310",
    "enode://b467ac6f93a161a1a3808847d16077e30f98810fe9740c54a7eb5f98d917fa555ca957fba7370b6f752b87de4a88ab6e9d4e0d71890dc25bfc17b33eba717217@15.204.110.99:30306",
    "enode://b467ac6f93a161a1a3808847d16077e30f98810fe9740c54a7eb5f98d917fa555ca957fba7370b6f752b87de4a88ab6e9d4e0d71890dc25bfc17b33eba717217@15.204.110.99:30310",
];

/// Returns parsed Fraxtal mainnet bootnodes.
pub fn fraxtal_mainnet_nodes() -> Vec<NodeRecord> {
    parse_nodes(FRAXTAL_MAINNET_BOOTNODES)
}

/// Returns parsed Fraxtal Hoodi testnet bootnodes.
pub fn fraxtal_hoodi_testnet_nodes() -> Vec<NodeRecord> {
    parse_nodes(FRAXTAL_HOODI_TESTNET_BOOTNODES)
}

/// Returns the built-in bootnodes for the given chain id, if any.
///
/// The chain ids are read from the genesis JSON files so they stay in sync with the
/// chainspecs themselves.
pub fn fraxtal_bootnodes(chain_id: u64) -> Option<Vec<NodeRecord>> {
    if chain_id == FRAXTAL_MAINNET.chain().id() {
        Some(fraxtal_mainnet_nodes())
    } else if chain_id == FRAXTAL_HOODI_TESTNET.chain().id() {
        Some(fraxtal_hoodi_testnet_nodes())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mainnet_bootnodes() {
        let nodes = fraxtal_mainnet_nodes();
        assert_eq!(nodes.len(), FRAXTAL_MAINNET_BOOTNODES.len());
    }

    #[test]
    fn parse_hoodi_testnet_bootnodes() {
        let nodes = fraxtal_hoodi_testnet_nodes();
        assert_eq!(nodes.len(), FRAXTAL_HOODI_TESTNET_BOOTNODES.len());
    }

    #[test]
    fn lookup_by_chain_id() {
        assert_eq!(fraxtal_bootnodes(FRAXTAL_MAINNET.chain().id()).unwrap().len(), 6);
        assert_eq!(fraxtal_bootnodes(FRAXTAL_HOODI_TESTNET.chain().id()).unwrap().len(), 4);
        assert!(fraxtal_bootnodes(1).is_none());
    }
}
