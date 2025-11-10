use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::anyhow;
use cln_plugin::ConfiguredPlugin;
use parking_lot::Mutex;
use url::Url;

use crate::{
    PluginState,
    OPT_CLNADDRESS_BASE_URL,
    OPT_CLNADDRESS_DESCRIPTION,
    OPT_CLNADDRESS_LISTEN,
    OPT_CLNADDRESS_MAX_RECEIVABLE,
    OPT_CLNADDRESS_MIN_RECEIVABLE,
    OPT_CLNADDRESS_NOSTR_PRIVKEY,
};

pub fn get_startup_options(
    plugin: &ConfiguredPlugin<PluginState, tokio::io::Stdin, tokio::io::Stdout>,
) -> Result<PluginState, anyhow::Error> {
    let rpc_path: PathBuf =
        Path::new(&plugin.configuration().lightning_dir).join(plugin.configuration().rpc_file);

    let listen_opt = plugin.option(&OPT_CLNADDRESS_LISTEN)?;
    let (listen_address_str, _listen_port_str) = if let Some((add, p)) = listen_opt.rsplit_once(':')
    {
        (add, p)
    } else {
        return Err(anyhow!(
            "`{}` is invalid, it should have one `:`",
            OPT_CLNADDRESS_LISTEN.name()
        ));
    };
    let listen_address: SocketAddr = match listen_address_str {
        i if i.eq("localhost") => listen_opt
            .to_socket_addrs()?
            .next()
            .ok_or(anyhow!("No address found for localhost"))?,
        _ => {
            if let Ok(addr) = listen_opt.parse() {
                addr
            } else {
                return Err(anyhow!(
                    "`{}` should be a valid IP.",
                    OPT_CLNADDRESS_LISTEN.name()
                ));
            }
        }
    };

    let mut base_url_str = if let Some(url) = plugin.option(&OPT_CLNADDRESS_BASE_URL)? {
        url
    } else {
        return Err(anyhow!("Please specify a base URL!"));
    };
    let base_url: Url = if base_url_str.ends_with('/') {
        base_url_str.parse()?
    } else {
        base_url_str.push('/');
        base_url_str.parse()?
    };

    if !base_url.has_host() {
        return Err(anyhow!("Invalid base URL! Missing host part! {}", base_url));
    }

    let min_sendable_msat = u64::try_from(plugin.option(&OPT_CLNADDRESS_MIN_RECEIVABLE)?)?;
    let max_sendable_msat = u64::try_from(plugin.option(&OPT_CLNADDRESS_MAX_RECEIVABLE)?)?;

    if min_sendable_msat > max_sendable_msat {
        return Err(anyhow!(
            "`{}` is greater than `{}`!",
            OPT_CLNADDRESS_MIN_RECEIVABLE.name(),
            OPT_CLNADDRESS_MAX_RECEIVABLE.name()
        ));
    }

    let default_description = plugin.option(&OPT_CLNADDRESS_DESCRIPTION)?;

    let nostr_zapper_keys = match plugin.option(&OPT_CLNADDRESS_NOSTR_PRIVKEY)? {
        Some(privkey) => Some(nostr_sdk::key::Keys::parse(&privkey)?),
        None => None,
    };

    let plugin_dir = Path::new(&plugin.configuration().lightning_dir).join("clnaddress");

    Ok(PluginState {
        rpc_path,
        max_sendable_msat,
        min_sendable_msat,
        default_description,
        users: Arc::new(Mutex::new(HashMap::new())),
        plugin_dir,
        base_url,
        nostr_zapper_keys,
        payindex: 0,
        listen_address,
    })
}
