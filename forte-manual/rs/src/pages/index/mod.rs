use anyhow::Result;
use http::{HeaderMap, HeaderValue};
use serde::Serialize;

#[derive(Serialize)]
pub struct UserProfile {
    pub username: String,
    pub avatar_url: Option<String>,
    pub notifications: u32,
}

#[derive(Serialize)]
pub struct Banner {
    pub id: String,
    pub title: String,
    pub link: String,
    pub variant: BannerVariant,
}

#[derive(Serialize)]
pub enum BannerVariant {
    Primary,
    Secondary,
    Alert,
}

#[derive(Serialize)]
pub struct FeedItem {
    pub id: usize,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub timestamp: String,
}

#[derive(Serialize)]
pub enum Props {
    Ok {
        user: Option<UserProfile>,
        banners: Vec<Banner>,
        feed: Vec<FeedItem>,
        server_time: String,
    },
}

pub async fn handler(headers: HeaderMap<HeaderValue>) -> Result<Props> {
    // Mocking auth check based on headers
    let is_logged_in = headers.contains_key("authorization");

    let user = if is_logged_in {
        Some(UserProfile {
            username: "RustDeveloper".to_string(),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            notifications: 5,
        })
    } else {
        None
    };

    let banners = vec![
        Banner {
            id: "b1".to_string(),
            title: "New Version Released!".to_string(),
            link: "/news/v2".to_string(),
            variant: BannerVariant::Primary,
        },
        Banner {
            id: "b2".to_string(),
            title: "Scheduled Maintenance".to_string(),
            link: "/notice/maintenance".to_string(),
            variant: BannerVariant::Alert,
        },
    ];

    let feed = (1..=5)
        .map(|i| FeedItem {
            id: i,
            title: format!("Recommended Post #{}", i),
            category: if i % 2 == 0 {
                "Tech".into()
            } else {
                "Life".into()
            },
            tags: vec!["rust".into(), "wasm".into()],
            timestamp: "2024-01-01T12:00:00Z".to_string(),
        })
        .collect();

    let props = Props::Ok {
        user,
        banners,
        feed,
        server_time: "2025-05-20T15:30:00Z".to_string(),
    };

    Ok(props)
}
