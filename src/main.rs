use axum::{routing::get, Router};
use bech32::{Bech32, Hrp};
use cln_plugin::{
    options::{
        ConfigOption,
        DefaultIntegerConfigOption,
        DefaultStringConfigOption,
        StringConfigOption,
    },
    RpcMethodBuilder,
};
use parse::get_startup_options;
use rpc::{user_add, user_del};
use structs::PluginState;
use tokio::{
    fs,
    io::{stdin, stdout},
};

use crate::{
    lnurl::{get_invoice, get_lnurlp_config},
    rpc::user_list,
};

mod lnurl;
mod parse;
mod rpc;
mod structs;
mod tasks;

const OPT_CLNADDRESS_MIN_RECEIVABLE: DefaultIntegerConfigOption =
    ConfigOption::new_i64_with_default(
        "clnaddress-min-receivable",
        0,
        "Minimum receivable amount in msat",
    );
const OPT_CLNADDRESS_MAX_RECEIVABLE: DefaultIntegerConfigOption =
    ConfigOption::new_i64_with_default(
        "clnaddress-max-receivable",
        100_000_000_000,
        "Maximum receivable amount in msat",
    );
const OPT_CLNADDRESS_DESCRIPTION: DefaultStringConfigOption = ConfigOption::new_str_with_default(
    "clnaddress-description",
    "Thank you :)",
    "Description shown in wallets",
);
const OPT_CLNADDRESS_LISTEN: DefaultStringConfigOption = ConfigOption::new_str_with_default(
    "clnaddress-listen",
    "localhost:9797",
    "Listen address for the LNURL web server",
);
const OPT_CLNADDRESS_BASE_URL: StringConfigOption = ConfigOption::new_str_no_default(
    "clnaddress-base-url",
    "Base URL of you lnaddress service, e.g. https://sub.domain.org/path/",
);
const OPT_CLNADDRESS_NOSTR_PRIVKEY: StringConfigOption = ConfigOption::new_str_no_default(
    "clnaddress-nostr-privkey",
    "Nostr private key for zap receipts",
);
const CLNADDRESS_USERS_FILENAME: &str = "users.json";
const CLNADDRESS_PAYINDEX_FILENAME: &str = "payindex.json";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var(
        "CLN_PLUGIN_LOG",
        "cln_plugin=info,cln_rpc=info,clnaddress=debug,info",
    );
    let Some(configured_plugin) = cln_plugin::Builder::new(stdin(), stdout())
        .option(OPT_CLNADDRESS_LISTEN)
        .option(OPT_CLNADDRESS_BASE_URL)
        .option(OPT_CLNADDRESS_MIN_RECEIVABLE)
        .option(OPT_CLNADDRESS_MAX_RECEIVABLE)
        .option(OPT_CLNADDRESS_DESCRIPTION)
        .option(OPT_CLNADDRESS_NOSTR_PRIVKEY)
        .rpcmethod_from_builder(
            RpcMethodBuilder::new("clnaddress-adduser", user_add)
                .description("Add a user with optional metadata to create a ln address")
                .usage("user [is_email] [description]"),
        )
        .rpcmethod_from_builder(
            RpcMethodBuilder::new("clnaddress-deluser", user_del)
                .description("Remove a user previously created by clnaddress-adduser")
                .usage("user"),
        )
        .rpcmethod_from_builder(
            RpcMethodBuilder::new("clnaddress-listuser", user_list)
                .description(
                    "List all users and their settings or just the settings of the user specified",
                )
                .usage("[user]"),
        )
        .dynamic()
        .configure()
        .await?
    else {
        return Ok(());
    };

    let mut state = match get_startup_options(&configured_plugin) {
        Ok(s) => s,
        Err(e) => {
            return configured_plugin
                .disable(&format!("Error parsing options: {e}"))
                .await
        }
    };

    read_plugin_config_files(&mut state).await?;

    let lnaddress_router = Router::new()
        .route("/lnurlp", get(get_lnurlp_config))
        .route("/.well-known/lnurlp/{user}", get(get_lnurlp_config))
        .route("/invoice", get(get_invoice))
        .route("/invoice/{user}", get(get_invoice))
        .with_state(state.clone());

    let listener = match tokio::net::TcpListener::bind(&state.listen_address).await {
        Ok(o) => o,
        Err(e) => {
            return configured_plugin
                .disable(&format!("Error binding to listen address: {e}"))
                .await
        }
    };

    let plugin = configured_plugin.start(state.clone()).await?;

    log::info!(
        "Starting lnurlp server. LISTEN:{} BASE_ADDRESS:{}",
        state.listen_address,
        state.base_url
    );
    log::info!(
        "LNURL: {}",
        bech32::encode_upper::<Bech32>(
            Hrp::parse("LNURL")?,
            state.base_url.join("lnurlp")?.to_string().as_bytes()
        )?
    );
    let plugin_clone = plugin.clone();
    tokio::spawn(async move {
        match axum::serve(listener, lnaddress_router.into_make_service()).await {
            Ok(()) => _ = plugin_clone.shutdown(),
            Err(e) => {
                log_error(&format!("Error running server: {e}"));
                _ = plugin_clone.shutdown();
            }
        }
    });
    if plugin.state().nostr_zapper_keys.is_some() {
        let plugin_zap_clone = plugin.clone();
        tokio::spawn(async move {
            match tasks::zap_receipt_sender(plugin_zap_clone.clone()).await {
                Ok(()) => _ = plugin_zap_clone.shutdown(),
                Err(e) => {
                    log_error(&format!("Error running zap_receipt_sender: {e}"));
                    _ = plugin_zap_clone.shutdown();
                }
            }
        });
    }

    plugin.join().await
}

fn log_error(error: &str) {
    println!(
        "{}",
        serde_json::json!({"jsonrpc": "2.0",
                          "method": "log",
                          "params": {"level":"warn", "message":error}})
    );
}

async fn read_plugin_config_files(state: &mut PluginState) -> Result<(), anyhow::Error> {
    match fs::create_dir_all(&state.plugin_dir).await {
        Ok(()) => (),
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => (),
            _ => log::warn!("Error creating directory: {e}"),
        },
    }
    match fs::read_to_string(state.plugin_dir.join(CLNADDRESS_USERS_FILENAME)).await {
        Ok(content) => *state.users.lock() = serde_json::from_str(&content)?,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => (),
            _ => log::warn!("Could not read {CLNADDRESS_USERS_FILENAME} file: {e}"),
        },
    }
    match fs::read_to_string(state.plugin_dir.join(CLNADDRESS_PAYINDEX_FILENAME)).await {
        Ok(content) => state.payindex = serde_json::from_str(&content)?,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => (),
            _ => log::warn!("Could not read {CLNADDRESS_PAYINDEX_FILENAME} file: {e}"),
        },
    }
    Ok(())
}
