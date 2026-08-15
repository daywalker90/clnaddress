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
    OPT_CLNADDRESS_BASE_URL,
    OPT_CLNADDRESS_DESCRIPTION,
    OPT_CLNADDRESS_LISTEN,
    OPT_CLNADDRESS_MAX_RECEIVABLE,
    OPT_CLNADDRESS_MIN_RECEIVABLE,
    OPT_CLNADDRESS_NOSTR_PRIVKEY,
    OPT_CLNADDRESS_NOSTR_PRIVKEY_FILE,
    PluginState,
};

const NOSTR_SECRET_KEY_FILENAME: &str = "nostr-secret-key";

pub fn get_startup_options(
    plugin: &ConfiguredPlugin<PluginState, tokio::io::Stdin, tokio::io::Stdout>,
) -> Result<PluginState, anyhow::Error> {
    let rpc_path: PathBuf =
        Path::new(&plugin.configuration().lightning_dir).join(plugin.configuration().rpc_file);

    let listen_opt = plugin.option(&OPT_CLNADDRESS_LISTEN)?;
    let Some((listen_address_str, _listen_port_str)) = listen_opt.rsplit_once(':') else {
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

    let Some(mut base_url_str) = plugin.option(&OPT_CLNADDRESS_BASE_URL)? else {
        return Err(anyhow!("Please specify a base URL!"));
    };
    let base_url: Url = if base_url_str.ends_with('/') {
        base_url_str.parse()?
    } else {
        base_url_str.push('/');
        base_url_str.parse()?
    };

    if !base_url.has_host() {
        return Err(anyhow!("Invalid base URL! Missing host part! {base_url}"));
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

    let plugin_dir = Path::new(&plugin.configuration().lightning_dir).join("clnaddress");

    let privkey_file = plugin.option(&OPT_CLNADDRESS_NOSTR_PRIVKEY_FILE)?;
    let legacy_privkey = plugin.option(&OPT_CLNADDRESS_NOSTR_PRIVKEY)?;
    let nostr_zapper_keys = resolve_nostr_zapper_keys(privkey_file, legacy_privkey, &plugin_dir)?;

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

fn resolve_nostr_zapper_keys(
    privkey_file: Option<String>,
    legacy_privkey: Option<String>,
    plugin_dir: &Path,
) -> Result<Option<nostr::key::Keys>, anyhow::Error> {
    let default_key_file = plugin_dir.join(NOSTR_SECRET_KEY_FILENAME);

    let key = match privkey_file {
        Some(path) => {
            let Some(privkey) = read_key_file(Path::new(&path))? else {
                return Err(anyhow!(
                    "`{}` file not found: {path}",
                    OPT_CLNADDRESS_NOSTR_PRIVKEY_FILE.name()
                ));
            };
            if legacy_privkey.is_some() {
                log::warn!(
                    "Both `{}` and `{}` are set, using `{}`. Consider removing the deprecated `{}`.",
                    OPT_CLNADDRESS_NOSTR_PRIVKEY_FILE.name(),
                    OPT_CLNADDRESS_NOSTR_PRIVKEY.name(),
                    OPT_CLNADDRESS_NOSTR_PRIVKEY_FILE.name(),
                    OPT_CLNADDRESS_NOSTR_PRIVKEY.name()
                );
            }
            privkey
        }
        None => match legacy_privkey {
            Some(privkey) => {
                log::warn!(
                    "`{}` is deprecated, migrating the nostr private key to {}. You can now remove `{}` from your config, it will be read from the file automatically.",
                    OPT_CLNADDRESS_NOSTR_PRIVKEY.name(),
                    default_key_file.display(),
                    OPT_CLNADDRESS_NOSTR_PRIVKEY.name()
                );
                write_key_file(&default_key_file, &privkey)?;
                privkey
            }
            None => {
                if let Some(privkey) = read_key_file(&default_key_file)? {
                    privkey
                } else {
                    return Ok(None);
                }
            }
        },
    };

    Ok(Some(nostr::key::Keys::parse(&key)?))
}

fn read_key_file(path: &Path) -> Result<Option<String>, anyhow::Error> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content.trim().to_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_key_file(path: &Path, privkey: &str) -> Result<(), anyhow::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, privkey.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
