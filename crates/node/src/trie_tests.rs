//! Regression test for the incremental state root, from
//! <https://github.com/paradigmxyz/reth/pull/28264>.
//!
//! It fails against a reth that lacks that fix, so keep it until the pinned reth carries it.

use alloy_primitives::{B256, U256};
use reth_primitives_traits::Account;
use reth_trie::{
    HashedPostState, HashedStorage, StateRoot,
    hashed_cursor::{HashedPostStateCursorFactory, noop::NoopHashedCursorFactory},
    trie_cursor::{InMemoryTrieCursorFactory, noop::NoopTrieCursorFactory},
};

#[test]
fn inserted_leaf_before_deeper_stored_branch_is_included() {
    fn slot(prefix: &[u8]) -> B256 {
        let mut bytes = [0; 32];
        bytes[..prefix.len()].copy_from_slice(prefix);
        B256::from(bytes)
    }

    let address = B256::repeat_byte(0xaa);
    let account = || Account { nonce: 1, ..Default::default() };
    let old_slots = [
        slot(&[0x00]),
        slot(&[0x0f, 0x00]),
        slot(&[0x0f, 0x10]),
        slot(&[0x10, 0xa0, 0x00]),
        slot(&[0x10, 0xa0, 0x01]),
        slot(&[0x10, 0xa0, 0x10]),
        slot(&[0x10, 0xa1]),
    ];
    let inserted = slot(&[0x10, 0x00]);
    let old_state =
        HashedPostState::default().with_accounts([(address, Some(account()))]).with_storages([(
            address,
            HashedStorage::from_iter(old_slots.map(|slot| (slot, U256::from(1)))),
        )]);
    let old_sorted = old_state.clone().into_sorted();
    let (old_root, old_updates) = StateRoot::new(
        NoopTrieCursorFactory::default(),
        HashedPostStateCursorFactory::new(NoopHashedCursorFactory::default(), &old_sorted),
    )
    .with_prefix_sets(old_state.construct_prefix_sets().freeze())
    .root_with_updates()
    .unwrap();

    let new_state =
        HashedPostState::default().with_accounts([(address, Some(account()))]).with_storages([(
            address,
            HashedStorage::from_iter(
                old_slots.into_iter().chain([inserted]).map(|slot| (slot, U256::from(1))),
            ),
        )]);
    let delta = HashedPostState::default()
        .with_accounts([(address, Some(account()))])
        .with_storages([(address, HashedStorage::from_iter([(inserted, U256::from(1))]))]);
    let new_sorted = new_state.clone().into_sorted();
    let rebuilt_root = StateRoot::new(
        NoopTrieCursorFactory::default(),
        HashedPostStateCursorFactory::new(NoopHashedCursorFactory::default(), &new_sorted),
    )
    .with_prefix_sets(new_state.construct_prefix_sets().freeze())
    .root()
    .unwrap();
    let old_trie = old_updates.into_sorted();
    let incremental_root = StateRoot::new(
        InMemoryTrieCursorFactory::new(NoopTrieCursorFactory::default(), &old_trie),
        HashedPostStateCursorFactory::new(NoopHashedCursorFactory::default(), &new_sorted),
    )
    .with_prefix_sets(delta.construct_prefix_sets().freeze())
    .root()
    .unwrap();

    assert_ne!(old_root, rebuilt_root);
    assert_eq!(incremental_root, rebuilt_root);
}
