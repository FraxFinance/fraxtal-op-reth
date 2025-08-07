use alloy_primitives::{address, b256, Address, B256};

pub(super) const MAINNET_ORACLES_ADDRESSES: &[Address] = &[
    address!("0xf750636e1df115e3b334ed06e5b45c375107fc60"),
    address!("0x1B680F4385f24420D264D78cab7C58365ED3F1FF"),
];

pub(super) const PROXY_ADDR: Address = address!("fc0000000000000000000000000000000000000a");
pub(super) const PROXY_ADMIN_ADDR: Address = address!("fc0000000000000000000000000000000000000a");
pub(super) const PROXY_ADMIN_SLOT: B256 =
    b256!("b53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103");
pub(super) const PROXY_IMPLEMENTATION_SLOT: B256 =
    b256!("360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc");
