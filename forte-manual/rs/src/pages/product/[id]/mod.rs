mod utils;

use anyhow::Result;
use http::{HeaderMap, HeaderValue};
use serde::Serialize;
use utils::{Review, get_dummy_reviews};

#[derive(Serialize)]
pub enum StockStatus {
    InStock(u32),
    OutOfStock,
    PreOrder { release_date: String },
}

#[derive(Serialize)]
pub struct ProductDetail {
    pub id: usize,
    pub name: String,
    pub description: String,
    pub price_formatted: String,
    pub features: Vec<String>,
    pub stock: StockStatus,
    pub images: Vec<String>,
}

#[derive(Serialize)]
pub enum Props {
    Ok {
        product: ProductDetail,
        reviews: Vec<Review>,
        related_ids: Vec<usize>,
    },
    NotFound {
        message: String,
    },
}

pub async fn handler(_headers: HeaderMap<HeaderValue>, id: usize) -> Result<Props> {
    if id == 999 {
        return Ok(Props::NotFound {
            message: "Product not found.".to_string(),
        });
    }

    let product = ProductDetail {
        id,
        name: format!("Super Widget Model X-{}", id),
        description: "This product features state-of-the-art technology.".to_string(),
        price_formatted: utils::format_price(100.0 + (id as f64 * 10.5)),
        features: vec!["Fast".into(), "Secure".into(), "Eco-friendly".into()],
        stock: if id.is_multiple_of(2) {
            StockStatus::InStock(50)
        } else {
            StockStatus::PreOrder {
                release_date: "2025-12-25".to_string(),
            }
        },
        images: vec![
            format!("/img/p{}_front.jpg", id),
            format!("/img/p{}_side.jpg", id),
        ],
    };

    let reviews = get_dummy_reviews(id);
    let related_ids = vec![id + 1, id + 2, id + 3];

    Ok(Props::Ok {
        product,
        reviews,
        related_ids,
    })
}
