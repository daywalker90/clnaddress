use std::path::Path;

use cln_plugin::Plugin;
use cln_rpc::{model::requests::WaitanyinvoiceRequest, ClnRpc};
use nostr_sdk::{
    event::{Event, EventBuilder, TagKind},
    types::Timestamp,
    util::JsonUtil,
    Client,
};
use tokio::fs;

use crate::{structs::PluginState, CLNADDRESS_PAYINDEX_FILENAME};

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
                    if let Ok(event) = Event::from_json(desc.as_bytes()) {
                        let Some(bolt11) = o.bolt11 else {
                            log::warn!("No bolt11 found for zap receipt!");
                            continue;
                        };
                        let mut zap_receipt = EventBuilder::zap_receipt(
                            bolt11,
                            o.payment_preimage
                                .map(|p| serde_json::to_string(&p).unwrap()),
                            &event,
                        );
                        if let Some(paid_at) = o.paid_at {
                            zap_receipt =
                                zap_receipt.custom_created_at(Timestamp::from_secs(paid_at));
                        }

                        let zap_receipt = match zap_receipt.sign_with_keys(&keys) {
                            Ok(o) => o,
                            Err(e) => {
                                log::warn!("Could not sign zap receipt:{e}");
                                continue;
                            }
                        };
                        log::debug!("{}", zap_receipt.as_json());

                        let client = Client::new(keys.clone());

                        if let Some(relay_tag) =
                            event.tags.iter().find(|t| t.kind() == TagKind::Relays)
                        {
                            for relay_url in relay_tag.as_slice().iter().skip(1) {
                                if let Err(e) = client.add_relay(relay_url).await {
                                    log::warn!("Could not add relay {relay_url} to client: {e}");
                                };
                            }
                            client.connect().await;
                            if let Err(e) = client.send_event(&zap_receipt).await {
                                log::warn!("Could not send zap receipt: {e}");
                            };
                        } else {
                            log::warn!("No relays included in zap request!");
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
