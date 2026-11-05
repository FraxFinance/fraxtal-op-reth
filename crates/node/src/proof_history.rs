//! Node launcher with proof history support.

use crate::node::FraxtalNode;
use eyre::ErrReport;
use futures_util::FutureExt;
use reth_db::DatabaseEnv;
use reth_db_api::database_metrics::DatabaseMetrics;
use reth_node_builder::{FullNodeComponents, NodeBuilder, WithLaunchContext};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_exex::OpProofsExEx;
use reth_optimism_rpc::{
    debug::{DebugApiExt, DebugApiOverrideServer},
    eth::proofs::{EthApiExt, EthApiOverrideServer},
};
use reth_optimism_trie::{OpProofsStorage, db::MdbxProofsStorage};
use reth_provider::{DatabaseProviderFactory, StorageSettingsCache};
use std::sync::Arc;
use tokio::time::sleep;
use tracing::{info, warn};

/// Launches a Fraxtal node, optionally installing proof history, then waits for it to exit.
///
/// Mirrors `reth_optimism_node::launch_node`, which only accepts an `OpNode`.
///
/// Fails if the engine config enables state masking, which op-reth does not support, or if the
/// test-only `--testing.sdm-fixed-policy` is set: upstream serves it with a stock `OpEvmConfig`
/// that skips the Fraxtal migrations, so [`FraxtalNode`] does not support it.
pub async fn launch_node(
    builder: WithLaunchContext<NodeBuilder<DatabaseEnv, OpChainSpec>>,
    node: FraxtalNode,
) -> eyre::Result<(), ErrReport> {
    // op-reth's CLI already defaults masking to 0 and rejects nonzero values; this catches
    // callers that build a `NodeConfig` without that CLI.
    eyre::ensure!(
        builder.config().engine.num_state_masking_blocks() == 0,
        "--engine.num-state-masking-blocks must be 0 on op-reth: state masking is not supported"
    );
    eyre::ensure!(
        node.args.testing_sdm_fixed_policy.is_none(),
        "--testing.sdm-fixed-policy is not supported by fraxtal-op-reth"
    );

    let builder = if node.args.proofs_history {
        // Defaults to `<reth-data-dir>/historical-proofs` when not supplied — see
        // [`ProofsHistoryStorageArgs::resolve_storage_path`].
        let path = node.args.history.resolve_storage_path(builder.config().datadir().as_ref());
        let window = node.args.proofs_history_window.window;
        let verification_interval = node.args.proofs_history_verification_interval;

        info!(target: "reth::cli", "Using on-disk storage for proofs history");
        let mdbx = Arc::new(
            MdbxProofsStorage::new(&path)
                .map_err(|e| eyre::eyre!("Failed to create MdbxProofsStorage: {e}"))?,
        );
        if mdbx.holds_only_legacy_v1_data()? {
            eyre::bail!(
                "Proofs storage at {} only holds legacy v1 data. Delete the proofs storage \
                 directory (or use a fresh --proofs-history.storage-path), then run \
                 'fraxtal-op-reth proofs init --proofs-history.storage-path <PATH>'.",
                path.display()
            );
        }
        let storage: OpProofsStorage<Arc<MdbxProofsStorage>> = mdbx.clone().into();
        let storage_exec = storage.clone();

        builder
            .node(node)
            .on_node_started(move |node| {
                let metrics_report_interval = node.config.metrics.push_gateway_interval;
                node.task_executor.spawn_critical_task(
                    "op-proofs-storage-metrics",
                    async move {
                        info!(
                            target: "reth::cli",
                            ?metrics_report_interval,
                            "Starting op-proofs-storage metrics task"
                        );

                        loop {
                            sleep(metrics_report_interval).await;
                            mdbx.report_metrics();
                        }
                    },
                );
                Ok(())
            })
            .install_exex("proofs-history", async move |exex_context| {
                Ok(OpProofsExEx::builder(exex_context, storage_exec)
                    .with_proofs_history_window(window)
                    .with_verification_interval(verification_interval)
                    .build()
                    .run()
                    .boxed())
            })
            .extend_rpc_modules(move |ctx| {
                info!(target: "reth::cli", "Installing proofs-history RPC overrides (eth_getProof, debug_executePayload)");
                let api_ext = EthApiExt::new(ctx.registry.eth_api().clone(), storage.clone());
                let auth_api_ext = EthApiExt::new(ctx.registry.eth_api().clone(), storage.clone());
                let debug_ext = DebugApiExt::new(
                    ctx.node().provider().clone(),
                    ctx.registry.eth_api().clone(),
                    storage,
                    ctx.node().task_executor().clone(),
                    ctx.node().evm_config().clone(),
                );
                let eth_replaced = ctx.modules.replace_configured(api_ext.into_rpc())?;
                let auth_eth_replaced =
                    ctx.auth_module.replace_auth_methods(auth_api_ext.into_rpc())?;
                let debug_replaced = ctx.modules.replace_configured(debug_ext.into_rpc())?;
                info!(target: "reth::cli", eth_replaced, auth_eth_replaced, debug_replaced, "Proofs-history RPC overrides installed");
                Ok(())
            })
    } else {
        builder.node(node)
    };

    let handle = builder.launch_with_debug_capabilities().await?;
    match handle.node.provider.database_provider_ro() {
        Ok(provider) if !provider.cached_storage_settings().is_v2() => {
            warn!(
                target: "reth::cli",
                "Storage V1 is deprecated and will be removed in January 2027. Stop the node, then migrate to Storage V2 with `fraxtal-op-reth db migrate-v2` using the same chain and data-directory arguments."
            );
        }
        Ok(_) => {}
        Err(err) => {
            warn!(target: "reth::cli", %err, "Failed to check whether Storage V1 deprecation applies");
        }
    }
    handle.node_exit_future.await
}
