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
#[allow(dead_code)]
enum SimpleEnum {
    Unit,
    Newtype(i32),
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
enum TaggedEnum {
    A(i32),
}

#[tokio::test]
async fn compare_basic_enum() {
    let value = SimpleEnum::Newtype(42);
    let forte_json_output = stream_to_string(&value).await;
    let serde_json_output = serde_json::to_string(&value).unwrap();

    println!("forte_json:   {}", forte_json_output);
    println!("serde_json:   {}", serde_json_output);
}

#[tokio::test]
async fn compare_tagged_enum() {
    let value = TaggedEnum::A(99);
    let forte_json_output = stream_to_string(&value).await;
    let serde_json_output = serde_json::to_string(&value).unwrap();

    println!("forte_json:   {}", forte_json_output);
    println!("serde_json:   {}", serde_json_output);
}
