use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone)]
pub struct PluginState {
    pub rpc_path: PathBuf,
    pub max_sendable_msat: u64,
    pub min_sendable_msat: u64,
    pub default_description: String,
    pub users: Arc<Mutex<HashMap<String, UserMetadata>>>,
    pub plugin_dir: PathBuf,
    pub base_url: Url,
    pub nostr_zapper_keys: Option<nostr_sdk::key::Keys>,
    pub payindex: u64,
    pub listen_address: SocketAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_email: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LnurlpConfig {
    pub callback: String,
    #[serde(rename = "maxSendable")]
    pub max_sendable: u64,
    #[serde(rename = "minSendable")]
    pub min_sendable: u64,
    pub metadata: String,
    pub tag: String,
    #[serde(rename = "commentAllowed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_allowed: Option<u64>,
    #[serde(rename = "allowsNostr")]
    pub allows_nostr: bool,
    #[serde(rename = "nostrPubkey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nostr_pubkey: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InvoiceQueryParams {
    pub amount: u64,
    pub nostr: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LnurlpCallback {
    pub pr: String,
    pub routes: Vec<String>,
}
