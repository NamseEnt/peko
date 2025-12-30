use serde::Serialize;

#[derive(Serialize)]
pub struct Review {
    pub author: String,
    pub rating: u8,
    pub comment: String,
}

pub fn get_dummy_reviews(product_id: usize) -> Vec<Review> {
    vec![
        Review {
            author: "User A".to_string(),
            rating: 5,
            comment: format!("Product {} is amazing!", product_id),
        },
        Review {
            author: "User B".to_string(),
            rating: 4,
            comment: "Good value for money.".to_string(),
        },
    ]
}

pub fn format_price(amount: f64) -> String {
    format!("$ {:.2}", amount)
}
