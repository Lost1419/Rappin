//! Default service origins, asset URL construction and nullable patch decoding.
/// Default base URL for the Yapin' Bot API.
pub const DEFAULT_API_BASE: &str = "https://bots.yapper.website/api/bot/v1";

/// Website API origin used when the dedicated bot API is unavailable.
pub const LEGACY_API_BASE: &str = "https://www.yapin.gg/api/bot/v1";

/// Base URL for user uploaded assets.
pub const DEFAULT_FILES_BASE: &str = "https://files.yapper.website";

/// Base website URL.
pub const DEFAULT_SITE_BASE: &str = "https://www.yapin.gg";

/// Flag specifying that components are a Components V2 layout.
pub const IS_COMPONENTS_V2: u32 = 1 << 15; // 32768

/// Constructs a full URL for a picture or media asset from a bucket and path.
///
/// If `path` starts with `'/'`, it points to `https://www.yapin.gg`.
/// Otherwise, it resolves to `https://files.yapper.website/<bucket>/<path>`.
/// # Example
///
/// ```rust
/// use rappin::utils::build_cdn_url;
/// assert_eq!(build_cdn_url("icons", "server.png"), "https://files.yapper.website/icons/server.png");
/// ```
pub fn build_cdn_url(bucket: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{}{}", DEFAULT_SITE_BASE, path)
    } else {
        format!("{}/{}/{}", DEFAULT_FILES_BASE, bucket, path)
    }
}

/// Constructs a URL for an avatar image.
/// # Example
///
/// ```rust
/// use rappin::utils::avatar_url;
/// assert_eq!(avatar_url("asset.png"), "https://files.yapper.website/avatars/asset.png");
/// ```
pub fn avatar_url(path: &str) -> String {
    build_cdn_url("avatars", path)
}

/// Constructs a URL for a banner image.
/// # Example
///
/// ```rust
/// use rappin::utils::banner_url;
/// assert_eq!(banner_url("asset.png"), "https://files.yapper.website/banners/asset.png");
/// ```
pub fn banner_url(path: &str) -> String {
    build_cdn_url("banners", path)
}

/// Constructs a URL for a server icon image.
/// # Example
///
/// ```rust
/// use rappin::utils::icon_url;
/// assert_eq!(icon_url("asset.png"), "https://files.yapper.website/icons/asset.png");
/// ```
pub fn icon_url(path: &str) -> String {
    build_cdn_url("icons", path)
}

/// Constructs a URL for a message attachment.
/// # Example
///
/// ```rust
/// use rappin::utils::attachment_url;
/// assert_eq!(attachment_url("asset.png"), "https://files.yapper.website/attachments/asset.png");
/// ```
pub fn attachment_url(path: &str) -> String {
    build_cdn_url("attachments", path)
}

/// Preserve explicit JSON null separately from an omitted PATCH field.
/// # Example
///
/// ```rust
/// use serde::Deserialize;
/// #[derive(Deserialize)]
/// struct Patch {
///     #[serde(default, deserialize_with = "rappin::utils::deserialize_patch")]
///     category_id: Option<Option<uuid::Uuid>>,
/// }
/// let omitted: Patch = serde_json::from_str("{}")?;
/// let cleared: Patch = serde_json::from_str(r#"{"category_id":null}"#)?;
/// assert_eq!(omitted.category_id, None);
/// assert_eq!(cleared.category_id, Some(None));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn deserialize_patch<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

/// Remove server-generated image proxy hints only from outbound typed payloads.
pub(crate) fn strip_proxy_metadata(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.remove("proxy_url");
            fields.remove("proxy_icon_url");
            for value in fields.values_mut() {
                strip_proxy_metadata(value);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strip_proxy_metadata(item);
            }
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cdn_urls() {
        assert_eq!(
            avatar_url("abc-123.png"),
            "https://files.yapper.website/avatars/abc-123.png"
        );
        assert_eq!(
            avatar_url("/icon-white.svg"),
            "https://www.yapin.gg/icon-white.svg"
        );
        assert_eq!(
            icon_url("server.png"),
            "https://files.yapper.website/icons/server.png"
        );
    }
}
