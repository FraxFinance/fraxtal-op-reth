#![allow(missing_docs, rustdoc::missing_crate_level_docs)]
// The `optimism` feature must be enabled to use this crate.

use std::sync::Arc;

use base_reth_flashblocks_rpc::rpc::EthApiExt;
use base_reth_flashblocks_rpc::rpc::EthApiOverrideServer;
use base_reth_flashblocks_rpc::state::FlashblocksState;
use base_reth_flashblocks_rpc::subscription::FlashblocksSubscriber;
use clap::Parser;
use fraxtal_chainspec::FraxtalChainSpecParser;
use fraxtal_node::node::FraxtalNode;
use futures_util::TryStreamExt;
use once_cell::sync::OnceCell;
use reth_exex::ExExEvent;
use reth_optimism_cli::Cli;
use reth_optimism_node::args::RollupArgs;
use tracing as _;
use tracing::info;
use url::Url;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
#[command(next_help_heading = "Rollup")]
struct FlashblocksRollupArgs {
    #[command(flatten)]
    pub rollup_args: RollupArgs,

    #[arg(
        long = "flashblocks-websocket-url",
        value_name = "FLASHBLOCKS_WEBSOCKET_URL",
        help = "Flashblocks upstream websocket url"
    )]
    pub websocket_url: Option<String>,
}

impl FlashblocksRollupArgs {
    fn flashblocks_enabled(&self) -> bool {
        self.websocket_url.is_some()
    }
}

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) = Cli::<FraxtalChainSpecParser, FlashblocksRollupArgs>::parse().run(
        |builder, flashbot_args| async move {
            info!(target: "reth::cli", "Launching node");

            let flashblocks_enabled = flashbot_args.flashblocks_enabled();
            let fb_cell: Arc<OnceCell<Arc<FlashblocksState<_>>>> = Arc::new(OnceCell::new());
            let node = FraxtalNode::new(flashbot_args.rollup_args.clone());

            let handle = builder
                .node(node)
                .install_exex_if(flashblocks_enabled, "flashblocks-canon", {
                    let fb_cell = fb_cell.clone();
                    move |mut ctx| async move {
                        let fb = fb_cell
                            .get_or_init(|| Arc::new(FlashblocksState::new(ctx.provider().clone())))
                            .clone();
                        Ok(async move {
                            while let Some(note) = ctx.notifications.try_next().await? {
                                if let Some(committed) = note.committed_chain() {
                                    for b in committed.blocks_iter() {
                                        fb.on_canonical_block_received(b);
                                    }
                                    let _ = ctx.events.send(ExExEvent::FinishedHeight(
                                        committed.tip().num_hash(),
                                    ));
                                }
                            }
                            Ok(())
                        })
                    }
                })
                .extend_rpc_modules(move |ctx| {
                    if flashblocks_enabled {
                        info!(message = "Starting Flashblocks");

                        let ws_url = Url::parse(
                            flashbot_args
                                .websocket_url
                                .expect("WEBSOCKET_URL must be set when Flashblocks is enabled")
                                .as_str(),
                        )?;

                        let fb = fb_cell
                            .get_or_init(|| Arc::new(FlashblocksState::new(ctx.provider().clone())))
                            .clone();

                        let mut flashblocks_client = FlashblocksSubscriber::new(fb.clone(), ws_url);
                        flashblocks_client.start();

                        let api_ext = EthApiExt::new(ctx.registry.eth_api().clone(), fb);
                        ctx.modules.replace_configured(api_ext.into_rpc())?;
                    } else {
                        info!(message = "flashblocks integration is disabled");
                    }
                    Ok(())
                })
                .launch()
                .await?;
            handle.node_exit_future.await
        },
    ) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
