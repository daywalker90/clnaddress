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
                    match desc {
                        serde_json::Value::Number(number) => Some(number.to_string()),
                        serde_json::Value::String(s) => Some(s.to_owned()),
                        _ => return Err(anyhow!("`description` has invalid type")),
                    }
                } else {
                    None
                };

                let user_val = values.first().ok_or_else(|| anyhow!("Empty array input"))?;
                let user_string = match user_val {
                    serde_json::Value::Number(number) => number.to_string(),
                    serde_json::Value::String(s) => s.to_owned(),
                    _ => return Err(anyhow!("Array user element has invalid type")),
                };

                (
                    user_string,
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
                    match desc {
                        serde_json::Value::Number(number) => Some(number.to_string()),
                        serde_json::Value::String(s) => Some(s.to_owned()),
                        _ => return Err(anyhow!("`description` has invalid type")),
                    }
                } else {
                    None
                };

                let user_val = map
                    .get("user")
                    .ok_or_else(|| anyhow!("`user` field not found in object"))?;
                let user_string = match user_val {
                    serde_json::Value::Number(number) => number.to_string(),
                    serde_json::Value::String(s) => s.to_owned(),
                    _ => return Err(anyhow!("user field has invalid type")),
                };

                (
                    user_string,
                    UserMetadata {
                        is_email,
                        description,
                    },
                )
            }
            serde_json::Value::Number(n) => (
                n.to_string(),
                UserMetadata {
                    is_email: None,
                    description: None,
                },
            ),
            _ => return Err(anyhow!("Not a valid input type")),
        };
        result = users.insert(user.clone(), metadata.clone());
        users_clone = users.clone();
    }
    save_users(&plugin.state().plugin_dir, users_clone).await?;
    let mut mode = if let Some(_res) = result {
        json!({"mode":"updated"})
    } else {
        json!({"mode":"added"})
    };

    mode.as_object_mut()
        .unwrap()
        .extend(json!({"user":user}).as_object().unwrap().clone());
    mode.as_object_mut()
        .unwrap()
        .extend(json!(metadata).as_object().unwrap().clone());

    Ok(mode)
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
            serde_json::Value::Number(n) => n.to_string(),
            _ => return Err(anyhow!("Not a valid input type")),
        };
        result = users.remove(&user);
        users_clone = users.clone();
    }
    if let Some(res) = result {
        save_users(&plugin.state().plugin_dir, users_clone).await?;
        let mut mode = json!({"mode":"deleted"});

        mode.as_object_mut()
            .unwrap()
            .extend(json!({"user":user}).as_object().unwrap().clone());
        mode.as_object_mut()
            .unwrap()
            .extend(json!(res).as_object().unwrap().clone());

        Ok(mode)
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
