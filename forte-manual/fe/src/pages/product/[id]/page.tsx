import { Props } from "./.props";

export default function (props: Props) {
  if (props.t === "NotFound") {
    return (
      <div>
        <h1>Product Not Found</h1>
        <p>{props.v.message}</p>
      </div>
    );
  }

  const { product, reviews, relatedIds } = props.v;
  const { stock } = product;

  return (
    <div>
      <h1>Product Details</h1>

      <section>
        <h2>Product Information</h2>
        <p>
          <strong>ID:</strong> {product.id}
        </p>
        <p>
          <strong>Name:</strong> {product.name}
        </p>
        <p>
          <strong>Description:</strong> {product.description}
        </p>
        <p>
          <strong>Price:</strong> {product.priceFormatted}
        </p>

        <div>
          <strong>Features:</strong>
          {product.features.length > 0 ? (
            <ul>
              {product.features.map((feature, index) => (
                <li key={index}>{feature}</li>
              ))}
            </ul>
          ) : (
            <p>No features available</p>
          )}
        </div>

        <div>
          <strong>Stock Status:</strong>
          {stock.t === "InStock" ? (
            <p>In Stock: {stock.v} units</p>
          ) : stock.t === "OutOfStock" ? (
            <p>Out of Stock</p>
          ) : (
            <p>Pre-Order - Release Date: {stock.v.releaseDate}</p>
          )}
        </div>

        <div>
          <strong>Images:</strong>
          {product.images.length > 0 ? (
            <ul>
              {product.images.map((image, index) => (
                <li key={index}>{image}</li>
              ))}
            </ul>
          ) : (
            <p>No images available</p>
          )}
        </div>
      </section>

      <section>
        <h2>Reviews</h2>
        {reviews.length > 0 ? (
          <ul>
            {reviews.map((review, index) => (
              <li key={index}>
                <p>
                  <strong>Author:</strong> {review.author}
                </p>
                <p>
                  <strong>Rating:</strong> {review.rating}
                </p>
                <p>
                  <strong>Comment:</strong> {review.comment}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p>No reviews yet</p>
        )}
      </section>

      <section>
        <h2>Related Products</h2>
        {relatedIds.length > 0 ? (
          <ul>
            {relatedIds.map((id) => (
              <li key={id}>Product ID: {id}</li>
            ))}
          </ul>
        ) : (
          <p>No related products</p>
        )}
      </section>
    </div>
  );
}
