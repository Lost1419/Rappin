use rappin::utils::*;

#[test]
fn test_all_cdn_buckets() {
    assert_eq!(
        avatar_url("user123.jpg"),
        "https://files.yapper.website/avatars/user123.jpg"
    );
    assert_eq!(
        banner_url("banner456.png"),
        "https://files.yapper.website/banners/banner456.png"
    );
    assert_eq!(
        icon_url("server789.webp"),
        "https://files.yapper.website/icons/server789.webp"
    );
    assert_eq!(
        attachment_url("file_abc.pdf"),
        "https://files.yapper.website/attachments/file_abc.pdf"
    );
}

#[test]
fn test_root_path_on_main_domain() {
    assert_eq!(avatar_url("/icon-white.svg"), "https://www.yapin.gg/icon-white.svg");
    assert_eq!(icon_url("/default-icon.png"), "https://www.yapin.gg/default-icon.png");
}

