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
pub fn build_cdn_url(bucket: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{}{}", DEFAULT_SITE_BASE, path)
    } else {
        format!("{}/{}/{}", DEFAULT_FILES_BASE, bucket, path)
    }
}

/// Constructs a URL for an avatar image.
pub fn avatar_url(path: &str) -> String {
    build_cdn_url("avatars", path)
}

/// Constructs a URL for a banner image.
pub fn banner_url(path: &str) -> String {
    build_cdn_url("banners", path)
}

/// Constructs a URL for a server icon image.
pub fn icon_url(path: &str) -> String {
    build_cdn_url("icons", path)
}

/// Constructs a URL for a message attachment.
pub fn attachment_url(path: &str) -> String {
    build_cdn_url("attachments", path)
}

/// Preserve explicit JSON null separately from an omitted PATCH field.
pub fn deserialize_patch<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
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
