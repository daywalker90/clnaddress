use anyhow::anyhow;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use bech32::{Bech32, Hrp};
use cln_plugin::options::{
    ConfigOption, DefaultIntegerConfigOption, DefaultStringConfigOption, StringConfigOption,
};
use cln_rpc::model::requests::InvoiceRequest;
use cln_rpc::primitives::{Amount, AmountOrAny};
use nostr_sdk::event::{Event, Kind, TagKind};
use nostr_sdk::util::JsonUtil;
use parse::get_startup_options;
use rpc::{user_add, user_del};
use serde_json::json;
use structs::{InvoiceQueryParams, LnurlpCallback, LnurlpConfig, PluginState};
use tokio::fs;
use tokio::io::{stdin, stdout};
use uuid::Uuid;

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
        100000000000,
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
    let configured_plugin = if let Some(plugin) = cln_plugin::Builder::new(stdin(), stdout())
        .option(OPT_CLNADDRESS_LISTEN)
        .option(OPT_CLNADDRESS_BASE_URL)
        .option(OPT_CLNADDRESS_MIN_RECEIVABLE)
        .option(OPT_CLNADDRESS_MAX_RECEIVABLE)
        .option(OPT_CLNADDRESS_DESCRIPTION)
        .option(OPT_CLNADDRESS_NOSTR_PRIVKEY)
        .rpcmethod(
            "clnaddress-adduser",
            "Add a user with optional metadata to create a ln address",
            user_add,
        )
        .rpcmethod(
            "clnaddress-deluser",
            "Remove a user previously created by clnaddress-adduser",
            user_del,
        )
        .dynamic()
        .configure()
        .await?
    {
        plugin
    } else {
        return Ok(());
    };

    let mut state = match get_startup_options(&configured_plugin) {
        Ok(s) => s,
        Err(e) => {
            return configured_plugin
                .disable(&format!("Error parsing options: {}", e))
                .await
        }
    };

    match fs::create_dir_all(&state.plugin_dir).await {
        Ok(_) => (),
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => (),
            _ => log::warn!("Error creating directory: {}", e),
        },
    };
    match fs::read_to_string(state.plugin_dir.join(CLNADDRESS_USERS_FILENAME)).await {
        Ok(content) => *state.users.lock() = serde_json::from_str(&content)?,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => (),
            _ => log::warn!("Could not read {} file: {}", CLNADDRESS_USERS_FILENAME, e),
        },
    };
    match fs::read_to_string(state.plugin_dir.join(CLNADDRESS_PAYINDEX_FILENAME)).await {
        Ok(content) => state.payindex = serde_json::from_str(&content)?,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => (),
            _ => log::warn!(
                "Could not read {} file: {}",
                CLNADDRESS_PAYINDEX_FILENAME,
                e
            ),
        },
    };

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
                .disable(&format!("Error binding to listen address: {}", e))
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
            Ok(_) => _ = plugin_clone.shutdown(),
            Err(e) => {
                println!(
                    "{}",
                    serde_json::json!({"jsonrpc": "2.0",
            "method": "log",
            "params": {"level":"warn",
            "message":format!("Error running server: {}", e)}})
                );
                _ = plugin_clone.shutdown();
            }
        }
    });
    if plugin.state().nostr_zapper_keys.is_some() {
        let plugin_zap_clone = plugin.clone();
        tokio::spawn(async move {
            match tasks::zap_receipt_sender(plugin_zap_clone.clone()).await {
                Ok(_) => _ = plugin_zap_clone.shutdown(),
                Err(e) => {
                    println!(
                        "{}",
                        serde_json::json!({"jsonrpc": "2.0",
                "method": "log",
                "params": {"level":"warn",
                "message":format!("Error running zap_receipt_sender: {}", e)}})
                    );
                    _ = plugin_zap_clone.shutdown();
                }
            }
        });
    }

    plugin.join().await
}

async fn get_lnurlp_config(
    maybe_user: Option<axum::extract::Path<String>>,
    State(state): State<PluginState>,
) -> Result<Json<LnurlpConfig>, axum::response::Response> {
    if let Some(axum::extract::Path(user)) = maybe_user {
        let metadata = generate_user_metadata(&state, &user)
            .map_err(|e| (StatusCode::NOT_FOUND, lnurl_error(e.to_string())).into_response())?;

        Ok(Json(LnurlpConfig {
            callback: state
                .base_url
                .join("invoice/")
                .unwrap()
                .join(&user)
                .unwrap()
                .to_string(),
            max_sendable: state.max_sendable_msat,
            min_sendable: state.min_sendable_msat,
            metadata: serde_json::to_string(&metadata).unwrap(),
            tag: "payRequest".to_owned(),
            comment_allowed: None,
            allows_nostr: state.nostr_zapper_keys.is_some(),
            nostr_pubkey: state.nostr_zapper_keys.map(|p| p.public_key().to_hex()),
        }))
    } else {
        Ok(Json(LnurlpConfig {
            callback: state.base_url.join("invoice").unwrap().to_string(),
            max_sendable: state.max_sendable_msat,
            min_sendable: state.min_sendable_msat,
            metadata: serde_json::to_string(&vec![vec![
                "text/plain".to_string(),
                state.default_description,
            ]])
            .unwrap(),
            tag: "payRequest".to_owned(),
            comment_allowed: None,
            allows_nostr: state.nostr_zapper_keys.is_some(),
            nostr_pubkey: state.nostr_zapper_keys.map(|p| p.public_key().to_hex()),
        }))
    }
}

async fn get_invoice(
    maybe_user: Option<axum::extract::Path<String>>,
    Query(params): Query<InvoiceQueryParams>,
    State(state): State<PluginState>,
) -> Result<Json<LnurlpCallback>, axum::response::Response> {
    if params.amount < state.min_sendable_msat {
        return Err((
            StatusCode::BAD_REQUEST,
            lnurl_error(format!(
                "`amount` below minimum: {}<{}",
                params.amount, state.min_sendable_msat,
            )),
        )
            .into_response());
    }
    if params.amount > state.max_sendable_msat {
        return Err((
            StatusCode::BAD_REQUEST,
            lnurl_error(format!(
                "`amount` above maximum: {}>{}",
                params.amount, state.max_sendable_msat,
            )),
        )
            .into_response());
    }

    let description = match &params.nostr {
        Some(d) => {
            if state.nostr_zapper_keys.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    lnurl_error("Nostr Zaps not configured".to_owned()),
                )
                    .into_response());
            }
            let zap_request: Event = Event::from_json(d).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    lnurl_error(e.to_string()),
                )
                    .into_response()
            })?;
            zap_request.verify().map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    lnurl_error(e.to_string()),
                )
                    .into_response()
            })?;
            verify_zap_request(&zap_request, params.amount).map_err(|e| {
                (StatusCode::BAD_REQUEST, lnurl_error(e.to_string())).into_response()
            })?;
            zap_request.as_json()
        }
        None => {
            if let Some(user) = maybe_user {
                serde_json::to_string(&generate_user_metadata(&state, &user).map_err(|e| {
                    (StatusCode::NOT_FOUND, lnurl_error(e.to_string())).into_response()
                })?)
                .unwrap()
            } else {
                serde_json::to_string(&vec![vec![
                    "text/plain".to_string(),
                    state.default_description,
                ]])
                .unwrap()
            }
        }
    };

    let mut cln_client = cln_rpc::ClnRpc::new(&state.rpc_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            lnurl_error(e.to_string()),
        )
            .into_response()
    })?;

    let amount_msat = if params.amount > 0 {
        AmountOrAny::Amount(Amount::from_msat(params.amount))
    } else {
        AmountOrAny::Any
    };

    let cln_response = cln_client
        .call_typed(&InvoiceRequest {
            amount_msat,
            description,
            label: Uuid::new_v4().to_string(),
            expiry: None,
            fallbacks: None,
            preimage: None,
            exposeprivatechannels: None,
            cltv: None,
            deschashonly: Some(true),
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                lnurl_error(e.to_string()),
            )
                .into_response()
        })?;

    Ok(Json(LnurlpCallback {
        pr: cln_response.bolt11,
        routes: vec![],
    }))
}

fn generate_user_metadata(
    state: &PluginState,
    user: &String,
) -> Result<Vec<Vec<String>>, anyhow::Error> {
    let users = state.users.lock();
    let user_meta = if let Some(um) = users.get(user) {
        um
    } else {
        return Err(anyhow!("User `{}` not found!", user));
    };

    let mut metadata = if let Some(user_desc) = &user_meta.description {
        vec![vec!["text/plain".to_owned(), user_desc.to_owned()]]
    } else {
        vec![vec![
            "text/plain".to_owned(),
            state.default_description.clone(),
        ]]
    };

    let port = state
        .base_url
        .port()
        .map(|p| format!(":{}", p))
        .unwrap_or_default();

    if let Some(is_email) = user_meta.is_email {
        if is_email {
            metadata.push(vec![
                "text/email".to_owned(),
                format!("{}@{}{}", user, state.base_url.host_str().unwrap(), port),
            ]);
        } else {
            metadata.push(vec![
                "text/identifier".to_owned(),
                format!("{}@{}{}", user, state.base_url.host_str().unwrap(), port),
            ]);
        }
    } else {
        metadata.push(vec![
            "text/identifier".to_owned(),
            format!("{}@{}{}", user, state.base_url.host_str().unwrap(), port),
        ]);
    }
    log::debug!("metadata: {:?}", metadata);
    Ok(metadata)
}

fn lnurl_error(error: String) -> Json<serde_json::Value> {
    log::debug!("lnurl_error: {}", error);
    Json(json!({"status":"ERROR", "reason":error}))
}

fn verify_zap_request(event: &Event, amount: u64) -> Result<(), anyhow::Error> {
    log::debug!("zap_request: {}", event.as_json());
    if event.kind != Kind::ZapRequest {
        return Err(anyhow!("Zap request has wrong kind: {}", event.kind));
    }
    if event.tags.is_empty() {
        return Err(anyhow!("Zap request MUST have tags"));
    }

    let mut e_tag = false;
    let mut p_tag = false;
    let mut relays_tag = false;
    let mut big_p_tag = None;
    for tag in event.tags.iter() {
        match tag.kind() {
            TagKind::Amount => {
                let zap_amount = tag.content().unwrap().parse::<u64>()?;
                if amount != zap_amount {
                    return Err(anyhow!(
                        "Zap request amount does not match query amount: {}!={}",
                        amount,
                        zap_amount
                    ));
                }
            }
            TagKind::Relays => {
                if tag.content().is_some() {
                    relays_tag = true;
                }
            }
            TagKind::SingleLetter(single_letter_tag) => match single_letter_tag.character {
                nostr_sdk::Alphabet::A => {
                    if !single_letter_tag.uppercase {
                        let coord = tag.content().ok_or(anyhow!("Missing value in `a` tag"))?;
                        let parts: Vec<&str> = coord.split(':').collect();
                        if parts.len() < 2 || parts.len() > 3 {
                            return Err(anyhow!("Invalid `a` tag format"));
                        }
                        let kind = parts[0]
                            .parse::<u16>()
                            .map_err(|_| anyhow!("Invalid kind"))?;
                        Kind::from_u16(kind);
                        nostr_sdk::PublicKey::from_hex(parts[1])
                            .map_err(|_| anyhow!("Invalid pubkey"))?;
                    }
                }
                nostr_sdk::Alphabet::E => {
                    if !single_letter_tag.uppercase {
                        if e_tag {
                            return Err(anyhow!("Zap request MUST have 0 or 1 e tags"));
                        } else {
                            e_tag = true;
                        }
                    }
                }
                nostr_sdk::Alphabet::P => {
                    if single_letter_tag.uppercase {
                        if big_p_tag.is_none() {
                            let key = tag.content().ok_or(anyhow!("Missing value in `P` tag"))?;
                            big_p_tag = Some(
                                nostr_sdk::PublicKey::from_hex(key)
                                    .map_err(|_| anyhow!("Invalid pubkey"))?,
                            );
                        } else {
                            return Err(anyhow!("Zap request has too many `P` tags"));
                        }
                    } else if !single_letter_tag.uppercase {
                        if p_tag {
                            return Err(anyhow!("Zap request MUST have only one p tag"));
                        } else {
                            p_tag = true;
                        }
                    }
                }
                _ => (),
            },
            _ => (),
        }
    }

    if !p_tag {
        return Err(anyhow!("Zap request MUST have only one p tag"));
    }
    if let Some(big_p) = big_p_tag {
        if *event.tags.public_keys().next().unwrap() != big_p {
            return Err(anyhow!("`P` tag must be equal to pubkey"));
        }
    }
    if !relays_tag {
        return Err(anyhow!(
            "There should be a `relays` tag in the Zap request!"
        ));
    }

    Ok(())
}
