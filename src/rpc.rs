use std::{collections::HashMap, path::Path};

use anyhow::anyhow;
use cln_plugin::Plugin;
use serde_json::json;
use tokio::fs;

use crate::{structs::UserMetadata, PluginState, CLNADDRESS_USERS_FILENAME};

pub async fn user_add(
    plugin: Plugin<PluginState>,
    args: serde_json::Value,
) -> Result<serde_json::Value, anyhow::Error> {
    let result;
    let user;
    let metadata;
    let users_clone;
    {
        let mut users = plugin.state().users.lock();
        (user, metadata) = match args {
            serde_json::Value::String(s) => (
                s,
                UserMetadata {
                    is_email: None,
                    description: None,
                },
            ),
            serde_json::Value::Array(values) => {
                let is_email_val = values.get(1);
                log::debug!("{:?}", is_email_val);
                let is_email = if let Some(val) = is_email_val {
                    match val {
                        serde_json::Value::Bool(b) => Some(*b),
                        serde_json::Value::String(s) => Some(s.parse()?),
                        _ => return Err(anyhow!("`is_email` has invalid type")),
                    }
                } else {
                    None
                };
                let description_val = values.get(2);
                let description = if let Some(desc) = description_val {
                    Some(
                        desc.as_str()
                            .ok_or_else(|| anyhow!("`description` is not a string"))?
                            .to_owned(),
                    )
                } else {
                    None
                };

                (
                    values
                        .first()
                        .ok_or_else(|| anyhow!("Empty array input"))?
                        .as_str()
                        .ok_or_else(|| anyhow!("Array elemnt not a string"))?
                        .to_owned(),
                    UserMetadata {
                        is_email,
                        description,
                    },
                )
            }
            serde_json::Value::Object(map) => {
                let is_email_val = map.get("is_email");
                let is_email = if let Some(val) = is_email_val {
                    match val {
                        serde_json::Value::Bool(b) => Some(*b),
                        serde_json::Value::String(s) => Some(s.parse()?),
                        _ => return Err(anyhow!("`is_email` has invalid type")),
                    }
                } else {
                    None
                };
                let description_val = map.get("description");
                let description = if let Some(desc) = description_val {
                    Some(
                        desc.as_str()
                            .ok_or_else(|| anyhow!("`description` is not a string"))?
                            .to_owned(),
                    )
                } else {
                    None
                };
                (
                    map.get("user")
                        .ok_or_else(|| anyhow!("`user` element not found in object"))?
                        .as_str()
                        .ok_or_else(|| anyhow!("Array elemnt not a string"))?
                        .to_owned(),
                    UserMetadata {
                        is_email,
                        description,
                    },
                )
            }
            _ => return Err(anyhow!("Not a valid input type")),
        };
        result = users.insert(user.clone(), metadata.clone());
        users_clone = users.clone();
    }
    save_users(&plugin.state().plugin_dir, users_clone).await?;
    if let Some(_res) = result {
        Ok(
            json!({"result":{"mode":"updated","user":user,"is_email":metadata.is_email,
            "description":metadata.description}}),
        )
    } else {
        Ok(
            json!({"result":{"mode":"added","user":user,"is_email":metadata.is_email,
            "description":metadata.description}}),
        )
    }
}
pub async fn user_del(
    plugin: Plugin<PluginState>,
    args: serde_json::Value,
) -> Result<serde_json::Value, anyhow::Error> {
    let result;
    let user;
    let users_clone;
    {
        let mut users = plugin.state().users.lock();
        user = match args {
            serde_json::Value::String(s) => s,
            serde_json::Value::Array(values) => values
                .first()
                .ok_or_else(|| anyhow!("Empty array input"))?
                .as_str()
                .ok_or_else(|| anyhow!("Array elemnt not a string"))?
                .to_owned(),
            serde_json::Value::Object(map) => map
                .get("user")
                .ok_or_else(|| anyhow!("`user` element not found in object"))?
                .as_str()
                .ok_or_else(|| anyhow!("Array elemnt not a string"))?
                .to_owned(),
            _ => return Err(anyhow!("Not a valid input type")),
        };
        result = users.remove(&user);
        users_clone = users.clone();
    }
    if let Some(res) = result {
        save_users(&plugin.state().plugin_dir, users_clone).await?;
        Ok(json!({"result":{"user":user,"metadata":res}}))
    } else {
        Err(anyhow!("User not found"))
    }
}

pub async fn save_users(
    path: &Path,
    users: HashMap<String, UserMetadata>,
) -> Result<(), anyhow::Error> {
    let serialized = serde_json::to_string(&users)?;
    fs::write(path.join(CLNADDRESS_USERS_FILENAME), serialized).await?;
    Ok(())
}
