import { Props } from "./.props";

export default function (props: Props) {
  const { user, banners, feed, serverTime } = props.v;

  return (
    <div>
      <h1>Index Page</h1>

      <section>
        <h2>User</h2>
        {user ? (
          <div>
            <p>
              <strong>Username:</strong> {user.username}
            </p>
            {user.avatarUrl && (
              <p>
                <strong>Avatar URL:</strong> {user.avatarUrl}
              </p>
            )}
            <p>
              <strong>Notifications:</strong> {user.notifications}
            </p>
          </div>
        ) : (
          <p>No user logged in</p>
        )}
      </section>

      <section>
        <h2>Banners</h2>
        {banners.length > 0 ? (
          <ul>
            {banners.map((banner) => (
              <li key={banner.id}>
                <p>
                  <strong>ID:</strong> {banner.id}
                </p>
                <p>
                  <strong>Title:</strong> {banner.title}
                </p>
                <p>
                  <strong>Link:</strong> {banner.link}
                </p>
                <p>
                  <strong>Variant:</strong> {banner.variant.t}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p>No banners</p>
        )}
      </section>

      <section>
        <h2>Feed</h2>
        {feed.length > 0 ? (
          <ul>
            {feed.map((item) => (
              <li key={item.id}>
                <p>
                  <strong>ID:</strong> {item.id}
                </p>
                <p>
                  <strong>Title:</strong> {item.title}
                </p>
                <p>
                  <strong>Category:</strong> {item.category}
                </p>
                <p>
                  <strong>Tags:</strong> {item.tags.join(", ")}
                </p>
                <p>
                  <strong>Timestamp:</strong> {item.timestamp}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p>No feed items</p>
        )}
      </section>

      <section>
        <h2>Server Time</h2>
        <p>{serverTime}</p>
      </section>
    </div>
  );
}
