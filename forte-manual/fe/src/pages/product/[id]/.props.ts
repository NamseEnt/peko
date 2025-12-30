// Auto-generated from src/pages/product/[id]/mod.rs

export type Props = { Ok: { product: { id: number, name: string, description: string, priceFormatted: string, features: string[], stock: { InStock: number } | "OutOfStock" | { PreOrder: { releaseDate: string } }, images: string[] }, reviews: { author: string, rating: number, comment: string }[], relatedIds: number[] } } | { NotFound: { message: string } };

export interface route_generated::pages_product_1id1::ProductDetail {
    id: number;
    name: string;
    description: string;
    priceFormatted: string;
    features: string[];
    stock: { InStock: number } | "OutOfStock" | { PreOrder: { releaseDate: string } };
    images: string[];
}

export interface route_generated::pages_product_1id1::utils::Review {
    author: string;
    rating: number;
    comment: string;
}
