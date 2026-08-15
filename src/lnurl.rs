use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use cln_rpc::{
    model::requests::InvoiceRequest,
    primitives::{Amount, AmountOrAny},
};
use nostr::{
    event::{Event, Kind},
    filter::SingleLetterTag,
    nips::nip57::Nip57Tag,
};
use serde_json::json;
use uuid::Uuid;

use crate::structs::{InvoiceQueryParams, LnurlpCallback, LnurlpConfig, PluginState};

pub async fn get_lnurlp_config(
    maybe_user: Option<axum::extract::Path<String>>,
    State(state): State<PluginState>,
) -> Result<Json<LnurlpConfig>, axum::response::Response> {
    if let Some(axum::extract::Path(user)) = maybe_user {
        let metadata = generate_user_metadata(&state, &user)
            .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;

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

pub async fn get_invoice(
    maybe_user: Option<axum::extract::Path<String>>,
    Query(params): Query<InvoiceQueryParams>,
    State(state): State<PluginState>,
) -> Result<Json<LnurlpCallback>, axum::response::Response> {
    validate_invoice_amount(
        params.amount,
        state.min_sendable_msat,
        state.max_sendable_msat,
    )
    .map_err(axum::response::IntoResponse::into_response)?;

    let description =
        match &params.nostr {
            Some(d) => {
                if state.nostr_zapper_keys.is_none() {
                    return Err(
                        (StatusCode::OK, lnurl_error("Nostr Zaps not configured")).into_response()
                    );
                }
                let zap_request: Event = Event::from_json(d)
                    .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;
                zap_request
                    .verify()
                    .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;
                let zap_request_json = zap_request
                    .try_as_json()
                    .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;
                log::debug!("zap_request: {zap_request_json}");
                verify_zap_request(
                    &zap_request,
                    params.amount,
                    &state.nostr_zapper_keys.unwrap(),
                )
                .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;
                zap_request_json
            }
            None => {
                if let Some(user) = maybe_user {
                    serde_json::to_string(&generate_user_metadata(&state, &user).map_err(|e| {
                        (StatusCode::OK, lnurl_error(&e.to_string())).into_response()
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

    let mut cln_client = cln_rpc::ClnRpc::new(&state.rpc_path)
        .await
        .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;

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
        .map_err(|e| (StatusCode::OK, lnurl_error(&e.to_string())).into_response())?;

    Ok(Json(LnurlpCallback {
        pr: cln_response.bolt11,
        routes: vec![],
    }))
}

fn validate_invoice_amount(
    requested_amount: u64,
    min_sendable_msat: u64,
    max_sendable_msat: u64,
) -> Result<(), (axum::http::StatusCode, axum::Json<serde_json::Value>)> {
    if requested_amount < min_sendable_msat {
        return Err((
            StatusCode::OK,
            lnurl_error(&format!(
                "`amount` below minimum: {requested_amount}<{min_sendable_msat}",
            )),
        ));
    }
    if requested_amount > max_sendable_msat {
        return Err((
            StatusCode::OK,
            lnurl_error(&format!(
                "`amount` above maximum: {requested_amount}>{max_sendable_msat}",
            )),
        ));
    }
    Ok(())
}

fn generate_user_metadata(
    state: &PluginState,
    user: &String,
) -> Result<Vec<Vec<String>>, anyhow::Error> {
    let users = state.users.lock();
    let Some(user_meta) = users.get(user) else {
        return Err(anyhow!("User `{user}` not found!"));
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
        .map(|p| format!(":{p}"))
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
    log::debug!("metadata: {metadata:?}");
    Ok(metadata)
}

fn lnurl_error(error: &str) -> Json<serde_json::Value> {
    log::debug!("lnurl_error: {error}");
    Json(json!({"status":"ERROR", "reason":error}))
}

pub fn verify_zap_request(
    event: &Event,
    amount: u64,
    nostr_zapper_keys: &nostr::key::Keys,
) -> Result<(), anyhow::Error> {
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
    for tag in event.tags.as_slice() {
        if let Ok(nip57tag) = Nip57Tag::parse(tag.clone()) {
            match nip57tag {
                Nip57Tag::Amount {
                    millisats,
                    bolt11: _,
                } => {
                    if amount != millisats {
                        return Err(anyhow!(
                            "Zap request amount does not match query amount: {amount}!={millisats}"
                        ));
                    }
                }
                Nip57Tag::Relays(relays) if !relays.is_empty() => {
                    relays_tag = true;
                }
                _ => {}
            }
        }
        if let Some(single_letter_tag) = tag.single_letter_tag() {
            match single_letter_tag {
                SingleLetterTag::LOWERCASE_A => {
                    let coord = tag.content().ok_or(anyhow!("Missing value in `a` tag"))?;
                    let parts: Vec<&str> = coord.split(':').collect();
                    if parts.len() < 2 || parts.len() > 3 {
                        return Err(anyhow!("Invalid `a` tag format"));
                    }
                    let kind = parts[0]
                        .parse::<u16>()
                        .map_err(|_| anyhow!("Invalid kind"))?;
                    Kind::from_u16(kind);
                    nostr::key::PublicKey::from_hex(parts[1])
                        .map_err(|_| anyhow!("Invalid pubkey"))?;
                }
                SingleLetterTag::LOWERCASE_E => {
                    if e_tag {
                        return Err(anyhow!("Zap request MUST have 0 or 1 e tags"));
                    }
                    e_tag = true;
                }
                SingleLetterTag::LOWERCASE_P => {
                    if p_tag {
                        return Err(anyhow!("Zap request MUST have only one p tag"));
                    }
                    p_tag = true;
                }
                SingleLetterTag::UPPERCASE_P => {
                    if big_p_tag.is_none() {
                        let key = tag.content().ok_or(anyhow!("Missing value in `P` tag"))?;
                        big_p_tag = Some(
                            nostr::key::PublicKey::from_hex(key)
                                .map_err(|_| anyhow!("Invalid pubkey"))?,
                        );
                    } else {
                        return Err(anyhow!("Zap request has too many `P` tags"));
                    }
                }
                _ => (),
            }
        }
    }

    if !p_tag {
        return Err(anyhow!("Zap request MUST have only one p tag"));
    }

    if let Some(big_p_tag) = big_p_tag {
        if big_p_tag != nostr_zapper_keys.public_key() {
            return Err(anyhow!("Zap request has wrong `P` tag"));
        }
    }

    if !relays_tag {
        log::info!("There should be a `relays` tag in the Zap request!");
    }

    Ok(())
}
