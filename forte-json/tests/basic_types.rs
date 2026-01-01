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

#[tokio::test]
async fn test_null() {
    let value: Option<i32> = None;
    assert_eq!(stream_to_string(&value).await, "null");
}

#[tokio::test]
async fn test_bool() {
    assert_eq!(stream_to_string(&true).await, "true");
    assert_eq!(stream_to_string(&false).await, "false");
}

#[tokio::test]
async fn test_integers() {
    assert_eq!(stream_to_string(&0).await, "0");
    assert_eq!(stream_to_string(&42).await, "42");
    assert_eq!(stream_to_string(&-100).await, "-100");
    assert_eq!(stream_to_string(&i64::MAX).await, "9223372036854775807");
    assert_eq!(stream_to_string(&u64::MAX).await, "18446744073709551615");
}

#[tokio::test]
async fn test_floats() {
    assert_eq!(stream_to_string(&0.0_f64).await, "0");
    assert_eq!(stream_to_string(&1.5_f64).await, "1.5");
    assert_eq!(stream_to_string(&-2.5_f64).await, "-2.5");
}

#[tokio::test]
async fn test_string() {
    assert_eq!(stream_to_string(&"hello").await, r#""hello""#);
    assert_eq!(stream_to_string(&"").await, r#""""#);
}

#[tokio::test]
async fn test_string_escaping() {
    assert_eq!(stream_to_string(&"hello\\world").await, r#""hello\\world""#);
    assert_eq!(stream_to_string(&"quote\"test").await, r#""quote\"test""#);
}

#[tokio::test]
async fn test_array() {
    let arr: Vec<i32> = vec![1, 2, 3];
    assert_eq!(stream_to_string(&arr).await, "[1,2,3]");
}

#[tokio::test]
async fn test_empty_array() {
    let arr: Vec<i32> = vec![];
    assert_eq!(stream_to_string(&arr).await, "[]");
}

#[derive(Serialize)]
struct Point {
    x: i32,
    y: i32,
}

#[tokio::test]
async fn test_struct() {
    let point = Point { x: 10, y: 20 };
    assert_eq!(stream_to_string(&point).await, r#"{"x":10,"y":20}"#);
}

#[derive(Serialize)]
struct ComplexStruct {
    name: String,
    age: u32,
    active: bool,
}

#[tokio::test]
async fn test_complex_struct() {
    let value = ComplexStruct {
        name: "Alice".to_string(),
        age: 30,
        active: true,
    };
    let json = stream_to_string(&value).await;
    assert_eq!(json, r#"{"name":"Alice","age":30,"active":true}"#);
}

#[tokio::test]
async fn test_nested_struct() {
    let point = Point { x: 5, y: 15 };
    let json = stream_to_string(&point).await;
    assert!(json.contains("x"));
    assert!(json.contains("10") || json.contains("5"));
}

#[tokio::test]
async fn test_option_some() {
    let value: Option<i32> = Some(42);
    assert_eq!(stream_to_string(&value).await, "42");
}

#[tokio::test]
async fn test_option_none() {
    let value: Option<i32> = None;
    assert_eq!(stream_to_string(&value).await, "null");
}

#[derive(Serialize)]
struct OptionalFields {
    required: i32,
    optional: Option<String>,
}

#[tokio::test]
async fn test_struct_with_option() {
    let v1 = OptionalFields {
        required: 10,
        optional: Some("value".to_string()),
    };
    let json = stream_to_string(&v1).await;
    assert_eq!(json, r#"{"required":10,"optional":"value"}"#);

    let v2 = OptionalFields {
        required: 20,
        optional: None,
    };
    let json = stream_to_string(&v2).await;
    assert_eq!(json, r#"{"required":20,"optional":null}"#);
}
