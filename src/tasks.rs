use std::{path::Path, time::Duration};

use cln_plugin::Plugin;
use cln_rpc::{ClnRpc, model::requests::WaitanyinvoiceRequest};
use nostr::{
    event::{Event, EventBuilder, FinalizeEventAsync, TagCodec},
    nips::nip57::Nip57Tag,
    types::Timestamp,
};
use nostr_sdk::{authenticator::SignerAuthenticator, client::Client};
use tokio::fs;

use crate::{CLNADDRESS_PAYINDEX_FILENAME, structs::PluginState};

pub async fn zap_receipt_sender(plugin: Plugin<PluginState>) -> Result<(), anyhow::Error> {
    let mut rpc = ClnRpc::new(&plugin.state().rpc_path).await?;
    let keys = plugin.state().nostr_zapper_keys.clone().unwrap();
    let mut lastpay_index = plugin.state().payindex;
    log::debug!("lastpay_index: {lastpay_index}");
    loop {
        match rpc
            .call_typed(&WaitanyinvoiceRequest {
                lastpay_index: Some(lastpay_index),
                timeout: None,
            })
            .await
        {
            Ok(o) => {
                log::debug!("{o:?}");
                lastpay_index = o.pay_index.unwrap_or(lastpay_index + 1);
                save_payindex(&plugin.state().plugin_dir, lastpay_index).await?;
                if let Some(desc) = o.description {
                    if let Ok(zap_request) = Event::from_json(desc.as_bytes()) {
                        let Some(bolt11) = o.bolt11 else {
                            log::warn!("No bolt11 found for zap receipt!");
                            continue;
                        };
                        let mut zap_receipt = EventBuilder::zap_receipt(
                            bolt11,
                            o.payment_preimage
                                .map(|p| serde_json::to_string(&p).unwrap()),
                            &zap_request,
                        );
                        if let Some(paid_at) = o.paid_at {
                            zap_receipt =
                                zap_receipt.custom_created_at(Timestamp::from_secs(paid_at));
                        }

                        let zap_receipt = match zap_receipt.finalize_async(&keys).await {
                            Ok(o) => o,
                            Err(e) => {
                                log::warn!("Could not sign zap receipt:{e}");
                                continue;
                            }
                        };
                        if let Ok(zap_receipt_json) = zap_receipt.try_as_json() {
                            log::debug!("{zap_receipt_json}");
                        }

                        let client = Client::builder()
                            .authenticator(SignerAuthenticator::new(keys.clone()))
                            .build();

                        for tag in zap_request.tags {
                            if let Ok(Nip57Tag::Relays(relay_urls)) = Nip57Tag::parse(tag) {
                                for relay_url in &relay_urls {
                                    if let Err(e) = client.add_relay(relay_url).await {
                                        log::warn!(
                                            "Could not add relay {relay_url} to client: {e}"
                                        );
                                    }
                                }
                            }
                        }
                        if client.relays().await.is_empty() {
                            log::warn!("No relays included in zap request!");
                        }
                        client.connect().and_wait(Duration::from_secs(30)).await;
                        match client.send_event(&zap_receipt).await {
                            Ok(o) => {
                                for (url, failure) in o.failed {
                                    log::warn!("Sending to relay {url} failed: {failure}");
                                }
                            }
                            Err(e) => log::warn!("Could not send zap receipt: {e}"),
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("Err waiting on invoices: {e}");
            }
        }
    }
}

pub async fn save_payindex(path: &Path, payindex: u64) -> Result<(), anyhow::Error> {
    let serialized = serde_json::to_string(&payindex)?;
    fs::write(path.join(CLNADDRESS_PAYINDEX_FILENAME), serialized).await?;
    Ok(())
}
