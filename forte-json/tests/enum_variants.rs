use forte_json::to_stream;
use futures::StreamExt;
use serde::Serialize;

async fn stream_to_string<T: Serialize + ?Sized>(value: &T) -> String {
    let stream = to_stream(value);
    let chunks: Vec<_> = stream.collect().await;
    chunks
        .iter()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Serialize)]
enum SimpleEnum {
    Unit,
    Newtype(i32),
    Tuple(i32, String),
    Struct { x: i32, y: String },
}

#[tokio::test]
async fn test_unit_variant() {
    let value = SimpleEnum::Unit;
    let json = stream_to_string(&value).await;
    assert_eq!(json, r#"{"t":"Unit","v":null}"#);
}

#[tokio::test]
async fn test_newtype_variant() {
    let value = SimpleEnum::Newtype(42);
    let json = stream_to_string(&value).await;
    assert_eq!(json, r#"{"t":"Newtype","v":42}"#);
}

#[tokio::test]
async fn test_tuple_variant() {
    let value = SimpleEnum::Tuple(1, "hello".to_string());
    let json = stream_to_string(&value).await;
    assert_eq!(json, r#"{"t":"Tuple","v":[1,"hello"]}"#);
}

#[tokio::test]
async fn test_struct_variant() {
    let value = SimpleEnum::Struct {
        x: 10,
        y: "world".to_string(),
    };
    let json = stream_to_string(&value).await;
    assert_eq!(json, r#"{"t":"Struct","v":{"x":10,"y":"world"}}"#);
}

#[tokio::test]
async fn test_nested_enum() {
    #[derive(Serialize)]
    struct Container {
        value: SimpleEnum,
    }

    let container = Container {
        value: SimpleEnum::Newtype(42),
    };
    let json = stream_to_string(&container).await;
    assert_eq!(json, r#"{"value":{"t":"Newtype","v":42}}"#);
}

#[tokio::test]
async fn test_vec_of_enum() {
    let values = vec![SimpleEnum::Unit, SimpleEnum::Newtype(1)];
    let json = stream_to_string(&values).await;
    assert_eq!(json, r#"[{"t":"Unit","v":null},{"t":"Newtype","v":1}]"#);
}
